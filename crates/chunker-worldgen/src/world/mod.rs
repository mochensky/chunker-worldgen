//! A Minecraft world opened for generation: its files, data packs and registries, one generator per
//! dimension, the current pre-generation job and the map previews.

mod dimension;
mod files;

pub use dimension::{Dimension, DimensionInfo};
pub use files::WorldFiles;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use chunker_data::blocks::BlockRegistry;
use chunker_data::vanilla::{self, ServerJar};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::biome::BiomeRegistry;
use crate::chunk::BlockEnv;
use crate::job::{self, Job, JobFile, JobSnapshot, JobSpec};
use crate::preview::{self, PreviewStore, RenderContext};
use crate::registry::WorldgenRegistries;

/// How to open a world.
#[derive(Clone)]
pub struct WorldOptions {
    /// A server folder (with `server.properties`) or a world folder (with `level.dat`).
    pub path: PathBuf,
    /// The server jar to read vanilla data from; found automatically when `None`.
    pub server_jar: Option<PathBuf>,
    /// Where downloaded server jars are kept.
    pub cache_dir: PathBuf,
    /// Whether the server jar may be downloaded from Mojang when none is found.
    pub allow_download: bool,
}

/// What a world is: folders, version, seed, data packs and dimensions.
#[derive(Clone, Debug, Serialize)]
pub struct WorldSummary {
    pub world_dir: String,
    pub server_dir: Option<String>,
    pub level_name: String,
    pub version: String,
    pub data_version: i32,
    pub seed: String,
    pub generate_structures: bool,
    pub data_packs: Vec<String>,
    pub server_jar: String,
    pub dimensions: Vec<DimensionInfo>,
    pub in_use: bool,
}

/// A world opened for generation. One pre-generation job runs at a time, on its own thread; it can
/// be paused, resumed (also after a restart, from `<world>/.chunker/job.json`) and cancelled.
pub struct World {
    pub files: Mutex<WorldFiles>,
    pub jar: ServerJar,
    pub registries: Arc<WorldgenRegistries>,
    pub biomes: Arc<BiomeRegistry>,
    pub env: Arc<BlockEnv>,
    dimensions: Mutex<FxHashMap<Identifier, Arc<Dimension>>>,
    previews: Mutex<FxHashMap<Identifier, Arc<PreviewStore>>>,
    job: Mutex<Option<Arc<Job>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    game_time: i64,
    /// Structure registry and template manager, loaded on first use.
    structure_data: std::sync::OnceLock<(
        Arc<crate::structure::registry::StructureRegistry>,
        Arc<crate::structure::template::TemplateManager>,
    )>,
}

impl World {
    /// Opens a world: reads its files, finds the server jar and loads the data packs.
    pub fn open(opts: WorldOptions) -> Result<Arc<World>> {
        let files = WorldFiles::open(&opts.path)?;
        let jar = vanilla::locate(
            files.version,
            opts.server_jar.as_deref(),
            files.server_dir.as_deref(),
            &opts.cache_dir,
            opts.allow_download,
        )?;
        let packs = files.data_packs(&jar)?;
        let blocks = BlockRegistry::for_version(files.version);
        let registries = Arc::new(WorldgenRegistries::new(packs, blocks));
        let biomes = BiomeRegistry::load(&registries.packs)?;
        let env = BlockEnv::load(&registries)?;
        let game_time = files.spawn.game_time;
        let world = Arc::new(World {
            files: Mutex::new(files),
            jar,
            registries,
            biomes,
            env,
            dimensions: Mutex::default(),
            previews: Mutex::default(),
            job: Mutex::new(None),
            worker: Mutex::new(None),
            game_time,
            structure_data: std::sync::OnceLock::new(),
        });
        world.restore_job()?;
        Ok(world)
    }

    fn chunker_dir(&self) -> PathBuf {
        self.files.lock().dir.join(".chunker")
    }

    fn job_file(&self) -> PathBuf {
        self.chunker_dir().join("job.json")
    }

    /// Loads an unfinished job from `.chunker/job.json` in the paused state.
    fn restore_job(&self) -> Result<()> {
        let path = self.job_file();
        let Ok(bytes) = std::fs::read(&path) else {
            return Ok(());
        };
        let file: JobFile = match serde_json::from_slice(&bytes) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("ignoring unreadable {}: {e}", path.display());
                return Ok(());
            }
        };
        tracing::info!(
            "found unfinished job ({} regions done); it can be resumed",
            file.completed_regions.len()
        );
        let job = Job::new(file.spec.clone(), Some(file), path);
        job.pause();
        *self.job.lock() = Some(job);
        Ok(())
    }

    pub fn summary(&self) -> WorldSummary {
        let world = self.files.lock();
        WorldSummary {
            world_dir: world.dir.display().to_string(),
            server_dir: world.server_dir.as_ref().map(|p| p.display().to_string()),
            level_name: world.level_name.clone(),
            version: world.version.to_string(),
            data_version: world.data_version,
            seed: world.seed.to_string(),
            generate_structures: world.generate_structures,
            data_packs: world.enabled_packs.clone(),
            server_jar: self.jar.origin.clone(),
            dimensions: world
                .dimensions
                .iter()
                .map(|(id, json)| Dimension::describe(&self.registries, id, json))
                .collect(),
            in_use: world.is_in_use(),
        }
    }

    fn structure_data(
        &self,
    ) -> Result<(
        Arc<crate::structure::registry::StructureRegistry>,
        Arc<crate::structure::template::TemplateManager>,
    )> {
        if let Some(d) = self.structure_data.get() {
            return Ok(d.clone());
        }
        let registry = Arc::new(crate::structure::registry::StructureRegistry::load(
            &self.registries,
            &self.biomes,
        )?);
        let templates = Arc::new(crate::structure::template::TemplateManager::new(
            Arc::new(self.registries.packs.clone()),
            self.registries.blocks.clone(),
        ));
        Ok(self.structure_data.get_or_init(|| (registry, templates)).clone())
    }

    pub fn dimension(&self, id: &Identifier) -> Result<Arc<Dimension>> {
        if let Some(d) = self.dimensions.lock().get(id) {
            return Ok(d.clone());
        }
        let generate_structures = self.files.lock().generate_structures;
        let structure_data = if generate_structures {
            Some(self.structure_data().context("loading structures")?)
        } else {
            None
        };
        let world = self.files.lock();
        let json = world
            .dimensions
            .iter()
            .find(|(d, _)| d == id)
            .map(|(_, j)| j.clone())
            .with_context(|| format!("world has no dimension {id}"))?;
        let dim = Arc::new(Dimension::build(
            &world,
            &self.registries,
            &self.biomes,
            &self.env,
            structure_data,
            id,
            &json,
        )?);
        drop(world);
        self.dimensions.lock().insert(id.clone(), dim.clone());
        Ok(dim)
    }

    pub fn previews(&self, id: &Identifier) -> Arc<PreviewStore> {
        if let Some(p) = self.previews.lock().get(id) {
            return p.clone();
        }
        let dir = self
            .chunker_dir()
            .join("preview")
            .join(id.namespace())
            .join(id.path());
        let (min_y, height, region_dir) = {
            let world = self.files.lock();
            let info = world
                .dimensions
                .iter()
                .find(|(d, _)| d == id)
                .map(|(d, json)| Dimension::describe(&self.registries, d, json));
            let (min_y, height) = info
                .filter(|i| i.height > 0)
                .map_or((-64, 384), |i| (i.min_y, i.height));
            (min_y, height, world.region_dir(id))
        };
        let ctx = RenderContext {
            blocks: self.registries.blocks.clone(),
            biomes: self.biomes.clone(),
            min_y,
            height,
            region_dir,
        };
        self.previews
            .lock()
            .entry(id.clone())
            .or_insert_with(|| Arc::new(PreviewStore::new(dir, ctx)))
            .clone()
    }

    /// Distinct map colour of every biome (indexed by biome id).
    pub fn biome_palette(&self) -> Vec<[u8; 3]> {
        preview::biome_palette(&self.biomes)
    }

    pub fn job(&self) -> Option<Arc<Job>> {
        self.job.lock().clone()
    }

    pub fn job_snapshot(&self) -> Option<JobSnapshot> {
        self.job().map(|j| j.snapshot())
    }

    fn worker_alive(&self) -> bool {
        self.worker.lock().as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Starts a new job. Fails while another job is still active.
    pub fn start(self: &Arc<Self>, spec: JobSpec) -> Result<Arc<Job>> {
        if spec.radius <= 0 {
            bail!("radius must be positive");
        }
        if let Some(j) = self.job()
            && j.is_active()
            && self.worker_alive()
        {
            bail!("a job is already running; cancel it first");
        }
        let job = Job::new(spec, None, self.job_file());
        self.spawn(job.clone())?;
        Ok(job)
    }

    /// Resumes a paused job (also one restored from disk after a restart).
    pub fn resume(self: &Arc<Self>) -> Result<()> {
        let job = self.job().context("no job to resume")?;
        if job.state() != job::JobState::Paused {
            bail!("job is not paused");
        }
        job.resume();
        if !self.worker_alive() {
            self.spawn(job)?;
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        self.job().context("no job")?.pause();
        Ok(())
    }

    pub fn cancel(&self) -> Result<()> {
        let job = self.job().context("no job")?;
        job.cancel();
        if !self.worker_alive() {
            // A restored job without a worker: forget it.
            let _ = std::fs::remove_file(self.job_file());
            *self.job.lock() = None;
        }
        Ok(())
    }

    fn spawn(self: &Arc<Self>, job: Arc<Job>) -> Result<()> {
        let dim_id = Identifier::parse(&job.spec.dimension).context("bad dimension id")?;
        let dim = self.dimension(&dim_id)?;
        self.files.lock().lock()?;
        let previews = self.previews(&dim_id);
        *self.job.lock() = Some(job.clone());
        let world = self.clone();
        let data_version = self.files.lock().data_version;
        let game_time = self.game_time;
        let handle = std::thread::Builder::new()
            .name("chunker-job".into())
            .spawn(move || {
                tracing::info!("job started: {:?}", job.spec);
                job.run(&dim, &previews, data_version, game_time);
                tracing::info!("job ended: {:?}", job.state());
                world.files.lock().unlock();
            })?;
        *self.worker.lock() = Some(handle);
        Ok(())
    }

    /// Blocks until the current job's worker thread exits.
    pub fn wait(&self) {
        let handle = self.worker.lock().take();
        if let Some(h) = handle {
            let _ = h.join();
        }
    }
}
