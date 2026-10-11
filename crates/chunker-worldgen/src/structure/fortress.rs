//! `NetherFortressStructure` + `NetherFortressPieces` (piece layout and NBT).

use chunker_nbt::Compound;

use super::geom::{BlockPos, BoundingBox, Direction};
use super::start::{PieceBase, PiecesBuilder};
use super::{GenContext, Stub};
use crate::piece_boilerplate;
use crate::random::RandomSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NfKind {
    Start,
    BridgeCrossing,
    BridgeEndFiller { seed: i32 },
    BridgeStraight,
    CorridorStairs,
    CorridorTBalcony,
    CastleEntrance,
    SmallCorridorCrossing,
    SmallCorridorLeftTurn { chest: bool },
    SmallCorridor,
    SmallCorridorRightTurn { chest: bool },
    StalkRoom,
    MonsterThrone { spawner: bool },
    RoomCrossing,
    StairsRoom,
}

/// `NetherFortressPieces.NetherBridgePiece` and subclasses.
pub struct NfPiece {
    pub base: PieceBase,
    pub kind: NfKind,
    /// `isNeedingChest` (corridor turns) / `hasPlacedSpawner` (throne) as changed by `postProcess`.
    pub flag: std::sync::atomic::AtomicBool,
}

impl NfPiece {
    pub fn new(base: PieceBase, kind: NfKind) -> Self {
        let flag = match kind {
            NfKind::SmallCorridorLeftTurn { chest } | NfKind::SmallCorridorRightTurn { chest } => chest,
            NfKind::MonsterThrone { spawner } => spawner,
            _ => false,
        };
        Self {
            base,
            kind,
            flag: std::sync::atomic::AtomicBool::new(flag),
        }
    }
}

impl super::start::Piece for NfPiece {
    piece_boilerplate!();
    fn type_id(&self) -> &'static str {
        match self.kind {
            // `StartPiece` is built through the `BridgeCrossing` constructor and saves as one.
            NfKind::Start => "minecraft:nebcr",
            NfKind::BridgeCrossing => "minecraft:nebcr",
            NfKind::BridgeEndFiller { .. } => "minecraft:nebef",
            NfKind::BridgeStraight => "minecraft:nebs",
            NfKind::CorridorStairs => "minecraft:neccs",
            NfKind::CorridorTBalcony => "minecraft:nectb",
            NfKind::CastleEntrance => "minecraft:nece",
            NfKind::SmallCorridorCrossing => "minecraft:nescsc",
            NfKind::SmallCorridorLeftTurn { .. } => "minecraft:nesclt",
            NfKind::SmallCorridor => "minecraft:nesc",
            NfKind::SmallCorridorRightTurn { .. } => "minecraft:nescrt",
            NfKind::StalkRoom => "minecraft:necsr",
            NfKind::MonsterThrone { .. } => "minecraft:nemt",
            NfKind::RoomCrossing => "minecraft:nerc",
            NfKind::StairsRoom => "minecraft:nesr",
        }
    }
    fn save_extra(&self, tag: &mut Compound) {
        match self.kind {
            NfKind::BridgeEndFiller { seed } => tag.insert("Seed", seed),
            NfKind::SmallCorridorLeftTurn { chest } | NfKind::SmallCorridorRightTurn { chest } => {
                tag.insert("Chest", chest)
            }
            NfKind::MonsterThrone { spawner } => tag.insert("Mob", spawner),
            _ => {}
        }
    }
    fn post_process_ported(&self) -> bool {
        true
    }
    fn post_process(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        self.post_process_nf(ctx, random);
    }
}

/// `(kind index, weight, maxPlaceCount, allowInRow)`; bridge weights then castle weights.
const BRIDGE: [(u8, i32, i32, bool); 6] = [
    (0, 30, 0, true),
    (1, 10, 4, false),
    (2, 10, 4, false),
    (3, 10, 3, false),
    (4, 5, 2, false),
    (5, 5, 1, false),
];
const CASTLE: [(u8, i32, i32, bool); 7] = [
    (6, 25, 0, true),
    (7, 15, 5, false),
    (8, 5, 10, false),
    (9, 5, 10, false),
    (10, 10, 3, true),
    (11, 7, 2, false),
    (12, 5, 2, false),
];

#[derive(Clone, Copy)]
struct Weight {
    kind: u8,
    weight: i32,
    max: i32,
    allow_in_row: bool,
}

struct Gen<'r> {
    pieces: Vec<NfPiece>,
    pending: Vec<usize>,
    bridge: Vec<Weight>,
    castle: Vec<Weight>,
    place_count: [i32; 13],
    previous: Option<u8>,
    random: &'r mut dyn RandomSource,
}

fn ok_box(b: &BoundingBox) -> bool {
    b.min_y > 10
}

impl Gen<'_> {
    fn collides(&self, b: &BoundingBox) -> bool {
        self.pieces.iter().any(|p| p.base.bbox.intersects(b))
    }

    fn free(&self, b: &BoundingBox) -> bool {
        ok_box(b) && !self.collides(b)
    }

    /// `BridgeEndFiller.createPiece`.
    fn end_filler(&mut self, x: i32, y: i32, z: i32, d: Direction, depth: i32) -> Option<NfPiece> {
        let b = BoundingBox::orient_box(x, y, z, -1, -3, 0, 5, 10, 8, d);
        if !self.free(&b) {
            return None;
        }
        let seed = self.random.next_int();
        Some(NfPiece::new(
            PieceBase::new(depth, b, Some(d)),
            NfKind::BridgeEndFiller { seed },
        ))
    }

    /// `findAndCreateBridgePieceFactory`.
    fn create(&mut self, kind: u8, x: i32, y: i32, z: i32, d: Direction, depth: i32) -> Option<NfPiece> {
        let orient = |ox, oy, oz, w, h, dp| BoundingBox::orient_box(x, y, z, ox, oy, oz, w, h, dp, d);
        let (b, k) = match kind {
            0 => (orient(-1, -3, 0, 5, 10, 19), NfKind::BridgeStraight),
            1 => (orient(-8, -3, 0, 19, 10, 19), NfKind::BridgeCrossing),
            2 => (orient(-2, 0, 0, 7, 9, 7), NfKind::RoomCrossing),
            3 => (orient(-2, 0, 0, 7, 11, 7), NfKind::StairsRoom),
            4 => (
                orient(-2, 0, 0, 7, 8, 9),
                NfKind::MonsterThrone { spawner: false },
            ),
            5 => (orient(-5, -3, 0, 13, 14, 13), NfKind::CastleEntrance),
            6 => (orient(-1, 0, 0, 5, 7, 5), NfKind::SmallCorridor),
            7 => (orient(-1, 0, 0, 5, 7, 5), NfKind::SmallCorridorCrossing),
            8 => (
                orient(-1, 0, 0, 5, 7, 5),
                NfKind::SmallCorridorRightTurn { chest: false },
            ),
            9 => (
                orient(-1, 0, 0, 5, 7, 5),
                NfKind::SmallCorridorLeftTurn { chest: false },
            ),
            10 => (orient(-1, -7, 0, 5, 14, 10), NfKind::CorridorStairs),
            11 => (orient(-3, 0, 0, 9, 7, 9), NfKind::CorridorTBalcony),
            _ => (orient(-5, -3, 0, 13, 14, 13), NfKind::StalkRoom),
        };
        if !self.free(&b) {
            return None;
        }
        let k = match k {
            NfKind::SmallCorridorRightTurn { .. } => NfKind::SmallCorridorRightTurn {
                chest: self.random.next_int_bounded(3) == 0,
            },
            NfKind::SmallCorridorLeftTurn { .. } => NfKind::SmallCorridorLeftTurn {
                chest: self.random.next_int_bounded(3) == 0,
            },
            k => k,
        };
        Some(NfPiece::new(PieceBase::new(depth, b, Some(d)), k))
    }

    /// `NetherBridgePiece.generatePiece`.
    fn generate_piece(
        &mut self,
        castle: bool,
        x: i32,
        y: i32,
        z: i32,
        d: Direction,
        depth: i32,
    ) -> Option<NfPiece> {
        let list = if castle { &self.castle } else { &self.bridge };
        let mut any = false;
        let mut total = 0;
        for w in list {
            if w.max > 0 && self.place_count[w.kind as usize] < w.max {
                any = true;
            }
            total += w.weight;
        }
        let total = if any { total } else { -1 };
        let do_stuff = total > 0 && depth <= 30;
        let mut attempts = 0;
        while attempts < 5 && do_stuff {
            attempts += 1;
            let mut selection = self.random.next_int_bounded(total);
            let mut i = 0;
            loop {
                let list = if castle { &self.castle } else { &self.bridge };
                if i >= list.len() {
                    break;
                }
                let w = list[i];
                selection -= w.weight;
                if selection < 0 {
                    let placeable = w.max == 0 || self.place_count[w.kind as usize] < w.max;
                    if !placeable || (Some(w.kind) == self.previous && !w.allow_in_row) {
                        break;
                    }
                    if let Some(p) = self.create(w.kind, x, y, z, d, depth) {
                        self.place_count[w.kind as usize] += 1;
                        self.previous = Some(w.kind);
                        if !(w.max == 0 || self.place_count[w.kind as usize] < w.max) {
                            let list = if castle {
                                &mut self.castle
                            } else {
                                &mut self.bridge
                            };
                            list.remove(i);
                        }
                        return Some(p);
                    }
                }
                i += 1;
            }
        }
        self.end_filler(x, y, z, d, depth)
    }

    /// `generateAndAddPiece`.
    fn generate_and_add(&mut self, x: i32, y: i32, z: i32, d: Direction, depth: i32, castle: bool) {
        let start = self.pieces[0].base.bbox;
        if (x - start.min_x).abs() <= 112 && (z - start.min_z).abs() <= 112 {
            if let Some(p) = self.generate_piece(castle, x, y, z, d, depth + 1) {
                self.pieces.push(p);
                self.pending.push(self.pieces.len() - 1);
            }
        } else {
            // Created but never added (vanilla quirk); still consumes randomness.
            let _ = self.end_filler(x, y, z, d, depth);
        }
    }

    fn forward(&mut self, i: usize, x_off: i32, y_off: i32, castle: bool) {
        let p = &self.pieces[i].base;
        let (b, depth) = (p.bbox, p.gen_depth);
        match p.orientation {
            Some(Direction::North) => self.generate_and_add(
                b.min_x + x_off,
                b.min_y + y_off,
                b.min_z - 1,
                Direction::North,
                depth,
                castle,
            ),
            Some(Direction::South) => self.generate_and_add(
                b.min_x + x_off,
                b.min_y + y_off,
                b.max_z + 1,
                Direction::South,
                depth,
                castle,
            ),
            Some(Direction::West) => self.generate_and_add(
                b.min_x - 1,
                b.min_y + y_off,
                b.min_z + x_off,
                Direction::West,
                depth,
                castle,
            ),
            Some(Direction::East) => self.generate_and_add(
                b.max_x + 1,
                b.min_y + y_off,
                b.min_z + x_off,
                Direction::East,
                depth,
                castle,
            ),
            _ => {}
        }
    }

    fn left(&mut self, i: usize, y_off: i32, z_off: i32, castle: bool) {
        let p = &self.pieces[i].base;
        let (b, depth) = (p.bbox, p.gen_depth);
        match p.orientation {
            Some(Direction::North) | Some(Direction::South) => self.generate_and_add(
                b.min_x - 1,
                b.min_y + y_off,
                b.min_z + z_off,
                Direction::West,
                depth,
                castle,
            ),
            Some(Direction::West) | Some(Direction::East) => self.generate_and_add(
                b.min_x + z_off,
                b.min_y + y_off,
                b.min_z - 1,
                Direction::North,
                depth,
                castle,
            ),
            _ => {}
        }
    }

    fn right(&mut self, i: usize, y_off: i32, z_off: i32, castle: bool) {
        let p = &self.pieces[i].base;
        let (b, depth) = (p.bbox, p.gen_depth);
        match p.orientation {
            Some(Direction::North) | Some(Direction::South) => self.generate_and_add(
                b.max_x + 1,
                b.min_y + y_off,
                b.min_z + z_off,
                Direction::East,
                depth,
                castle,
            ),
            Some(Direction::West) | Some(Direction::East) => self.generate_and_add(
                b.min_x + z_off,
                b.min_y + y_off,
                b.max_z + 1,
                Direction::South,
                depth,
                castle,
            ),
            _ => {}
        }
    }

    fn add_children(&mut self, i: usize) {
        let orientation = self.pieces[i].base.orientation;
        match self.pieces[i].kind {
            NfKind::Start | NfKind::BridgeCrossing => {
                self.forward(i, 8, 3, false);
                self.left(i, 3, 8, false);
                self.right(i, 3, 8, false);
            }
            NfKind::BridgeStraight => self.forward(i, 1, 3, false),
            NfKind::CorridorStairs | NfKind::SmallCorridor => self.forward(i, 1, 0, true),
            NfKind::CorridorTBalcony => {
                let z_off = if orientation == Some(Direction::West) || orientation == Some(Direction::North) {
                    5
                } else {
                    1
                };
                let l = self.random.next_int_bounded(8) > 0;
                self.left(i, 0, z_off, l);
                let r = self.random.next_int_bounded(8) > 0;
                self.right(i, 0, z_off, r);
            }
            NfKind::CastleEntrance => self.forward(i, 5, 3, true),
            NfKind::SmallCorridorCrossing => {
                self.forward(i, 1, 0, true);
                self.left(i, 0, 1, true);
                self.right(i, 0, 1, true);
            }
            NfKind::SmallCorridorLeftTurn { .. } => self.left(i, 0, 1, true),
            NfKind::SmallCorridorRightTurn { .. } => self.right(i, 0, 1, true),
            NfKind::StalkRoom => {
                self.forward(i, 5, 3, true);
                self.forward(i, 5, 11, true);
            }
            NfKind::RoomCrossing => {
                self.forward(i, 2, 0, false);
                self.left(i, 0, 2, false);
                self.right(i, 0, 2, false);
            }
            NfKind::StairsRoom => self.right(i, 6, 2, false),
            NfKind::BridgeEndFiller { .. } | NfKind::MonsterThrone { .. } => {}
        }
    }
}

fn weights(list: &[(u8, i32, i32, bool)]) -> Vec<Weight> {
    list.iter()
        .map(|&(kind, weight, max, allow_in_row)| Weight {
            kind,
            weight,
            max,
            allow_in_row,
        })
        .collect()
}

/// `NetherFortressStructure.findGenerationPoint`.
pub fn find_generation_point<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    let pos = BlockPos::new(ctx.min_block_x(), 64, ctx.min_block_z());
    Some(Stub::lazy(
        pos,
        |ctx: &mut GenContext, builder: &mut PiecesBuilder| {
            let (west, north) = (ctx.min_block_x() + 2, ctx.min_block_z() + 2);
            {
                let random: &mut dyn RandomSource = &mut ctx.random;
                let d = Direction::random_horizontal(random);
                let bbox = PieceBase::make_bounding_box(west, 64, north, d, 19, 10, 19);
                let start = NfPiece::new(PieceBase::new(0, bbox, Some(d)), NfKind::Start);
                let mut g = Gen {
                    pieces: vec![start],
                    pending: Vec::new(),
                    bridge: weights(&BRIDGE),
                    castle: weights(&CASTLE),
                    place_count: [0; 13],
                    previous: None,
                    random,
                };
                g.add_children(0);
                while !g.pending.is_empty() {
                    let pos = g.random.next_int_bounded(g.pending.len() as i32) as usize;
                    let piece = g.pending.remove(pos);
                    g.add_children(piece);
                }
                for p in g.pieces {
                    builder.add(Box::new(p));
                }
            }
            builder.move_inside_heights(&mut ctx.random, 48, 70);
        },
    ))
}

// --- postProcess (bodies translated statement by statement from `NetherFortressPieces`) ----------

use super::build::PieceCtx;
use std::sync::atomic::Ordering::Relaxed;

impl NfPiece {
    fn post_process_nf(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        let b = &self.base;
        match &self.kind {
            NfKind::Start | NfKind::BridgeCrossing => {
                ctx.generate_box(
                    b,
                    7,
                    3,
                    0,
                    11,
                    4,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    3,
                    7,
                    18,
                    4,
                    11,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 8, 5, 0, 10, 7, 18, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(b, 0, 5, 8, 18, 7, 10, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    7,
                    5,
                    0,
                    7,
                    5,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    7,
                    5,
                    11,
                    7,
                    5,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    11,
                    5,
                    0,
                    11,
                    5,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    11,
                    5,
                    11,
                    11,
                    5,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    5,
                    7,
                    7,
                    5,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    11,
                    5,
                    7,
                    18,
                    5,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    5,
                    11,
                    7,
                    5,
                    11,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    11,
                    5,
                    11,
                    18,
                    5,
                    11,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    7,
                    2,
                    0,
                    11,
                    2,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    7,
                    2,
                    13,
                    11,
                    2,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    7,
                    0,
                    0,
                    11,
                    1,
                    3,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    7,
                    0,
                    15,
                    11,
                    1,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 7..=11 {
                    for z in 0..=2 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, 18 - z);
                    }
                }

                ctx.generate_box(
                    b,
                    0,
                    2,
                    7,
                    5,
                    2,
                    11,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    13,
                    2,
                    7,
                    18,
                    2,
                    11,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    0,
                    7,
                    3,
                    1,
                    11,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    15,
                    0,
                    7,
                    18,
                    1,
                    11,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 0..=2 {
                    for z in 7..=11 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), 18 - x, -1, z);
                    }
                }
            }
            NfKind::BridgeEndFiller { seed } => {
                let mut self_random = crate::random::LegacyRandom::new(*seed as i64);

                for x in 0..=4 {
                    for y in 3..=4 {
                        let z = self_random.next_int_bounded(8);
                        ctx.generate_box(
                            b,
                            x,
                            y,
                            0,
                            x,
                            y,
                            z,
                            ctx.bs("nether_bricks"),
                            ctx.bs("nether_bricks"),
                            false,
                        );
                    }
                }

                let mut z = self_random.next_int_bounded(8);
                ctx.generate_box(
                    b,
                    0,
                    5,
                    0,
                    0,
                    5,
                    z,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                z = self_random.next_int_bounded(8);
                ctx.generate_box(
                    b,
                    4,
                    5,
                    0,
                    4,
                    5,
                    z,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 0..=4 {
                    let zx = self_random.next_int_bounded(5);
                    ctx.generate_box(
                        b,
                        x,
                        2,
                        0,
                        x,
                        2,
                        zx,
                        ctx.bs("nether_bricks"),
                        ctx.bs("nether_bricks"),
                        false,
                    );
                }

                for x in 0..=4 {
                    for y in 0..=1 {
                        let zx = self_random.next_int_bounded(3);
                        ctx.generate_box(
                            b,
                            x,
                            y,
                            0,
                            x,
                            y,
                            zx,
                            ctx.bs("nether_bricks"),
                            ctx.bs("nether_bricks"),
                            false,
                        );
                    }
                }
            }
            NfKind::BridgeStraight => {
                ctx.generate_box(
                    b,
                    0,
                    3,
                    0,
                    4,
                    4,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 1, 5, 0, 3, 7, 18, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    0,
                    5,
                    0,
                    0,
                    5,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    5,
                    0,
                    4,
                    5,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    4,
                    2,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    13,
                    4,
                    2,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    0,
                    0,
                    4,
                    1,
                    3,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    0,
                    15,
                    4,
                    1,
                    18,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 0..=4 {
                    for z in 0..=2 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, 18 - z);
                    }
                }

                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                let nse_fence = ns_fence.with("east", "true");
                let nsw_fence = ns_fence.with("west", "true");
                ctx.generate_box(b, 0, 1, 1, 0, 4, 1, nse_fence, nse_fence, false);
                ctx.generate_box(b, 0, 3, 4, 0, 4, 4, nse_fence, nse_fence, false);
                ctx.generate_box(b, 0, 3, 14, 0, 4, 14, nse_fence, nse_fence, false);
                ctx.generate_box(b, 0, 1, 17, 0, 4, 17, nse_fence, nse_fence, false);
                ctx.generate_box(b, 4, 1, 1, 4, 4, 1, nsw_fence, nsw_fence, false);
                ctx.generate_box(b, 4, 3, 4, 4, 4, 4, nsw_fence, nsw_fence, false);
                ctx.generate_box(b, 4, 3, 14, 4, 4, 14, nsw_fence, nsw_fence, false);
                ctx.generate_box(b, 4, 1, 17, 4, 4, 17, nsw_fence, nsw_fence, false);
            }
            NfKind::CorridorStairs => {
                let stairs = ctx.bs("nether_brick_stairs").with("facing", "south");
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");

                for step in 0..=9 {
                    let floor = (7 - step).max(1);
                    let roof = (floor + 5).max(14 - step).min(13);
                    let z = step;
                    ctx.generate_box(
                        b,
                        0,
                        0,
                        step,
                        4,
                        floor,
                        step,
                        ctx.bs("nether_bricks"),
                        ctx.bs("nether_bricks"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        1,
                        floor + 1,
                        step,
                        3,
                        roof - 1,
                        step,
                        ctx.bs("air"),
                        ctx.bs("air"),
                        false,
                    );
                    if step <= 6 {
                        ctx.place_block(b, stairs, 1, floor + 1, step);
                        ctx.place_block(b, stairs, 2, floor + 1, step);
                        ctx.place_block(b, stairs, 3, floor + 1, step);
                    }

                    ctx.generate_box(
                        b,
                        0,
                        roof,
                        step,
                        4,
                        roof,
                        step,
                        ctx.bs("nether_bricks"),
                        ctx.bs("nether_bricks"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        0,
                        floor + 1,
                        step,
                        0,
                        roof - 1,
                        step,
                        ctx.bs("nether_bricks"),
                        ctx.bs("nether_bricks"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        4,
                        floor + 1,
                        step,
                        4,
                        roof - 1,
                        step,
                        ctx.bs("nether_bricks"),
                        ctx.bs("nether_bricks"),
                        false,
                    );
                    if (step & 1) == 0 {
                        ctx.generate_box(
                            b,
                            0,
                            floor + 2,
                            step,
                            0,
                            floor + 3,
                            step,
                            ns_fence,
                            ns_fence,
                            false,
                        );
                        ctx.generate_box(
                            b,
                            4,
                            floor + 2,
                            step,
                            4,
                            floor + 3,
                            step,
                            ns_fence,
                            ns_fence,
                            false,
                        );
                    }

                    for x in 0..=4 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
            NfKind::CorridorTBalcony => {
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                let we_fence = ctx
                    .bs("nether_brick_fence")
                    .with("west", "true")
                    .with("east", "true");
                ctx.generate_box(
                    b,
                    0,
                    0,
                    0,
                    8,
                    1,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 2, 0, 8, 5, 8, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    0,
                    6,
                    0,
                    8,
                    6,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    2,
                    5,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    6,
                    2,
                    0,
                    8,
                    5,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 1, 3, 0, 1, 4, 0, we_fence, we_fence, false);
                ctx.generate_box(b, 7, 3, 0, 7, 4, 0, we_fence, we_fence, false);
                ctx.generate_box(
                    b,
                    0,
                    2,
                    4,
                    8,
                    2,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 1, 1, 4, 2, 2, 4, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(b, 6, 1, 4, 7, 2, 4, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(b, 1, 3, 8, 7, 3, 8, we_fence, we_fence, false);
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("east", "true")
                        .with("south", "true"),
                    0,
                    3,
                    8,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("west", "true")
                        .with("south", "true"),
                    8,
                    3,
                    8,
                );
                ctx.generate_box(b, 0, 3, 6, 0, 3, 7, ns_fence, ns_fence, false);
                ctx.generate_box(b, 8, 3, 6, 8, 3, 7, ns_fence, ns_fence, false);
                ctx.generate_box(
                    b,
                    0,
                    3,
                    4,
                    0,
                    5,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    3,
                    4,
                    8,
                    5,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    3,
                    5,
                    2,
                    5,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    6,
                    3,
                    5,
                    7,
                    5,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 1, 4, 5, 1, 5, 5, we_fence, we_fence, false);
                ctx.generate_box(b, 7, 4, 5, 7, 5, 5, we_fence, we_fence, false);

                for z in 0..=5 {
                    for x in 0..=8 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
            NfKind::CastleEntrance => {
                ctx.generate_box(
                    b,
                    0,
                    3,
                    0,
                    12,
                    4,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 5, 0, 12, 13, 12, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    0,
                    5,
                    0,
                    1,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    11,
                    5,
                    0,
                    12,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    5,
                    11,
                    4,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    5,
                    11,
                    10,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    9,
                    11,
                    7,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    5,
                    0,
                    4,
                    12,
                    1,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    5,
                    0,
                    10,
                    12,
                    1,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    9,
                    0,
                    7,
                    12,
                    1,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    11,
                    2,
                    10,
                    12,
                    10,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    8,
                    0,
                    7,
                    8,
                    0,
                    ctx.bs("nether_brick_fence"),
                    ctx.bs("nether_brick_fence"),
                    false,
                );
                let we_fence = ctx
                    .bs("nether_brick_fence")
                    .with("west", "true")
                    .with("east", "true");
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");

                for i in (1..=11).step_by(2) {
                    ctx.generate_box(b, i, 10, 0, i, 11, 0, we_fence, we_fence, false);
                    ctx.generate_box(b, i, 10, 12, i, 11, 12, we_fence, we_fence, false);
                    ctx.generate_box(b, 0, 10, i, 0, 11, i, ns_fence, ns_fence, false);
                    ctx.generate_box(b, 12, 10, i, 12, 11, i, ns_fence, ns_fence, false);
                    ctx.place_block(b, ctx.bs("nether_bricks"), i, 13, 0);
                    ctx.place_block(b, ctx.bs("nether_bricks"), i, 13, 12);
                    ctx.place_block(b, ctx.bs("nether_bricks"), 0, 13, i);
                    ctx.place_block(b, ctx.bs("nether_bricks"), 12, 13, i);
                    if i != 11 {
                        ctx.place_block(b, we_fence, i + 1, 13, 0);
                        ctx.place_block(b, we_fence, i + 1, 13, 12);
                        ctx.place_block(b, ns_fence, 0, 13, i + 1);
                        ctx.place_block(b, ns_fence, 12, 13, i + 1);
                    }
                }

                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("north", "true")
                        .with("east", "true"),
                    0,
                    13,
                    0,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("south", "true")
                        .with("east", "true"),
                    0,
                    13,
                    12,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("south", "true")
                        .with("west", "true"),
                    12,
                    13,
                    12,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("north", "true")
                        .with("west", "true"),
                    12,
                    13,
                    0,
                );

                for z in (3..=9).step_by(2) {
                    ctx.generate_box(
                        b,
                        1,
                        7,
                        z,
                        1,
                        8,
                        z,
                        ns_fence.with("west", "true"),
                        ns_fence.with("west", "true"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        11,
                        7,
                        z,
                        11,
                        8,
                        z,
                        ns_fence.with("east", "true"),
                        ns_fence.with("east", "true"),
                        false,
                    );
                }

                ctx.generate_box(
                    b,
                    4,
                    2,
                    0,
                    8,
                    2,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    4,
                    12,
                    2,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    0,
                    0,
                    8,
                    1,
                    3,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    0,
                    9,
                    8,
                    1,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    0,
                    4,
                    3,
                    1,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    9,
                    0,
                    4,
                    12,
                    1,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 4..=8 {
                    for z in 0..=2 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, 12 - z);
                    }
                }

                for x in 0..=2 {
                    for z in 4..=8 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), 12 - x, -1, z);
                    }
                }

                ctx.generate_box(
                    b,
                    5,
                    5,
                    5,
                    7,
                    5,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 6, 1, 6, 6, 4, 6, ctx.bs("air"), ctx.bs("air"), false);
                ctx.place_block(b, ctx.bs("nether_bricks"), 6, 0, 6);
                ctx.place_block(b, ctx.bs("lava"), 6, 5, 6);
                let pos = b.world_pos(6, 5, 6);
                if ctx.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
                    ctx.region
                        .schedule_fluid_tick(pos, chunker_data::blocks::Fluid::Lava);
                }
            }
            NfKind::SmallCorridorCrossing => {
                ctx.generate_box(
                    b,
                    0,
                    0,
                    0,
                    4,
                    1,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 2, 0, 4, 5, 4, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    0,
                    5,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    2,
                    0,
                    4,
                    5,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    4,
                    0,
                    5,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    2,
                    4,
                    4,
                    5,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    6,
                    0,
                    4,
                    6,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 0..=4 {
                    for z in 0..=4 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
            NfKind::SmallCorridorLeftTurn { .. } => {
                ctx.generate_box(
                    b,
                    0,
                    0,
                    0,
                    4,
                    1,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 2, 0, 4, 5, 4, ctx.bs("air"), ctx.bs("air"), false);
                let we_fence = ctx
                    .bs("nether_brick_fence")
                    .with("west", "true")
                    .with("east", "true");
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                ctx.generate_box(
                    b,
                    4,
                    2,
                    0,
                    4,
                    5,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 4, 3, 1, 4, 4, 1, ns_fence, ns_fence, false);
                ctx.generate_box(b, 4, 3, 3, 4, 4, 3, ns_fence, ns_fence, false);
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    0,
                    5,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    4,
                    3,
                    5,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 1, 3, 4, 1, 4, 4, we_fence, we_fence, false);
                ctx.generate_box(b, 3, 3, 4, 3, 4, 4, we_fence, we_fence, false);
                let chest_pos = b.world_pos(3, 2, 3);
                if self.flag.load(Relaxed) && ctx.chunk_bb.is_inside(chest_pos.x, chest_pos.y, chest_pos.z) {
                    self.flag.store(false, Relaxed);
                    ctx.create_chest(b, random, 3, 2, 3, "minecraft:chests/nether_bridge");
                }

                ctx.generate_box(
                    b,
                    0,
                    6,
                    0,
                    4,
                    6,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 0..=4 {
                    for z in 0..=4 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
            NfKind::SmallCorridor => {
                ctx.generate_box(
                    b,
                    0,
                    0,
                    0,
                    4,
                    1,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 2, 0, 4, 5, 4, ctx.bs("air"), ctx.bs("air"), false);
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    0,
                    5,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    2,
                    0,
                    4,
                    5,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 3, 1, 0, 4, 1, ns_fence, ns_fence, false);
                ctx.generate_box(b, 0, 3, 3, 0, 4, 3, ns_fence, ns_fence, false);
                ctx.generate_box(b, 4, 3, 1, 4, 4, 1, ns_fence, ns_fence, false);
                ctx.generate_box(b, 4, 3, 3, 4, 4, 3, ns_fence, ns_fence, false);
                ctx.generate_box(
                    b,
                    0,
                    6,
                    0,
                    4,
                    6,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 0..=4 {
                    for z in 0..=4 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
            NfKind::SmallCorridorRightTurn { .. } => {
                ctx.generate_box(
                    b,
                    0,
                    0,
                    0,
                    4,
                    1,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 2, 0, 4, 5, 4, ctx.bs("air"), ctx.bs("air"), false);
                let we_fence = ctx
                    .bs("nether_brick_fence")
                    .with("west", "true")
                    .with("east", "true");
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    0,
                    5,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 3, 1, 0, 4, 1, ns_fence, ns_fence, false);
                ctx.generate_box(b, 0, 3, 3, 0, 4, 3, ns_fence, ns_fence, false);
                ctx.generate_box(
                    b,
                    4,
                    2,
                    0,
                    4,
                    5,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    2,
                    4,
                    4,
                    5,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 1, 3, 4, 1, 4, 4, we_fence, we_fence, false);
                ctx.generate_box(b, 3, 3, 4, 3, 4, 4, we_fence, we_fence, false);
                let chest_pos = b.world_pos(1, 2, 3);
                if self.flag.load(Relaxed) && ctx.chunk_bb.is_inside(chest_pos.x, chest_pos.y, chest_pos.z) {
                    self.flag.store(false, Relaxed);
                    ctx.create_chest(b, random, 1, 2, 3, "minecraft:chests/nether_bridge");
                }

                ctx.generate_box(
                    b,
                    0,
                    6,
                    0,
                    4,
                    6,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 0..=4 {
                    for z in 0..=4 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
            NfKind::StalkRoom => {
                ctx.generate_box(
                    b,
                    0,
                    3,
                    0,
                    12,
                    4,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 5, 0, 12, 13, 12, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    0,
                    5,
                    0,
                    1,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    11,
                    5,
                    0,
                    12,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    5,
                    11,
                    4,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    5,
                    11,
                    10,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    9,
                    11,
                    7,
                    12,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    5,
                    0,
                    4,
                    12,
                    1,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    5,
                    0,
                    10,
                    12,
                    1,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    9,
                    0,
                    7,
                    12,
                    1,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    11,
                    2,
                    10,
                    12,
                    10,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                let we_fence = ctx
                    .bs("nether_brick_fence")
                    .with("west", "true")
                    .with("east", "true");
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                let nsw_fence = ns_fence.with("west", "true");
                let nse_fence = ns_fence.with("east", "true");

                for i in (1..=11).step_by(2) {
                    ctx.generate_box(b, i, 10, 0, i, 11, 0, we_fence, we_fence, false);
                    ctx.generate_box(b, i, 10, 12, i, 11, 12, we_fence, we_fence, false);
                    ctx.generate_box(b, 0, 10, i, 0, 11, i, ns_fence, ns_fence, false);
                    ctx.generate_box(b, 12, 10, i, 12, 11, i, ns_fence, ns_fence, false);
                    ctx.place_block(b, ctx.bs("nether_bricks"), i, 13, 0);
                    ctx.place_block(b, ctx.bs("nether_bricks"), i, 13, 12);
                    ctx.place_block(b, ctx.bs("nether_bricks"), 0, 13, i);
                    ctx.place_block(b, ctx.bs("nether_bricks"), 12, 13, i);
                    if i != 11 {
                        ctx.place_block(b, we_fence, i + 1, 13, 0);
                        ctx.place_block(b, we_fence, i + 1, 13, 12);
                        ctx.place_block(b, ns_fence, 0, 13, i + 1);
                        ctx.place_block(b, ns_fence, 12, 13, i + 1);
                    }
                }

                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("north", "true")
                        .with("east", "true"),
                    0,
                    13,
                    0,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("south", "true")
                        .with("east", "true"),
                    0,
                    13,
                    12,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("south", "true")
                        .with("west", "true"),
                    12,
                    13,
                    12,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("north", "true")
                        .with("west", "true"),
                    12,
                    13,
                    0,
                );

                for z in (3..=9).step_by(2) {
                    ctx.generate_box(b, 1, 7, z, 1, 8, z, nsw_fence, nsw_fence, false);
                    ctx.generate_box(b, 11, 7, z, 11, 8, z, nse_fence, nse_fence, false);
                }

                let stairs = ctx.bs("nether_brick_stairs").with("facing", "north");

                for ix in 0..=6 {
                    let z = ix + 4;

                    for x in 5..=7 {
                        ctx.place_block(b, stairs, x, 5 + ix, z);
                    }

                    if (5..=8).contains(&z) {
                        ctx.generate_box(
                            b,
                            5,
                            5,
                            z,
                            7,
                            ix + 4,
                            z,
                            ctx.bs("nether_bricks"),
                            ctx.bs("nether_bricks"),
                            false,
                        );
                    } else if (9..=10).contains(&z) {
                        ctx.generate_box(
                            b,
                            5,
                            8,
                            z,
                            7,
                            ix + 4,
                            z,
                            ctx.bs("nether_bricks"),
                            ctx.bs("nether_bricks"),
                            false,
                        );
                    }

                    if ix >= 1 {
                        ctx.generate_box(b, 5, 6 + ix, z, 7, 9 + ix, z, ctx.bs("air"), ctx.bs("air"), false);
                    }
                }

                for x in 5..=7 {
                    ctx.place_block(b, stairs, x, 12, 11);
                }

                ctx.generate_box(b, 5, 6, 7, 5, 7, 7, nse_fence, nse_fence, false);
                ctx.generate_box(b, 7, 6, 7, 7, 7, 7, nsw_fence, nsw_fence, false);
                ctx.generate_box(b, 5, 13, 12, 7, 13, 12, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    2,
                    5,
                    2,
                    3,
                    5,
                    3,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    5,
                    9,
                    3,
                    5,
                    10,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    5,
                    4,
                    2,
                    5,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    9,
                    5,
                    2,
                    10,
                    5,
                    3,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    9,
                    5,
                    9,
                    10,
                    5,
                    10,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    10,
                    5,
                    4,
                    10,
                    5,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                let east_stairs = stairs.with("facing", "east");
                let west_stairs = stairs.with("facing", "west");
                ctx.place_block(b, west_stairs, 4, 5, 2);
                ctx.place_block(b, west_stairs, 4, 5, 3);
                ctx.place_block(b, west_stairs, 4, 5, 9);
                ctx.place_block(b, west_stairs, 4, 5, 10);
                ctx.place_block(b, east_stairs, 8, 5, 2);
                ctx.place_block(b, east_stairs, 8, 5, 3);
                ctx.place_block(b, east_stairs, 8, 5, 9);
                ctx.place_block(b, east_stairs, 8, 5, 10);
                ctx.generate_box(
                    b,
                    3,
                    4,
                    4,
                    4,
                    4,
                    8,
                    ctx.bs("soul_sand"),
                    ctx.bs("soul_sand"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    4,
                    4,
                    9,
                    4,
                    8,
                    ctx.bs("soul_sand"),
                    ctx.bs("soul_sand"),
                    false,
                );
                ctx.generate_box(
                    b,
                    3,
                    5,
                    4,
                    4,
                    5,
                    8,
                    ctx.bs("nether_wart"),
                    ctx.bs("nether_wart"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    5,
                    4,
                    9,
                    5,
                    8,
                    ctx.bs("nether_wart"),
                    ctx.bs("nether_wart"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    2,
                    0,
                    8,
                    2,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    4,
                    12,
                    2,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    0,
                    0,
                    8,
                    1,
                    3,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    0,
                    9,
                    8,
                    1,
                    12,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    0,
                    4,
                    3,
                    1,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    9,
                    0,
                    4,
                    12,
                    1,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );

                for x in 4..=8 {
                    for z in 0..=2 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, 12 - z);
                    }
                }

                for x in 0..=2 {
                    for z in 4..=8 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), 12 - x, -1, z);
                    }
                }
            }
            NfKind::MonsterThrone { .. } => {
                ctx.generate_box(b, 0, 2, 0, 6, 7, 7, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    1,
                    0,
                    0,
                    5,
                    1,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    2,
                    1,
                    5,
                    2,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    3,
                    2,
                    5,
                    3,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    4,
                    3,
                    5,
                    4,
                    7,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    2,
                    0,
                    1,
                    4,
                    2,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    2,
                    0,
                    5,
                    4,
                    2,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    5,
                    2,
                    1,
                    5,
                    3,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    5,
                    2,
                    5,
                    5,
                    3,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    5,
                    3,
                    0,
                    5,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    6,
                    5,
                    3,
                    6,
                    5,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    5,
                    8,
                    5,
                    5,
                    8,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                let we_fence = ctx
                    .bs("nether_brick_fence")
                    .with("west", "true")
                    .with("east", "true");
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                ctx.place_block(b, ctx.bs("nether_brick_fence").with("west", "true"), 1, 6, 3);
                ctx.place_block(b, ctx.bs("nether_brick_fence").with("east", "true"), 5, 6, 3);
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("east", "true")
                        .with("north", "true"),
                    0,
                    6,
                    3,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("west", "true")
                        .with("north", "true"),
                    6,
                    6,
                    3,
                );
                ctx.generate_box(b, 0, 6, 4, 0, 6, 7, ns_fence, ns_fence, false);
                ctx.generate_box(b, 6, 6, 4, 6, 6, 7, ns_fence, ns_fence, false);
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("east", "true")
                        .with("south", "true"),
                    0,
                    6,
                    8,
                );
                ctx.place_block(
                    b,
                    ctx.bs("nether_brick_fence")
                        .with("west", "true")
                        .with("south", "true"),
                    6,
                    6,
                    8,
                );
                ctx.generate_box(b, 1, 6, 8, 5, 6, 8, we_fence, we_fence, false);
                ctx.place_block(b, ctx.bs("nether_brick_fence").with("east", "true"), 1, 7, 8);
                ctx.generate_box(b, 2, 7, 8, 4, 7, 8, we_fence, we_fence, false);
                ctx.place_block(b, ctx.bs("nether_brick_fence").with("west", "true"), 5, 7, 8);
                ctx.place_block(b, ctx.bs("nether_brick_fence").with("east", "true"), 2, 8, 8);
                ctx.place_block(b, we_fence, 3, 8, 8);
                ctx.place_block(b, ctx.bs("nether_brick_fence").with("west", "true"), 4, 8, 8);
                if !self.flag.load(Relaxed) {
                    let pos = b.world_pos(3, 5, 5);
                    if ctx.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
                        self.flag.store(true, Relaxed);
                        let spawner = ctx.bs("spawner").s();
                        ctx.region.set(pos, spawner, 2);
                        if let Some(be) = ctx.region.block_entity(pos)
                            && be.get_str("id") == Some("minecraft:mob_spawner")
                        {
                            crate::feature::behaviour::set_spawner_entity(be, "minecraft:blaze");
                        }
                    }
                }

                for x in 0..=6 {
                    for z in 0..=6 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
            NfKind::RoomCrossing => {
                ctx.generate_box(
                    b,
                    0,
                    0,
                    0,
                    6,
                    1,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 2, 0, 6, 7, 6, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    1,
                    6,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    6,
                    1,
                    6,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    2,
                    0,
                    6,
                    6,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    2,
                    6,
                    6,
                    6,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    0,
                    6,
                    1,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    5,
                    0,
                    6,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    6,
                    2,
                    0,
                    6,
                    6,
                    1,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    6,
                    2,
                    5,
                    6,
                    6,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                let we_fence = ctx
                    .bs("nether_brick_fence")
                    .with("west", "true")
                    .with("east", "true");
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                ctx.generate_box(
                    b,
                    2,
                    6,
                    0,
                    4,
                    6,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 2, 5, 0, 4, 5, 0, we_fence, we_fence, false);
                ctx.generate_box(
                    b,
                    2,
                    6,
                    6,
                    4,
                    6,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 2, 5, 6, 4, 5, 6, we_fence, we_fence, false);
                ctx.generate_box(
                    b,
                    0,
                    6,
                    2,
                    0,
                    6,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 5, 2, 0, 5, 4, ns_fence, ns_fence, false);
                ctx.generate_box(
                    b,
                    6,
                    6,
                    2,
                    6,
                    6,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 6, 5, 2, 6, 5, 4, ns_fence, ns_fence, false);

                for x in 0..=6 {
                    for z in 0..=6 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
            NfKind::StairsRoom => {
                ctx.generate_box(
                    b,
                    0,
                    0,
                    0,
                    6,
                    1,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 0, 2, 0, 6, 10, 6, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    0,
                    2,
                    0,
                    1,
                    8,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    2,
                    0,
                    6,
                    8,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    0,
                    2,
                    1,
                    0,
                    8,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    6,
                    2,
                    1,
                    6,
                    8,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    2,
                    6,
                    5,
                    8,
                    6,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                let we_fence = ctx
                    .bs("nether_brick_fence")
                    .with("west", "true")
                    .with("east", "true");
                let ns_fence = ctx
                    .bs("nether_brick_fence")
                    .with("north", "true")
                    .with("south", "true");
                ctx.generate_box(b, 0, 3, 2, 0, 5, 4, ns_fence, ns_fence, false);
                ctx.generate_box(b, 6, 3, 2, 6, 5, 2, ns_fence, ns_fence, false);
                ctx.generate_box(b, 6, 3, 4, 6, 5, 4, ns_fence, ns_fence, false);
                ctx.place_block(b, ctx.bs("nether_bricks"), 5, 2, 5);
                ctx.generate_box(
                    b,
                    4,
                    2,
                    5,
                    4,
                    3,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    3,
                    2,
                    5,
                    3,
                    4,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    2,
                    2,
                    5,
                    2,
                    5,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    2,
                    5,
                    1,
                    6,
                    5,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    7,
                    1,
                    5,
                    7,
                    4,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 6, 8, 2, 6, 8, 4, ctx.bs("air"), ctx.bs("air"), false);
                ctx.generate_box(
                    b,
                    2,
                    6,
                    0,
                    4,
                    8,
                    0,
                    ctx.bs("nether_bricks"),
                    ctx.bs("nether_bricks"),
                    false,
                );
                ctx.generate_box(b, 2, 5, 0, 4, 5, 0, we_fence, we_fence, false);

                for x in 0..=6 {
                    for z in 0..=6 {
                        ctx.fill_column_down(b, ctx.bs("nether_bricks"), x, -1, z);
                    }
                }
            }
        }
    }
}
