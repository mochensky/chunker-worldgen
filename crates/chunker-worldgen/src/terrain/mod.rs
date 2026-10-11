//! Terrain generation (`ChunkStatus.TERRAIN`: noise fill, aquifers, material rules, carvers).

pub mod aquifer;
pub mod beardifier;
pub mod carver;
pub mod material;
pub mod noise;
pub mod profile;

use std::sync::Arc;

use anyhow::{Context, Result};
use chunker_data::Identifier;
use chunker_data::blocks::BlockId;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;

use crate::biome::manager::BiomeManager;
use crate::biome::{BiomeId, BiomeRegistry, BiomeResolver, BiomeSource, ClimateSampler};
use crate::block_set::StateSet;
use crate::chunk::{BlockEnv, HeightmapKind, ProtoChunk};
use crate::density::SamplerContext;
use crate::random::{AnyRandom, LegacyRandom, WorldgenRandom};
use crate::random_state::RandomState;
use crate::registry::WorldgenRegistries;
use crate::settings::NoiseGeneratorSettings;
use carver::{Carver, CarvingMask};
use material::{GenerationContext, MaterialParser, MaterialSystem, SurfaceInputs};
use noise::NoiseTerrain;

/// Biomes of the chunks around the one being generated (`WorldGenRegion` at `BIOMES` status).
pub struct NeighborBiomes<'a> {
    pub center_x: i32,
    pub center_z: i32,
    /// 3x3 resolvers, index `(dz + 1) * 3 + (dx + 1)`.
    pub chunks: [&'a BiomeResolver; 9],
    /// `Beardifier.forStructuresInChunk` result (`None` = `Beardifier.EMPTY`).
    pub beardifier: Option<crate::density::sampler::SamplerRef>,
}

impl NeighborBiomes<'_> {
    fn get(&self, qx: i32, qy: i32, qz: i32) -> Option<BiomeId> {
        let dx = (qx >> 2) - self.center_x;
        let dz = (qz >> 2) - self.center_z;
        if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dz) {
            return None;
        }
        Some(self.chunks[((dz + 1) * 3 + (dx + 1)) as usize].get(qx, qy, qz))
    }
}

/// Everything needed to run the terrain step for one dimension.
pub struct TerrainGenerator {
    pub settings: Arc<NoiseGeneratorSettings>,
    pub state: Arc<RandomState>,
    pub noise: NoiseTerrain,
    pub material: MaterialSystem,
    pub biome_source: Arc<BiomeSource>,
    pub climate: ClimateSampler,
    pub biomes: Arc<BiomeRegistry>,
    pub biome_manager: BiomeManager,
    carvers_by_biome: Vec<Vec<Arc<Carver>>>,
    uncarvable: StateSet,
    /// `ChunkAccess.carverBiome` cache shared by all threads (biome at quart y=0 of each chunk).
    carver_biomes: ShardedCache,
    grass: BlockId,
    mycelium: BlockId,
    dirt: BlockId,
}

impl TerrainGenerator {
    pub fn new(
        reg: &WorldgenRegistries,
        biomes: Arc<BiomeRegistry>,
        settings: Arc<NoiseGeneratorSettings>,
        state: Arc<RandomState>,
        biome_source: Arc<BiomeSource>,
        env: Arc<BlockEnv>,
    ) -> Result<Self> {
        let parser = MaterialParser::new(reg, &biomes);
        let material = MaterialSystem::new(state.clone(), biomes.clone(), &settings, &parser)?;
        let noise = NoiseTerrain::new(settings.clone(), state.clone(), env.clone())?;
        let climate = ClimateSampler::new(&state, &settings.router);
        let mut carver_cache: FxHashMap<Identifier, Arc<Carver>> = FxHashMap::default();
        let mut carvers_by_biome = Vec::with_capacity(biomes.len());
        for i in 0..biomes.len() {
            let data = biomes.data(BiomeId(i as u16));
            let list = match data.json.get("carvers") {
                Some(v) => carver_holder_set(reg, v)?,
                None => Vec::new(),
            };
            let mut out = Vec::with_capacity(list.len());
            for id in list {
                let c = match carver_cache.get(&id) {
                    Some(c) => c.clone(),
                    None => {
                        let c = Arc::new(
                            Carver::parse(&reg.packs.require_json("worldgen/carver", &id)?)
                                .with_context(|| format!("carver {id}"))?,
                        );
                        carver_cache.insert(id.clone(), c.clone());
                        c
                    }
                };
                out.push(c);
            }
            carvers_by_biome.push(out);
        }
        let blocks = &reg.blocks;
        Ok(Self {
            biome_manager: BiomeManager::new(state.seed),
            uncarvable: (*reg.block_tag(&Identifier::vanilla("uncarvable"))?).clone(),
            grass: blocks.block_of(blocks.vanilla_default("grass_block")),
            mycelium: blocks.block_of(blocks.vanilla_default("mycelium")),
            dirt: blocks.block_of(blocks.vanilla_default("dirt")),
            carvers_by_biome,
            carver_biomes: ShardedCache::new(),
            settings,
            state,
            noise,
            material,
            biome_source,
            climate,
            biomes,
        })
    }

    /// `WorldGenerationContext(generator, chunk)`.
    pub fn generation_context(&self, chunk: &ProtoChunk) -> GenerationContext {
        GenerationContext {
            min_y: chunk.min_y.max(self.settings.noise.min_y),
            height: chunk.height.min(self.settings.noise.height),
            sea_level: self.settings.sea_level,
        }
    }

    /// `WorldGenerationContext(generator, heightAccessor)` for a dimension height range.
    pub fn generation_context_for(&self, min_y: i32, height: i32) -> GenerationContext {
        GenerationContext {
            min_y: min_y.max(self.settings.noise.min_y),
            height: height.min(self.settings.noise.height),
            sea_level: self.settings.sea_level,
        }
    }

    /// `ChunkGenerator.doCreateBiomes` for any chunk.
    pub fn resolve_biomes(&self, chunk_x: i32, chunk_z: i32, min_y: i32, height: i32) -> BiomeResolver {
        let mut ctx = SamplerContext::cached(Default::default());
        self.biome_source
            .resolve_chunk(&self.climate, &mut ctx, chunk_x, chunk_z, min_y, height)
    }

    /// `NoiseBasedChunkGenerator.buildTerrain` + heightmap priming of `ChunkStatusTasks.buildTerrain`.
    pub fn build_terrain(&self, chunk: &mut ProtoChunk, neighbors: &NeighborBiomes) {
        let t = profile::Timer::start();
        let Some(mut nc) = self.noise.noise_chunk(
            chunk.x,
            chunk.z,
            chunk.min_y,
            chunk.height,
            neighbors.beardifier.clone(),
        ) else {
            chunk.prime_heightmaps(&HeightmapKind::FINAL);
            return;
        };
        let t = t.lap(profile::Phase::NoiseSetup);
        self.noise.do_fill(&mut nc, chunk);
        let t = t.lap(profile::Phase::Fill);
        let gen_ctx = self.generation_context(chunk);

        // buildSurface
        let mut possible: FxHashSet<BiomeId> = FxHashSet::default();
        for r in &neighbors.chunks {
            possible.extend(r.biomes.iter().copied());
        }
        let mut uncached = SamplerContext::uncached();
        {
            let manager = self.biome_manager;
            let mut getter = |x: i32, y: i32, z: i32| -> BiomeId {
                manager.get_biome(x, y, z, &mut |qx, qy, qz| match neighbors.get(qx, qy, qz) {
                    Some(b) => b,
                    None => self
                        .biome_source
                        .noise_biome(&self.climate, &mut uncached, qx, qy, qz),
                })
            };
            let volume = nc.volume;
            self.material.build_surface(
                chunk,
                &mut nc.ctx,
                &volume,
                SurfaceInputs {
                    biome_getter: &mut getter,
                    possible_biomes: &possible,
                    gen_ctx,
                },
            );
        }
        let t = t.lap(profile::Phase::Surface);

        self.generate_carvers(chunk, &mut nc, gen_ctx);
        let t = t.lap(profile::Phase::Carvers);
        chunk.prime_heightmaps(&HeightmapKind::FINAL);
        t.lap(profile::Phase::Heightmaps);
    }

    /// `NoiseBasedChunkGenerator.generateCarvers`.
    fn generate_carvers(
        &self,
        chunk: &mut ProtoChunk,
        nc: &mut noise::NoiseChunk,
        gen_ctx: GenerationContext,
    ) {
        let mut random = WorldgenRandom::new(AnyRandom::Legacy(LegacyRandom::new(0)));
        let max_y = gen_ctx.min_y + gen_ctx.height - 1 - 7;
        let mut mask = CarvingMask::new(gen_ctx.min_y + 1, max_y);
        let mut uncached = SamplerContext::uncached();
        for dx in -8..=8 {
            for dz in -8..=8 {
                let sx = chunk.x + dx;
                let sz = chunk.z + dz;
                let biome = self.carver_biomes.get_or_insert((sx, sz), || {
                    self.biome_source.noise_biome(
                        &self.climate,
                        &mut uncached,
                        (sx << 4) >> 2,
                        0,
                        (sz << 4) >> 2,
                    )
                });
                for (index, carver) in self.carvers_by_biome[biome.0 as usize].iter().enumerate() {
                    random.set_large_feature_seed(self.state.seed.wrapping_add(index as i64), sx, sz);
                    if carver.is_start_chunk(&mut random) {
                        carver.carve(&gen_ctx, &mut random, chunk.x, chunk.z, sx, sz, &mut mask);
                    }
                }
            }
        }
        if mask.is_empty() {
            return;
        }
        self.apply_carving_mask(chunk, &mask, nc, gen_ctx, &mut uncached);
    }

    fn apply_carving_mask(
        &self,
        chunk: &mut ProtoChunk,
        mask: &CarvingMask,
        nc: &mut noise::NoiseChunk,
        gen_ctx: GenerationContext,
        uncached: &mut SamplerContext,
    ) {
        let reg = chunk.blocks.clone();
        let (cmx, cmz) = (chunk.min_block_x(), chunk.min_block_z());
        let manager = self.biome_manager;
        let mut columns: Vec<(i32, i32, i32, i32)> = Vec::new();
        mask.visit(|x, z, bottom, top| columns.push((x, z, bottom, top)));
        for (x, z, bottom, top) in columns {
            let mut has_grass = false;
            let wx = cmx + x;
            let wz = cmz + z;
            let mut wy = top;
            while wy >= bottom {
                let state = chunk.get_block(wx, wy, wz);
                if !self.uncarvable.contains(state) {
                    let b = reg.block_of(state);
                    if b == self.grass || b == self.mycelium {
                        has_grass = true;
                    }
                    if let Some(new_state) = nc.aquifer.compute_substance(&mut nc.ctx, wx, wy, wz, 0.0) {
                        chunk.set_block(wx, wy, wz, new_state);
                        let fluid = reg.flags(new_state).has_fluid();
                        if nc.aquifer.should_schedule_fluid_update() && fluid {
                            chunk.mark_post_processing(wx, wy, wz);
                        }
                        if has_grass && reg.block_of(chunk.get_block(wx, wy - 1, wz)) == self.dirt {
                            let mut getter = |bx: i32, by: i32, bz: i32| -> BiomeId {
                                manager.get_biome(bx, by, bz, &mut |qx, qy, qz| {
                                    self.biome_source.noise_biome(&self.climate, uncached, qx, qy, qz)
                                })
                            };
                            if let Some(top_state) = self.material.top_material(
                                chunk,
                                &mut nc.ctx,
                                &mut getter,
                                gen_ctx,
                                wx,
                                wy - 1,
                                wz,
                                fluid,
                            ) {
                                chunk.set_block(wx, wy - 1, wz, top_state);
                                if reg.flags(top_state).has_fluid() {
                                    chunk.mark_post_processing(wx, wy - 1, wz);
                                }
                            }
                        }
                    }
                }
                wy -= 1;
            }
        }
    }
}

/// `HolderSet<ConfiguredWorldCarver>`: id, list or `#tag`.
fn carver_holder_set(reg: &WorldgenRegistries, json: &Value) -> Result<Vec<Identifier>> {
    let parse = |s: &str| Identifier::parse(s).with_context(|| format!("bad carver id {s}"));
    match json {
        Value::String(s) if s.starts_with('#') => {
            let tag = parse(&s[1..])?;
            let exists = |id: &Identifier| reg.packs.read_raw("worldgen/carver", id, "json").is_some();
            reg.packs.resolve_tag("worldgen/carver", &tag, &exists)
        }
        Value::String(s) => Ok(vec![parse(s)?]),
        Value::Array(a) => a
            .iter()
            .map(|v| parse(v.as_str().context("carver id")?))
            .collect(),
        other => anyhow::bail!("invalid carver list {other}"),
    }
}

/// A small concurrent map with a size cap (cleared shard-wise when full).
struct ShardedCache {
    shards: Vec<parking_lot::Mutex<FxHashMap<(i32, i32), BiomeId>>>,
}

impl ShardedCache {
    const SHARDS: usize = 64;
    const MAX_PER_SHARD: usize = 1 << 16;

    fn new() -> Self {
        Self {
            shards: (0..Self::SHARDS)
                .map(|_| parking_lot::Mutex::new(FxHashMap::default()))
                .collect(),
        }
    }

    fn get_or_insert(&self, key: (i32, i32), compute: impl FnOnce() -> BiomeId) -> BiomeId {
        let h =
            (key.0.wrapping_mul(0x9E37_79B1u32 as i32) ^ key.1.wrapping_mul(0x85EB_CA77u32 as i32)) as u32;
        let shard = &self.shards[(h as usize >> 7) % Self::SHARDS];
        if let Some(b) = shard.lock().get(&key) {
            return *b;
        }
        let b = compute();
        let mut map = shard.lock();
        if map.len() >= Self::MAX_PER_SHARD {
            map.clear();
        }
        map.insert(key, b);
        b
    }
}
