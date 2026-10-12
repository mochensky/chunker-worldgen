//! Generation jobs: shape -> regions -> parallel chunk generation -> atomic region writes.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::biome::BiomeResolver;
use anyhow::{Context, Result};
use chunker_anvil::{Compression, RawChunk, Region, RegionPos, local_index, now_timestamp};
use parking_lot::Mutex;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::terrain::profile;

use crate::world::Dimension;

pub mod estimate;
mod pipeline;
use crate::preview::{PreviewStore, render_chunk};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Square,
    Circle,
    /// `radius` along x, `radius_z` along z.
    Rectangle,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    pub dimension: String,
    /// Center in block coordinates.
    pub center_x: i32,
    pub center_z: i32,
    /// Radius in blocks (along x for a rectangle).
    pub radius: i32,
    /// Radius along z of a rectangle (`None`: the same as `radius`); ignored by other shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_z: Option<i32>,
    pub shape: Shape,
    /// Skip chunks that already exist in the region files (never overwrite).
    #[serde(default = "default_true")]
    pub skip_existing: bool,
    /// Worker threads (0 = all logical CPUs).
    #[serde(default)]
    pub threads: usize,
    /// Run the `features` step (features and structure pieces) and save chunks at `features`;
    /// otherwise chunks are saved at `terrain` and the server decorates them.
    #[serde(default = "default_true")]
    pub decorate: bool,
}

fn default_true() -> bool {
    true
}

impl JobSpec {
    /// Half the size of the shape along x and z, in blocks.
    pub fn half_extents(&self) -> (i32, i32) {
        match self.shape {
            Shape::Rectangle => (self.radius, self.radius_z.unwrap_or(self.radius)),
            Shape::Square | Shape::Circle => (self.radius, self.radius),
        }
    }

    /// Chunks covered by the shape, like Chunky: chunk centers inside the shape.
    pub fn contains_chunk(&self, cx: i32, cz: i32) -> bool {
        let dx = ((cx << 4) + 8 - self.center_x) as i64;
        let dz = ((cz << 4) + 8 - self.center_z) as i64;
        let (rx, rz) = self.half_extents();
        let (rx, rz) = (rx as i64, rz as i64);
        match self.shape {
            Shape::Square | Shape::Rectangle => dx.abs() <= rx && dz.abs() <= rz,
            Shape::Circle => dx * dx + dz * dz <= rx * rx,
        }
    }

    pub fn chunk_bounds(&self) -> (i32, i32, i32, i32) {
        let (rx, rz) = self.half_extents();
        (
            (self.center_x - rx) >> 4,
            (self.center_z - rz) >> 4,
            (self.center_x + rx) >> 4,
            (self.center_z + rz) >> 4,
        )
    }

    /// Regions intersecting the shape, nearest to the center first (spiral-like order).
    pub fn regions(&self) -> Vec<RegionPos> {
        let (x0, z0, x1, z1) = self.chunk_bounds();
        let mut out = Vec::new();
        for rz in (z0 >> 5)..=(z1 >> 5) {
            for rx in (x0 >> 5)..=(x1 >> 5) {
                let pos = RegionPos::new(rx, rz);
                if self.region_chunks(pos).next().is_some() {
                    out.push(pos);
                }
            }
        }
        let (ccx, ccz) = (self.center_x >> 9, self.center_z >> 9);
        out.sort_by_key(|p| {
            let (dx, dz) = ((p.x - ccx) as i64, (p.z - ccz) as i64);
            (dx * dx + dz * dz, dx.abs().max(dz.abs()), p.x, p.z)
        });
        out
    }

    pub fn region_chunks(&self, pos: RegionPos) -> impl Iterator<Item = (i32, i32)> + '_ {
        let (x0, z0, x1, z1) = self.chunk_bounds();
        let bx = pos.min_chunk_x();
        let bz = pos.min_chunk_z();
        (0..32)
            .flat_map(move |lz| (0..32).map(move |lx| (bx + lx, bz + lz)))
            .filter(move |&(cx, cz)| {
                cx >= x0 && cx <= x1 && cz >= z0 && cz <= z1 && self.contains_chunk(cx, cz)
            })
    }

    pub fn total_chunks(&self) -> u64 {
        let (x0, z0, x1, z1) = self.chunk_bounds();
        let mut n = 0u64;
        for cz in z0..=z1 {
            for cx in x0..=x1 {
                if self.contains_chunk(cx, cz) {
                    n += 1;
                }
            }
        }
        n
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Running,
    Paused,
    Cancelling,
    Cancelled,
    Finished,
    Failed,
}

impl JobState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => JobState::Running,
            1 => JobState::Paused,
            2 => JobState::Cancelling,
            3 => JobState::Cancelled,
            4 => JobState::Finished,
            _ => JobState::Failed,
        }
    }
}

/// Persistent job progress (`<world>/.chunker/job.json`), enables resuming.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobFile {
    pub spec: JobSpec,
    pub completed_regions: Vec<(i32, i32)>,
    pub generated_chunks: u64,
    pub skipped_chunks: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobSnapshot {
    pub state: JobState,
    pub spec: JobSpec,
    pub total_chunks: u64,
    pub generated_chunks: u64,
    pub skipped_chunks: u64,
    pub total_regions: u64,
    pub completed_regions: u64,
    /// Regions being generated right now (several overlap so that no worker idles at region ends).
    pub current_regions: Vec<(i32, i32)>,
    /// Speed over the last 15 seconds while running, the average over the whole job otherwise.
    pub chunks_per_second: f64,
    pub elapsed_seconds: f64,
    pub eta_seconds: Option<f64>,
    pub error: Option<String>,
}

pub struct Job {
    pub spec: JobSpec,
    state: AtomicU8,
    total_chunks: u64,
    total_regions: u64,
    generated: AtomicU64,
    skipped: AtomicU64,
    completed_regions: Mutex<BTreeSet<(i32, i32)>>,
    current_regions: Mutex<Vec<(i32, i32)>>,
    error: Mutex<Option<String>>,
    started: Instant,
    /// When the job finished, failed or was cancelled (the elapsed time stops there).
    ended: Mutex<Option<Instant>>,
    paused_time: Mutex<(Duration, Option<Instant>)>,
    rate_samples: Mutex<Vec<(Instant, u64)>>,
    job_file: PathBuf,
    /// Keep the job file when cancelled (the world is being closed; the task can be resumed later).
    keep_file: std::sync::atomic::AtomicBool,
}

/// Regions generated concurrently by one job.
const REGIONS_IN_FLIGHT: usize = 3;

impl Job {
    pub fn new(spec: JobSpec, resume: Option<JobFile>, job_file: PathBuf) -> Arc<Job> {
        let total_chunks = spec.total_chunks();
        let total_regions = spec.regions().len() as u64;
        let (completed, generated, skipped) = match resume {
            Some(f) => (
                f.completed_regions.into_iter().collect(),
                f.generated_chunks,
                f.skipped_chunks,
            ),
            None => (BTreeSet::new(), 0, 0),
        };
        Arc::new(Job {
            spec,
            state: AtomicU8::new(0),
            total_chunks,
            total_regions,
            generated: AtomicU64::new(generated),
            skipped: AtomicU64::new(skipped),
            completed_regions: Mutex::new(completed),
            current_regions: Mutex::new(Vec::new()),
            error: Mutex::new(None),
            started: Instant::now(),
            ended: Mutex::new(None),
            paused_time: Mutex::new((Duration::ZERO, None)),
            rate_samples: Mutex::new(Vec::new()),
            job_file,
            keep_file: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn state(&self) -> JobState {
        JobState::from_u8(self.state.load(Ordering::SeqCst))
    }

    fn set_state(&self, s: JobState) {
        if matches!(s, JobState::Finished | JobState::Cancelled | JobState::Failed) {
            self.ended.lock().get_or_insert_with(Instant::now);
        }
        self.state.store(s as u8, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        if self.state() == JobState::Running {
            self.set_state(JobState::Paused);
            self.paused_time.lock().1 = Some(Instant::now());
        }
    }

    pub fn resume(&self) {
        if self.state() == JobState::Paused {
            let mut p = self.paused_time.lock();
            if let Some(t) = p.1.take() {
                p.0 += t.elapsed();
            }
            self.set_state(JobState::Running);
        }
    }

    pub fn cancel(&self) {
        let s = self.state();
        if matches!(s, JobState::Running | JobState::Paused) {
            self.set_state(JobState::Cancelling);
        }
    }

    /// Stops the task because its world is being closed; it stays resumable from the job file.
    pub fn cancel_for_switch(&self) {
        self.keep_file.store(true, Ordering::SeqCst);
        let s = self.state();
        if matches!(s, JobState::Running | JobState::Paused) {
            self.set_state(JobState::Cancelling);
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.state(),
            JobState::Running | JobState::Paused | JobState::Cancelling
        )
    }

    /// Blocks while paused; returns false if the job should stop.
    fn wait_if_paused(&self) -> bool {
        loop {
            match self.state() {
                JobState::Running => return true,
                JobState::Paused => std::thread::sleep(Duration::from_millis(100)),
                _ => return false,
            }
        }
    }

    /// Time spent running: from the start to the end (or now), without the pauses.
    fn active_elapsed(&self) -> f64 {
        let end = self.ended.lock().unwrap_or_else(Instant::now);
        let p = self.paused_time.lock();
        let paused = p.0 + p.1.map(|t| end.saturating_duration_since(t)).unwrap_or_default();
        (end.saturating_duration_since(self.started).saturating_sub(paused)).as_secs_f64()
    }

    pub fn snapshot(&self) -> JobSnapshot {
        let generated = self.generated.load(Ordering::Relaxed);
        let skipped = self.skipped.load(Ordering::Relaxed);
        let now = Instant::now();
        let rate = {
            let mut samples = self.rate_samples.lock();
            if samples
                .last()
                .is_none_or(|(t, _)| now.duration_since(*t) >= Duration::from_millis(500))
            {
                samples.push((now, generated));
            }
            samples.retain(|(t, _)| now.duration_since(*t) <= Duration::from_secs(15));
            match (samples.first(), samples.last()) {
                (Some((t0, g0)), Some((t1, g1))) if t1 > t0 && self.state() == JobState::Running => {
                    (g1 - g0) as f64 / t1.duration_since(*t0).as_secs_f64()
                }
                // Not running: the average over the whole job.
                _ if self.state() != JobState::Running && self.active_elapsed() > 0.0 => {
                    generated as f64 / self.active_elapsed()
                }
                _ => 0.0,
            }
        };
        let remaining = self.total_chunks.saturating_sub(generated + skipped);
        JobSnapshot {
            state: self.state(),
            spec: self.spec.clone(),
            total_chunks: self.total_chunks,
            generated_chunks: generated,
            skipped_chunks: skipped,
            total_regions: self.total_regions,
            completed_regions: self.completed_regions.lock().len() as u64,
            current_regions: self.current_regions.lock().clone(),
            chunks_per_second: rate,
            elapsed_seconds: self.active_elapsed(),
            eta_seconds: (rate > 0.0).then(|| remaining as f64 / rate),
            error: self.error.lock().clone(),
        }
    }

    pub fn completed_regions(&self) -> Vec<(i32, i32)> {
        self.completed_regions.lock().iter().copied().collect()
    }

    fn save_file(&self) {
        let file = JobFile {
            spec: self.spec.clone(),
            completed_regions: self.completed_regions(),
            generated_chunks: self.generated.load(Ordering::Relaxed),
            skipped_chunks: self.skipped.load(Ordering::Relaxed),
        };
        if let Some(parent) = self.job_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&file) {
            let tmp = self.job_file.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.job_file);
            }
        }
    }

    /// Runs the job to completion (blocking). Called on a dedicated thread.
    pub fn run(
        self: &Arc<Self>,
        dim: &Dimension,
        previews: &PreviewStore,
        data_version: i32,
        game_time: i64,
    ) {
        let threads = if self.spec.threads == 0 {
            std::thread::available_parallelism().map_or(4, |n| n.get())
        } else {
            self.spec.threads
        };
        let pool = match rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("chunker-gen-{i}"))
            .build()
        {
            Ok(p) => p,
            Err(e) => {
                *self.error.lock() = Some(e.to_string());
                self.set_state(JobState::Failed);
                return;
            }
        };
        let regions: Vec<RegionPos> = {
            let done = self.completed_regions.lock();
            self.spec
                .regions()
                .into_iter()
                .filter(|p| !done.contains(&(p.x, p.z)))
                .collect()
        };
        // A few regions are in flight at once: their chunk tasks share the pool, so the tail of one
        // region (and its single-threaded load/save) overlaps with the next instead of idling workers.
        let next = std::sync::atomic::AtomicUsize::new(0);
        // Structure starts are shared by all regions of the job (references reach 8 chunks out).
        let starts = dim.start_cache();
        let starts = &starts;
        if self.spec.decorate {
            match pool
                .install(|| self.run_decorated(dim, previews, starts, &regions, data_version, game_time))
            {
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("job failed: {e:#}");
                    *self.error.lock() = Some(format!("{e:#}"));
                    self.set_state(JobState::Failed);
                    self.save_file();
                    return;
                }
            }
        }
        let drivers = if self.spec.decorate {
            0
        } else {
            REGIONS_IN_FLIGHT.min(regions.len()).max(1)
        };
        std::thread::scope(|scope| {
            for _ in 0..drivers {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        let Some(&pos) = regions.get(i) else { break };
                        if !self.wait_if_paused() {
                            break;
                        }
                        self.current_regions.lock().push((pos.x, pos.z));
                        let result = pool.install(|| {
                            self.process_region(dim, previews, starts, pos, data_version, game_time)
                        });
                        self.current_regions.lock().retain(|&r| r != (pos.x, pos.z));
                        match result {
                            Ok(true) => {
                                self.completed_regions.lock().insert((pos.x, pos.z));
                                self.save_file();
                            }
                            Ok(false) => break,
                            Err(e) => {
                                tracing::error!("region {:?} failed: {e:#}", pos);
                                *self.error.lock() = Some(format!("region r.{}.{}: {e:#}", pos.x, pos.z));
                                self.set_state(JobState::Failed);
                                self.save_file();
                                break;
                            }
                        }
                    }
                });
            }
        });
        if self.state() == JobState::Failed {
            return;
        }
        self.save_file();
        let phases: Vec<String> = profile::snapshot()
            .iter()
            .map(|(p, s)| format!("{p:?} {s:.1}s"))
            .collect();
        tracing::info!("CPU time by phase: {}", phases.join(", "));
        match self.state() {
            JobState::Cancelling => {
                self.set_state(JobState::Cancelled);
                if !self.keep_file.load(Ordering::SeqCst) {
                    let _ = std::fs::remove_file(&self.job_file);
                }
            }
            JobState::Running | JobState::Paused => {
                self.set_state(JobState::Finished);
                let _ = std::fs::remove_file(&self.job_file);
            }
            _ => {}
        }
    }

    /// Generates the missing target chunks of one region. Returns `Ok(false)` when interrupted.
    fn process_region(
        &self,
        dim: &Dimension,
        previews: &PreviewStore,
        starts: &crate::structure::cache::StartCache,
        pos: RegionPos,
        data_version: i32,
        game_time: i64,
    ) -> Result<bool> {
        let path = dim.region_dir.join(pos.file_name());
        let presence = Region::read_presence(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut targets: Vec<(i32, i32)> = Vec::new();
        let mut skipped = 0u64;
        for (cx, cz) in self.spec.region_chunks(pos) {
            let exists = presence.is_some_and(|p| p[local_index(cx, cz)]);
            if exists && self.spec.skip_existing {
                skipped += 1;
            } else {
                targets.push((cx, cz));
            }
        }
        self.skipped.fetch_add(skipped, Ordering::Relaxed);
        if targets.is_empty() {
            return Ok(true);
        }

        // The preview starts from the chunks that already exist in this region.
        previews.begin_region(pos);

        // Biomes of all target chunks and their neighbours (the `BIOMES` step in radius 1).
        let mut needed: Vec<(i32, i32)> = Vec::with_capacity(targets.len() * 2);
        {
            let mut seen = rustc_hash::FxHashSet::default();
            for &(cx, cz) in &targets {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        if seen.insert((cx + dx, cz + dz)) {
                            needed.push((cx + dx, cz + dz));
                        }
                    }
                }
            }
        }
        let biomes: FxHashMap<(i32, i32), BiomeResolver> = needed
            .par_iter()
            .map(|&(cx, cz)| {
                let t = profile::Timer::start();
                let r = dim.resolve_biomes(cx, cz);
                t.lap(profile::Phase::Biomes);
                ((cx, cz), r)
            })
            .collect();

        let biome_names = dim.biomes.clone();
        let results: Vec<Option<(usize, RawChunk)>> = targets
            .par_iter()
            .map(|&(cx, cz)| {
                if !self.wait_if_paused() {
                    return None;
                }
                let mut chunk = dim.generate_terrain(cx, cz, |dx, dz| &biomes[&(cx + dx, cz + dz)], starts);
                let t = profile::Timer::start();
                let pixels = render_chunk(&mut chunk, dim.has_ceiling);
                previews.put_chunk(cx, cz, &pixels);
                let nbt = crate::chunk::serialize::write_chunk(&chunk, &biome_names, data_version, game_time);
                let raw = chunker_nbt::write_root(&nbt);
                let payload = Compression::Zlib.compress(&raw);
                t.lap(profile::Phase::Serialize);
                self.generated.fetch_add(1, Ordering::Relaxed);
                Some((
                    local_index(cx, cz),
                    RawChunk {
                        compression: Compression::Zlib.id(),
                        payload,
                        timestamp: now_timestamp(),
                    },
                ))
            })
            .collect();
        let interrupted = results.iter().any(Option::is_none);

        // Merge into the existing region file; chunks that appeared meanwhile are kept.
        let mut region = Region::load(&dim.region_dir, pos)?;
        let mut written = 0u64;
        for (index, raw) in results.into_iter().flatten() {
            if !region.has(index) || !self.spec.skip_existing {
                region.set(index, raw);
                written += 1;
            }
        }
        if written > 0 {
            region.save(&dim.region_dir)?;
        }
        previews.finish_region(pos);
        Ok(!interrupted)
    }
}
