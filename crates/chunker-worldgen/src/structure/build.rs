//! `StructurePiece.postProcess` support: the piece building helpers (`placeBlock`, `generateBox`,
//! `createChest`, ...) and `StructureStart.placeInChunk` per decoration step.

use chunker_data::blocks::{BlockId, BlockState};

use super::StructureGenerator;
use super::cache::Reference;
use super::geom::{BlockPos, BoundingBox, Direction};
use super::start::{PieceBase, StructureStart};
use crate::chunk::HeightmapKind;
use crate::feature::Region;
use crate::random::{RandomSource, WorldgenRandom};

/// Context of one `postProcess` call.
pub struct PieceCtx<'r, 'a> {
    pub region: &'r mut Region<'a>,
    pub structures: &'r StructureGenerator,
    /// The writable area of the decorated chunk (`getWritableArea`).
    pub chunk_bb: BoundingBox,
    pub chunk_x: i32,
    pub chunk_z: i32,
    /// `referencePos`: center of the first piece at its minimum y.
    pub reference: BlockPos,
    /// `canBeReplaced` override (mineshafts): `placeBlock` keeps these blocks.
    pub keep: Vec<BlockId>,
}

/// `StructurePiece.BlockSelector`.
pub trait BlockSelector {
    fn next(&mut self, random: &mut dyn RandomSource, x: i32, y: i32, z: i32, edge: bool) -> BlockState;
}

/// `StructurePiece.SHAPE_CHECK_BLOCKS`.
fn is_shape_check_block(ctx: &PieceCtx, block: BlockId) -> bool {
    let b = &ctx.region.env.b;
    [
        b.nether_brick_fence,
        b.torch,
        b.wall_torch,
        b.oak_fence,
        b.spruce_fence,
        b.dark_oak_fence,
        b.pale_oak_fence,
        b.acacia_fence,
        b.birch_fence,
        b.jungle_fence,
        b.ladder,
        b.iron_bars,
    ]
    .contains(&block)
}

impl PieceBase {
    /// `getWorldPos(x, y, z)`.
    pub fn world_pos(&self, x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(self.world_x(x, z), self.world_y(y), self.world_z(x, z))
    }
}

/// A block state being built from a vanilla block name (`Blocks.X.defaultBlockState().setValue(...)`).
#[derive(Clone, Copy)]
pub struct St<'a> {
    blocks: &'a chunker_data::blocks::BlockRegistry,
    pub state: BlockState,
}

impl From<St<'_>> for BlockState {
    fn from(s: St<'_>) -> Self {
        s.state
    }
}

impl St<'_> {
    /// `setValue(property, value)`; panics on a property the block doesn't have (a porting bug).
    pub fn with(self, property: &str, value: &str) -> Self {
        let state = self
            .blocks
            .with_property(self.state, property, value)
            .unwrap_or_else(|| panic!("{} has no {property}={value}", self.blocks.describe(self.state)));
        Self { state, ..self }
    }

    pub fn s(self) -> BlockState {
        self.state
    }

    /// `BlockState.rotate(rotation)`.
    pub fn rotated(self, rotation: super::geom::Rotation) -> Self {
        Self {
            state: self.blocks.rotate(self.state, rotation as u8),
            ..self
        }
    }
}

impl<'a> PieceCtx<'_, 'a> {
    /// `Blocks.<NAME>.defaultBlockState()`.
    pub fn bs(&self, name: &str) -> St<'a> {
        let env: &'a crate::feature::FeatureEnv = self.region.env;
        St {
            blocks: &env.blocks,
            state: env.blocks.vanilla_default(name),
        }
    }
}

impl PieceCtx<'_, '_> {
    pub fn state(&self, block: BlockId) -> BlockState {
        self.region.env.blocks.block(block).default_state
    }

    /// `placeBlock(level, state, x, y, z, chunkBB)`.
    pub fn place_block(&mut self, b: &PieceBase, state: impl Into<BlockState>, x: i32, y: i32, z: i32) {
        self.place_block_checked(b, state.into(), x, y, z, &mut |_, _| true);
    }

    /// `placeBlock` with an overridden `canBeReplaced(level, x, y, z, chunkBB)`.
    pub fn place_block_checked(
        &mut self,
        b: &PieceBase,
        mut state: BlockState,
        x: i32,
        y: i32,
        z: i32,
        can_be_replaced: &mut dyn FnMut(&mut Self, BlockPos) -> bool,
    ) {
        let pos = b.world_pos(x, y, z);
        if !self.chunk_bb.is_inside(pos.x, pos.y, pos.z) || !can_be_replaced(self, pos) {
            return;
        }
        if !self.keep.is_empty() && self.keep.contains(&self.region.block_of(pos)) {
            return;
        }
        let blocks = &self.region.env.blocks;
        let mirror = b.mirror();
        if mirror != super::geom::Mirror::None {
            state = blocks.mirror(state, mirror as u8);
        }
        let rotation = b.rotation();
        if rotation != super::geom::Rotation::None {
            state = blocks.rotate(state, rotation as u8);
        }
        self.region.set(pos, state, 2);
        let fluid = self.region.fluid(pos);
        if fluid != chunker_data::blocks::Fluid::Empty {
            self.region.schedule_fluid_tick(pos, fluid);
        }
        if is_shape_check_block(self, blocks.block_of(state)) {
            self.region.mark_post_processing(pos);
        }
    }

    /// `getBlock(level, x, y, z, chunkBB)`: air outside the chunk box.
    pub fn get_block(&self, b: &PieceBase, x: i32, y: i32, z: i32) -> BlockState {
        let pos = b.world_pos(x, y, z);
        if self.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
            self.region.get(pos)
        } else {
            BlockState::AIR
        }
    }

    pub fn is_air_at(&self, b: &PieceBase, x: i32, y: i32, z: i32) -> bool {
        self.region.env.blocks.flags(self.get_block(b, x, y, z)).is_air()
    }

    /// `isInterior(level, x, y, z, chunkBB)`.
    pub fn is_interior(&mut self, b: &PieceBase, x: i32, y: i32, z: i32) -> bool {
        let pos = b.world_pos(x, y + 1, z);
        if !self.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
            return false;
        }
        pos.y < self.region.height(HeightmapKind::OceanFloorWg, pos.x, pos.z)
    }

    /// `generateAirBox`.
    pub fn generate_air_box(&mut self, b: &PieceBase, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    self.place_block(b, BlockState::AIR, x, y, z);
                }
            }
        }
    }

    /// `generateBox(level, chunkBB, x0, y0, z0, x1, y1, z1, edge, fill, skipAir)`.
    pub fn generate_box(
        &mut self,
        b: &PieceBase,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        edge: impl Into<BlockState>,
        fill: impl Into<BlockState>,
        skip_air: bool,
    ) {
        let (edge, fill) = (edge.into(), fill.into());
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if !skip_air || !self.is_air_at(b, x, y, z) {
                        let inner = y != y0 && y != y1 && x != x0 && x != x1 && z != z0 && z != z1;
                        self.place_block(b, if inner { fill } else { edge }, x, y, z);
                    }
                }
            }
        }
    }

    pub fn generate_box_bb(
        &mut self,
        b: &PieceBase,
        bb: &BoundingBox,
        edge: impl Into<BlockState>,
        fill: impl Into<BlockState>,
        skip_air: bool,
    ) {
        self.generate_box(
            b, bb.min_x, bb.min_y, bb.min_z, bb.max_x, bb.max_y, bb.max_z, edge, fill, skip_air,
        );
    }

    /// `generateBox(..., skipAir, random, selector)`.
    pub fn generate_box_selector(
        &mut self,
        b: &PieceBase,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        skip_air: bool,
        random: &mut dyn RandomSource,
        selector: &mut dyn BlockSelector,
    ) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if !skip_air || !self.is_air_at(b, x, y, z) {
                        let edge = y == y0 || y == y1 || x == x0 || x == x1 || z == z0 || z == z1;
                        let s = selector.next(random, x, y, z, edge);
                        self.place_block(b, s, x, y, z);
                    }
                }
            }
        }
    }

    /// `generateMaybeBox`.
    pub fn generate_maybe_box(
        &mut self,
        b: &PieceBase,
        random: &mut dyn RandomSource,
        probability: f32,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        edge: impl Into<BlockState>,
        fill: impl Into<BlockState>,
        skip_air: bool,
        has_to_be_inside: bool,
    ) {
        let (edge, fill) = (edge.into(), fill.into());
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if !(random.next_float() > probability)
                        && (!skip_air || !self.is_air_at(b, x, y, z))
                        && (!has_to_be_inside || self.is_interior(b, x, y, z))
                    {
                        let inner = y != y0 && y != y1 && x != x0 && x != x1 && z != z0 && z != z1;
                        self.place_block(b, if inner { fill } else { edge }, x, y, z);
                    }
                }
            }
        }
    }

    /// `maybeGenerateBlock`.
    pub fn maybe_generate_block(
        &mut self,
        b: &PieceBase,
        random: &mut dyn RandomSource,
        probability: f32,
        x: i32,
        y: i32,
        z: i32,
        state: impl Into<BlockState>,
    ) {
        let state = state.into();
        if random.next_float() < probability {
            self.place_block(b, state, x, y, z);
        }
    }

    /// `generateUpperHalfSphere`.
    pub fn generate_upper_half_sphere(
        &mut self,
        b: &PieceBase,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        fill: impl Into<BlockState>,
        skip_air: bool,
    ) {
        let fill = fill.into();
        let diag_x = (x1 - x0 + 1) as f32;
        let diag_y = (y1 - y0 + 1) as f32;
        let diag_z = (z1 - z0 + 1) as f32;
        let cx = x0 as f32 + diag_x / 2.0;
        let cz = z0 as f32 + diag_z / 2.0;
        for y in y0..=y1 {
            let ny = (y - y0) as f32 / diag_y;
            for x in x0..=x1 {
                let nx = (x as f32 - cx) / (diag_x * 0.5);
                for z in z0..=z1 {
                    let nz = (z as f32 - cz) / (diag_z * 0.5);
                    if !skip_air || !self.is_air_at(b, x, y, z) {
                        let dist = nx * nx + ny * ny + nz * nz;
                        if dist <= 1.05 {
                            self.place_block(b, fill, x, y, z);
                        }
                    }
                }
            }
        }
    }

    /// `isReplaceableByStructures`.
    pub fn is_replaceable_by_structures(&self, state: BlockState) -> bool {
        let env = self.region.env;
        let f = env.blocks.flags(state);
        let block = env.blocks.block_of(state);
        f.is_air()
            || f.liquid()
            || block == env.b.glow_lichen
            || block == env.b.seagrass
            || block == env.b.tall_seagrass
    }

    /// `fillColumnDown`.
    pub fn fill_column_down(
        &mut self,
        b: &PieceBase,
        state: impl Into<BlockState>,
        x: i32,
        start_y: i32,
        z: i32,
    ) {
        let state = state.into();
        let mut pos = b.world_pos(x, start_y, z);
        if !self.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
            return;
        }
        while self.is_replaceable_by_structures(self.region.get(pos)) && pos.y > self.region.min_y() + 1 {
            self.region.set(pos, state, 2);
            pos = pos.below();
        }
    }

    /// `createChest(level, chunkBB, random, pos, lootTable, state)`.
    pub fn create_chest_at(
        &mut self,
        random: &mut dyn RandomSource,
        pos: BlockPos,
        loot_table: &str,
        state: Option<BlockState>,
    ) -> bool {
        if !self.chunk_bb.is_inside(pos.x, pos.y, pos.z) || self.region.is(pos, self.region.env.b.chest) {
            return false;
        }
        let state = state.unwrap_or_else(|| {
            let chest = self.state(self.region.env.b.chest);
            crate::feature::kinds::reorient(self.region, pos, chest)
        });
        self.region.set(pos, state, 2);
        crate::feature::kinds::set_loot_table(self.region, random, pos, loot_table);
        true
    }

    /// `createChest(level, chunkBB, random, x, y, z, lootTable)`.
    pub fn create_chest(
        &mut self,
        b: &PieceBase,
        random: &mut dyn RandomSource,
        x: i32,
        y: i32,
        z: i32,
        loot_table: &str,
    ) -> bool {
        let pos = b.world_pos(x, y, z);
        self.create_chest_at(random, pos, loot_table, None)
    }

    /// `createDispenser`.
    pub fn create_dispenser(
        &mut self,
        b: &PieceBase,
        random: &mut dyn RandomSource,
        x: i32,
        y: i32,
        z: i32,
        facing: Direction,
        loot_table: &str,
    ) -> bool {
        let pos = b.world_pos(x, y, z);
        let dispenser = self.region.env.b.dispenser;
        if !self.chunk_bb.is_inside(pos.x, pos.y, pos.z) || self.region.is(pos, dispenser) {
            return false;
        }
        let blocks = &self.region.env.blocks;
        let state = blocks
            .with_property(blocks.block(dispenser).default_state, "facing", facing.name())
            .unwrap();
        self.place_block(b, state, x, y, z);
        crate::feature::kinds::set_loot_table(self.region, random, pos, loot_table);
        true
    }
}

/// The decorated chunk's writable area (`ChunkGenerator.getWritableArea`).
pub fn writable_area(region: &Region) -> BoundingBox {
    let (x, z) = (region.center_x << 4, region.center_z << 4);
    BoundingBox::new(x, region.min_y() + 1, z, x + 15, region.max_y(), z + 15)
}

impl StructureStart {
    /// `StructureStart.placeInChunk`. `chunk_bb` is the decoration's writable area, shared by all
    /// starts of the chunk (ruined portals and nether fossils enlarge it, like vanilla).
    pub fn place_in_chunk(
        &self,
        region: &mut Region,
        structures: &StructureGenerator,
        random: &mut dyn RandomSource,
        chunk_bb: &mut BoundingBox,
    ) {
        let Some(first) = self.pieces.first() else { return };
        let center_bb = first.bbox();
        let c = center_bb.center();
        let mut ctx = PieceCtx {
            chunk_x: region.center_x,
            chunk_z: region.center_z,
            region,
            structures,
            chunk_bb: *chunk_bb,
            reference: BlockPos::new(c.x, center_bb.min_y, c.z),
            keep: Vec::new(),
        };
        for p in &self.pieces {
            if p.bbox().intersects(&ctx.chunk_bb) {
                p.post_process(&mut ctx, random);
            }
        }
        let kind = &structures.registry.structures[self.structure].kind;
        super::after_place(kind, self, &mut ctx, random);
        *chunk_bb = ctx.chunk_bb;
    }
}

/// `HashCommon.mix(long)`.
fn mix(x: i64) -> i64 {
    let h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64);
    let h = h ^ ((h as u64) >> 32) as i64;
    h ^ ((h as u64) >> 16) as i64
}

/// Iteration order of a fastutil `LongOpenHashSet` (default capacity, load factor 0.75) filled with
/// `keys` in order: the 0 key first, then table slots from high to low.
pub fn long_open_hash_set_order(keys: &[i64]) -> Vec<i64> {
    let array_size = |expected: usize| -> usize {
        ((expected as f64 / 0.75).ceil() as usize)
            .next_power_of_two()
            .max(2)
    };
    let max_fill = |n: usize| -> usize { ((n as f64 * 0.75).ceil() as usize).min(n - 1) };
    let mut n = array_size(16);
    let mut table = vec![0i64; n];
    let mut size = 0usize;
    let mut contains_null = false;
    let insert = |table: &mut Vec<i64>, n: usize, k: i64| -> bool {
        let mask = n - 1;
        let mut pos = (mix(k) as i32 as usize) & mask;
        while table[pos] != 0 {
            if table[pos] == k {
                return false;
            }
            pos = (pos + 1) & mask;
        }
        table[pos] = k;
        true
    };
    for &k in keys {
        if k == 0 {
            if contains_null {
                continue;
            }
            contains_null = true;
        } else if !insert(&mut table, n, k) {
            continue;
        }
        let before = size;
        size += 1;
        if before >= max_fill(n) {
            // `rehash(arraySize(size + 1, f))`: old slots from high to low into the new table.
            let new_n = array_size(size + 1);
            let mut new_table = vec![0i64; new_n];
            for &v in table.iter().rev() {
                if v != 0 {
                    insert(&mut new_table, new_n, v);
                }
            }
            table = new_table;
            n = new_n;
        }
    }
    let mut out = Vec::with_capacity(size);
    if contains_null {
        out.push(0);
    }
    out.extend(table.iter().rev().copied().filter(|&v| v != 0));
    out
}

/// The structures part of `applyBiomeDecoration` for one step: every structure of the step in
/// registry order gets `setFeatureSeed(decorationSeed, index, step)`, then its starts referenced by
/// the chunk place their pieces. `filter` restricts the placed structures (oracle tests).
pub fn decorate_step(
    region: &mut Region,
    structures: &StructureGenerator,
    references: &[Reference],
    step: usize,
    random: &mut WorldgenRandom,
    decoration_seed: i64,
    chunk_bb: &mut BoundingBox,
    filter: Option<&dyn Fn(&str) -> bool>,
) {
    let reg = &structures.registry;
    region.structure_pieces = references
        .iter()
        .map(|r| {
            let boxes = r.start().pieces.iter().map(|p| p.current_bbox()).collect();
            (reg.structures[r.structure].id.clone(), boxes)
        })
        .collect();
    let mut index = 0;
    for (si, s) in reg.structures.iter().enumerate() {
        if s.step as usize != step {
            continue;
        }
        random.set_feature_seed(decoration_seed, index, step as i32);
        index += 1;
        if filter.is_some_and(|f| !f(s.id.as_str())) {
            continue;
        }
        let keys: Vec<i64> = references
            .iter()
            .filter(|r| r.structure == si)
            .map(|r| (r.chunk_x as u32 as i64) | ((r.chunk_z as u32 as i64) << 32))
            .collect();
        for key in long_open_hash_set_order(&keys) {
            let (x, z) = (key as i32, (key >> 32) as i32);
            if let Some(r) = references
                .iter()
                .find(|r| r.structure == si && r.chunk_x == x && r.chunk_z == z)
            {
                r.start().place_in_chunk(region, structures, random, chunk_bb);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_set_order_puts_zero_first_and_scans_down() {
        let order = long_open_hash_set_order(&[5, 0, 7]);
        assert_eq!(order[0], 0);
        assert_eq!(order.len(), 3);
        let many: Vec<i64> = (1..100).collect();
        let order = long_open_hash_set_order(&many);
        assert_eq!(order.len(), 99);
    }
}
