//! The decorating pipeline: terrain, the `features` step (placed features and structure pieces) and
//! deferred saving.
//!
//! Decorating a chunk writes into its 3x3 neighbourhood (`WorldGenRegion` write radius 1), so a chunk
//! is only final once every chunk around it that this job decorates has been decorated. Chunks wait
//! in a job-wide store until then. Regions are processed one at a time, nearest first; inside a region
//! the target chunks are decorated in 9 phases by `(x mod 3, z mod 3)`, so the chunks of one phase
//! have disjoint neighbourhoods and run in parallel. Neighbours outside the job are generated up to
//! `terrain`, receive the spill-over and are saved at `terrain` (the server continues from there, as
//! after vanilla's own `features` step); existing chunks are never modified: their regenerated
//! terrain only stands in for reads.

use std::sync::atomic::Ordering;

use crate::biome::BiomeResolver;
use crate::chunk::{ChunkStatus, ProtoChunk};
use crate::feature::{Region as FeatureRegion, decorate};
use crate::structure::build::{decorate_step, writable_area};
use crate::structure::cache::StartCache;
use crate::terrain::profile;
use chunker_anvil::{Compression, RawChunk, Region, RegionPos, local_index, now_timestamp};
use parking_lot::Mutex;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use super::Job;
use crate::preview::{PreviewStore, render_chunk};
use crate::world::Dimension;

type Pos = (i32, i32);

/// A chunk waiting in memory.
struct Slot {
    chunk: Mutex<Box<ProtoChunk>>,
    /// False for existing chunks whose regenerated terrain only stands in for reads.
    writable: bool,
}

#[derive(Default)]
struct Store {
    slots: FxHashMap<Pos, std::sync::Arc<Slot>>,
    decorated: FxHashSet<Pos>,
    /// Chunks present before the job (or written by an earlier run), per region file.
    presence: FxHashMap<(i32, i32), Option<Box<[bool; 1024]>>>,
}

fn neighbours(p: Pos) -> impl Iterator<Item = Pos> {
    (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (p.0 + dx, p.1 + dz)))
}

impl Job {
    fn exists(&self, dim: &Dimension, store: &mut Store, p: Pos) -> bool {
        let rp = RegionPos::of_chunk(p.0, p.1);
        let presence = store.presence.entry((rp.x, rp.z)).or_insert_with(|| {
            Region::read_presence(&dim.region_dir.join(rp.file_name()))
                .ok()
                .flatten()
                .map(Box::new)
        });
        presence.as_ref().is_some_and(|b| b[local_index(p.0, p.1)])
    }

    /// Whether this job decorates the chunk (now or later).
    fn is_target(&self, dim: &Dimension, store: &mut Store, p: Pos) -> bool {
        if !self.spec.contains_chunk(p.0, p.1) {
            return false;
        }
        let rp = RegionPos::of_chunk(p.0, p.1);
        if self.completed_regions.lock().contains(&(rp.x, rp.z)) {
            return false;
        }
        !(self.spec.skip_existing && self.exists(dim, store, p))
    }

    /// A chunk is final when nothing this job still decorates can write into it.
    fn is_final(&self, dim: &Dimension, store: &mut Store, p: Pos) -> bool {
        neighbours(p).all(|n| store.decorated.contains(&n) || !self.is_target(dim, store, n))
    }

    /// Runs the decorating pipeline over `regions` (blocking). Returns `Ok(false)` when interrupted.
    pub(super) fn run_decorated(
        &self,
        dim: &Dimension,
        previews: &PreviewStore,
        starts: &StartCache,
        regions: &[RegionPos],
        data_version: i32,
        game_time: i64,
    ) -> anyhow::Result<bool> {
        let mut store = Store::default();
        for &pos in regions {
            if !self.wait_if_paused() {
                return Ok(false);
            }
            self.current_regions.lock().push((pos.x, pos.z));
            let r = self.decorate_region(dim, previews, starts, &mut store, pos, data_version, game_time);
            self.current_regions.lock().retain(|&r| r != (pos.x, pos.z));
            if !r? {
                return Ok(false);
            }
        }
        // Anything left (stand-ins, chunks next to interrupted work) is final now.
        let left: Vec<Pos> = store.slots.keys().copied().collect();
        self.write_final(dim, previews, &mut store, left, data_version, game_time)?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn decorate_region(
        &self,
        dim: &Dimension,
        previews: &PreviewStore,
        starts: &StartCache,
        store: &mut Store,
        pos: RegionPos,
        data_version: i32,
        game_time: i64,
    ) -> anyhow::Result<bool> {
        let mut targets: Vec<Pos> = Vec::new();
        let mut skipped = 0u64;
        for p in self.spec.region_chunks(pos) {
            if store.decorated.contains(&p) {
                continue;
            }
            if self.is_target(dim, store, p) {
                targets.push(p);
            } else {
                skipped += 1;
            }
        }
        self.skipped.fetch_add(skipped, Ordering::Relaxed);
        if targets.is_empty() {
            self.mark_complete(dim, store, pos);
            return Ok(true);
        }

        // Terrain for the targets and their neighbours.
        let mut needed: Vec<Pos> = Vec::new();
        {
            let mut seen = FxHashSet::default();
            for &t in &targets {
                for n in neighbours(t) {
                    if !store.slots.contains_key(&n) && seen.insert(n) {
                        needed.push(n);
                    }
                }
            }
        }
        let mut biome_chunks: Vec<Pos> = Vec::new();
        {
            let mut seen = FxHashSet::default();
            for &n in &needed {
                for b in neighbours(n) {
                    if seen.insert(b) {
                        biome_chunks.push(b);
                    }
                }
            }
        }
        let biomes: FxHashMap<Pos, BiomeResolver> = biome_chunks
            .par_iter()
            .map(|&(cx, cz)| {
                let t = profile::Timer::start();
                let r = dim.resolve_biomes(cx, cz);
                t.lap(profile::Phase::Biomes);
                ((cx, cz), r)
            })
            .collect();
        let writable: Vec<bool> = needed
            .iter()
            .map(|&p| self.is_target(dim, store, p) || !self.exists(dim, store, p))
            .collect();
        let generated: Vec<Option<(Pos, ProtoChunk)>> = needed
            .par_iter()
            .map(|&(cx, cz)| {
                if !self.wait_if_paused() {
                    return None;
                }
                Some((
                    (cx, cz),
                    dim.generate_terrain(cx, cz, |dx, dz| &biomes[&(cx + dx, cz + dz)], starts),
                ))
            })
            .collect();
        if generated.iter().any(Option::is_none) {
            return Ok(false);
        }
        for ((p, chunk), w) in generated.into_iter().flatten().zip(writable) {
            store.slots.insert(
                p,
                std::sync::Arc::new(Slot {
                    chunk: Mutex::new(Box::new(chunk)),
                    writable: w,
                }),
            );
        }
        drop(biomes);

        // The `features` step in 9 phases of independent neighbourhoods.
        // `CHUNKER_PHASE_ORDER=rev` (experiments): decorate the phases in reverse order.
        let reverse = std::env::var("CHUNKER_PHASE_ORDER").is_ok_and(|v| v == "rev");
        for phase in 0..9 {
            let phase = if reverse { 8 - phase } else { phase };
            if !self.wait_if_paused() {
                return Ok(false);
            }
            let batch: Vec<Pos> = targets
                .iter()
                .copied()
                .filter(|&(x, z)| (x.rem_euclid(3) + 3 * z.rem_euclid(3)) == phase)
                .collect();
            let slots: Vec<[std::sync::Arc<Slot>; 9]> = batch
                .iter()
                .map(|&t| {
                    let n: Vec<Pos> = neighbours(t).collect();
                    std::array::from_fn(|i| store.slots[&n[i]].clone())
                })
                .collect();
            batch
                .par_iter()
                .zip(slots.par_iter())
                .for_each(|(&(cx, cz), slots)| {
                    let t = profile::Timer::start();
                    let mut guards: Vec<_> = slots.iter().map(|s| s.chunk.lock()).collect();
                    let writable: [bool; 9] = std::array::from_fn(|i| slots[i].writable);
                    let chunks: Vec<&mut ProtoChunk> = guards.iter_mut().map(|g| &mut ***g).collect();
                    decorate_chunk(dim, starts, cx, cz, chunks, writable);
                    t.lap(profile::Phase::Features);
                    self.generated.fetch_add(1, Ordering::Relaxed);
                });
            store.decorated.extend(batch);
        }

        // Save what no later decoration can touch.
        let mut cands: Vec<Pos> = Vec::new();
        {
            let mut seen = FxHashSet::default();
            for &t in &targets {
                for n in neighbours(t) {
                    if store.slots.contains_key(&n) && seen.insert(n) {
                        cands.push(n);
                    }
                }
            }
        }
        let finals: Vec<Pos> = cands
            .into_iter()
            .filter(|&p| self.is_final(dim, store, p))
            .collect();
        self.write_final(dim, previews, store, finals, data_version, game_time)?;
        Ok(true)
    }

    /// Serializes and writes final chunks (writable ones), grouped by region file, and drops them.
    fn write_final(
        &self,
        dim: &Dimension,
        previews: &PreviewStore,
        store: &mut Store,
        finals: Vec<Pos>,
        data_version: i32,
        game_time: i64,
    ) -> anyhow::Result<()> {
        let slots: Vec<(Pos, std::sync::Arc<Slot>)> = finals
            .iter()
            .filter_map(|p| store.slots.remove(p).map(|s| (*p, s)))
            .collect();
        let biome_names = dim.biomes.clone();
        let raws: Vec<(Pos, RawChunk)> = slots
            .par_iter()
            .filter(|(_, s)| s.writable)
            .map(|(p, s)| {
                let t = profile::Timer::start();
                let mut chunk = s.chunk.lock();
                let pixels = render_chunk(&mut chunk, dim.has_ceiling);
                previews.put_chunk(p.0, p.1, &pixels);
                let nbt = crate::chunk::serialize::write_chunk(&chunk, &biome_names, data_version, game_time);
                let raw = chunker_nbt::write_root(&nbt);
                let payload = Compression::Zlib.compress(&raw);
                t.lap(profile::Phase::Serialize);
                (
                    *p,
                    RawChunk {
                        compression: Compression::Zlib.id(),
                        payload,
                        timestamp: now_timestamp(),
                    },
                )
            })
            .collect();
        let mut by_region: FxHashMap<(i32, i32), Vec<(Pos, RawChunk)>> = FxHashMap::default();
        for (p, raw) in raws {
            // The original presence must be known before our first write to the region file.
            self.exists(dim, store, p);
            let rp = RegionPos::of_chunk(p.0, p.1);
            by_region.entry((rp.x, rp.z)).or_default().push((p, raw));
        }
        let mut touched = Vec::new();
        for ((rx, rz), chunks) in by_region {
            let rp = RegionPos { x: rx, z: rz };
            previews.begin_region(rp);
            let mut region = Region::load(&dim.region_dir, rp)?;
            let mut written = 0;
            let presence = store.presence.get(&(rx, rz)).cloned().flatten();
            for (p, raw) in chunks {
                let index = local_index(p.0, p.1);
                // Existing chunks stay unless the job regenerates them.
                let pre_existing = presence.as_ref().is_some_and(|b| b[index]);
                if !pre_existing || (!self.spec.skip_existing && self.spec.contains_chunk(p.0, p.1)) {
                    region.set(index, raw);
                    written += 1;
                }
            }
            if written > 0 {
                region.save(&dim.region_dir)?;
            }
            previews.finish_region(rp);
            touched.push(rp);
        }
        for rp in touched {
            self.mark_complete(dim, store, rp);
        }
        Ok(())
    }

    /// Marks a region complete once none of its chunks still waits for decoration or saving.
    fn mark_complete(&self, dim: &Dimension, store: &mut Store, pos: RegionPos) {
        if self.completed_regions.lock().contains(&(pos.x, pos.z)) {
            return;
        }
        let chunks: Vec<Pos> = self.spec.region_chunks(pos).collect();
        if chunks.is_empty() {
            return;
        }
        let pending = chunks.iter().any(|&p| {
            store.slots.contains_key(&p) || (!store.decorated.contains(&p) && self.is_target(dim, store, p))
        });
        if !pending {
            self.completed_regions.lock().insert((pos.x, pos.z));
            self.save_file();
        }
    }
}

/// The `features` step of the center of `chunks` (its 3x3 neighbourhood, z-major from -1,-1),
/// structure pieces included; the center ends at `features`.
pub(crate) fn decorate_chunk(
    dim: &Dimension,
    starts: &StartCache,
    cx: i32,
    cz: i32,
    mut chunks: Vec<&mut ProtoChunk>,
    writable: [bool; 9],
) {
    {
        let neighbourhood: Vec<&mut ProtoChunk> = chunks.iter_mut().map(|c| &mut **c).collect();
        let mut region = FeatureRegion::new(&dim.features, cx, cz, neighbourhood, writable);
        match &dim.structures {
            Some(structures) => {
                let refs = starts.references(structures, &dim.terrain, cx, cz);
                let mut chunk_bb = writable_area(&region);
                decorate(
                    &mut region,
                    |region, step, random, seed| {
                        decorate_step(region, structures, &refs, step, random, seed, &mut chunk_bb, None)
                    },
                    None,
                );
            }
            None => decorate(&mut region, |_, _, _, _| {}, None),
        }
    }
    chunks[4].status = ChunkStatus::Features;
}
