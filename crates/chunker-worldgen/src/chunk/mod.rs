//! In-memory proto chunks (`ProtoChunk` / `LevelChunkSection` / `Heightmap`).

mod heightmap;
pub mod serialize;

use std::sync::Arc;

use chunker_data::blocks::{BlockRegistry, BlockState};

use crate::block_set::StateSet;

pub use heightmap::{HeightmapKind, Heightmaps, is_opaque_in};

/// `Heightmap.update` for one heightmap (used by `doFill`, which bypasses `setBlockState`).
pub fn heightmap_update(
    chunk: &mut ProtoChunk,
    kind: HeightmapKind,
    x: i32,
    y: i32,
    z: i32,
    state: BlockState,
) -> bool {
    heightmap::update(chunk, kind, x, y, z, state)
}

use crate::biome::BiomeId;

/// `ChunkStatus` of 26.3 (`terrain` replaced `noise`/`surface`/`carvers`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChunkStatus {
    Empty,
    StructureStarts,
    StructureReferences,
    Biomes,
    Terrain,
    Features,
    InitializeLight,
    Light,
    Spawn,
    Full,
}

impl ChunkStatus {
    pub const ALL: [ChunkStatus; 10] = [
        ChunkStatus::Empty,
        ChunkStatus::StructureStarts,
        ChunkStatus::StructureReferences,
        ChunkStatus::Biomes,
        ChunkStatus::Terrain,
        ChunkStatus::Features,
        ChunkStatus::InitializeLight,
        ChunkStatus::Light,
        ChunkStatus::Spawn,
        ChunkStatus::Full,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ChunkStatus::Empty => "minecraft:empty",
            ChunkStatus::StructureStarts => "minecraft:structure_starts",
            ChunkStatus::StructureReferences => "minecraft:structure_references",
            ChunkStatus::Biomes => "minecraft:biomes",
            ChunkStatus::Terrain => "minecraft:terrain",
            ChunkStatus::Features => "minecraft:features",
            ChunkStatus::InitializeLight => "minecraft:initialize_light",
            ChunkStatus::Light => "minecraft:light",
            ChunkStatus::Spawn => "minecraft:spawn",
            ChunkStatus::Full => "minecraft:full",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        let id = id.strip_prefix("minecraft:").unwrap_or(id);
        Self::ALL
            .iter()
            .copied()
            .find(|s| &s.id()["minecraft:".len()..] == id)
    }

    /// Heightmaps maintained once this status is persisted (`heightmapsAfter`).
    pub fn heightmaps_after(self) -> &'static [HeightmapKind] {
        if self <= ChunkStatus::Biomes {
            &HeightmapKind::WORLDGEN
        } else {
            &HeightmapKind::FINAL
        }
    }
}

/// A 16x16x16 section of block states (`LevelChunkSection`), y-major index `(y << 8) | (z << 4) | x`.
#[derive(Clone)]
pub struct Section {
    blocks: Option<Box<[BlockState; 4096]>>,
    uniform: BlockState,
    non_empty: u16,
    pub biomes: [BiomeId; 64],
}

impl Section {
    fn new() -> Self {
        Self {
            blocks: None,
            uniform: BlockState::AIR,
            non_empty: 0,
            biomes: [BiomeId(0); 64],
        }
    }

    #[inline(always)]
    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockState {
        match &self.blocks {
            Some(b) => b[(y << 8) | (z << 4) | x],
            None => self.uniform,
        }
    }

    /// `LevelChunkSection.setBlockState`, returns the previous state.
    #[inline]
    pub fn set(
        &mut self,
        reg: &BlockRegistry,
        x: usize,
        y: usize,
        z: usize,
        state: BlockState,
    ) -> BlockState {
        let blocks = self.blocks.get_or_insert_with(|| Box::new([self.uniform; 4096]));
        let i = (y << 8) | (z << 4) | x;
        let previous = blocks[i];
        blocks[i] = state;
        if !reg.flags(previous).is_air() {
            self.non_empty -= 1;
        }
        if !reg.flags(state).is_air() {
            self.non_empty += 1;
        }
        previous
    }

    pub fn has_only_air(&self) -> bool {
        self.non_empty == 0
    }

    /// Raw block array (`None` if the section was never written: uniformly [`Section::uniform`]).
    pub fn raw(&self) -> Option<&[BlockState; 4096]> {
        self.blocks.as_deref()
    }

    pub fn uniform(&self) -> BlockState {
        self.uniform
    }

    /// Biome at section-relative quart coordinates.
    #[inline]
    pub fn biome(&self, qx: usize, qy: usize, qz: usize) -> BiomeId {
        self.biomes[(qy << 4) | (qz << 2) | qx]
    }
}

pub struct ProtoChunk {
    pub x: i32,
    pub z: i32,
    pub min_y: i32,
    pub height: i32,
    pub status: ChunkStatus,
    pub sections: Vec<Section>,
    pub heightmaps: Heightmaps,
    /// `postProcessing`: packed `x | y << 4 | z << 8` offsets per section.
    pub post_processing: Vec<Vec<i16>>,
    pub blocks: Arc<BlockRegistry>,
    pub env: Arc<BlockEnv>,
    /// The `structures` compound (`starts` + `References`) to save with the chunk.
    pub structures_nbt: Option<chunker_nbt::Compound>,
    /// Block entities by world position as saved (`getBlockEntityNbtForSaving`): pending `DUMMY`
    /// tags for entities never accessed, full NBT otherwise.
    pub block_entities: std::collections::BTreeMap<(i32, i32, i32), chunker_nbt::Compound>,
    pub block_ticks: Ticks<chunker_data::blocks::BlockId>,
    pub fluid_ticks: Ticks<chunker_data::blocks::Fluid>,
    /// Entity NBT added during generation (`ProtoChunk.addEntity`).
    pub entities: Vec<chunker_nbt::Compound>,
}

/// `ProtoChunkTicks`: scheduled ticks in insertion order, unique per (type, position). Proto chunk
/// ticks are always saved with delay 0.
#[derive(Clone)]
pub struct Ticks<T> {
    pub list: Vec<(T, i32, i32, i32, i32)>,
    seen: rustc_hash::FxHashSet<(T, i32, i32, i32)>,
}

impl<T> Default for Ticks<T> {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            seen: Default::default(),
        }
    }
}

impl<T: Copy + Eq + std::hash::Hash> Ticks<T> {
    /// `schedule(tick)`: (x, y, z) world position, `priority` as `TickPriority.getValue()`.
    pub fn schedule(&mut self, kind: T, x: i32, y: i32, z: i32, priority: i32) {
        if self.seen.insert((kind, x, y, z)) {
            self.list.push((kind, x, y, z, priority));
        }
    }

    pub fn has(&self, kind: T, x: i32, y: i32, z: i32) -> bool {
        self.seen.contains(&(kind, x, y, z))
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// Block data shared by all chunks of a world (registry + tag sets used by chunk bookkeeping).
pub struct BlockEnv {
    pub blocks: Arc<BlockRegistry>,
    /// `#minecraft:blocks_motion_in_heightmap`.
    pub motion_blocking: StateSet,
    /// `#minecraft:blocks_motion_in_heightmap_no_leaves`.
    pub motion_blocking_no_leaves: StateSet,
}

impl ProtoChunk {
    pub fn new(env: Arc<BlockEnv>, x: i32, z: i32, min_y: i32, height: i32) -> Self {
        let count = (height >> 4) as usize;
        Self {
            blocks: env.blocks.clone(),
            env,
            x,
            z,
            min_y,
            height,
            status: ChunkStatus::Empty,
            sections: (0..count).map(|_| Section::new()).collect(),
            heightmaps: Heightmaps::new(min_y, height),
            post_processing: vec![Vec::new(); count],
            structures_nbt: None,
            block_entities: Default::default(),
            block_ticks: Ticks::default(),
            fluid_ticks: Ticks::default(),
            entities: Vec::new(),
        }
    }

    pub fn min_block_x(&self) -> i32 {
        self.x << 4
    }

    pub fn min_block_z(&self) -> i32 {
        self.z << 4
    }

    pub fn max_y(&self) -> i32 {
        self.min_y + self.height - 1
    }

    pub fn min_section_y(&self) -> i32 {
        self.min_y >> 4
    }

    #[inline(always)]
    pub fn section_index(&self, y: i32) -> usize {
        ((y >> 4) - (self.min_y >> 4)) as usize
    }

    #[inline(always)]
    pub fn is_outside_build_height(&self, y: i32) -> bool {
        y < self.min_y || y > self.max_y()
    }

    /// `getBlockState` with chunk-relative or world x/z (only the low 4 bits are used).
    #[inline]
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> BlockState {
        if self.is_outside_build_height(y) {
            return BlockState::AIR;
        }
        self.sections[self.section_index(y)].get((x & 15) as usize, (y & 15) as usize, (z & 15) as usize)
    }

    /// Direct section write without heightmap updates (`section.setBlockState` in `doFill`).
    #[inline]
    pub fn set_block_raw(&mut self, x: i32, y: i32, z: i32, state: BlockState) -> BlockState {
        let si = self.section_index(y);
        // Borrow the registry separately instead of cloning the `Arc`: a refcount bump per block
        // made all worker threads fight over one cache line.
        let Self { sections, blocks, .. } = self;
        sections[si].set(
            blocks,
            (x & 15) as usize,
            (y & 15) as usize,
            (z & 15) as usize,
            state,
        )
    }

    /// `ProtoChunk.setBlockState`: writes and updates the heightmaps of the persisted status.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: BlockState) -> BlockState {
        if self.is_outside_build_height(y) {
            return BlockState::AIR;
        }
        let si = self.section_index(y);
        if self.sections[si].has_only_air() && state == BlockState::AIR {
            return state;
        }
        let old = self.set_block_raw(x, y, z, state);
        for &kind in self.status.heightmaps_after() {
            if !self.heightmaps.is_primed(kind) {
                heightmap::prime(self, &[kind]);
            } else {
                heightmap::update(self, kind, x & 15, y, z & 15, state);
            }
        }
        old
    }

    /// `markPosForPostProcessing`.
    pub fn mark_post_processing(&mut self, x: i32, y: i32, z: i32) {
        if self.is_outside_build_height(y) {
            return;
        }
        let si = self.section_index(y);
        self.post_processing[si].push(((x & 15) | ((y & 15) << 4) | ((z & 15) << 8)) as i16);
    }

    /// `getHeight(type, x, z)` = first available - 1.
    pub fn height_at(&mut self, kind: HeightmapKind, x: i32, z: i32) -> i32 {
        if !self.heightmaps.is_primed(kind) {
            heightmap::prime(self, &[kind]);
        }
        self.heightmaps.first_available(kind, x & 15, z & 15) - 1
    }

    /// `getHighestFilledSectionIndex`, -1 if every section is empty.
    pub fn highest_filled_section_index(&self) -> i32 {
        (0..self.sections.len())
            .rev()
            .find(|&i| !self.sections[i].has_only_air())
            .map_or(-1, |i| i as i32)
    }

    /// `getHighestSectionPosition`.
    pub fn highest_section_position(&self) -> i32 {
        let i = self.highest_filled_section_index();
        if i == -1 {
            self.min_y
        } else {
            (self.min_section_y() + i) << 4
        }
    }

    /// Biome at quart coordinates (absolute), clamped vertically like `ChunkAccess.getNoiseBiome`.
    pub fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId {
        let min_q = self.min_y >> 2;
        let max_q = min_q + (self.height >> 2) - 1;
        let qy = qy.clamp(min_q, max_q);
        let si = self.section_index(qy << 2);
        self.sections[si].biome((qx & 3) as usize, (qy & 3) as usize, (qz & 3) as usize)
    }

    /// `fillBiomesFromNoise` from a chunk biome resolver.
    pub fn fill_biomes(&mut self, resolver: &crate::biome::BiomeResolver) {
        let min_qy = self.min_y >> 2;
        for (si, section) in self.sections.iter_mut().enumerate() {
            let base_qy = min_qy + (si as i32) * 4;
            for x in 0..4 {
                for y in 0..4 {
                    for z in 0..4 {
                        section.biomes[((y << 4) | (z << 2) | x) as usize] =
                            resolver.get(resolver.min_qx + x, base_qy + y, resolver.min_qz + z);
                    }
                }
            }
        }
    }

    pub fn prime_heightmaps(&mut self, kinds: &[HeightmapKind]) {
        heightmap::prime(self, kinds);
    }
}
