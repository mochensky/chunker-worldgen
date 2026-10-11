//! `postProcess` of the `ScatteredFeaturePiece` family: swamp hut, desert pyramid, jungle temple
//! (bodies translated statement by statement from vanilla).

use chunker_data::blocks::BlockState;

use super::build::{BlockSelector, PieceCtx};
use super::geom::{BlockPos, Direction, Rotation};
use super::simple::{ScatteredKind, ScatteredPiece};
use super::start::PieceBase;
use crate::chunk::HeightmapKind;
use crate::random::RandomSource;

/// State `postProcess` changes on the shared piece (vanilla mutates the piece object).
#[derive(Clone, Copy, Debug)]
pub struct ScatteredLive {
    pub height_position: i32,
    pub bbox: super::geom::BoundingBox,
    /// Chests/traps/mobs already placed (meaning per kind).
    pub flags: [bool; 4],
}

impl ScatteredPiece {
    /// `updateAverageGroundHeight(level, chunkBB, offset)`.
    fn update_average_ground_height(live: &mut ScatteredLive, ctx: &mut PieceCtx, offset: i32) -> bool {
        if live.height_position >= 0 {
            return true;
        }
        let (mut total, mut count) = (0, 0);
        let bb = live.bbox;
        for z in bb.min_z..=bb.max_z {
            for x in bb.min_x..=bb.max_x {
                if ctx.chunk_bb.is_inside(x, 64, z) {
                    total += ctx.region.height(HeightmapKind::MotionBlockingNoLeaves, x, z);
                    count += 1;
                }
            }
        }
        if count == 0 {
            return false;
        }
        live.height_position = total / count;
        let old = live.bbox;
        live.bbox
            .move_by(0, live.height_position - live.bbox.min_y + offset, 0);
        ctx.region.move_structure_piece(old, live.bbox);
        true
    }

    /// `updateHeightPositionToLowestGroundHeight(level, offset)`.
    fn update_to_lowest_ground_height(live: &mut ScatteredLive, ctx: &mut PieceCtx, offset: i32) -> bool {
        if live.height_position >= 0 {
            return true;
        }
        let mut lowest = ctx.region.max_y() + 1;
        let bb = live.bbox;
        for z in bb.min_z..=bb.max_z {
            for x in bb.min_x..=bb.max_x {
                lowest = lowest.min(ctx.region.height(HeightmapKind::MotionBlockingNoLeaves, x, z));
            }
        }
        live.height_position = lowest;
        let old = live.bbox;
        live.bbox
            .move_by(0, live.height_position - live.bbox.min_y + offset, 0);
        ctx.region.move_structure_piece(old, live.bbox);
        true
    }

    pub(super) fn post_process_scattered(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        let mut guard = self.live.lock();
        let live = guard.get_or_insert_with(|| ScatteredLive {
            height_position: self.height_position,
            bbox: self.base.bbox,
            flags: match self.kind {
                ScatteredKind::SwampHut { witch, cat } => [witch, cat, false, false],
                ScatteredKind::DesertPyramid { chests } => chests,
                ScatteredKind::JungleTemple {
                    main_chest,
                    hidden_chest,
                    trap1,
                    trap2,
                } => [main_chest, trap1, trap2, hidden_chest],
            },
        });
        let (w, d) = (self.width, self.depth);
        match self.kind {
            ScatteredKind::SwampHut { .. } => {
                if !Self::update_average_ground_height(live, ctx, 0) {
                    return;
                }
                let base = PieceBase {
                    bbox: live.bbox,
                    ..self.base.clone()
                };
                let b = &base;
                ctx.generate_box(
                    b,
                    1,
                    1,
                    1,
                    5,
                    1,
                    7,
                    ctx.bs("spruce_planks"),
                    ctx.bs("spruce_planks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    4,
                    2,
                    5,
                    4,
                    7,
                    ctx.bs("spruce_planks"),
                    ctx.bs("spruce_planks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    1,
                    0,
                    4,
                    1,
                    0,
                    ctx.bs("spruce_planks"),
                    ctx.bs("spruce_planks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    2,
                    2,
                    3,
                    3,
                    2,
                    ctx.bs("spruce_planks"),
                    ctx.bs("spruce_planks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    2,
                    3,
                    1,
                    3,
                    6,
                    ctx.bs("spruce_planks"),
                    ctx.bs("spruce_planks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    2,
                    3,
                    5,
                    3,
                    6,
                    ctx.bs("spruce_planks"),
                    ctx.bs("spruce_planks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    2,
                    7,
                    4,
                    3,
                    7,
                    ctx.bs("spruce_planks"),
                    ctx.bs("spruce_planks"),
                    false,
                );
                ctx.generate_box(b, 1, 0, 2, 1, 3, 2, ctx.bs("oak_log"), ctx.bs("oak_log"), false);
                ctx.generate_box(b, 5, 0, 2, 5, 3, 2, ctx.bs("oak_log"), ctx.bs("oak_log"), false);
                ctx.generate_box(b, 1, 0, 7, 1, 3, 7, ctx.bs("oak_log"), ctx.bs("oak_log"), false);
                ctx.generate_box(b, 5, 0, 7, 5, 3, 7, ctx.bs("oak_log"), ctx.bs("oak_log"), false);
                ctx.place_block(b, ctx.bs("oak_fence"), 2, 3, 2);
                ctx.place_block(b, ctx.bs("oak_fence"), 3, 3, 7);
                ctx.place_block(b, ctx.bs("air"), 1, 3, 4);
                ctx.place_block(b, ctx.bs("air"), 5, 3, 4);
                ctx.place_block(b, ctx.bs("air"), 5, 3, 5);
                ctx.place_block(b, ctx.bs("potted_red_mushroom"), 1, 3, 5);
                ctx.place_block(b, ctx.bs("crafting_table"), 3, 2, 6);
                ctx.place_block(b, ctx.bs("cauldron"), 4, 2, 6);
                ctx.place_block(b, ctx.bs("oak_fence"), 1, 2, 1);
                ctx.place_block(b, ctx.bs("oak_fence"), 5, 2, 1);
                let north_stairs = ctx.bs("spruce_stairs").with("facing", "north");
                let east_stairs = ctx.bs("spruce_stairs").with("facing", "east");
                let west_stairs = ctx.bs("spruce_stairs").with("facing", "west");
                let south_stairs = ctx.bs("spruce_stairs").with("facing", "south");
                ctx.generate_box(b, 0, 4, 1, 6, 4, 1, north_stairs, north_stairs, false);
                ctx.generate_box(b, 0, 4, 2, 0, 4, 7, east_stairs, east_stairs, false);
                ctx.generate_box(b, 6, 4, 2, 6, 4, 7, west_stairs, west_stairs, false);
                ctx.generate_box(b, 0, 4, 8, 6, 4, 8, south_stairs, south_stairs, false);
                ctx.place_block(b, north_stairs.with("shape", "outer_right"), 0, 4, 1);
                ctx.place_block(b, north_stairs.with("shape", "outer_left"), 6, 4, 1);
                ctx.place_block(b, south_stairs.with("shape", "outer_left"), 0, 4, 8);
                ctx.place_block(b, south_stairs.with("shape", "outer_right"), 6, 4, 8);

                for z in (2..=7).step_by(5) {
                    for x in (1..=5).step_by(4) {
                        ctx.fill_column_down(b, ctx.bs("oak_log"), x, -1, z);
                    }
                }
                let pos = b.world_pos(2, 2, 5);
                for (i, id) in [(0, "minecraft:witch"), (1, "minecraft:cat")] {
                    if !live.flags[i] && ctx.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
                        live.flags[i] = true;
                        crate::entity::spawn_mob(ctx.region, super::templated::structure_mob(id, pos));
                    }
                }
            }
            ScatteredKind::DesertPyramid { .. } => {
                let offset = -random.next_int_bounded(3);
                if !Self::update_to_lowest_ground_height(live, ctx, offset) {
                    return;
                }
                let base = PieceBase {
                    bbox: live.bbox,
                    ..self.base.clone()
                };
                let b = &base;
                ctx.generate_box(
                    b,
                    0,
                    -4,
                    0,
                    w - 1,
                    0,
                    d - 1,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );

                for pos in 1..=9 {
                    ctx.generate_box(
                        b,
                        pos,
                        pos,
                        pos,
                        w - 1 - pos,
                        pos,
                        d - 1 - pos,
                        ctx.bs("sandstone"),
                        ctx.bs("sandstone"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        pos + 1,
                        pos,
                        pos + 1,
                        w - 2 - pos,
                        pos,
                        d - 2 - pos,
                        ctx.bs("air"),
                        ctx.bs("air"),
                        false,
                    );
                }

                for x in 0..w {
                    for z in 0..d {
                        ctx.fill_column_down(b, ctx.bs("sandstone"), x, -5, z);
                    }
                }

                let north_stairs = ctx.bs("sandstone_stairs").with("facing", "north");
                let south_stairs = ctx.bs("sandstone_stairs").with("facing", "south");
                let east_stairs = ctx.bs("sandstone_stairs").with("facing", "east");
                let west_stairs = ctx.bs("sandstone_stairs").with("facing", "west");
                ctx.generate_box(b, 0, 0, 0, 4, 9, 4, ctx.bs("sandstone"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    1,
                    10,
                    1,
                    3,
                    10,
                    3,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.place_block(b, north_stairs, 2, 10, 0);
                ctx.place_block(b, south_stairs, 2, 10, 4);
                ctx.place_block(b, east_stairs, 0, 10, 2);
                ctx.place_block(b, west_stairs, 4, 10, 2);
                ctx.generate_box(
                    b,
                    w - 5,
                    0,
                    0,
                    w - 1,
                    9,
                    4,
                    ctx.bs("sandstone"),
                    ctx.bs("air"),
                    false,
                );
                ctx.generate_box(
                    b,
                    w - 4,
                    10,
                    1,
                    w - 2,
                    10,
                    3,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.place_block(b, north_stairs, w - 3, 10, 0);
                ctx.place_block(b, south_stairs, w - 3, 10, 4);
                ctx.place_block(b, east_stairs, w - 5, 10, 2);
                ctx.place_block(b, west_stairs, w - 1, 10, 2);
                ctx.generate_box(b, 8, 0, 0, 12, 4, 4, ctx.bs("sandstone"), ctx.bs("air"), false);
                ctx.generate_box(b, 9, 1, 0, 11, 3, 4, ctx.bs("air"), ctx.bs("air"), false);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 9, 1, 1);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 9, 2, 1);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 9, 3, 1);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 10, 3, 1);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 11, 3, 1);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 11, 2, 1);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 11, 1, 1);
                ctx.generate_box(b, 4, 1, 1, 8, 3, 3, ctx.bs("sandstone"), ctx.bs("air"), false);
                ctx.generate_box(b, 4, 1, 2, 8, 2, 2, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(b, 12, 1, 1, 16, 3, 3, ctx.bs("sandstone"), ctx.bs("air"), false);
                ctx.generate_box(b, 12, 1, 2, 16, 2, 2, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    5,
                    4,
                    5,
                    w - 6,
                    4,
                    d - 6,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(b, 9, 4, 9, 11, 4, 11, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    8,
                    1,
                    8,
                    8,
                    3,
                    8,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    12,
                    1,
                    8,
                    12,
                    3,
                    8,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    1,
                    12,
                    8,
                    3,
                    12,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    12,
                    1,
                    12,
                    12,
                    3,
                    12,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    1,
                    5,
                    4,
                    4,
                    11,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    w - 5,
                    1,
                    5,
                    w - 2,
                    4,
                    11,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    6,
                    7,
                    9,
                    6,
                    7,
                    11,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    w - 7,
                    7,
                    9,
                    w - 7,
                    7,
                    11,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    5,
                    9,
                    5,
                    7,
                    11,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    w - 6,
                    5,
                    9,
                    w - 6,
                    7,
                    11,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.place_block(b, ctx.bs("air"), 5, 5, 10);
                ctx.place_block(b, ctx.bs("air"), 5, 6, 10);
                ctx.place_block(b, ctx.bs("air"), 6, 6, 10);
                ctx.place_block(b, ctx.bs("air"), w - 6, 5, 10);
                ctx.place_block(b, ctx.bs("air"), w - 6, 6, 10);
                ctx.place_block(b, ctx.bs("air"), w - 7, 6, 10);
                ctx.generate_box(b, 2, 4, 4, 2, 6, 4, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(b, w - 3, 4, 4, w - 3, 6, 4, ctx.bs("air"), ctx.bs("air"), false);
                ctx.place_block(b, north_stairs, 2, 4, 5);
                ctx.place_block(b, north_stairs, 2, 3, 4);
                ctx.place_block(b, north_stairs, w - 3, 4, 5);
                ctx.place_block(b, north_stairs, w - 3, 3, 4);
                ctx.generate_box(
                    b,
                    1,
                    1,
                    3,
                    2,
                    2,
                    3,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    w - 3,
                    1,
                    3,
                    w - 2,
                    2,
                    3,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.place_block(b, ctx.bs("sandstone"), 1, 1, 2);
                ctx.place_block(b, ctx.bs("sandstone"), w - 2, 1, 2);
                ctx.place_block(b, ctx.bs("sandstone_slab"), 1, 2, 2);
                ctx.place_block(b, ctx.bs("sandstone_slab"), w - 2, 2, 2);
                ctx.place_block(b, west_stairs, 2, 1, 2);
                ctx.place_block(b, east_stairs, w - 3, 1, 2);
                ctx.generate_box(
                    b,
                    4,
                    3,
                    5,
                    4,
                    3,
                    17,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    w - 5,
                    3,
                    5,
                    w - 5,
                    3,
                    17,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(b, 3, 1, 5, 4, 2, 16, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(b, w - 6, 1, 5, w - 5, 2, 16, ctx.bs("air"), ctx.bs("air"), false);

                for z in (5..=17).step_by(2) {
                    ctx.place_block(b, ctx.bs("cut_sandstone"), 4, 1, z);
                    ctx.place_block(b, ctx.bs("chiseled_sandstone"), 4, 2, z);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), w - 5, 1, z);
                    ctx.place_block(b, ctx.bs("chiseled_sandstone"), w - 5, 2, z);
                }

                ctx.place_block(b, ctx.bs("orange_terracotta"), 10, 0, 7);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 10, 0, 8);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 9, 0, 9);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 11, 0, 9);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 8, 0, 10);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 12, 0, 10);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 7, 0, 10);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 13, 0, 10);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 9, 0, 11);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 11, 0, 11);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 10, 0, 12);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 10, 0, 13);
                ctx.place_block(b, ctx.bs("blue_terracotta"), 10, 0, 10);

                for x in (0..=w - 1).step_by((w - 1) as usize) {
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 2, 1);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 2, 2);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 2, 3);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 3, 1);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 3, 2);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 3, 3);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 4, 1);
                    ctx.place_block(b, ctx.bs("chiseled_sandstone"), x, 4, 2);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 4, 3);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 5, 1);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 5, 2);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 5, 3);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 6, 1);
                    ctx.place_block(b, ctx.bs("chiseled_sandstone"), x, 6, 2);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 6, 3);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 7, 1);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 7, 2);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 7, 3);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 8, 1);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 8, 2);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 8, 3);
                }

                for x in (2..=w - 3).step_by((w - 3 - 2) as usize) {
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x - 1, 2, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 2, 0);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x + 1, 2, 0);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x - 1, 3, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 3, 0);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x + 1, 3, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x - 1, 4, 0);
                    ctx.place_block(b, ctx.bs("chiseled_sandstone"), x, 4, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x + 1, 4, 0);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x - 1, 5, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 5, 0);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x + 1, 5, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x - 1, 6, 0);
                    ctx.place_block(b, ctx.bs("chiseled_sandstone"), x, 6, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x + 1, 6, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x - 1, 7, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x, 7, 0);
                    ctx.place_block(b, ctx.bs("orange_terracotta"), x + 1, 7, 0);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x - 1, 8, 0);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x, 8, 0);
                    ctx.place_block(b, ctx.bs("cut_sandstone"), x + 1, 8, 0);
                }

                ctx.generate_box(
                    b,
                    8,
                    4,
                    0,
                    12,
                    6,
                    0,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.place_block(b, ctx.bs("air"), 8, 6, 0);
                ctx.place_block(b, ctx.bs("air"), 12, 6, 0);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 9, 5, 0);
                ctx.place_block(b, ctx.bs("chiseled_sandstone"), 10, 5, 0);
                ctx.place_block(b, ctx.bs("orange_terracotta"), 11, 5, 0);
                ctx.generate_box(
                    b,
                    8,
                    -14,
                    8,
                    12,
                    -11,
                    12,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    -10,
                    8,
                    12,
                    -10,
                    12,
                    ctx.bs("chiseled_sandstone"),
                    ctx.bs("chiseled_sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    -9,
                    8,
                    12,
                    -9,
                    12,
                    ctx.bs("cut_sandstone"),
                    ctx.bs("cut_sandstone"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    -8,
                    8,
                    12,
                    -1,
                    12,
                    ctx.bs("sandstone"),
                    ctx.bs("sandstone"),
                    false,
                );
                ctx.generate_box(b, 9, -11, 9, 11, -1, 11, ctx.bs("air"), ctx.bs("air"), false);
                ctx.place_block(b, ctx.bs("stone_pressure_plate"), 10, -11, 10);
                ctx.generate_box(b, 9, -13, 9, 11, -13, 11, ctx.bs("tnt"), ctx.bs("air"), false);
                ctx.place_block(b, ctx.bs("air"), 8, -11, 10);
                ctx.place_block(b, ctx.bs("air"), 8, -10, 10);
                ctx.place_block(b, ctx.bs("chiseled_sandstone"), 7, -10, 10);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 7, -11, 10);
                ctx.place_block(b, ctx.bs("air"), 12, -11, 10);
                ctx.place_block(b, ctx.bs("air"), 12, -10, 10);
                ctx.place_block(b, ctx.bs("chiseled_sandstone"), 13, -10, 10);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 13, -11, 10);
                ctx.place_block(b, ctx.bs("air"), 10, -11, 8);
                ctx.place_block(b, ctx.bs("air"), 10, -10, 8);
                ctx.place_block(b, ctx.bs("chiseled_sandstone"), 10, -10, 7);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 10, -11, 7);
                ctx.place_block(b, ctx.bs("air"), 10, -11, 12);
                ctx.place_block(b, ctx.bs("air"), 10, -10, 12);
                ctx.place_block(b, ctx.bs("chiseled_sandstone"), 10, -10, 13);
                ctx.place_block(b, ctx.bs("cut_sandstone"), 10, -11, 13);
                for direction in Direction::HORIZONTAL {
                    let i = direction.data_2d() as usize;
                    if !live.flags[i] {
                        let xo = direction.step_x() * 2;
                        let zo = direction.step_z() * 2;
                        live.flags[i] = ctx.create_chest(
                            b,
                            random,
                            10 + xo,
                            -11,
                            10 + zo,
                            "minecraft:chests/desert_pyramid",
                        );
                    }
                }
                // `addCellar`
                let room_center = BlockPos::new(16, -4, 13);
                cellar_stairs(ctx, b, room_center);
                let (sands, roof) = cellar_room(ctx, b, room_center);
                *self.sand.lock() = Some((sands, roof));
            }
            ScatteredKind::JungleTemple { .. } => {
                if !Self::update_average_ground_height(live, ctx, 0) {
                    return;
                }
                let base = PieceBase {
                    bbox: live.bbox,
                    ..self.base.clone()
                };
                let b = &base;
                let mut stone = MossStone {
                    cobble: ctx.bs("cobblestone").s(),
                    mossy: ctx.bs("mossy_cobblestone").s(),
                };
                ctx.generate_box_selector(b, 0, -4, 0, w - 1, 0, d - 1, false, random, &mut stone);
                ctx.generate_box_selector(b, 2, 1, 2, 9, 2, 2, false, random, &mut stone);
                ctx.generate_box_selector(b, 2, 1, 12, 9, 2, 12, false, random, &mut stone);
                ctx.generate_box_selector(b, 2, 1, 3, 2, 2, 11, false, random, &mut stone);
                ctx.generate_box_selector(b, 9, 1, 3, 9, 2, 11, false, random, &mut stone);
                ctx.generate_box_selector(b, 1, 3, 1, 10, 6, 1, false, random, &mut stone);
                ctx.generate_box_selector(b, 1, 3, 13, 10, 6, 13, false, random, &mut stone);
                ctx.generate_box_selector(b, 1, 3, 2, 1, 6, 12, false, random, &mut stone);
                ctx.generate_box_selector(b, 10, 3, 2, 10, 6, 12, false, random, &mut stone);
                ctx.generate_box_selector(b, 2, 3, 2, 9, 3, 12, false, random, &mut stone);
                ctx.generate_box_selector(b, 2, 6, 2, 9, 6, 12, false, random, &mut stone);
                ctx.generate_box_selector(b, 3, 7, 3, 8, 7, 11, false, random, &mut stone);
                ctx.generate_box_selector(b, 4, 8, 4, 7, 8, 10, false, random, &mut stone);
                ctx.generate_air_box(b, 3, 1, 3, 8, 2, 11);
                ctx.generate_air_box(b, 4, 3, 6, 7, 3, 9);
                ctx.generate_air_box(b, 2, 4, 2, 9, 5, 12);
                ctx.generate_air_box(b, 4, 6, 5, 7, 6, 9);
                ctx.generate_air_box(b, 5, 7, 6, 6, 7, 8);
                ctx.generate_air_box(b, 5, 1, 2, 6, 2, 2);
                ctx.generate_air_box(b, 5, 2, 12, 6, 2, 12);
                ctx.generate_air_box(b, 5, 5, 1, 6, 5, 1);
                ctx.generate_air_box(b, 5, 5, 13, 6, 5, 13);
                ctx.place_block(b, ctx.bs("air"), 1, 5, 5);
                ctx.place_block(b, ctx.bs("air"), 10, 5, 5);
                ctx.place_block(b, ctx.bs("air"), 1, 5, 9);
                ctx.place_block(b, ctx.bs("air"), 10, 5, 9);

                for z in (0..=14).step_by(14) {
                    ctx.generate_box_selector(b, 2, 4, z, 2, 5, z, false, random, &mut stone);
                    ctx.generate_box_selector(b, 4, 4, z, 4, 5, z, false, random, &mut stone);
                    ctx.generate_box_selector(b, 7, 4, z, 7, 5, z, false, random, &mut stone);
                    ctx.generate_box_selector(b, 9, 4, z, 9, 5, z, false, random, &mut stone);
                }

                ctx.generate_box_selector(b, 5, 6, 0, 6, 6, 0, false, random, &mut stone);

                for x in (0..=11).step_by(11) {
                    for z in (2..=12).step_by(2) {
                        ctx.generate_box_selector(b, x, 4, z, x, 5, z, false, random, &mut stone);
                    }

                    ctx.generate_box_selector(b, x, 6, 5, x, 6, 5, false, random, &mut stone);
                    ctx.generate_box_selector(b, x, 6, 9, x, 6, 9, false, random, &mut stone);
                }

                ctx.generate_box_selector(b, 2, 7, 2, 2, 9, 2, false, random, &mut stone);
                ctx.generate_box_selector(b, 9, 7, 2, 9, 9, 2, false, random, &mut stone);
                ctx.generate_box_selector(b, 2, 7, 12, 2, 9, 12, false, random, &mut stone);
                ctx.generate_box_selector(b, 9, 7, 12, 9, 9, 12, false, random, &mut stone);
                ctx.generate_box_selector(b, 4, 9, 4, 4, 9, 4, false, random, &mut stone);
                ctx.generate_box_selector(b, 7, 9, 4, 7, 9, 4, false, random, &mut stone);
                ctx.generate_box_selector(b, 4, 9, 10, 4, 9, 10, false, random, &mut stone);
                ctx.generate_box_selector(b, 7, 9, 10, 7, 9, 10, false, random, &mut stone);
                ctx.generate_box_selector(b, 5, 9, 7, 6, 9, 7, false, random, &mut stone);
                let east_stairs = ctx.bs("cobblestone_stairs").with("facing", "east");
                let west_stairs = ctx.bs("cobblestone_stairs").with("facing", "west");
                let south_stairs = ctx.bs("cobblestone_stairs").with("facing", "south");
                let north_stairs = ctx.bs("cobblestone_stairs").with("facing", "north");
                ctx.place_block(b, north_stairs, 5, 9, 6);
                ctx.place_block(b, north_stairs, 6, 9, 6);
                ctx.place_block(b, south_stairs, 5, 9, 8);
                ctx.place_block(b, south_stairs, 6, 9, 8);
                ctx.place_block(b, north_stairs, 4, 0, 0);
                ctx.place_block(b, north_stairs, 5, 0, 0);
                ctx.place_block(b, north_stairs, 6, 0, 0);
                ctx.place_block(b, north_stairs, 7, 0, 0);
                ctx.place_block(b, north_stairs, 4, 1, 8);
                ctx.place_block(b, north_stairs, 4, 2, 9);
                ctx.place_block(b, north_stairs, 4, 3, 10);
                ctx.place_block(b, north_stairs, 7, 1, 8);
                ctx.place_block(b, north_stairs, 7, 2, 9);
                ctx.place_block(b, north_stairs, 7, 3, 10);
                ctx.generate_box_selector(b, 4, 1, 9, 4, 1, 9, false, random, &mut stone);
                ctx.generate_box_selector(b, 7, 1, 9, 7, 1, 9, false, random, &mut stone);
                ctx.generate_box_selector(b, 4, 1, 10, 7, 2, 10, false, random, &mut stone);
                ctx.generate_box_selector(b, 5, 4, 5, 6, 4, 5, false, random, &mut stone);
                ctx.place_block(b, east_stairs, 4, 4, 5);
                ctx.place_block(b, west_stairs, 7, 4, 5);

                for i in 0..4 {
                    ctx.place_block(b, south_stairs, 5, 0 - i, 6 + i);
                    ctx.place_block(b, south_stairs, 6, 0 - i, 6 + i);
                    ctx.generate_air_box(b, 5, 0 - i, 7 + i, 6, 0 - i, 9 + i);
                }

                ctx.generate_air_box(b, 1, -3, 12, 10, -1, 13);
                ctx.generate_air_box(b, 1, -3, 1, 3, -1, 13);
                ctx.generate_air_box(b, 1, -3, 1, 9, -1, 5);

                for z in (1..=13).step_by(2) {
                    ctx.generate_box_selector(b, 1, -3, z, 1, -2, z, false, random, &mut stone);
                }

                for z in (2..=12).step_by(2) {
                    ctx.generate_box_selector(b, 1, -1, z, 3, -1, z, false, random, &mut stone);
                }

                ctx.generate_box_selector(b, 2, -2, 1, 5, -2, 1, false, random, &mut stone);
                ctx.generate_box_selector(b, 7, -2, 1, 9, -2, 1, false, random, &mut stone);
                ctx.generate_box_selector(b, 6, -3, 1, 6, -3, 1, false, random, &mut stone);
                ctx.generate_box_selector(b, 6, -1, 1, 6, -1, 1, false, random, &mut stone);
                ctx.place_block(
                    b,
                    ctx.bs("tripwire_hook")
                        .with("facing", "east")
                        .with("attached", "true"),
                    1,
                    -3,
                    8,
                );
                ctx.place_block(
                    b,
                    ctx.bs("tripwire_hook")
                        .with("facing", "west")
                        .with("attached", "true"),
                    4,
                    -3,
                    8,
                );
                ctx.place_block(
                    b,
                    ctx.bs("tripwire")
                        .with("east", "true")
                        .with("west", "true")
                        .with("attached", "true"),
                    2,
                    -3,
                    8,
                );
                ctx.place_block(
                    b,
                    ctx.bs("tripwire")
                        .with("east", "true")
                        .with("west", "true")
                        .with("attached", "true"),
                    3,
                    -3,
                    8,
                );
                let redstone_wire_ns = ctx
                    .bs("redstone_wire")
                    .with("north", "side")
                    .with("south", "side");
                ctx.place_block(b, redstone_wire_ns, 5, -3, 7);
                ctx.place_block(b, redstone_wire_ns, 5, -3, 6);
                ctx.place_block(b, redstone_wire_ns, 5, -3, 5);
                ctx.place_block(b, redstone_wire_ns, 5, -3, 4);
                ctx.place_block(b, redstone_wire_ns, 5, -3, 3);
                ctx.place_block(b, redstone_wire_ns, 5, -3, 2);
                ctx.place_block(
                    b,
                    ctx.bs("redstone_wire").with("north", "side").with("west", "side"),
                    5,
                    -3,
                    1,
                );
                ctx.place_block(
                    b,
                    ctx.bs("redstone_wire").with("east", "side").with("west", "side"),
                    4,
                    -3,
                    1,
                );
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 3, -3, 1);
                if !live.flags[1] {
                    live.flags[1] = ctx.create_dispenser(
                        b,
                        random,
                        3,
                        -2,
                        1,
                        Direction::North,
                        "minecraft:chests/jungle_temple_dispenser",
                    );
                }

                ctx.place_block(b, ctx.bs("vine").with("south", "true"), 3, -2, 2);
                ctx.place_block(
                    b,
                    ctx.bs("tripwire_hook")
                        .with("facing", "north")
                        .with("attached", "true"),
                    7,
                    -3,
                    1,
                );
                ctx.place_block(
                    b,
                    ctx.bs("tripwire_hook")
                        .with("facing", "south")
                        .with("attached", "true"),
                    7,
                    -3,
                    5,
                );
                ctx.place_block(
                    b,
                    ctx.bs("tripwire")
                        .with("north", "true")
                        .with("south", "true")
                        .with("attached", "true"),
                    7,
                    -3,
                    2,
                );
                ctx.place_block(
                    b,
                    ctx.bs("tripwire")
                        .with("north", "true")
                        .with("south", "true")
                        .with("attached", "true"),
                    7,
                    -3,
                    3,
                );
                ctx.place_block(
                    b,
                    ctx.bs("tripwire")
                        .with("north", "true")
                        .with("south", "true")
                        .with("attached", "true"),
                    7,
                    -3,
                    4,
                );
                ctx.place_block(
                    b,
                    ctx.bs("redstone_wire").with("east", "side").with("west", "side"),
                    8,
                    -3,
                    6,
                );
                ctx.place_block(
                    b,
                    ctx.bs("redstone_wire").with("west", "side").with("south", "side"),
                    9,
                    -3,
                    6,
                );
                ctx.place_block(
                    b,
                    ctx.bs("redstone_wire").with("north", "side").with("south", "up"),
                    9,
                    -3,
                    5,
                );
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 9, -3, 4);
                ctx.place_block(b, redstone_wire_ns, 9, -2, 4);
                if !live.flags[2] {
                    live.flags[2] = ctx.create_dispenser(
                        b,
                        random,
                        9,
                        -2,
                        3,
                        Direction::West,
                        "minecraft:chests/jungle_temple_dispenser",
                    );
                }

                ctx.place_block(b, ctx.bs("vine").with("east", "true"), 8, -1, 3);
                ctx.place_block(b, ctx.bs("vine").with("east", "true"), 8, -2, 3);
                if !live.flags[0] {
                    live.flags[0] = ctx.create_chest(b, random, 8, -3, 3, "minecraft:chests/jungle_temple");
                }

                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 9, -3, 2);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 8, -3, 1);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 4, -3, 5);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 5, -2, 5);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 5, -1, 5);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 6, -3, 5);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 7, -2, 5);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 7, -1, 5);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 8, -3, 5);
                ctx.generate_box_selector(b, 9, -1, 1, 9, -1, 5, false, random, &mut stone);
                ctx.generate_air_box(b, 8, -3, 8, 10, -1, 10);
                ctx.place_block(b, ctx.bs("chiseled_stone_bricks"), 8, -2, 11);
                ctx.place_block(b, ctx.bs("chiseled_stone_bricks"), 9, -2, 11);
                ctx.place_block(b, ctx.bs("chiseled_stone_bricks"), 10, -2, 11);
                let lever = ctx.bs("lever").with("facing", "north").with("face", "wall");
                ctx.place_block(b, lever, 8, -2, 12);
                ctx.place_block(b, lever, 9, -2, 12);
                ctx.place_block(b, lever, 10, -2, 12);
                ctx.generate_box_selector(b, 8, -3, 8, 8, -3, 10, false, random, &mut stone);
                ctx.generate_box_selector(b, 10, -3, 8, 10, -3, 10, false, random, &mut stone);
                ctx.place_block(b, ctx.bs("mossy_cobblestone"), 10, -2, 9);
                ctx.place_block(b, redstone_wire_ns, 8, -2, 9);
                ctx.place_block(b, redstone_wire_ns, 8, -2, 10);
                ctx.place_block(
                    b,
                    ctx.bs("redstone_wire")
                        .with("north", "side")
                        .with("south", "side")
                        .with("east", "side")
                        .with("west", "side"),
                    10,
                    -1,
                    9,
                );
                ctx.place_block(b, ctx.bs("sticky_piston").with("facing", "up"), 9, -2, 8);
                ctx.place_block(b, ctx.bs("sticky_piston").with("facing", "west"), 10, -2, 8);
                ctx.place_block(b, ctx.bs("sticky_piston").with("facing", "west"), 10, -1, 8);
                ctx.place_block(b, ctx.bs("repeater").with("facing", "north"), 10, -2, 10);
                if !live.flags[3] {
                    live.flags[3] = ctx.create_chest(b, random, 9, -3, 10, "minecraft:chests/jungle_temple");
                }
            }
        }
    }
}

/// `JungleTemplePiece.MossStoneSelector`.
struct MossStone {
    cobble: BlockState,
    mossy: BlockState,
}

impl BlockSelector for MossStone {
    fn next(&mut self, random: &mut dyn RandomSource, _x: i32, _y: i32, _z: i32, _edge: bool) -> BlockState {
        if random.next_float() < 0.4 {
            self.cobble
        } else {
            self.mossy
        }
    }
}

/// `DesertPyramidPiece.addCellarStairs`.
fn cellar_stairs(ctx: &mut PieceCtx, b: &PieceBase, room_center: BlockPos) {
    let x = room_center.x;
    let y = room_center.y;
    let z = room_center.z;
    let sand_stone_stairs = ctx.bs("sandstone_stairs");
    ctx.place_block(
        b,
        sand_stone_stairs.rotated(Rotation::CounterClockwise90),
        13,
        -1,
        17,
    );
    ctx.place_block(
        b,
        sand_stone_stairs.rotated(Rotation::CounterClockwise90),
        14,
        -2,
        17,
    );
    ctx.place_block(
        b,
        sand_stone_stairs.rotated(Rotation::CounterClockwise90),
        15,
        -3,
        17,
    );
    let sand = ctx.bs("sand");
    let sand_stone = ctx.bs("sandstone");
    let variant = ctx.region.random.next_bool();
    ctx.place_block(b, sand, x - 4, y + 4, z + 4);
    ctx.place_block(b, sand, x - 3, y + 4, z + 4);
    ctx.place_block(b, sand, x - 2, y + 4, z + 4);
    ctx.place_block(b, sand, x - 1, y + 4, z + 4);
    ctx.place_block(b, sand, x, y + 4, z + 4);
    ctx.place_block(b, sand, x - 2, y + 3, z + 4);
    ctx.place_block(b, if variant { sand } else { sand_stone }, x - 1, y + 3, z + 4);
    ctx.place_block(b, if !variant { sand } else { sand_stone }, x, y + 3, z + 4);
    ctx.place_block(b, sand, x - 1, y + 2, z + 4);
    ctx.place_block(b, sand_stone, x, y + 2, z + 4);
    ctx.place_block(b, sand, x, y + 1, z + 4);
}

/// `DesertPyramidPiece.addCellarRoom`: returns the potential suspicious sand positions and the
/// collapsed roof position.
fn cellar_room(ctx: &mut PieceCtx, b: &PieceBase, room_center: BlockPos) -> (Vec<BlockPos>, BlockPos) {
    let mut sands: Vec<BlockPos> = Vec::new();

    let x = room_center.x;
    let y = room_center.y;
    let z = room_center.z;
    let cut_sand_stone = ctx.bs("cut_sandstone");
    let hieroglyphs_sand_stone = ctx.bs("chiseled_sandstone");
    ctx.generate_box(
        b,
        x - 3,
        y + 1,
        z - 3,
        x - 3,
        y + 1,
        z + 2,
        cut_sand_stone,
        cut_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x + 3,
        y + 1,
        z - 3,
        x + 3,
        y + 1,
        z + 2,
        cut_sand_stone,
        cut_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x - 3,
        y + 1,
        z - 3,
        x + 3,
        y + 1,
        z - 2,
        cut_sand_stone,
        cut_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x - 3,
        y + 1,
        z + 3,
        x + 3,
        y + 1,
        z + 3,
        cut_sand_stone,
        cut_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x - 3,
        y + 2,
        z - 3,
        x - 3,
        y + 2,
        z + 2,
        hieroglyphs_sand_stone,
        hieroglyphs_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x + 3,
        y + 2,
        z - 3,
        x + 3,
        y + 2,
        z + 2,
        hieroglyphs_sand_stone,
        hieroglyphs_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x - 3,
        y + 2,
        z - 3,
        x + 3,
        y + 2,
        z - 2,
        hieroglyphs_sand_stone,
        hieroglyphs_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x - 3,
        y + 2,
        z + 3,
        x + 3,
        y + 2,
        z + 3,
        hieroglyphs_sand_stone,
        hieroglyphs_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x - 3,
        -1,
        z - 3,
        x - 3,
        -1,
        z + 2,
        cut_sand_stone,
        cut_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x + 3,
        -1,
        z - 3,
        x + 3,
        -1,
        z + 2,
        cut_sand_stone,
        cut_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x - 3,
        -1,
        z - 3,
        x + 3,
        -1,
        z - 2,
        cut_sand_stone,
        cut_sand_stone,
        true,
    );
    ctx.generate_box(
        b,
        x - 3,
        -1,
        z + 3,
        x + 3,
        -1,
        z + 3,
        cut_sand_stone,
        cut_sand_stone,
        true,
    );
    place_sand_box(&mut sands, b, x - 2, y + 1, z - 2, x + 2, y + 3, z + 2);
    let roof = collapsed_roof(ctx, b, x - 2, y + 4, z - 2, x + 2, z + 2);
    let orange_teracotta = ctx.bs("orange_terracotta");
    let blue_teracotta = ctx.bs("blue_terracotta");
    ctx.place_block(b, blue_teracotta, x, y, z);
    ctx.place_block(b, orange_teracotta, x + 1, y, z - 1);
    ctx.place_block(b, orange_teracotta, x + 1, y, z + 1);
    ctx.place_block(b, orange_teracotta, x - 1, y, z - 1);
    ctx.place_block(b, orange_teracotta, x - 1, y, z + 1);
    ctx.place_block(b, orange_teracotta, x + 2, y, z);
    ctx.place_block(b, orange_teracotta, x - 2, y, z);
    ctx.place_block(b, orange_teracotta, x, y, z + 2);
    ctx.place_block(b, orange_teracotta, x, y, z - 2);
    ctx.place_block(b, orange_teracotta, x + 3, y, z);
    sands.push(b.world_pos(x + 3, y + 1, z));
    sands.push(b.world_pos(x + 3, y + 2, z));
    ctx.place_block(b, cut_sand_stone, x + 4, y + 1, z);
    ctx.place_block(b, hieroglyphs_sand_stone, x + 4, y + 2, z);
    ctx.place_block(b, orange_teracotta, x - 3, y, z);
    sands.push(b.world_pos(x - 3, y + 1, z));
    sands.push(b.world_pos(x - 3, y + 2, z));
    ctx.place_block(b, cut_sand_stone, x - 4, y + 1, z);
    ctx.place_block(b, hieroglyphs_sand_stone, x - 4, y + 2, z);
    ctx.place_block(b, orange_teracotta, x, y, z + 3);
    sands.push(b.world_pos(x, y + 1, z + 3));
    sands.push(b.world_pos(x, y + 2, z + 3));
    ctx.place_block(b, orange_teracotta, x, y, z - 3);
    sands.push(b.world_pos(x, y + 1, z - 3));
    sands.push(b.world_pos(x, y + 2, z - 3));
    ctx.place_block(b, cut_sand_stone, x, y + 1, z - 4);
    ctx.place_block(b, hieroglyphs_sand_stone, x, -2, z - 4);
    (sands, roof)
}

/// `placeSandBox`.
fn place_sand_box(
    sands: &mut Vec<BlockPos>,
    b: &PieceBase,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            for z in z0..=z1 {
                sands.push(b.world_pos(x, y, z));
            }
        }
    }
}

/// `placeCollapsedRoof`: random sand/sandstone (region random), then the roof position from a
/// positional random.
fn collapsed_roof(
    ctx: &mut PieceCtx,
    b: &PieceBase,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    z1: i32,
) -> BlockPos {
    for x in x0..=x1 {
        for z in z0..=z1 {
            let s = if ctx.region.random.next_float() < 0.33 {
                ctx.bs("sandstone")
            } else {
                ctx.bs("sand")
            };
            ctx.place_block(b, s, x, y0, z);
        }
    }
    let p = b.world_pos(x0, y0, z0);
    let mut r = crate::random::LegacyRandom::new(ctx.region.env.seed)
        .fork_positional()
        .at(p.x, p.y, p.z);
    let rx = r.next_int_bounded(x1 - x0 + 1) + x0;
    let rz = r.next_int_bounded(z1 - z0 + 1) + z0;
    BlockPos::new(b.world_x(rx, rz), b.world_y(y0), b.world_z(rx, rz))
}

/// `DesertPyramidStructure.afterPlace`: some of the cellar sand becomes suspicious sand.
pub fn desert_pyramid_after_place(start: &super::start::StructureStart, ctx: &mut PieceCtx) {
    let mut unique: Vec<BlockPos> = Vec::new();
    let mut roofs = Vec::new();
    for p in &start.pieces {
        if let Some(sp) = p.as_any().downcast_ref::<ScatteredPiece>()
            && let Some((sands, roof)) = sp.sand.lock().clone()
        {
            unique.extend(sands);
            roofs.push(roof);
        }
    }
    for roof in roofs {
        place_suspicious_sand(ctx, roof);
    }
    // `SortedArraySet` with `Vec3i.compareTo`: y, then z, then x.
    unique.sort_by_key(|p| (p.y, p.z, p.x));
    unique.dedup();
    // `pieces.calculateBoundingBox()` sees the boxes `postProcess` moved to the ground.
    let boxes: Vec<super::geom::BoundingBox> = start
        .pieces
        .iter()
        .map(|p| {
            match p
                .as_any()
                .downcast_ref::<ScatteredPiece>()
                .and_then(|sp| *sp.live.lock())
            {
                Some(live) => live.bbox,
                None => *p.bbox(),
            }
        })
        .collect();
    let Some(bb) = super::geom::BoundingBox::encapsulating(boxes.iter()) else {
        return;
    };
    let c = bb.center();
    let mut r = crate::random::LegacyRandom::new(ctx.region.env.seed)
        .fork_positional()
        .at(c.x, c.y, c.z);
    let mut shuffled = unique.clone();
    super::util::shuffle(&mut shuffled, &mut r);
    let mut to_place = (unique.len() as i32).min(5 + r.next_int_bounded(3));
    let sand = ctx.bs("sand").s();
    for p in shuffled {
        if to_place > 0 {
            to_place -= 1;
            place_suspicious_sand(ctx, p);
        } else if ctx.chunk_bb.is_inside(p.x, p.y, p.z) {
            ctx.region.set(p, sand, 2);
        }
    }
}

fn place_suspicious_sand(ctx: &mut PieceCtx, p: BlockPos) {
    if !ctx.chunk_bb.is_inside(p.x, p.y, p.z) {
        return;
    }
    let s = ctx.bs("suspicious_sand").s();
    ctx.region.set(p, s, 2);
    if let Some(be) = ctx.region.block_entity(p)
        && be.get_str("id") == Some("minecraft:brushable_block")
    {
        be.insert("LootTable", "minecraft:archaeology/desert_pyramid");
        // `BlockPos.asLong`
        let packed =
            ((p.x as i64 & 0x3FF_FFFF) << 38) | (p.y as i64 & 0xFFF) | ((p.z as i64 & 0x3FF_FFFF) << 12);
        be.insert("LootTableSeed", packed);
    }
}
