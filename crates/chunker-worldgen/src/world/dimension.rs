//! Per-dimension generator setup from the world's `dimensions` configuration.

use std::path::PathBuf;
use std::sync::Arc;

use crate::biome::{BiomeRegistry, BiomeResolver, BiomeSource};
use crate::chunk::{BlockEnv, ChunkStatus, ProtoChunk};
use crate::feature::{FeatureEnv, FeatureRegistry};
use crate::random_state::RandomState;
use crate::registry::WorldgenRegistries;
use crate::structure::StructureGenerator;
use crate::structure::cache::{StartCache, beardifier, structures_nbt};
use crate::structure::registry::StructureRegistry;
use crate::structure::template::TemplateManager;
use crate::terrain::{NeighborBiomes, TerrainGenerator};
use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use serde_json::Value;

use super::WorldFiles;

/// A dimension ready to generate: its terrain, structure and feature generators.
pub struct Dimension {
    pub id: Identifier,
    pub min_y: i32,
    pub height: i32,
    pub region_dir: PathBuf,
    pub terrain: TerrainGenerator,
    pub biomes: Arc<BiomeRegistry>,
    pub env: Arc<BlockEnv>,
    /// `None` when the world has `generate_structures: false`.
    pub structures: Option<StructureGenerator>,
    /// The `features` step (placed features and structure pieces).
    pub features: FeatureEnv,
}

/// What a dimension is and whether it can be generated (and why not).
#[derive(Clone, Debug, serde::Serialize)]
pub struct DimensionInfo {
    pub id: String,
    pub generator: String,
    pub supported: bool,
    pub reason: Option<String>,
    pub min_y: i32,
    pub height: i32,
}

/// `(min_y, height, has_skylight, json)` of a dimension type (id or inline).
fn dimension_type_full(reg: &WorldgenRegistries, v: &Value) -> Result<(i32, i32, bool, Value)> {
    let json = match v {
        Value::String(id) => reg.packs.require_json(
            "dimension_type",
            &Identifier::parse(id).context("bad dimension type")?,
        )?,
        other => other.clone(),
    };
    let (min_y, height) = dimension_type(reg, &json)?;
    Ok((
        min_y,
        height,
        json.get("has_skylight").and_then(Value::as_bool).unwrap_or(true),
        json,
    ))
}

fn dimension_type(reg: &WorldgenRegistries, v: &Value) -> Result<(i32, i32)> {
    let json = match v {
        Value::String(id) => reg.packs.require_json(
            "dimension_type",
            &Identifier::parse(id).context("bad dimension type")?,
        )?,
        other => other.clone(),
    };
    let min_y = json
        .get("min_y")
        .and_then(Value::as_i64)
        .context("dimension type min_y")? as i32;
    let height = json
        .get("height")
        .and_then(Value::as_i64)
        .context("dimension type height")? as i32;
    Ok((min_y, height))
}

impl Dimension {
    pub fn describe(reg: &WorldgenRegistries, id: &Identifier, json: &Value) -> DimensionInfo {
        let generator = json
            .pointer("/generator/type")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned();
        let (min_y, height) = json
            .get("type")
            .and_then(|t| dimension_type(reg, t).ok())
            .unwrap_or((0, 0));
        let supported = generator == "minecraft:noise" || generator == "noise";
        DimensionInfo {
            id: id.to_string(),
            reason: (!supported).then(|| format!("generator {generator} is not supported yet")),
            generator,
            supported,
            min_y,
            height,
        }
    }

    pub fn build(
        world: &WorldFiles,
        reg: &Arc<WorldgenRegistries>,
        biomes: &Arc<BiomeRegistry>,
        env: &Arc<BlockEnv>,
        structure_data: Option<(Arc<StructureRegistry>, Arc<TemplateManager>)>,
        id: &Identifier,
        json: &Value,
    ) -> Result<Dimension> {
        let (min_y, height, has_skylight, dimension_type) =
            dimension_type_full(reg, json.get("type").context("dimension without type")?)?;
        let generator = json.get("generator").context("dimension without generator")?;
        let gen_type = generator.get("type").and_then(Value::as_str).unwrap_or_default();
        if gen_type != "minecraft:noise" && gen_type != "noise" {
            bail!("dimension {id}: generator {gen_type} is not supported yet");
        }
        let settings = match generator
            .get("settings")
            .context("noise generator without settings")?
        {
            Value::String(s) => reg.noise_settings(&Identifier::parse(s).context("bad settings id")?)?,
            inline => Arc::new(reg.parse_noise_settings(
                Identifier::new(&format!("{}_inline", id.as_str().replace(':', "_"))),
                inline,
            )?),
        };
        let source = Arc::new(BiomeSource::parse(
            reg,
            biomes,
            generator.get("biome_source").context("no biome_source")?,
        )?);
        let state = Arc::new(RandomState::new(
            reg.clone(),
            world.seed,
            settings.legacy_random_source,
        ));
        let terrain = TerrainGenerator::new(reg, biomes.clone(), settings, state, source, env.clone())
            .with_context(|| format!("setting up {id}"))?;
        let feature_registry = Arc::new(
            FeatureRegistry::load(reg, biomes, &terrain.biome_source.possible_biomes())
                .with_context(|| format!("features of {id}"))?,
        );
        let mut features = FeatureEnv::new(
            reg.clone(),
            &terrain,
            env.clone(),
            feature_registry,
            min_y,
            height,
            has_skylight,
            id.clone(),
            &dimension_type,
        )?;
        features.spawn = world.spawn;
        if let Some((_, templates)) = &structure_data {
            features.templates = templates.clone();
        }
        let structures = structure_data
            .map(|(registry, templates)| StructureGenerator::new(registry, templates, &terrain));
        Ok(Dimension {
            structures,
            features,
            id: id.clone(),
            min_y,
            height,
            region_dir: world.region_dir(id),
            terrain,
            biomes: biomes.clone(),
            env: env.clone(),
        })
    }

    pub fn resolve_biomes(&self, chunk_x: i32, chunk_z: i32) -> BiomeResolver {
        self.terrain
            .resolve_biomes(chunk_x, chunk_z, self.min_y, self.height)
    }

    /// A structure start cache for a job in this dimension.
    pub fn start_cache(&self) -> StartCache {
        StartCache::new(self.min_y, self.height)
    }

    /// Generates one chunk up to `terrain` (structure starts, references and the beardifier
    /// included). `neighbor(dx, dz)` returns the biomes of nearby chunks.
    pub fn generate_terrain<'a>(
        &self,
        chunk_x: i32,
        chunk_z: i32,
        neighbor: impl Fn(i32, i32) -> &'a BiomeResolver,
        starts: &StartCache,
    ) -> ProtoChunk {
        let mut chunk = ProtoChunk::new(self.env.clone(), chunk_x, chunk_z, self.min_y, self.height);
        let mut beard = None;
        if let Some(structures) = &self.structures {
            let own = starts.starts(structures, &self.terrain, chunk_x, chunk_z);
            let references = starts.references(structures, &self.terrain, chunk_x, chunk_z);
            beard = beardifier(structures, &references, chunk_x, chunk_z).map(|b| b.into_sampler());
            chunk.structures_nbt = Some(structures_nbt(structures, &own, &references));
        }
        chunk.status = ChunkStatus::StructureReferences;
        chunk.fill_biomes(neighbor(0, 0));
        chunk.status = ChunkStatus::Biomes;
        let neighbors = NeighborBiomes {
            center_x: chunk_x,
            center_z: chunk_z,
            chunks: std::array::from_fn(|i| neighbor(i as i32 % 3 - 1, i as i32 / 3 - 1)),
            beardifier: beard,
        };
        self.terrain.build_terrain(&mut chunk, &neighbors);
        chunk.status = ChunkStatus::Terrain;
        chunk
    }
}
