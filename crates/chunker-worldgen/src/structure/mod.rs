//! Structures: `structure_starts` (`ChunkGenerator.createStructures`), `structure_references`,
//! and the per-type piece generators (`levelgen.structure.structures.*`).

pub mod build;
pub mod cache;
pub mod endcity;
pub mod fortress;
pub mod geom;
pub mod jigsaw;
pub mod mansion;
pub mod mineshaft;
pub mod monument;
pub mod place;
pub mod placement;
pub mod processor;
pub mod registry;
pub mod scattered;
pub mod simple;
pub mod start;
pub mod stronghold;
pub mod template;
pub mod templated;
pub mod util;

use std::sync::{Arc, OnceLock};

use chunker_data::blocks::BlockState;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::biome::BiomeId;
use crate::chunk::HeightmapKind;
use crate::density::SamplerContext;
use crate::random::{LegacyRandom, RandomSource, WorldgenRandom};
use crate::terrain::TerrainGenerator;
use geom::BlockPos;
use placement::PlacementKind;
pub use registry::Structure;
use registry::{StructureKind, StructureRegistry, TerrainAdjustment};
use start::{PiecesBuilder, StructureStart};
use util::legacy_worldgen_random;

/// Structure generation state of one dimension (`ChunkGeneratorStructureState` + registries).
pub struct StructureGenerator {
    pub registry: Arc<StructureRegistry>,
    pub templates: Arc<template::TemplateManager>,
    pub seed: i64,
    /// Indices of the structure sets whose structures can appear in this dimension's biomes.
    possible_sets: Vec<usize>,
    /// Lazily computed concentric ring positions, per set index.
    rings: OnceLock<FxHashMap<usize, FxHashSet<(i32, i32)>>>,
    /// Ring positions in vanilla order (for locating), per set index.
    ring_lists: OnceLock<FxHashMap<usize, Vec<(i32, i32)>>>,
}

/// `Structure.GenerationContext` for one structure attempt in one chunk.
pub struct GenContext<'a> {
    pub structures: &'a StructureGenerator,
    pub terrain: &'a TerrainGenerator,
    pub seed: i64,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub random: WorldgenRandom,
    pub min_y: i32,
    pub height: i32,
    pub climate_ctx: &'a mut SamplerContext,
    pub valid_biomes: &'a FxHashSet<BiomeId>,
}

impl GenContext<'_> {
    pub fn min_block_x(&self) -> i32 {
        self.chunk_x << 4
    }

    pub fn min_block_z(&self) -> i32 {
        self.chunk_z << 4
    }

    pub fn middle_block_x(&self) -> i32 {
        (self.chunk_x << 4) + 8
    }

    pub fn middle_block_z(&self) -> i32 {
        (self.chunk_z << 4) + 8
    }

    /// `heightAccessor.getMaxY()`.
    pub fn max_y(&self) -> i32 {
        self.min_y + self.height - 1
    }

    pub fn sea_level(&self) -> i32 {
        self.terrain.settings.sea_level
    }

    /// `ChunkGenerator.getBaseHeight`.
    pub fn base_height(&self, x: i32, z: i32, kind: HeightmapKind) -> i32 {
        self.terrain
            .noise
            .iterate_column(x, z, self.min_y, self.height, Some(kind), None)
            .0
            .unwrap_or(self.min_y)
    }

    /// `ChunkGenerator.getFirstOccupiedHeight`.
    pub fn first_occupied_height(&self, x: i32, z: i32, kind: HeightmapKind) -> i32 {
        self.base_height(x, z, kind) - 1
    }

    /// `ChunkGenerator.getFirstFreeHeight`.
    pub fn first_free_height(&self, x: i32, z: i32, kind: HeightmapKind) -> i32 {
        self.base_height(x, z, kind)
    }

    /// `ChunkGenerator.getBaseColumn`: `(min y, blocks bottom-up)`.
    pub fn base_column(&self, x: i32, z: i32) -> (i32, Vec<BlockState>) {
        let mut column = Vec::new();
        let (_, min_y) =
            self.terrain
                .noise
                .iterate_column(x, z, self.min_y, self.height, None, Some(&mut column));
        (min_y, column)
    }

    fn corner_heights(&self, min_x: i32, size_x: i32, min_z: i32, size_z: i32) -> [i32; 4] {
        let k = HeightmapKind::WorldSurfaceWg;
        [
            self.first_occupied_height(min_x, min_z, k),
            self.first_occupied_height(min_x, min_z + size_z, k),
            self.first_occupied_height(min_x + size_x, min_z, k),
            self.first_occupied_height(min_x + size_x, min_z + size_z, k),
        ]
    }

    /// `Structure.getMeanFirstOccupiedHeight`.
    pub fn mean_first_occupied_height(&self, min_x: i32, size_x: i32, min_z: i32, size_z: i32) -> i32 {
        let h = self.corner_heights(min_x, size_x, min_z, size_z);
        (h[0] + h[1] + h[2] + h[3]) / 4
    }

    /// `Structure.getLowestY(context, minX, minZ, sizeX, sizeZ)`.
    pub fn lowest_y_at(&self, min_x: i32, min_z: i32, size_x: i32, size_z: i32) -> i32 {
        let h = self.corner_heights(min_x, size_x, min_z, size_z);
        h[0].min(h[1]).min(h[2].min(h[3]))
    }

    /// `Structure.getLowestY(context, sizeX, sizeZ)`.
    pub fn lowest_y(&self, size_x: i32, size_z: i32) -> i32 {
        self.lowest_y_at(self.min_block_x(), self.min_block_z(), size_x, size_z)
    }

    /// Point biome lookup through the shared caching climate sampler (`biomeResolver`).
    pub fn biome_at_quart(&mut self, qx: i32, qy: i32, qz: i32) -> BiomeId {
        self.terrain
            .biome_source
            .noise_biome(&self.terrain.climate, self.climate_ctx, qx, qy, qz)
    }

    /// `isValidBiome(stub)`.
    pub fn is_valid_biome(&mut self, pos: BlockPos) -> bool {
        let b = self.biome_at_quart(pos.x >> 2, pos.y >> 2, pos.z >> 2);
        self.valid_biomes.contains(&b)
    }

    /// `couldStructureExistInColumn`.
    pub fn could_structure_exist_in_column(&mut self, bx: i32, bz: i32, min_by: i32, max_by: i32) -> bool {
        let (qx, qz, min_qy, max_qy) = (bx >> 2, bz >> 2, min_by >> 2, max_by >> 2);
        let terrain = self.terrain;
        let resolver = terrain.biome_source.box_resolver(
            &terrain.climate,
            self.climate_ctx,
            qx,
            min_qy,
            qz,
            1,
            max_qy - min_qy + 1,
            1,
        );
        for qy in min_qy..=max_qy {
            let b = resolver.get(
                &terrain.biome_source,
                &terrain.climate,
                self.climate_ctx,
                qx,
                qy,
                qz,
            );
            if self.valid_biomes.contains(&b) {
                return true;
            }
        }
        false
    }

    /// `couldValidBiomeExistInTerrainColumn`.
    pub fn could_valid_biome_exist_in_terrain_column(&mut self, bx: i32, bz: i32) -> bool {
        let (min, max) = (self.min_y - 1, self.max_y());
        self.could_structure_exist_in_column(bx, bz, min, max)
    }

    /// `couldValidBiomeExistOnTopOfChunkCenter`.
    pub fn could_valid_biome_exist_on_top_of_chunk_center(&mut self) -> bool {
        let (x, z) = (self.middle_block_x(), self.middle_block_z());
        self.could_valid_biome_exist_in_terrain_column(x, z)
    }

    /// `onTopOfChunkCenterWithoutBiomeCheck`: the generation point on the given heightmap.
    pub fn top_of_chunk_center(&self, kind: HeightmapKind) -> BlockPos {
        let (x, z) = (self.middle_block_x(), self.middle_block_z());
        BlockPos::new(x, self.first_occupied_height(x, z, kind), z)
    }
}

/// `Structure.GenerationStub`: the start position and the (lazy) piece generator.
pub struct Stub<'c> {
    pub pos: BlockPos,
    pub pieces: StubPieces<'c>,
}

/// Deferred piece generation of a start (run only if the start survives the biome check).
pub type LazyPieces<'c> = Box<dyn FnOnce(&mut GenContext, &mut PiecesBuilder) + 'c>;

pub enum StubPieces<'c> {
    Lazy(LazyPieces<'c>),
    Built(PiecesBuilder),
}

impl<'c> Stub<'c> {
    pub fn lazy(pos: BlockPos, f: impl FnOnce(&mut GenContext, &mut PiecesBuilder) + 'c) -> Self {
        Self {
            pos,
            pieces: StubPieces::Lazy(Box::new(f)),
        }
    }
}

impl StructureGenerator {
    /// `ChunkGeneratorStructureState.createForNormal`.
    pub fn new(
        registry: Arc<StructureRegistry>,
        templates: Arc<template::TemplateManager>,
        terrain: &TerrainGenerator,
    ) -> Self {
        let possible: FxHashSet<BiomeId> = terrain.biome_source.possible_biomes().into_iter().collect();
        let possible_sets = registry
            .sets
            .iter()
            .enumerate()
            .filter(|(_, set)| {
                set.structures.iter().any(|(s, _)| {
                    registry.structures[*s]
                        .biomes
                        .iter()
                        .any(|b| possible.contains(b))
                })
            })
            .map(|(i, _)| i)
            .collect();
        Self {
            registry,
            templates,
            seed: terrain.state.seed,
            possible_sets,
            rings: OnceLock::new(),
            ring_lists: OnceLock::new(),
        }
    }

    /// Concentric ring positions of every ring placement (`generatePositions`), computed once.
    pub fn ring_positions(&self, terrain: &TerrainGenerator) -> &FxHashMap<usize, Vec<(i32, i32)>> {
        self.ring_lists.get_or_init(|| {
            let mut out = FxHashMap::default();
            for &si in &self.possible_sets {
                let set = &self.registry.sets[si];
                if let PlacementKind::ConcentricRings {
                    distance,
                    spread,
                    count,
                    preferred_biomes,
                } = &set.placement.kind
                {
                    out.insert(
                        si,
                        generate_ring_positions(
                            terrain,
                            self.seed,
                            *distance,
                            *spread,
                            *count,
                            preferred_biomes,
                        ),
                    );
                }
            }
            out
        })
    }

    fn ring_sets(&self, terrain: &TerrainGenerator) -> &FxHashMap<usize, FxHashSet<(i32, i32)>> {
        self.rings.get_or_init(|| {
            self.ring_positions(terrain)
                .iter()
                .map(|(k, v)| (*k, v.iter().copied().collect()))
                .collect()
        })
    }

    /// `StructurePlacement.isStructureChunk` including the exclusion zone.
    pub fn is_structure_chunk(&self, terrain: &TerrainGenerator, set_index: usize, x: i32, z: i32) -> bool {
        let placement = &self.registry.sets[set_index].placement;
        let rings = match placement.kind {
            PlacementKind::ConcentricRings { .. } => self.ring_sets(terrain).get(&set_index),
            _ => None,
        };
        if !placement.is_placement_chunk(self.seed, rings, x, z) {
            return false;
        }
        match placement.exclusion_zone {
            Some((other, range)) => {
                for tx in x - range..=x + range {
                    for tz in z - range..=z + range {
                        if self.is_structure_chunk(terrain, other, tx, tz) {
                            return false;
                        }
                    }
                }
                true
            }
            None => true,
        }
    }

    /// `ChunkGenerator.createStructures` for a fresh chunk.
    pub fn create_starts(
        &self,
        terrain: &TerrainGenerator,
        chunk_x: i32,
        chunk_z: i32,
        min_y: i32,
        height: i32,
    ) -> Vec<StructureStart> {
        let mut ctx = SamplerContext::cached(Default::default());
        let mut out = Vec::new();
        for &si in &self.possible_sets {
            let set = &self.registry.sets[si];
            if !self.is_structure_chunk(terrain, si, chunk_x, chunk_z) {
                continue;
            }
            let attempt = |structure: usize, ctx: &mut SamplerContext| {
                self.try_generate(terrain, structure, chunk_x, chunk_z, min_y, height, ctx)
            };
            if set.structures.len() == 1 {
                if let Some(s) = attempt(set.structures[0].0, &mut ctx) {
                    out.push(s);
                }
                continue;
            }
            let mut options = set.structures.clone();
            let mut random = legacy_worldgen_random();
            random.set_large_feature_seed(self.seed, chunk_x, chunk_z);
            let mut total: i32 = options.iter().map(|(_, w)| *w).sum();
            while !options.is_empty() {
                let mut choice = random.next_int_bounded(total);
                let mut index = 0;
                for (_, w) in &options {
                    choice -= w;
                    if choice < 0 {
                        break;
                    }
                    index += 1;
                }
                let (structure, weight) = options[index];
                if let Some(s) = attempt(structure, &mut ctx) {
                    out.push(s);
                    break;
                }
                options.remove(index);
                total -= weight;
            }
        }
        out
    }

    fn try_generate(
        &self,
        terrain: &TerrainGenerator,
        structure: usize,
        chunk_x: i32,
        chunk_z: i32,
        min_y: i32,
        height: i32,
        climate_ctx: &mut SamplerContext,
    ) -> Option<StructureStart> {
        let s = &self.registry.structures[structure];
        let mut random = legacy_worldgen_random();
        random.set_large_feature_seed(self.seed, chunk_x, chunk_z);
        let mut ctx = GenContext {
            structures: self,
            terrain,
            seed: self.seed,
            chunk_x,
            chunk_z,
            random,
            min_y,
            height,
            climate_ctx,
            valid_biomes: &s.biomes,
        };
        let stub = match &s.kind {
            StructureKind::BuriedTreasure => simple::buried_treasure(&mut ctx),
            StructureKind::SwampHut => simple::swamp_hut(&mut ctx),
            StructureKind::DesertPyramid => simple::desert_pyramid(&mut ctx),
            StructureKind::JungleTemple => simple::jungle_temple(&mut ctx),
            StructureKind::Jigsaw(config) => jigsaw::find_generation_point(&mut ctx, config),
            StructureKind::Mineshaft { mesa } => mineshaft::find_generation_point(&mut ctx, *mesa),
            StructureKind::Shipwreck { beached } => templated::shipwreck(&mut ctx, *beached),
            StructureKind::Igloo => templated::igloo(&mut ctx),
            StructureKind::Stronghold => stronghold::find_generation_point(&mut ctx),
            StructureKind::Mansion => mansion::find_generation_point(&mut ctx),
            StructureKind::Fortress => fortress::find_generation_point(&mut ctx),
            StructureKind::EndCity => endcity::find_generation_point(&mut ctx),
            StructureKind::OceanMonument { surrounding } => simple::ocean_monument(&mut ctx, surrounding),
            StructureKind::OceanRuin {
                warm,
                large_probability,
                cluster_probability,
            } => templated::ocean_ruin(&mut ctx, *warm, *large_probability, *cluster_probability),
            StructureKind::RuinedPortal { setups } => templated::ruined_portal(&mut ctx, setups),
            StructureKind::NetherFossil { height } => templated::nether_fossil(&mut ctx, height),
        }?;
        if !ctx.is_valid_biome(stub.pos) {
            return None;
        }
        let builder = match stub.pieces {
            StubPieces::Built(b) => b,
            StubPieces::Lazy(f) => {
                let mut b = PiecesBuilder::default();
                f(&mut ctx, &mut b);
                b
            }
        };
        if builder.is_empty() {
            return None;
        }
        let adapts = s.terrain_adaptation != TerrainAdjustment::None;
        Some(StructureStart::new(
            structure,
            adapts,
            chunk_x,
            chunk_z,
            0,
            builder.pieces,
        ))
    }

    pub fn structure_id(&self, start: &StructureStart) -> &chunker_data::Identifier {
        &self.registry.structures[start.structure].id
    }
}

/// Whether chunker-worldgen can generate this structure type yet (others are skipped like failed attempts).
pub fn kind_implemented(kind: &StructureKind) -> bool {
    matches!(
        kind,
        StructureKind::BuriedTreasure
            | StructureKind::SwampHut
            | StructureKind::DesertPyramid
            | StructureKind::JungleTemple
            | StructureKind::Jigsaw(_)
            | StructureKind::Mineshaft { .. }
            | StructureKind::Shipwreck { .. }
            | StructureKind::Igloo
            | StructureKind::Stronghold
            | StructureKind::Mansion
            | StructureKind::Fortress
            | StructureKind::EndCity
            | StructureKind::OceanMonument { .. }
            | StructureKind::OceanRuin { .. }
            | StructureKind::RuinedPortal { .. }
            | StructureKind::NetherFossil { .. }
    )
}

/// `ChunkGeneratorStructureState.generateRingPositions`.
fn generate_ring_positions(
    terrain: &TerrainGenerator,
    seed: i64,
    distance: i32,
    spread: i32,
    count: i32,
    preferred: &FxHashSet<BiomeId>,
) -> Vec<(i32, i32)> {
    if count == 0 {
        return Vec::new();
    }
    let mut random = LegacyRandom::new(0);
    random.set_seed(seed);
    let mut angle = random.next_double() * std::f64::consts::PI * 2.0;
    let mut position_in_circle = 0;
    let mut circle = 0;
    let mut spread = spread;
    let mut tasks = Vec::with_capacity(count as usize);
    for i in 0..count {
        let dist = (4 * distance + distance * circle * 6) as f64
            + (random.next_double() - 0.5) * (distance as f64 * 2.5);
        let initial_x = crate::math::round_f64(crate::math::trig::cos(angle) * dist) as i32;
        let initial_z = crate::math::round_f64(crate::math::trig::sin(angle) * dist) as i32;
        let search_random = random.fork();
        tasks.push((initial_x, initial_z, search_random));
        angle += std::f64::consts::PI * 2.0 / spread as f64;
        position_in_circle += 1;
        if position_in_circle == spread {
            circle += 1;
            position_in_circle = 0;
            spread += 2 * spread / (circle + 1);
            spread = spread.min(count - i);
            angle += random.next_double() * std::f64::consts::PI * 2.0;
        }
    }
    // The biome searches are independent (vanilla runs them on the background executor).
    let results: Vec<(i32, i32)> = std::thread::scope(|scope| {
        let threads = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .clamp(1, 16);
        let chunk = tasks.len().div_ceil(threads);
        let handles: Vec<_> = tasks
            .chunks_mut(chunk.max(1))
            .map(|part| {
                scope.spawn(move || {
                    part.iter_mut()
                        .map(|(x, z, r)| {
                            match find_biome_horizontal(
                                terrain,
                                (*x << 4) + 8,
                                0,
                                (*z << 4) + 8,
                                112,
                                preferred,
                                r,
                            ) {
                                Some((bx, bz)) => (bx >> 4, bz >> 4),
                                None => (*x, *z),
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("ring search panicked"))
            .collect()
    });
    results
}

/// `BiomeSource.findBiomeHorizontal(x, y, z, radius, 1, allowed, random, false, randomState)`.
fn find_biome_horizontal(
    terrain: &TerrainGenerator,
    x: i32,
    y: i32,
    z: i32,
    radius: i32,
    allowed: &FxHashSet<BiomeId>,
    random: &mut crate::random::AnyRandom,
) -> Option<(i32, i32)> {
    let (cx, cz, r, qy) = (x >> 2, z >> 2, radius >> 2, y >> 2);
    let mut ctx = SamplerContext::cached(Default::default());
    let mut result = None;
    let mut found = 0;
    for dz in -r..=r {
        for dx in -r..=r {
            let (qx, qz) = (cx + dx, cz + dz);
            let b = terrain
                .biome_source
                .noise_biome(&terrain.climate, &mut ctx, qx, qy, qz);
            if allowed.contains(&b) {
                if result.is_none() || random.next_int_bounded(found + 1) == 0 {
                    result = Some((qx << 2, qz << 2));
                }
                found += 1;
            }
        }
    }
    result
}

/// `Structure.afterPlace` (desert pyramids and mansions override it).
pub fn after_place(
    kind: &registry::StructureKind,
    start: &StructureStart,
    ctx: &mut build::PieceCtx,
    random: &mut dyn crate::random::RandomSource,
) {
    let _ = random;
    match kind {
        registry::StructureKind::Mansion => {
            // `WoodlandMansionStructure.afterPlace`: cobblestone columns under the floor.
            let Some(bb) = geom::BoundingBox::encapsulating(start.pieces.iter().map(|p| p.bbox())) else {
                return;
            };
            let env = ctx.region.env;
            let cobble = env.blocks.block(env.b.cobblestone).default_state;
            let y_start = bb.min_y;
            let cb = ctx.chunk_bb;
            for x in cb.min_x..=cb.max_x {
                for z in cb.min_z..=cb.max_z {
                    let p = geom::BlockPos::new(x, y_start, z);
                    if !ctx.region.is_empty_block(p)
                        && bb.is_inside(x, y_start, z)
                        && start
                            .pieces
                            .iter()
                            .any(|piece| piece.bbox().is_inside(x, y_start, z))
                    {
                        let mut y = y_start - 1;
                        while y > ctx.region.min_y() {
                            let q = geom::BlockPos::new(x, y, z);
                            if !ctx.region.is_empty_block(q) && !env.blocks.flags(ctx.region.get(q)).liquid()
                            {
                                break;
                            }
                            ctx.region.set(q, cobble, 2);
                            y -= 1;
                        }
                    }
                }
            }
        }
        registry::StructureKind::DesertPyramid => scattered::desert_pyramid_after_place(start, ctx),
        _ => {}
    }
}
