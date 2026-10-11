//! `WorldGenRegion` of the `features` step: the decorated chunk and its 8 neighbours (all at
//! `terrain` or later), write radius 1.
//!
//! Vanilla's region cache also holds chunks out to distance 8 at `structure_starts`: they have no
//! blocks, so reads there see air (and heightmaps the bottom of the world). Writes there are refused.

use chunker_data::blocks::{BlockId, BlockState, Fluid};
use chunker_nbt::Compound;

use super::FeatureEnv;
use crate::biome::BiomeId;
use crate::chunk::{HeightmapKind, ProtoChunk};
use crate::random::AnyRandom;
use crate::structure::geom::{BlockPos, BoundingBox, Direction};

/// `Block.UPDATE_*` flags used by worldgen.
pub mod flags {
    pub const NEIGHBORS: u32 = 1;
    pub const CLIENTS: u32 = 2;
    pub const KNOWN_SHAPE: u32 = 16;
    pub const ALL: u32 = 3;
}

pub struct Region<'a> {
    pub env: &'a FeatureEnv,
    chunks: Vec<&'a mut ProtoChunk>,
    /// False for chunks loaded from disk: vanilla would write into them, we never modify them.
    writable: [bool; 9],
    pub center_x: i32,
    pub center_z: i32,
    /// `level.getRandom()`.
    pub random: AnyRandom,
    /// Debug log of every `setBlock` call (`x,y,z=state`), compared with the oracle's.
    pub log: Option<Vec<String>>,
    /// Structures referenced by the center chunk with their piece boxes
    /// (`structureManager().getStructureWithPieceAt`, used by spawn conditions).
    pub structure_pieces: Vec<(chunker_data::Identifier, Vec<BoundingBox>)>,
}

impl<'a> Region<'a> {
    /// `chunks` in z-major order from (-1, -1) to (1, 1).
    pub fn new(
        env: &'a FeatureEnv,
        center_x: i32,
        center_z: i32,
        chunks: Vec<&'a mut ProtoChunk>,
        writable: [bool; 9],
    ) -> Self {
        assert_eq!(chunks.len(), 9);
        let random = env.region_random.at(center_x << 4, 0, center_z << 4);
        Self {
            env,
            chunks,
            writable,
            center_x,
            center_z,
            random,
            log: None,
            structure_pieces: Vec::new(),
        }
    }

    pub fn for_each_chunk(&self, mut f: impl FnMut(&ProtoChunk)) {
        for c in &self.chunks {
            f(c);
        }
    }

    #[inline]
    fn slot(&self, x: i32, z: i32) -> Option<usize> {
        let dx = (x >> 4) - self.center_x;
        let dz = (z >> 4) - self.center_z;
        if (-1..=1).contains(&dx) && (-1..=1).contains(&dz) {
            Some(((dz + 1) * 3 + dx + 1) as usize)
        } else {
            None
        }
    }

    pub fn min_y(&self) -> i32 {
        self.env.min_y
    }

    pub fn max_y(&self) -> i32 {
        self.env.min_y + self.env.height - 1
    }

    #[inline]
    pub fn is_outside_build_height(&self, y: i32) -> bool {
        y < self.env.min_y || y > self.max_y()
    }

    /// `getBlockState`.
    #[inline]
    pub fn get(&self, p: BlockPos) -> BlockState {
        if self.is_outside_build_height(p.y) {
            return self.env.b.void_air_state;
        }
        match self.slot(p.x, p.z) {
            Some(i) => self.chunks[i].get_block(p.x, p.y, p.z),
            None => BlockState::AIR,
        }
    }

    #[inline]
    pub fn flags(&self, p: BlockPos) -> chunker_data::blocks::BlockFlags {
        self.env.blocks.flags(self.get(p))
    }

    /// `getFluidState(pos).getType()`.
    #[inline]
    pub fn fluid(&self, p: BlockPos) -> Fluid {
        self.flags(p).fluid()
    }

    /// `isEmptyBlock`.
    #[inline]
    pub fn is_empty_block(&self, p: BlockPos) -> bool {
        self.flags(p).is_air()
    }

    #[inline]
    pub fn block_of(&self, p: BlockPos) -> BlockId {
        self.env.blocks.block_of(self.get(p))
    }

    #[inline]
    pub fn is(&self, p: BlockPos, block: BlockId) -> bool {
        self.block_of(p) == block
    }

    /// `ensureCanWrite`.
    pub fn ensure_can_write(&self, p: BlockPos) -> bool {
        self.slot(p.x, p.z).is_some()
    }

    /// The writable chunk holding `p`, if any.
    fn chunk_mut(&mut self, p: BlockPos) -> Option<&mut ProtoChunk> {
        let i = self.slot(p.x, p.z)?;
        if !self.writable[i] {
            return None;
        }
        Some(&mut *self.chunks[i])
    }

    /// `WorldGenRegion.setBlock(pos, state, flags)`.
    pub fn set(&mut self, p: BlockPos, state: BlockState, update_flags: u32) -> bool {
        if let Some(log) = self.log.as_mut() {
            log.push(format!("{},{},{}={}", p.x, p.y, p.z, state.0));
        }
        if !self.ensure_can_write(p) {
            return false;
        }
        let env = self.env;
        let Some(chunk) = self.chunk_mut(p) else {
            return true;
        };
        if chunk.is_outside_build_height(p.y) {
            return true;
        }
        let si = chunk.section_index(p.y);
        if chunk.sections[si].has_only_air() && state == BlockState::AIR {
            // `ProtoChunk.setBlockState` returns early (no old state, no block entity changes).
        } else {
            let old = chunk.set_block(p.x, p.y, p.z, state);
            let reg = &env.blocks;
            if reg.flags(state).has_block_entity() {
                let mut tag = Compound::with_capacity(4);
                tag.insert("x", p.x);
                tag.insert("y", p.y);
                tag.insert("z", p.z);
                tag.insert("id", "DUMMY");
                // `setBlockEntityNbt` keeps an existing (real) block entity at the position.
                if !chunk
                    .block_entities
                    .get(&(p.x, p.y, p.z))
                    .is_some_and(|t| !is_dummy(t))
                {
                    chunk.block_entities.insert((p.x, p.y, p.z), tag);
                }
            } else if reg.flags(old).has_block_entity() {
                chunk.block_entities.remove(&(p.x, p.y, p.z));
            }
        }
        if update_flags & flags::KNOWN_SHAPE == 0
            && let Some(pp) = super::behaviour::post_process_pos(env, state, p)
        {
            self.mark_post_processing(pp);
        }
        true
    }

    /// `setBlockAndUpdate` (flags 3).
    pub fn set_and_update(&mut self, p: BlockPos, state: BlockState) -> bool {
        self.set(p, state, flags::ALL)
    }

    /// Direct section write without heightmaps, block entities or post-processing
    /// (`LevelChunkSection.setBlockState` through `BulkSectionAccess`).
    pub fn set_section_raw(&mut self, p: BlockPos, state: BlockState) {
        if let Some(chunk) = self.chunk_mut(p)
            && !chunk.is_outside_build_height(p.y)
        {
            chunk.set_block_raw(p.x, p.y, p.z, state);
        }
    }

    /// `getHeight(type, x, z)` (first free y).
    pub fn height(&mut self, kind: HeightmapKind, x: i32, z: i32) -> i32 {
        match self.slot(x, z) {
            Some(i) => self.chunks[i].height_at(kind, x, z) + 1,
            None => self.env.min_y,
        }
    }

    /// `getBiome(pos)` through the region's `BiomeManager`.
    pub fn biome(&self, p: BlockPos) -> BiomeId {
        let env = self.env;
        let min_q = env.min_y >> 2;
        let max_q = min_q + (env.height >> 2) - 1;
        env.biome_manager.get_biome(p.x, p.y, p.z, &mut |qx, qy, qz| {
            let x = qx << 2;
            let z = qz << 2;
            match self.slot(x, z) {
                Some(i) => self.chunks[i].noise_biome(qx, qy.clamp(min_q, max_q), qz),
                // Vanilla would crash here (biomes beyond the 3x3 are not available).
                None => {
                    let i = self
                        .slot(
                            x.clamp((self.center_x - 1) << 4, ((self.center_x + 1) << 4) + 15),
                            z.clamp((self.center_z - 1) << 4, ((self.center_z + 1) << 4) + 15),
                        )
                        .unwrap();
                    self.chunks[i].noise_biome(qx, qy.clamp(min_q, max_q), qz)
                }
            }
        })
    }

    /// `scheduleTick(pos, block, delay)` (priority normal).
    pub fn schedule_block_tick(&mut self, p: BlockPos, block: BlockId) {
        if let Some(chunk) = self.chunk_mut(p) {
            chunk.block_ticks.schedule(block, p.x, p.y, p.z, 0);
        }
    }

    /// `scheduleTick(pos, fluid, delay)`.
    pub fn schedule_fluid_tick(&mut self, p: BlockPos, fluid: Fluid) {
        if let Some(chunk) = self.chunk_mut(p) {
            chunk.fluid_ticks.schedule(fluid, p.x, p.y, p.z, 0);
        }
    }

    /// `getChunk(pos).markPosForPostProcessing(pos)`.
    pub fn mark_post_processing(&mut self, p: BlockPos) {
        if let Some(chunk) = self.chunk_mut(p) {
            chunk.mark_post_processing(p.x, p.y, p.z);
        }
    }

    /// `getBlockEntity(pos)`: materializes a pending `DUMMY` entry into the block entity's saved
    /// form, so callers can edit its fields.
    pub fn block_entity(&mut self, p: BlockPos) -> Option<&mut Compound> {
        let state = self.get(p);
        let env = self.env;
        let chunk = self.chunk_mut(p)?;
        let key = (p.x, p.y, p.z);
        let tag = chunk.block_entities.get(&key)?;
        if is_dummy(tag) {
            if !env.blocks.flags(state).has_block_entity() {
                return None;
            }
            let tag = super::behaviour::new_block_entity(env, state, p)?;
            chunk.block_entities.insert(key, tag);
        }
        chunk.block_entities.get_mut(&key)
    }

    /// A piece moved its box during `postProcess` (scattered pieces settling on the ground).
    pub fn move_structure_piece(&mut self, old: BoundingBox, new: BoundingBox) {
        for (_, boxes) in &mut self.structure_pieces {
            for b in boxes.iter_mut().filter(|b| **b == old) {
                *b = new;
            }
        }
    }

    /// Adds entity NBT to the chunk holding the entity position (`addFreshEntity`).
    pub fn add_entity(&mut self, x: i32, z: i32, tag: Compound) {
        let i = self.slot(x, z).expect("entity outside the region");
        if self.writable[i] {
            self.chunks[i].entities.push(tag);
        }
    }

    /// Sky light at a position: chunks in the `features` step have no light data yet; vanilla's
    /// sky light storage then answers 15 (dimensions with sky light), block light answers 0.
    pub fn raw_brightness(&self, _p: BlockPos, darkening: i32) -> i32 {
        if self.env.has_skylight {
            (15 - darkening).max(0)
        } else {
            0
        }
    }

    /// `getBrightness(LightLayer.BLOCK, pos)`.
    pub fn block_light(&self, _p: BlockPos) -> i32 {
        0
    }

    pub fn relative(p: BlockPos, d: Direction) -> BlockPos {
        p.relative(d, 1)
    }
}

fn is_dummy(tag: &Compound) -> bool {
    matches!(tag.get("id"), Some(chunker_nbt::Tag::String(s)) if s == "DUMMY")
}
