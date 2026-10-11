//! The `features` step (`ChunkGenerator.applyBiomeDecoration`): placed features and structure
//! pieces written through a [`Region`] over the 3x3 chunks around the decorated chunk.

pub mod behaviour;
pub mod blocks;
pub mod kinds;
pub mod placement;
pub mod predicate;
pub mod provider;
pub mod region;
pub mod registry;
pub mod rule;
pub mod tree;

use std::sync::Arc;

use anyhow::Result;
use chunker_data::Identifier;
use chunker_data::blocks::BlockRegistry;

use crate::biome::manager::BiomeManager;
use crate::biome::{BiomeId, BiomeRegistry};
use crate::chunk::BlockEnv;
use crate::random::{PositionalRandomFactory, RandomSource, WorldgenRandom, XoroshiroRandom};
use crate::registry::WorldgenRegistries;
use crate::structure::geom::BlockPos;
use crate::terrain::TerrainGenerator;
use crate::terrain::material::GenerationContext;

pub use region::Region;
pub use registry::{FeatureRegistry, PlacedFeature};

/// World level data shared by all decorations of a dimension.
pub struct FeatureEnv {
    pub reg: Arc<WorldgenRegistries>,
    pub blocks: Arc<BlockRegistry>,
    pub chunk_env: Arc<BlockEnv>,
    pub biomes: Arc<BiomeRegistry>,
    pub b: blocks::KnownBlocks,
    pub t: blocks::KnownTags,
    pub behaviours: behaviour::Behaviours,
    pub seed: i64,
    pub sea_level: i32,
    pub min_y: i32,
    pub height: i32,
    /// `ChunkGenerator.getGenDepth` (noise settings height).
    pub gen_depth: i32,
    /// `WorldGenerationContext` of the dimension (vertical anchors).
    pub gen_ctx: GenerationContext,
    pub biome_manager: BiomeManager,
    /// `randomState.getOrCreateRandomFactory("worldgen_region_random")`.
    pub region_random: PositionalRandomFactory,
    pub has_skylight: bool,
    pub features: Arc<FeatureRegistry>,
    /// Possible biomes of the biome source (`biomeSource.possibleBiomes()`).
    pub possible_biomes: Vec<BiomeId>,
    pub possible_set: rustc_hash::FxHashSet<BiomeId>,
    /// `StructureTemplateManager` (template and fossil features, structure pieces).
    pub templates: Arc<crate::structure::template::TemplateManager>,
    /// The dimension key (`level.dimension()`).
    pub dimension: Identifier,
    /// The moon phase timeline of the dimension type, if it has one (difficulty of spawned mobs).
    pub moon: Option<crate::entity::MoonTimeline>,
    /// Difficulty and time of the world, seen by `finalizeSpawn` of structure mobs.
    pub spawn: crate::entity::SpawnSettings,
    mobs: crate::entity::MobData,
    mineshaft_blocking: std::sync::OnceLock<registry::BiomeSet>,
}

impl FeatureEnv {
    /// Registries and tables of `finalizeSpawn`.
    pub fn mobs(&self) -> &crate::entity::MobData {
        &self.mobs
    }

    /// `#minecraft:mineshaft_blocking` (biomes where mineshaft pieces are skipped).
    pub fn mineshaft_blocking(&self) -> &registry::BiomeSet {
        self.mineshaft_blocking.get_or_init(|| {
            registry::Parser::new(&self.reg, &self.biomes)
                .biome_set(&serde_json::Value::String("#minecraft:mineshaft_blocking".into()))
                .unwrap_or_else(|e| panic!("biome tag mineshaft_blocking: {e:#}"))
        })
    }
}

impl FeatureEnv {
    pub fn new(
        reg: Arc<WorldgenRegistries>,
        terrain: &TerrainGenerator,
        chunk_env: Arc<BlockEnv>,
        features: Arc<FeatureRegistry>,
        dim_min_y: i32,
        dim_height: i32,
        has_skylight: bool,
        dimension: Identifier,
        dimension_type: &serde_json::Value,
    ) -> Result<Self> {
        let blocks = reg.blocks.clone();
        let mobs = crate::entity::MobData::load(&reg, &terrain.biomes)?;
        let moon = crate::entity::MoonTimeline::load(&reg, dimension_type)?;
        let possible_biomes = terrain.biome_source.possible_biomes();
        Ok(Self {
            b: blocks::KnownBlocks::load(&blocks)?,
            t: blocks::KnownTags::load(&reg)?,
            behaviours: behaviour::Behaviours::load(&reg)?,
            blocks,
            chunk_env,
            biomes: terrain.biomes.clone(),
            seed: terrain.state.seed,
            sea_level: terrain.settings.sea_level,
            min_y: dim_min_y,
            height: dim_height,
            gen_depth: terrain.settings.noise.height,
            gen_ctx: terrain.generation_context_for(dim_min_y, dim_height),
            biome_manager: terrain.biome_manager,
            region_random: terrain.state.random_factory("minecraft:worldgen_region_random"),
            has_skylight,
            features,
            possible_set: possible_biomes.iter().copied().collect(),
            possible_biomes,
            templates: Arc::new(crate::structure::template::TemplateManager::new(
                Arc::new(reg.packs.clone()),
                reg.blocks.clone(),
            )),
            dimension,
            moon,
            spawn: crate::entity::SpawnSettings::default(),
            mobs,
            mineshaft_blocking: std::sync::OnceLock::new(),
            reg,
        })
    }

    pub fn max_y(&self) -> i32 {
        self.min_y + self.height - 1
    }
}

/// `ChunkGenerator.applyBiomeDecoration` for the center chunk of `region`. Structure pieces are
/// placed by `structures` (called per step with the structure index and the decoration random);
/// `filter` restricts the placed features (oracle tests while porting).
pub fn decorate(
    region: &mut Region,
    mut structures: impl FnMut(&mut Region, usize, &mut WorldgenRandom, i64),
    filter: Option<&dyn Fn(&str) -> bool>,
) {
    let env = region.env;
    let origin = BlockPos::new(region.center_x << 4, env.min_y, region.center_z << 4);
    let mut random = WorldgenRandom::new(crate::random::AnyRandom::Xoroshiro(XoroshiroRandom::new(0)));
    let decoration_seed = random.set_decoration_seed(env.seed, origin.x, origin.z);
    let features = &env.features;
    // Biomes present in the sections of the 3x3 chunks, restricted to the source's biomes.
    let mut present = vec![false; env.biomes.len()];
    region.for_each_chunk(|chunk| {
        for s in &chunk.sections {
            for b in s.biomes {
                present[b.0 as usize] = true;
            }
        }
    });
    let steps = features.steps.len().max(registry::DECORATION_STEPS);
    for step in 0..steps {
        structures(region, step, &mut random, decoration_seed);
        let Some(step_data) = features.steps.get(step) else {
            continue;
        };
        let mut indices: Vec<usize> = Vec::new();
        for (biome, used) in present.iter().enumerate() {
            let id = BiomeId(biome as u16);
            if !*used || !env.possible_set.contains(&id) {
                continue;
            }
            if let Some(list) = features.biome_steps.get(biome).and_then(|s| s.get(step)) {
                for f in list {
                    if let Some(&i) = step_data.index.get(&Arc::as_ptr(f)) {
                        indices.push(i);
                    }
                }
            }
        }
        indices.sort_unstable();
        indices.dedup();
        for i in indices {
            let feature = &step_data.features[i];
            if let Some(filter) = filter
                && !filter(feature.id.as_deref().unwrap_or(""))
            {
                continue;
            }
            random.set_feature_seed(decoration_seed, i as i32, step as i32);
            placement::place_with_biome_check(region, feature, &mut random, origin);
        }
    }
}

/// `RandomSource` passed through feature code.
pub type Rng<'a> = &'a mut dyn RandomSource;
