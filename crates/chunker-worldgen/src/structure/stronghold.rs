//! `StrongholdStructure` + `StrongholdPieces` (piece layout and NBT).

use chunker_nbt::Compound;

use super::geom::{BlockPos, BoundingBox, Direction};
use super::start::{PieceBase, PiecesBuilder};
use super::{GenContext, Stub};
use crate::piece_boilerplate;
use crate::random::RandomSource;

/// `StrongholdPiece.SmallDoorType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Door {
    Opening,
    WoodDoor,
    Grates,
    IronDoor,
}

impl Door {
    fn name(self) -> &'static str {
        match self {
            Door::Opening => "OPENING",
            Door::WoodDoor => "WOOD_DOOR",
            Door::Grates => "GRATES",
            Door::IronDoor => "IRON_DOOR",
        }
    }

    fn random(random: &mut dyn RandomSource) -> Door {
        match random.next_int_bounded(5) {
            2 => Door::WoodDoor,
            3 => Door::Grates,
            4 => Door::IronDoor,
            _ => Door::Opening,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShKind {
    Start,
    Straight {
        left: bool,
        right: bool,
    },
    PrisonHall,
    LeftTurn,
    RightTurn,
    RoomCrossing {
        kind: i32,
    },
    StraightStairsDown,
    StairsDown {
        source: bool,
    },
    FiveCrossing {
        left_low: bool,
        left_high: bool,
        right_low: bool,
        right_high: bool,
    },
    ChestCorridor {
        chest: bool,
    },
    Library {
        tall: bool,
    },
    PortalRoom {
        spawner: bool,
    },
    FillerCorridor {
        steps: i32,
    },
}

/// `StrongholdPieces.StrongholdPiece` and subclasses.
pub struct ShPiece {
    pub base: PieceBase,
    pub entry_door: Door,
    pub kind: ShKind,
    /// `hasPlacedChest` / `hasPlacedSpawner` as changed by `postProcess`.
    pub flag: std::sync::atomic::AtomicBool,
}

impl ShPiece {
    pub fn new(base: PieceBase, entry_door: Door, kind: ShKind) -> Self {
        let flag = match kind {
            ShKind::ChestCorridor { chest } => chest,
            ShKind::PortalRoom { spawner } => spawner,
            _ => false,
        };
        Self {
            base,
            entry_door,
            kind,
            flag: std::sync::atomic::AtomicBool::new(flag),
        }
    }
}

impl super::start::Piece for ShPiece {
    piece_boilerplate!();
    fn post_process_ported(&self) -> bool {
        true
    }
    fn post_process(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        self.post_process_sh(ctx, random);
    }
    fn type_id(&self) -> &'static str {
        match self.kind {
            ShKind::Start => "minecraft:shstart",
            ShKind::Straight { .. } => "minecraft:shs",
            ShKind::PrisonHall => "minecraft:shph",
            ShKind::LeftTurn => "minecraft:shlt",
            ShKind::RightTurn => "minecraft:shrt",
            ShKind::RoomCrossing { .. } => "minecraft:shrc",
            ShKind::StraightStairsDown => "minecraft:shssd",
            ShKind::StairsDown { .. } => "minecraft:shsd",
            ShKind::FiveCrossing { .. } => "minecraft:sh5c",
            ShKind::ChestCorridor { .. } => "minecraft:shcc",
            ShKind::Library { .. } => "minecraft:shli",
            ShKind::PortalRoom { .. } => "minecraft:shpr",
            ShKind::FillerCorridor { .. } => "minecraft:shfc",
        }
    }
    fn save_extra(&self, tag: &mut Compound) {
        tag.insert("EntryDoor", self.entry_door.name());
        match self.kind {
            ShKind::Start => tag.insert("Source", true),
            ShKind::StairsDown { source } => tag.insert("Source", source),
            ShKind::Straight { left, right } => {
                tag.insert("Left", left);
                tag.insert("Right", right);
            }
            ShKind::RoomCrossing { kind } => tag.insert("Type", kind),
            ShKind::FiveCrossing {
                left_low,
                left_high,
                right_low,
                right_high,
            } => {
                tag.insert("leftLow", left_low);
                tag.insert("leftHigh", left_high);
                tag.insert("rightLow", right_low);
                tag.insert("rightHigh", right_high);
            }
            ShKind::ChestCorridor { chest } => tag.insert("Chest", chest),
            ShKind::Library { tall } => tag.insert("Tall", tall),
            ShKind::PortalRoom { spawner } => tag.insert("Mob", spawner),
            ShKind::FillerCorridor { steps } => tag.insert("Steps", steps),
            ShKind::PrisonHall | ShKind::LeftTurn | ShKind::RightTurn | ShKind::StraightStairsDown => {}
        }
    }
}

/// Indices into [`WEIGHTS`] (`STRONGHOLD_PIECE_WEIGHTS` order).
const STRAIGHT: usize = 0;
const PRISON_HALL: usize = 1;
const LEFT_TURN: usize = 2;
const RIGHT_TURN: usize = 3;
const ROOM_CROSSING: usize = 4;
const STRAIGHT_STAIRS_DOWN: usize = 5;
const STAIRS_DOWN: usize = 6;
const FIVE_CROSSING: usize = 7;
const CHEST_CORRIDOR: usize = 8;
const LIBRARY: usize = 9;
const PORTAL_ROOM: usize = 10;

/// `(weight, maxPlaceCount)`.
const WEIGHTS: [(i32, i32); 11] = [
    (40, 0),
    (5, 5),
    (20, 0),
    (20, 0),
    (10, 6),
    (5, 5),
    (5, 5),
    (5, 4),
    (5, 4),
    (10, 2),
    (20, 1),
];

struct Gen<'r> {
    pieces: Vec<ShPiece>,
    pending: Vec<usize>,
    /// `currentPieces`: weight entries still available, with their place counts.
    current: Vec<usize>,
    place_count: [i32; 11],
    imposed: Option<usize>,
    previous: Option<usize>,
    total_weight: i32,
    portal_room: bool,
    random: &'r mut dyn RandomSource,
}

fn ok_box(b: &BoundingBox) -> bool {
    b.min_y > 10
}

impl Gen<'_> {
    fn collides(&self, b: &BoundingBox) -> Option<usize> {
        self.pieces.iter().position(|p| p.base.bbox.intersects(b))
    }

    fn do_place(&self, w: usize, depth: i32) -> bool {
        let (_, max) = WEIGHTS[w];
        let base = max == 0 || self.place_count[w] < max;
        match w {
            LIBRARY => base && depth > 4,
            PORTAL_ROOM => base && depth > 5,
            _ => base,
        }
    }

    fn is_valid(&self, w: usize) -> bool {
        let (_, max) = WEIGHTS[w];
        max == 0 || self.place_count[w] < max
    }

    fn update_weight(&mut self) -> bool {
        let mut any = false;
        self.total_weight = 0;
        for &w in &self.current {
            let (weight, max) = WEIGHTS[w];
            if max > 0 && self.place_count[w] < max {
                any = true;
            }
            self.total_weight += weight;
        }
        any
    }

    /// `findAndCreatePieceFactory`.
    fn create(&mut self, w: usize, x: i32, y: i32, z: i32, d: Direction, depth: i32) -> Option<ShPiece> {
        let orient = |ox: i32, oy: i32, oz: i32, wd: i32, h: i32, dp: i32| {
            BoundingBox::orient_box(x, y, z, ox, oy, oz, wd, h, dp, d)
        };
        let free = |g: &Self, b: &BoundingBox| ok_box(b) && g.collides(b).is_none();
        let piece = |base: BoundingBox, door: Door, kind: ShKind| {
            ShPiece::new(PieceBase::new(depth, base, Some(d)), door, kind)
        };
        match w {
            STRAIGHT => {
                let b = orient(-1, -1, 0, 5, 5, 7);
                free(self, &b).then(|| {
                    let door = Door::random(self.random);
                    let left = self.random.next_int_bounded(2) == 0;
                    let right = self.random.next_int_bounded(2) == 0;
                    piece(b, door, ShKind::Straight { left, right })
                })
            }
            PRISON_HALL => {
                let b = orient(-1, -1, 0, 9, 5, 11);
                free(self, &b).then(|| piece(b, Door::random(self.random), ShKind::PrisonHall))
            }
            LEFT_TURN | RIGHT_TURN => {
                let b = orient(-1, -1, 0, 5, 5, 5);
                let kind = if w == LEFT_TURN {
                    ShKind::LeftTurn
                } else {
                    ShKind::RightTurn
                };
                free(self, &b).then(|| piece(b, Door::random(self.random), kind))
            }
            ROOM_CROSSING => {
                let b = orient(-4, -1, 0, 11, 7, 11);
                free(self, &b).then(|| {
                    let door = Door::random(self.random);
                    let kind = self.random.next_int_bounded(5);
                    piece(b, door, ShKind::RoomCrossing { kind })
                })
            }
            STRAIGHT_STAIRS_DOWN => {
                let b = orient(-1, -7, 0, 5, 11, 8);
                free(self, &b).then(|| piece(b, Door::random(self.random), ShKind::StraightStairsDown))
            }
            STAIRS_DOWN => {
                let b = orient(-1, -7, 0, 5, 11, 5);
                free(self, &b)
                    .then(|| piece(b, Door::random(self.random), ShKind::StairsDown { source: false }))
            }
            FIVE_CROSSING => {
                let b = orient(-4, -3, 0, 10, 9, 11);
                free(self, &b).then(|| {
                    let door = Door::random(self.random);
                    let left_low = self.random.next_bool();
                    let left_high = self.random.next_bool();
                    let right_low = self.random.next_bool();
                    let right_high = self.random.next_int_bounded(3) > 0;
                    piece(
                        b,
                        door,
                        ShKind::FiveCrossing {
                            left_low,
                            left_high,
                            right_low,
                            right_high,
                        },
                    )
                })
            }
            CHEST_CORRIDOR => {
                let b = orient(-1, -1, 0, 5, 5, 7);
                free(self, &b).then(|| {
                    piece(
                        b,
                        Door::random(self.random),
                        ShKind::ChestCorridor { chest: false },
                    )
                })
            }
            LIBRARY => {
                let mut b = orient(-4, -1, 0, 14, 11, 15);
                if !free(self, &b) {
                    b = orient(-4, -1, 0, 14, 6, 15);
                    if !free(self, &b) {
                        return None;
                    }
                }
                let door = Door::random(self.random);
                Some(piece(b, door, ShKind::Library { tall: b.y_span() > 6 }))
            }
            PORTAL_ROOM => {
                let b = orient(-4, -1, 0, 11, 8, 16);
                free(self, &b).then(|| piece(b, Door::Opening, ShKind::PortalRoom { spawner: false }))
            }
            _ => None,
        }
    }

    /// `FillerCorridor.findPieceBox`.
    fn filler_box(&self, x: i32, y: i32, z: i32, d: Direction) -> Option<BoundingBox> {
        let b = BoundingBox::orient_box(x, y, z, -1, -1, 0, 5, 5, 4, d);
        let collision = self.pieces[self.collides(&b)?].base.bbox;
        if collision.min_y == b.min_y {
            for depth in (1..=2).rev() {
                let b = BoundingBox::orient_box(x, y, z, -1, -1, 0, 5, 5, depth, d);
                if !collision.intersects(&b) {
                    return Some(BoundingBox::orient_box(x, y, z, -1, -1, 0, 5, 5, depth + 1, d));
                }
            }
        }
        None
    }

    /// `generatePieceFromSmallDoor`.
    fn piece_from_small_door(&mut self, x: i32, y: i32, z: i32, d: Direction, depth: i32) -> Option<ShPiece> {
        if !self.update_weight() {
            return None;
        }
        if let Some(imposed) = self.imposed.take()
            && let Some(p) = self.create(imposed, x, y, z, d, depth)
        {
            return Some(p);
        }
        for _ in 0..5 {
            let mut selection = self.random.next_int_bounded(self.total_weight);
            let mut i = 0;
            while i < self.current.len() {
                let w = self.current[i];
                selection -= WEIGHTS[w].0;
                if selection < 0 {
                    if !self.do_place(w, depth) || Some(w) == self.previous {
                        break;
                    }
                    if let Some(p) = self.create(w, x, y, z, d, depth) {
                        self.place_count[w] += 1;
                        self.previous = Some(w);
                        if !self.is_valid(w) {
                            self.current.remove(i);
                        }
                        return Some(p);
                    }
                }
                i += 1;
            }
        }
        let b = self.filler_box(x, y, z, d)?;
        if b.min_y <= 1 {
            return None;
        }
        let steps = if d != Direction::North && d != Direction::South {
            b.x_span()
        } else {
            b.z_span()
        };
        Some(ShPiece::new(
            PieceBase::new(depth, b, Some(d)),
            Door::Opening,
            ShKind::FillerCorridor { steps },
        ))
    }

    /// `generateAndAddPiece`.
    fn generate_and_add(&mut self, x: i32, y: i32, z: i32, d: Direction, depth: i32) {
        if depth > 50 {
            return;
        }
        let start = self.pieces[0].base.bbox;
        if (x - start.min_x).abs() > 112 || (z - start.min_z).abs() > 112 {
            return;
        }
        if let Some(p) = self.piece_from_small_door(x, y, z, d, depth + 1) {
            self.pieces.push(p);
            self.pending.push(self.pieces.len() - 1);
        }
    }

    fn forward(&mut self, i: usize, x_off: i32, y_off: i32) {
        let p = &self.pieces[i].base;
        let (b, depth) = (p.bbox, p.gen_depth);
        match p.orientation {
            Some(Direction::North) => self.generate_and_add(
                b.min_x + x_off,
                b.min_y + y_off,
                b.min_z - 1,
                Direction::North,
                depth,
            ),
            Some(Direction::South) => self.generate_and_add(
                b.min_x + x_off,
                b.min_y + y_off,
                b.max_z + 1,
                Direction::South,
                depth,
            ),
            Some(Direction::West) => self.generate_and_add(
                b.min_x - 1,
                b.min_y + y_off,
                b.min_z + x_off,
                Direction::West,
                depth,
            ),
            Some(Direction::East) => self.generate_and_add(
                b.max_x + 1,
                b.min_y + y_off,
                b.min_z + x_off,
                Direction::East,
                depth,
            ),
            _ => {}
        }
    }

    fn left(&mut self, i: usize, y_off: i32, z_off: i32) {
        let p = &self.pieces[i].base;
        let (b, depth) = (p.bbox, p.gen_depth);
        match p.orientation {
            Some(Direction::North) | Some(Direction::South) => self.generate_and_add(
                b.min_x - 1,
                b.min_y + y_off,
                b.min_z + z_off,
                Direction::West,
                depth,
            ),
            Some(Direction::West) | Some(Direction::East) => self.generate_and_add(
                b.min_x + z_off,
                b.min_y + y_off,
                b.min_z - 1,
                Direction::North,
                depth,
            ),
            _ => {}
        }
    }

    fn right(&mut self, i: usize, y_off: i32, z_off: i32) {
        let p = &self.pieces[i].base;
        let (b, depth) = (p.bbox, p.gen_depth);
        match p.orientation {
            Some(Direction::North) | Some(Direction::South) => self.generate_and_add(
                b.max_x + 1,
                b.min_y + y_off,
                b.min_z + z_off,
                Direction::East,
                depth,
            ),
            Some(Direction::West) | Some(Direction::East) => self.generate_and_add(
                b.min_x + z_off,
                b.min_y + y_off,
                b.max_z + 1,
                Direction::South,
                depth,
            ),
            _ => {}
        }
    }

    /// `addChildren` of piece `i`.
    fn add_children(&mut self, i: usize) {
        let orientation = self.pieces[i].base.orientation;
        match self.pieces[i].kind {
            ShKind::Start => {
                self.imposed = Some(FIVE_CROSSING);
                self.forward(i, 1, 1);
            }
            ShKind::StairsDown { source } => {
                if source {
                    self.imposed = Some(FIVE_CROSSING);
                }
                self.forward(i, 1, 1);
            }
            ShKind::Straight { left, right } => {
                self.forward(i, 1, 1);
                if left {
                    self.left(i, 1, 2);
                }
                if right {
                    self.right(i, 1, 2);
                }
            }
            ShKind::PrisonHall | ShKind::ChestCorridor { .. } | ShKind::StraightStairsDown => {
                self.forward(i, 1, 1)
            }
            ShKind::LeftTurn => {
                if orientation != Some(Direction::North) && orientation != Some(Direction::East) {
                    self.right(i, 1, 1);
                } else {
                    self.left(i, 1, 1);
                }
            }
            ShKind::RightTurn => {
                if orientation != Some(Direction::North) && orientation != Some(Direction::East) {
                    self.left(i, 1, 1);
                } else {
                    self.right(i, 1, 1);
                }
            }
            ShKind::RoomCrossing { .. } => {
                self.forward(i, 4, 1);
                self.left(i, 1, 4);
                self.right(i, 1, 4);
            }
            ShKind::FiveCrossing {
                left_low,
                left_high,
                right_low,
                right_high,
            } => {
                let (mut a, mut b) = (3, 5);
                if orientation == Some(Direction::West) || orientation == Some(Direction::North) {
                    a = 8 - a;
                    b = 8 - b;
                }
                self.forward(i, 5, 1);
                if left_low {
                    self.left(i, a, 1);
                }
                if left_high {
                    self.left(i, b, 7);
                }
                if right_low {
                    self.right(i, a, 1);
                }
                if right_high {
                    self.right(i, b, 7);
                }
            }
            ShKind::PortalRoom { .. } => self.portal_room = true,
            ShKind::Library { .. } | ShKind::FillerCorridor { .. } => {}
        }
    }
}

/// `StrongholdStructure.findGenerationPoint`.
pub fn find_generation_point<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    let pos = BlockPos::new(ctx.min_block_x(), 0, ctx.min_block_z());
    Some(Stub::lazy(
        pos,
        |ctx: &mut GenContext, builder: &mut PiecesBuilder| {
            let mut tries = 0i64;
            loop {
                builder.pieces.clear();
                let seed = ctx.seed.wrapping_add(tries);
                tries += 1;
                ctx.random.set_large_feature_seed(seed, ctx.chunk_x, ctx.chunk_z);
                let (west, north) = (ctx.min_block_x() + 2, ctx.min_block_z() + 2);
                let portal_room = {
                    let random: &mut dyn RandomSource = &mut ctx.random;
                    let d = Direction::random_horizontal(random);
                    let bbox = PieceBase::make_bounding_box(west, 64, north, d, 5, 11, 5);
                    let start = ShPiece::new(PieceBase::new(0, bbox, Some(d)), Door::Opening, ShKind::Start);
                    let mut g = Gen {
                        pieces: vec![start],
                        pending: Vec::new(),
                        current: (0..WEIGHTS.len()).collect(),
                        place_count: [0; 11],
                        imposed: None,
                        previous: None,
                        total_weight: 0,
                        portal_room: false,
                        random,
                    };
                    g.add_children(0);
                    while !g.pending.is_empty() {
                        let pos = g.random.next_int_bounded(g.pending.len() as i32) as usize;
                        let piece = g.pending.remove(pos);
                        g.add_children(piece);
                    }
                    let portal_room = g.portal_room;
                    for p in g.pieces {
                        builder.add(Box::new(p));
                    }
                    portal_room
                };
                let sea_level = ctx.sea_level();
                let min_y = ctx.terrain.settings.noise.min_y;
                builder.move_below_sea_level(sea_level, min_y, &mut ctx.random, 10);
                if !builder.is_empty() && portal_room {
                    break;
                }
            }
        },
    ))
}

// --- postProcess (bodies translated statement by statement from `StrongholdPieces`) --------------

use super::build::{BlockSelector, PieceCtx};
use chunker_data::blocks::BlockState;
use std::sync::atomic::Ordering::Relaxed;

/// `StrongholdPieces.SmoothStoneSelector`.
struct SmoothStone {
    cracked: BlockState,
    mossy: BlockState,
    infested: BlockState,
    bricks: BlockState,
    cave_air: BlockState,
}

impl BlockSelector for SmoothStone {
    fn next(&mut self, random: &mut dyn RandomSource, _x: i32, _y: i32, _z: i32, edge: bool) -> BlockState {
        if !edge {
            return self.cave_air;
        }
        let s = random.next_float();
        if s < 0.2 {
            self.cracked
        } else if s < 0.5 {
            self.mossy
        } else if s < 0.55 {
            self.infested
        } else {
            self.bricks
        }
    }
}

/// `StrongholdPiece.generateSmallDoor`.
fn small_door(ctx: &mut PieceCtx, b: &PieceBase, door: Door, x: i32, y: i32, z: i32) {
    let cave_air = ctx.bs("cave_air");
    let bricks = ctx.bs("stone_bricks");
    match door {
        Door::Opening => ctx.generate_box(b, x, y, z, x + 3 - 1, y + 3 - 1, z, cave_air, cave_air, false),
        Door::WoodDoor | Door::IronDoor => {
            for (dx, dy) in [(0, 0), (0, 1), (0, 2), (1, 2), (2, 2), (2, 1), (2, 0)] {
                ctx.place_block(b, bricks, x + dx, y + dy, z);
            }
            let name = if door == Door::WoodDoor {
                "oak_door"
            } else {
                "iron_door"
            };
            ctx.place_block(b, ctx.bs(name), x + 1, y, z);
            ctx.place_block(b, ctx.bs(name).with("half", "upper"), x + 1, y + 1, z);
            if door == Door::IronDoor {
                ctx.place_block(
                    b,
                    ctx.bs("stone_button").with("facing", "north"),
                    x + 2,
                    y + 1,
                    z + 1,
                );
                ctx.place_block(
                    b,
                    ctx.bs("stone_button").with("facing", "south"),
                    x + 2,
                    y + 1,
                    z - 1,
                );
            }
        }
        Door::Grates => {
            ctx.place_block(b, cave_air, x + 1, y, z);
            ctx.place_block(b, cave_air, x + 1, y + 1, z);
            ctx.place_block(b, ctx.bs("iron_bars").with("west", "true"), x, y, z);
            ctx.place_block(b, ctx.bs("iron_bars").with("west", "true"), x, y + 1, z);
            ctx.place_block(
                b,
                ctx.bs("iron_bars").with("east", "true").with("west", "true"),
                x,
                y + 2,
                z,
            );
            ctx.place_block(
                b,
                ctx.bs("iron_bars").with("east", "true").with("west", "true"),
                x + 1,
                y + 2,
                z,
            );
            ctx.place_block(
                b,
                ctx.bs("iron_bars").with("east", "true").with("west", "true"),
                x + 2,
                y + 2,
                z,
            );
            ctx.place_block(b, ctx.bs("iron_bars").with("east", "true"), x + 2, y + 1, z);
            ctx.place_block(b, ctx.bs("iron_bars").with("east", "true"), x + 2, y, z);
        }
    }
}

impl ShPiece {
    fn post_process_sh(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        let b = &self.base;
        let cave_air = ctx.bs("cave_air");
        let mut smooth = SmoothStone {
            cracked: ctx.bs("cracked_stone_bricks").s(),
            mossy: ctx.bs("mossy_stone_bricks").s(),
            infested: ctx.bs("infested_stone_bricks").s(),
            bricks: ctx.bs("stone_bricks").s(),
            cave_air: cave_air.s(),
        };
        match &self.kind {
            ShKind::ChestCorridor { .. } => {
                ctx.generate_box_selector(b, 0, 0, 0, 4, 4, 6, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 1, 1, 0);
                small_door(ctx, b, Door::Opening, 1, 1, 6);
                ctx.generate_box(
                    b,
                    3,
                    1,
                    2,
                    3,
                    1,
                    4,
                    ctx.bs("stone_bricks"),
                    ctx.bs("stone_bricks"),
                    false,
                );
                ctx.place_block(b, ctx.bs("stone_brick_slab"), 3, 1, 1);
                ctx.place_block(b, ctx.bs("stone_brick_slab"), 3, 1, 5);
                ctx.place_block(b, ctx.bs("stone_brick_slab"), 3, 2, 2);
                ctx.place_block(b, ctx.bs("stone_brick_slab"), 3, 2, 4);

                for z in 2..=4 {
                    ctx.place_block(b, ctx.bs("stone_brick_slab"), 2, 1, z);
                }

                let chest_pos = b.world_pos(3, 2, 3);
                if !self.flag.load(Relaxed) && ctx.chunk_bb.is_inside(chest_pos.x, chest_pos.y, chest_pos.z) {
                    self.flag.store(true, Relaxed);
                    ctx.create_chest(b, random, 3, 2, 3, "minecraft:chests/stronghold_corridor");
                }
            }
            ShKind::FillerCorridor { steps } => {
                for i in 0..*steps {
                    ctx.place_block(b, ctx.bs("stone_bricks"), 0, 0, i);
                    ctx.place_block(b, ctx.bs("stone_bricks"), 1, 0, i);
                    ctx.place_block(b, ctx.bs("stone_bricks"), 2, 0, i);
                    ctx.place_block(b, ctx.bs("stone_bricks"), 3, 0, i);
                    ctx.place_block(b, ctx.bs("stone_bricks"), 4, 0, i);

                    for y in 1..=3 {
                        ctx.place_block(b, ctx.bs("stone_bricks"), 0, y, i);
                        ctx.place_block(b, ctx.bs("cave_air"), 1, y, i);
                        ctx.place_block(b, ctx.bs("cave_air"), 2, y, i);
                        ctx.place_block(b, ctx.bs("cave_air"), 3, y, i);
                        ctx.place_block(b, ctx.bs("stone_bricks"), 4, y, i);
                    }

                    ctx.place_block(b, ctx.bs("stone_bricks"), 0, 4, i);
                    ctx.place_block(b, ctx.bs("stone_bricks"), 1, 4, i);
                    ctx.place_block(b, ctx.bs("stone_bricks"), 2, 4, i);
                    ctx.place_block(b, ctx.bs("stone_bricks"), 3, 4, i);
                    ctx.place_block(b, ctx.bs("stone_bricks"), 4, 4, i);
                }
            }
            ShKind::FiveCrossing {
                left_low,
                left_high,
                right_low,
                right_high,
            } => {
                ctx.generate_box_selector(b, 0, 0, 0, 9, 8, 10, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 4, 3, 0);
                if *left_low {
                    ctx.generate_box(b, 0, 3, 1, 0, 5, 3, cave_air, cave_air, false);
                }

                if *right_low {
                    ctx.generate_box(b, 9, 3, 1, 9, 5, 3, cave_air, cave_air, false);
                }

                if *left_high {
                    ctx.generate_box(b, 0, 5, 7, 0, 7, 9, cave_air, cave_air, false);
                }

                if *right_high {
                    ctx.generate_box(b, 9, 5, 7, 9, 7, 9, cave_air, cave_air, false);
                }

                ctx.generate_box(b, 5, 1, 10, 7, 3, 10, cave_air, cave_air, false);
                ctx.generate_box_selector(b, 1, 2, 1, 8, 2, 6, false, random, &mut smooth);
                ctx.generate_box_selector(b, 4, 1, 5, 4, 4, 9, false, random, &mut smooth);
                ctx.generate_box_selector(b, 8, 1, 5, 8, 4, 9, false, random, &mut smooth);
                ctx.generate_box_selector(b, 1, 4, 7, 3, 4, 9, false, random, &mut smooth);
                ctx.generate_box_selector(b, 1, 3, 5, 3, 3, 6, false, random, &mut smooth);
                ctx.generate_box(
                    b,
                    1,
                    3,
                    4,
                    3,
                    3,
                    4,
                    ctx.bs("smooth_stone_slab"),
                    ctx.bs("smooth_stone_slab"),
                    false,
                );
                ctx.generate_box(
                    b,
                    1,
                    4,
                    6,
                    3,
                    4,
                    6,
                    ctx.bs("smooth_stone_slab"),
                    ctx.bs("smooth_stone_slab"),
                    false,
                );
                ctx.generate_box_selector(b, 5, 1, 7, 7, 1, 8, false, random, &mut smooth);
                ctx.generate_box(
                    b,
                    5,
                    1,
                    9,
                    7,
                    1,
                    9,
                    ctx.bs("smooth_stone_slab"),
                    ctx.bs("smooth_stone_slab"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    2,
                    7,
                    7,
                    2,
                    7,
                    ctx.bs("smooth_stone_slab"),
                    ctx.bs("smooth_stone_slab"),
                    false,
                );
                ctx.generate_box(
                    b,
                    4,
                    5,
                    7,
                    4,
                    5,
                    9,
                    ctx.bs("smooth_stone_slab"),
                    ctx.bs("smooth_stone_slab"),
                    false,
                );
                ctx.generate_box(
                    b,
                    8,
                    5,
                    7,
                    8,
                    5,
                    9,
                    ctx.bs("smooth_stone_slab"),
                    ctx.bs("smooth_stone_slab"),
                    false,
                );
                ctx.generate_box(
                    b,
                    5,
                    5,
                    7,
                    7,
                    5,
                    9,
                    ctx.bs("smooth_stone_slab").with("type", "double"),
                    ctx.bs("smooth_stone_slab").with("type", "double"),
                    false,
                );
                ctx.place_block(b, ctx.bs("wall_torch").with("facing", "south"), 6, 5, 6);
            }
            ShKind::LeftTurn => {
                ctx.generate_box_selector(b, 0, 0, 0, 4, 4, 4, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 1, 1, 0);
                let orientation = b.orientation;
                if orientation != Some(Direction::North) && orientation != Some(Direction::East) {
                    ctx.generate_box(b, 4, 1, 1, 4, 3, 3, cave_air, cave_air, false);
                } else {
                    ctx.generate_box(b, 0, 1, 1, 0, 3, 3, cave_air, cave_air, false);
                }
            }
            ShKind::Library { tall } => {
                let current_height = if *tall { 11 } else { 6 };

                ctx.generate_box_selector(b, 0, 0, 0, 13, current_height - 1, 14, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 4, 1, 0);
                ctx.generate_maybe_box(
                    b,
                    random,
                    0.07,
                    2,
                    1,
                    1,
                    11,
                    4,
                    13,
                    ctx.bs("cobweb"),
                    ctx.bs("cobweb"),
                    false,
                    false,
                );

                for d in 1..=13 {
                    if (d - 1) % 4 == 0 {
                        ctx.generate_box(
                            b,
                            1,
                            1,
                            d,
                            1,
                            4,
                            d,
                            ctx.bs("oak_planks"),
                            ctx.bs("oak_planks"),
                            false,
                        );
                        ctx.generate_box(
                            b,
                            12,
                            1,
                            d,
                            12,
                            4,
                            d,
                            ctx.bs("oak_planks"),
                            ctx.bs("oak_planks"),
                            false,
                        );
                        ctx.place_block(b, ctx.bs("wall_torch").with("facing", "east"), 2, 3, d);
                        ctx.place_block(b, ctx.bs("wall_torch").with("facing", "west"), 11, 3, d);
                        if *tall {
                            ctx.generate_box(
                                b,
                                1,
                                6,
                                d,
                                1,
                                9,
                                d,
                                ctx.bs("oak_planks"),
                                ctx.bs("oak_planks"),
                                false,
                            );
                            ctx.generate_box(
                                b,
                                12,
                                6,
                                d,
                                12,
                                9,
                                d,
                                ctx.bs("oak_planks"),
                                ctx.bs("oak_planks"),
                                false,
                            );
                        }
                    } else {
                        ctx.generate_box(
                            b,
                            1,
                            1,
                            d,
                            1,
                            4,
                            d,
                            ctx.bs("bookshelf"),
                            ctx.bs("bookshelf"),
                            false,
                        );
                        ctx.generate_box(
                            b,
                            12,
                            1,
                            d,
                            12,
                            4,
                            d,
                            ctx.bs("bookshelf"),
                            ctx.bs("bookshelf"),
                            false,
                        );
                        if *tall {
                            ctx.generate_box(
                                b,
                                1,
                                6,
                                d,
                                1,
                                9,
                                d,
                                ctx.bs("bookshelf"),
                                ctx.bs("bookshelf"),
                                false,
                            );
                            ctx.generate_box(
                                b,
                                12,
                                6,
                                d,
                                12,
                                9,
                                d,
                                ctx.bs("bookshelf"),
                                ctx.bs("bookshelf"),
                                false,
                            );
                        }
                    }
                }

                for dx in (3..12).step_by(2) {
                    ctx.generate_box(
                        b,
                        3,
                        1,
                        dx,
                        4,
                        3,
                        dx,
                        ctx.bs("bookshelf"),
                        ctx.bs("bookshelf"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        6,
                        1,
                        dx,
                        7,
                        3,
                        dx,
                        ctx.bs("bookshelf"),
                        ctx.bs("bookshelf"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        9,
                        1,
                        dx,
                        10,
                        3,
                        dx,
                        ctx.bs("bookshelf"),
                        ctx.bs("bookshelf"),
                        false,
                    );
                }

                if *tall {
                    ctx.generate_box(
                        b,
                        1,
                        5,
                        1,
                        3,
                        5,
                        13,
                        ctx.bs("oak_planks"),
                        ctx.bs("oak_planks"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        10,
                        5,
                        1,
                        12,
                        5,
                        13,
                        ctx.bs("oak_planks"),
                        ctx.bs("oak_planks"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        4,
                        5,
                        1,
                        9,
                        5,
                        2,
                        ctx.bs("oak_planks"),
                        ctx.bs("oak_planks"),
                        false,
                    );
                    ctx.generate_box(
                        b,
                        4,
                        5,
                        12,
                        9,
                        5,
                        13,
                        ctx.bs("oak_planks"),
                        ctx.bs("oak_planks"),
                        false,
                    );
                    ctx.place_block(b, ctx.bs("oak_planks"), 9, 5, 11);
                    ctx.place_block(b, ctx.bs("oak_planks"), 8, 5, 11);
                    ctx.place_block(b, ctx.bs("oak_planks"), 9, 5, 10);
                    let we_fence = ctx.bs("oak_fence").with("west", "true").with("east", "true");
                    let ns_fence = ctx.bs("oak_fence").with("north", "true").with("south", "true");
                    ctx.generate_box(b, 3, 6, 3, 3, 6, 11, ns_fence, ns_fence, false);
                    ctx.generate_box(b, 10, 6, 3, 10, 6, 9, ns_fence, ns_fence, false);
                    ctx.generate_box(b, 4, 6, 2, 9, 6, 2, we_fence, we_fence, false);
                    ctx.generate_box(b, 4, 6, 12, 7, 6, 12, we_fence, we_fence, false);
                    ctx.place_block(
                        b,
                        ctx.bs("oak_fence").with("north", "true").with("east", "true"),
                        3,
                        6,
                        2,
                    );
                    ctx.place_block(
                        b,
                        ctx.bs("oak_fence").with("south", "true").with("east", "true"),
                        3,
                        6,
                        12,
                    );
                    ctx.place_block(
                        b,
                        ctx.bs("oak_fence").with("north", "true").with("west", "true"),
                        10,
                        6,
                        2,
                    );

                    for i in 0..=2 {
                        ctx.place_block(
                            b,
                            ctx.bs("oak_fence").with("south", "true").with("west", "true"),
                            8 + i,
                            6,
                            12 - i,
                        );
                        if i != 2 {
                            ctx.place_block(
                                b,
                                ctx.bs("oak_fence").with("north", "true").with("east", "true"),
                                8 + i,
                                6,
                                11 - i,
                            );
                        }
                    }

                    let ladder = ctx.bs("ladder").with("facing", "south");
                    ctx.place_block(b, ladder, 10, 1, 13);
                    ctx.place_block(b, ladder, 10, 2, 13);
                    ctx.place_block(b, ladder, 10, 3, 13);
                    ctx.place_block(b, ladder, 10, 4, 13);
                    ctx.place_block(b, ladder, 10, 5, 13);
                    ctx.place_block(b, ladder, 10, 6, 13);
                    ctx.place_block(b, ladder, 10, 7, 13);
                    // (vanilla declares x = 7, z = 7 here, unused)
                    let e_fence = ctx.bs("oak_fence").with("east", "true");
                    ctx.place_block(b, e_fence, 6, 9, 7);
                    let w_fence = ctx.bs("oak_fence").with("west", "true");
                    ctx.place_block(b, w_fence, 7, 9, 7);
                    ctx.place_block(b, e_fence, 6, 8, 7);
                    ctx.place_block(b, w_fence, 7, 8, 7);
                    let nswe_fence = ns_fence.with("west", "true").with("east", "true");
                    ctx.place_block(b, nswe_fence, 6, 7, 7);
                    ctx.place_block(b, nswe_fence, 7, 7, 7);
                    ctx.place_block(b, e_fence, 5, 7, 7);
                    ctx.place_block(b, w_fence, 8, 7, 7);
                    ctx.place_block(b, e_fence.with("north", "true"), 6, 7, 6);
                    ctx.place_block(b, e_fence.with("south", "true"), 6, 7, 8);
                    ctx.place_block(b, w_fence.with("north", "true"), 7, 7, 6);
                    ctx.place_block(b, w_fence.with("south", "true"), 7, 7, 8);
                    let torch = ctx.bs("torch");
                    ctx.place_block(b, torch, 5, 8, 7);
                    ctx.place_block(b, torch, 8, 8, 7);
                    ctx.place_block(b, torch, 6, 8, 6);
                    ctx.place_block(b, torch, 6, 8, 8);
                    ctx.place_block(b, torch, 7, 8, 6);
                    ctx.place_block(b, torch, 7, 8, 8);
                }

                ctx.create_chest(b, random, 3, 3, 5, "minecraft:chests/stronghold_library");
                if *tall {
                    ctx.place_block(b, cave_air, 12, 9, 1);
                    ctx.create_chest(b, random, 12, 8, 1, "minecraft:chests/stronghold_library");
                }
            }
            ShKind::PortalRoom { .. } => {
                ctx.generate_box_selector(b, 0, 0, 0, 10, 7, 15, false, random, &mut smooth);
                small_door(ctx, b, Door::Grates, 4, 1, 0);
                ctx.generate_box_selector(b, 1, 6, 1, 1, 6, 14, false, random, &mut smooth);
                ctx.generate_box_selector(b, 9, 6, 1, 9, 6, 14, false, random, &mut smooth);
                ctx.generate_box_selector(b, 2, 6, 1, 8, 6, 2, false, random, &mut smooth);
                ctx.generate_box_selector(b, 2, 6, 14, 8, 6, 14, false, random, &mut smooth);
                ctx.generate_box_selector(b, 1, 1, 1, 2, 1, 4, false, random, &mut smooth);
                ctx.generate_box_selector(b, 8, 1, 1, 9, 1, 4, false, random, &mut smooth);
                ctx.generate_box(b, 1, 1, 1, 1, 1, 3, ctx.bs("lava"), ctx.bs("lava"), false);
                ctx.generate_box(b, 9, 1, 1, 9, 1, 3, ctx.bs("lava"), ctx.bs("lava"), false);
                ctx.generate_box_selector(b, 3, 1, 8, 7, 1, 12, false, random, &mut smooth);
                ctx.generate_box(b, 4, 1, 9, 6, 1, 11, ctx.bs("lava"), ctx.bs("lava"), false);
                let ns_bars = ctx.bs("iron_bars").with("north", "true").with("south", "true");
                let we_bars = ctx.bs("iron_bars").with("west", "true").with("east", "true");

                for z in (3..14).step_by(2) {
                    ctx.generate_box(b, 0, 3, z, 0, 4, z, ns_bars, ns_bars, false);
                    ctx.generate_box(b, 10, 3, z, 10, 4, z, ns_bars, ns_bars, false);
                }

                for x in (2..9).step_by(2) {
                    ctx.generate_box(b, x, 3, 15, x, 4, 15, we_bars, we_bars, false);
                }

                let block_state = ctx.bs("stone_brick_stairs").with("facing", "north");
                ctx.generate_box_selector(b, 4, 1, 5, 6, 1, 7, false, random, &mut smooth);
                ctx.generate_box_selector(b, 4, 2, 6, 6, 2, 7, false, random, &mut smooth);
                ctx.generate_box_selector(b, 4, 3, 7, 6, 3, 7, false, random, &mut smooth);

                for x in 4..=6 {
                    ctx.place_block(b, block_state, x, 1, 4);
                    ctx.place_block(b, block_state, x, 2, 5);
                    ctx.place_block(b, block_state, x, 3, 6);
                }

                let north_frame = ctx.bs("end_portal_frame").with("facing", "north");
                let south_frame = ctx.bs("end_portal_frame").with("facing", "south");
                let east_frame = ctx.bs("end_portal_frame").with("facing", "east");
                let west_frame = ctx.bs("end_portal_frame").with("facing", "west");
                let mut all_eyes = true;
                let mut eyes = [false; 12];
                for eye in eyes.iter_mut() {
                    *eye = random.next_float() > 0.9;
                    all_eyes &= *eye;
                }

                ctx.place_block(
                    b,
                    north_frame.with("eye", if eyes[0] { "true" } else { "false" }),
                    4,
                    3,
                    8,
                );
                ctx.place_block(
                    b,
                    north_frame.with("eye", if eyes[1] { "true" } else { "false" }),
                    5,
                    3,
                    8,
                );
                ctx.place_block(
                    b,
                    north_frame.with("eye", if eyes[2] { "true" } else { "false" }),
                    6,
                    3,
                    8,
                );
                ctx.place_block(
                    b,
                    south_frame.with("eye", if eyes[3] { "true" } else { "false" }),
                    4,
                    3,
                    12,
                );
                ctx.place_block(
                    b,
                    south_frame.with("eye", if eyes[4] { "true" } else { "false" }),
                    5,
                    3,
                    12,
                );
                ctx.place_block(
                    b,
                    south_frame.with("eye", if eyes[5] { "true" } else { "false" }),
                    6,
                    3,
                    12,
                );
                ctx.place_block(
                    b,
                    east_frame.with("eye", if eyes[6] { "true" } else { "false" }),
                    3,
                    3,
                    9,
                );
                ctx.place_block(
                    b,
                    east_frame.with("eye", if eyes[7] { "true" } else { "false" }),
                    3,
                    3,
                    10,
                );
                ctx.place_block(
                    b,
                    east_frame.with("eye", if eyes[8] { "true" } else { "false" }),
                    3,
                    3,
                    11,
                );
                ctx.place_block(
                    b,
                    west_frame.with("eye", if eyes[9] { "true" } else { "false" }),
                    7,
                    3,
                    9,
                );
                ctx.place_block(
                    b,
                    west_frame.with("eye", if eyes[10] { "true" } else { "false" }),
                    7,
                    3,
                    10,
                );
                ctx.place_block(
                    b,
                    west_frame.with("eye", if eyes[11] { "true" } else { "false" }),
                    7,
                    3,
                    11,
                );
                if all_eyes {
                    let portal = ctx.bs("end_portal");
                    ctx.place_block(b, portal, 4, 3, 9);
                    ctx.place_block(b, portal, 5, 3, 9);
                    ctx.place_block(b, portal, 6, 3, 9);
                    ctx.place_block(b, portal, 4, 3, 10);
                    ctx.place_block(b, portal, 5, 3, 10);
                    ctx.place_block(b, portal, 6, 3, 10);
                    ctx.place_block(b, portal, 4, 3, 11);
                    ctx.place_block(b, portal, 5, 3, 11);
                    ctx.place_block(b, portal, 6, 3, 11);
                }

                if !self.flag.load(Relaxed) {
                    let pos = b.world_pos(5, 3, 6);
                    if ctx.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
                        self.flag.store(true, Relaxed);
                        let spawner = ctx.bs("spawner").s();
                        ctx.region.set(pos, spawner, 2);
                        if let Some(be) = ctx.region.block_entity(pos)
                            && be.get_str("id") == Some("minecraft:mob_spawner")
                        {
                            crate::feature::behaviour::set_spawner_entity(be, "minecraft:silverfish");
                        }
                    }
                }
            }
            ShKind::PrisonHall => {
                ctx.generate_box_selector(b, 0, 0, 0, 8, 4, 10, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 1, 1, 0);
                ctx.generate_box(b, 1, 1, 10, 3, 3, 10, cave_air, cave_air, false);
                ctx.generate_box_selector(b, 4, 1, 1, 4, 3, 1, false, random, &mut smooth);
                ctx.generate_box_selector(b, 4, 1, 3, 4, 3, 3, false, random, &mut smooth);
                ctx.generate_box_selector(b, 4, 1, 7, 4, 3, 7, false, random, &mut smooth);
                ctx.generate_box_selector(b, 4, 1, 9, 4, 3, 9, false, random, &mut smooth);

                for y in 1..=3 {
                    ctx.place_block(
                        b,
                        ctx.bs("iron_bars").with("north", "true").with("south", "true"),
                        4,
                        y,
                        4,
                    );
                    ctx.place_block(
                        b,
                        ctx.bs("iron_bars")
                            .with("north", "true")
                            .with("south", "true")
                            .with("east", "true"),
                        4,
                        y,
                        5,
                    );
                    ctx.place_block(
                        b,
                        ctx.bs("iron_bars").with("north", "true").with("south", "true"),
                        4,
                        y,
                        6,
                    );
                    ctx.place_block(
                        b,
                        ctx.bs("iron_bars").with("west", "true").with("east", "true"),
                        5,
                        y,
                        5,
                    );
                    ctx.place_block(
                        b,
                        ctx.bs("iron_bars").with("west", "true").with("east", "true"),
                        6,
                        y,
                        5,
                    );
                    ctx.place_block(
                        b,
                        ctx.bs("iron_bars").with("west", "true").with("east", "true"),
                        7,
                        y,
                        5,
                    );
                }

                ctx.place_block(
                    b,
                    ctx.bs("iron_bars").with("north", "true").with("south", "true"),
                    4,
                    3,
                    2,
                );
                ctx.place_block(
                    b,
                    ctx.bs("iron_bars").with("north", "true").with("south", "true"),
                    4,
                    3,
                    8,
                );
                let door_bottom = ctx.bs("iron_door").with("facing", "west");
                let door_top = ctx.bs("iron_door").with("facing", "west").with("half", "upper");
                ctx.place_block(b, door_bottom, 4, 1, 2);
                ctx.place_block(b, door_top, 4, 2, 2);
                ctx.place_block(b, door_bottom, 4, 1, 8);
                ctx.place_block(b, door_top, 4, 2, 8);
            }
            ShKind::RightTurn => {
                ctx.generate_box_selector(b, 0, 0, 0, 4, 4, 4, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 1, 1, 0);
                let orientation = b.orientation;
                if orientation != Some(Direction::North) && orientation != Some(Direction::East) {
                    ctx.generate_box(b, 0, 1, 1, 0, 3, 3, cave_air, cave_air, false);
                } else {
                    ctx.generate_box(b, 4, 1, 1, 4, 3, 3, cave_air, cave_air, false);
                }
            }
            ShKind::RoomCrossing { kind } => {
                ctx.generate_box_selector(b, 0, 0, 0, 10, 6, 10, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 4, 1, 0);
                ctx.generate_box(b, 4, 1, 10, 6, 3, 10, cave_air, cave_air, false);
                ctx.generate_box(b, 0, 1, 4, 0, 3, 6, cave_air, cave_air, false);
                ctx.generate_box(b, 10, 1, 4, 10, 3, 6, cave_air, cave_air, false);
                match *kind {
                    0 => {
                        ctx.place_block(b, ctx.bs("stone_bricks"), 5, 1, 5);
                        ctx.place_block(b, ctx.bs("stone_bricks"), 5, 2, 5);
                        ctx.place_block(b, ctx.bs("stone_bricks"), 5, 3, 5);
                        ctx.place_block(b, ctx.bs("wall_torch").with("facing", "west"), 4, 3, 5);
                        ctx.place_block(b, ctx.bs("wall_torch").with("facing", "east"), 6, 3, 5);
                        ctx.place_block(b, ctx.bs("wall_torch").with("facing", "south"), 5, 3, 4);
                        ctx.place_block(b, ctx.bs("wall_torch").with("facing", "north"), 5, 3, 6);
                        ctx.place_block(b, ctx.bs("smooth_stone_slab"), 4, 1, 4);
                        ctx.place_block(b, ctx.bs("smooth_stone_slab"), 4, 1, 5);
                        ctx.place_block(b, ctx.bs("smooth_stone_slab"), 4, 1, 6);
                        ctx.place_block(b, ctx.bs("smooth_stone_slab"), 6, 1, 4);
                        ctx.place_block(b, ctx.bs("smooth_stone_slab"), 6, 1, 5);
                        ctx.place_block(b, ctx.bs("smooth_stone_slab"), 6, 1, 6);
                        ctx.place_block(b, ctx.bs("smooth_stone_slab"), 5, 1, 4);
                        ctx.place_block(b, ctx.bs("smooth_stone_slab"), 5, 1, 6);
                    }
                    1 => {
                        for i in 0..5 {
                            ctx.place_block(b, ctx.bs("stone_bricks"), 3, 1, 3 + i);
                            ctx.place_block(b, ctx.bs("stone_bricks"), 7, 1, 3 + i);
                            ctx.place_block(b, ctx.bs("stone_bricks"), 3 + i, 1, 3);
                            ctx.place_block(b, ctx.bs("stone_bricks"), 3 + i, 1, 7);
                        }

                        ctx.place_block(b, ctx.bs("stone_bricks"), 5, 1, 5);
                        ctx.place_block(b, ctx.bs("stone_bricks"), 5, 2, 5);
                        ctx.place_block(b, ctx.bs("stone_bricks"), 5, 3, 5);
                        ctx.place_block(b, ctx.bs("water"), 5, 4, 5);
                    }
                    2 => {
                        for z in 1..=9 {
                            ctx.place_block(b, ctx.bs("cobblestone"), 1, 3, z);
                            ctx.place_block(b, ctx.bs("cobblestone"), 9, 3, z);
                        }

                        for x in 1..=9 {
                            ctx.place_block(b, ctx.bs("cobblestone"), x, 3, 1);
                            ctx.place_block(b, ctx.bs("cobblestone"), x, 3, 9);
                        }

                        ctx.place_block(b, ctx.bs("cobblestone"), 5, 1, 4);
                        ctx.place_block(b, ctx.bs("cobblestone"), 5, 1, 6);
                        ctx.place_block(b, ctx.bs("cobblestone"), 5, 3, 4);
                        ctx.place_block(b, ctx.bs("cobblestone"), 5, 3, 6);
                        ctx.place_block(b, ctx.bs("cobblestone"), 4, 1, 5);
                        ctx.place_block(b, ctx.bs("cobblestone"), 6, 1, 5);
                        ctx.place_block(b, ctx.bs("cobblestone"), 4, 3, 5);
                        ctx.place_block(b, ctx.bs("cobblestone"), 6, 3, 5);

                        for y in 1..=3 {
                            ctx.place_block(b, ctx.bs("cobblestone"), 4, y, 4);
                            ctx.place_block(b, ctx.bs("cobblestone"), 6, y, 4);
                            ctx.place_block(b, ctx.bs("cobblestone"), 4, y, 6);
                            ctx.place_block(b, ctx.bs("cobblestone"), 6, y, 6);
                        }

                        ctx.place_block(b, ctx.bs("wall_torch"), 5, 3, 5);

                        for z in 2..=8 {
                            ctx.place_block(b, ctx.bs("oak_planks"), 2, 3, z);
                            ctx.place_block(b, ctx.bs("oak_planks"), 3, 3, z);
                            if !(4..=6).contains(&z) {
                                ctx.place_block(b, ctx.bs("oak_planks"), 4, 3, z);
                                ctx.place_block(b, ctx.bs("oak_planks"), 5, 3, z);
                                ctx.place_block(b, ctx.bs("oak_planks"), 6, 3, z);
                            }

                            ctx.place_block(b, ctx.bs("oak_planks"), 7, 3, z);
                            ctx.place_block(b, ctx.bs("oak_planks"), 8, 3, z);
                        }

                        let ladder = ctx.bs("ladder").with("facing", "west");
                        ctx.place_block(b, ladder, 9, 1, 3);
                        ctx.place_block(b, ladder, 9, 2, 3);
                        ctx.place_block(b, ladder, 9, 3, 3);
                        ctx.create_chest(b, random, 3, 4, 8, "minecraft:chests/stronghold_crossing");
                    }
                    _ => {}
                }
            }
            ShKind::Start | ShKind::StairsDown { .. } => {
                ctx.generate_box_selector(b, 0, 0, 0, 4, 10, 4, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 1, 7, 0);
                small_door(ctx, b, Door::Opening, 1, 1, 4);
                ctx.place_block(b, ctx.bs("stone_bricks"), 2, 6, 1);
                ctx.place_block(b, ctx.bs("stone_bricks"), 1, 5, 1);
                ctx.place_block(b, ctx.bs("smooth_stone_slab"), 1, 6, 1);
                ctx.place_block(b, ctx.bs("stone_bricks"), 1, 5, 2);
                ctx.place_block(b, ctx.bs("stone_bricks"), 1, 4, 3);
                ctx.place_block(b, ctx.bs("smooth_stone_slab"), 1, 5, 3);
                ctx.place_block(b, ctx.bs("stone_bricks"), 2, 4, 3);
                ctx.place_block(b, ctx.bs("stone_bricks"), 3, 3, 3);
                ctx.place_block(b, ctx.bs("smooth_stone_slab"), 3, 4, 3);
                ctx.place_block(b, ctx.bs("stone_bricks"), 3, 3, 2);
                ctx.place_block(b, ctx.bs("stone_bricks"), 3, 2, 1);
                ctx.place_block(b, ctx.bs("smooth_stone_slab"), 3, 3, 1);
                ctx.place_block(b, ctx.bs("stone_bricks"), 2, 2, 1);
                ctx.place_block(b, ctx.bs("stone_bricks"), 1, 1, 1);
                ctx.place_block(b, ctx.bs("smooth_stone_slab"), 1, 2, 1);
                ctx.place_block(b, ctx.bs("stone_bricks"), 1, 1, 2);
                ctx.place_block(b, ctx.bs("smooth_stone_slab"), 1, 1, 3);
            }
            ShKind::Straight { left, right } => {
                ctx.generate_box_selector(b, 0, 0, 0, 4, 4, 6, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 1, 1, 0);
                small_door(ctx, b, Door::Opening, 1, 1, 6);
                let east_torch = ctx.bs("wall_torch").with("facing", "east");
                let west_torch = ctx.bs("wall_torch").with("facing", "west");
                ctx.maybe_generate_block(b, random, 0.1, 1, 2, 1, east_torch);
                ctx.maybe_generate_block(b, random, 0.1, 3, 2, 1, west_torch);
                ctx.maybe_generate_block(b, random, 0.1, 1, 2, 5, east_torch);
                ctx.maybe_generate_block(b, random, 0.1, 3, 2, 5, west_torch);
                if *left {
                    ctx.generate_box(b, 0, 1, 2, 0, 3, 4, cave_air, cave_air, false);
                }

                if *right {
                    ctx.generate_box(b, 4, 1, 2, 4, 3, 4, cave_air, cave_air, false);
                }
            }
            ShKind::StraightStairsDown => {
                ctx.generate_box_selector(b, 0, 0, 0, 4, 10, 7, true, random, &mut smooth);
                small_door(ctx, b, self.entry_door, 1, 7, 0);
                small_door(ctx, b, Door::Opening, 1, 1, 7);
                let stairs = ctx.bs("cobblestone_stairs").with("facing", "south");

                for i in 0..6 {
                    ctx.place_block(b, stairs, 1, 6 - i, 1 + i);
                    ctx.place_block(b, stairs, 2, 6 - i, 1 + i);
                    ctx.place_block(b, stairs, 3, 6 - i, 1 + i);
                    if i < 5 {
                        ctx.place_block(b, ctx.bs("stone_bricks"), 1, 5 - i, 1 + i);
                        ctx.place_block(b, ctx.bs("stone_bricks"), 2, 5 - i, 1 + i);
                        ctx.place_block(b, ctx.bs("stone_bricks"), 3, 5 - i, 1 + i);
                    }
                }
            }
        }
    }
}
