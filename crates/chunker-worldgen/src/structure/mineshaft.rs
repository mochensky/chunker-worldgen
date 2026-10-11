//! `MineshaftStructure` + `MineshaftPieces` (piece layout and NBT).

use chunker_nbt::{Compound, List};

use super::geom::{BlockPos, BoundingBox, Direction};
use super::start::{PieceBase, PiecesBuilder};
use super::{GenContext, Stub, StubPieces};
use crate::chunk::HeightmapKind;
use crate::piece_boilerplate;
use crate::random::RandomSource;

#[derive(Clone, Debug)]
pub enum MsKind {
    Room {
        entrances: Vec<BoundingBox>,
    },
    Corridor {
        has_rails: bool,
        spider_corridor: bool,
        has_placed_spider: bool,
        num_sections: i32,
    },
    Crossing {
        direction: Direction,
        two_floored: bool,
    },
    Stairs,
}

/// `MineshaftPieces.MineShaftPiece` and subclasses.
pub struct MsPiece {
    pub base: PieceBase,
    /// `MineshaftStructure.Type` ordinal: 0 normal, 1 mesa.
    pub mesa: bool,
    pub kind: MsKind,
    /// `MineShaftCorridor.hasPlacedSpider` as changed by `postProcess`.
    pub spider_placed: std::sync::atomic::AtomicBool,
}

impl super::start::Piece for MsPiece {
    piece_boilerplate!();
    fn type_id(&self) -> &'static str {
        match self.kind {
            MsKind::Room { .. } => "minecraft:msroom",
            MsKind::Corridor { .. } => "minecraft:mscorridor",
            MsKind::Crossing { .. } => "minecraft:mscrossing",
            MsKind::Stairs => "minecraft:msstairs",
        }
    }
    fn save_extra(&self, tag: &mut Compound) {
        tag.insert("MST", self.mesa as i32);
        match &self.kind {
            MsKind::Room { entrances } => {
                let mut l = List::new();
                for b in entrances {
                    l.push(b.to_array());
                }
                tag.insert("Entrances", l);
            }
            MsKind::Corridor {
                has_rails,
                spider_corridor,
                has_placed_spider,
                num_sections,
            } => {
                tag.insert("hr", *has_rails);
                tag.insert("sc", *spider_corridor);
                tag.insert("hps", *has_placed_spider);
                tag.insert("Num", *num_sections);
            }
            MsKind::Crossing {
                direction,
                two_floored,
            } => {
                tag.insert("tf", *two_floored);
                tag.insert("D", direction.data_2d() as i8);
            }
            MsKind::Stairs => {}
        }
    }
    fn post_process_ported(&self) -> bool {
        true
    }
    fn post_process(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        self.post_process_ms(ctx, random);
    }
    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.move_by(dx, dy, dz);
        if let MsKind::Room { entrances } = &mut self.kind {
            for b in entrances {
                b.move_by(dx, dy, dz);
            }
        }
    }
}

struct Gen<'r> {
    pieces: Vec<MsPiece>,
    random: &'r mut dyn RandomSource,
    mesa: bool,
}

impl Gen<'_> {
    fn collides(&self, b: &BoundingBox) -> bool {
        self.pieces.iter().any(|p| p.base.bbox.intersects(b))
    }

    fn find_corridor(&mut self, x: i32, y: i32, z: i32, d: Direction) -> Option<BoundingBox> {
        let mut length = self.random.next_int_bounded(3) + 2;
        while length > 0 {
            let l = length * 5;
            let mut b = match d {
                Direction::South => BoundingBox::new(0, 0, 0, 2, 2, l - 1),
                Direction::West => BoundingBox::new(-(l - 1), 0, 0, 0, 2, 2),
                Direction::East => BoundingBox::new(0, 0, 0, l - 1, 2, 2),
                _ => BoundingBox::new(0, 0, -(l - 1), 2, 2, 0),
            };
            b.move_by(x, y, z);
            if !self.collides(&b) {
                return Some(b);
            }
            length -= 1;
        }
        None
    }

    fn find_crossing(&mut self, x: i32, y: i32, z: i32, d: Direction) -> Option<BoundingBox> {
        let y1 = if self.random.next_int_bounded(4) == 0 {
            6
        } else {
            2
        };
        let mut b = match d {
            Direction::South => BoundingBox::new(-1, 0, 0, 3, y1, 4),
            Direction::West => BoundingBox::new(-4, 0, -1, 0, y1, 3),
            Direction::East => BoundingBox::new(0, 0, -1, 4, y1, 3),
            _ => BoundingBox::new(-1, 0, -4, 3, y1, 0),
        };
        b.move_by(x, y, z);
        (!self.collides(&b)).then_some(b)
    }

    fn find_stairs(&mut self, x: i32, y: i32, z: i32, d: Direction) -> Option<BoundingBox> {
        let mut b = match d {
            Direction::South => BoundingBox::new(0, -5, 0, 2, 2, 8),
            Direction::West => BoundingBox::new(-8, -5, 0, 0, 2, 2),
            Direction::East => BoundingBox::new(0, -5, 0, 8, 2, 2),
            _ => BoundingBox::new(0, -5, -8, 2, 2, 0),
        };
        b.move_by(x, y, z);
        (!self.collides(&b)).then_some(b)
    }

    /// `createRandomShaftPiece`.
    fn create_random(&mut self, x: i32, y: i32, z: i32, d: Direction, depth: i32) -> Option<MsPiece> {
        let selection = self.random.next_int_bounded(100);
        let mesa = self.mesa;
        if selection >= 80 {
            let b = self.find_crossing(x, y, z, d)?;
            Some(MsPiece {
                base: PieceBase::new(depth, b, None),
                mesa,
                kind: MsKind::Crossing {
                    direction: d,
                    two_floored: b.y_span() > 3,
                },
                spider_placed: Default::default(),
            })
        } else if selection >= 70 {
            let b = self.find_stairs(x, y, z, d)?;
            Some(MsPiece {
                base: PieceBase::new(depth, b, Some(d)),
                mesa,
                kind: MsKind::Stairs,
                spider_placed: Default::default(),
            })
        } else {
            let b = self.find_corridor(x, y, z, d)?;
            let has_rails = self.random.next_int_bounded(3) == 0;
            let spider_corridor = !has_rails && self.random.next_int_bounded(23) == 0;
            let num_sections = if d.axis() == super::geom::Axis::Z {
                b.z_span() / 5
            } else {
                b.x_span() / 5
            };
            Some(MsPiece {
                base: PieceBase::new(depth, b, Some(d)),
                mesa,
                kind: MsKind::Corridor {
                    has_rails,
                    spider_corridor,
                    has_placed_spider: false,
                    num_sections,
                },
                spider_placed: Default::default(),
            })
        }
    }

    /// `generateAndAddPiece`: returns the index of the new piece.
    fn generate_and_add(&mut self, x: i32, y: i32, z: i32, d: Direction, depth: i32) -> Option<usize> {
        if depth > 8 {
            return None;
        }
        let start = self.pieces[0].base.bbox;
        if (x - start.min_x).abs() > 80 || (z - start.min_z).abs() > 80 {
            return None;
        }
        let piece = self.create_random(x, y, z, d, depth + 1)?;
        self.pieces.push(piece);
        let index = self.pieces.len() - 1;
        self.add_children(index);
        Some(index)
    }

    fn add_children(&mut self, index: usize) {
        let p = &self.pieces[index];
        let (b, depth, orientation) = (p.base.bbox, p.base.gen_depth, p.base.orientation);
        match p.kind.clone() {
            MsKind::Room { .. } => self.room_children(index, b, depth),
            MsKind::Corridor { .. } => self.corridor_children(b, depth, orientation),
            MsKind::Crossing {
                direction,
                two_floored,
            } => self.crossing_children(b, depth, direction, two_floored),
            MsKind::Stairs => {
                match orientation {
                    Some(Direction::South) => {
                        self.generate_and_add(b.min_x, b.min_y, b.max_z + 1, Direction::South, depth)
                    }
                    Some(Direction::West) => {
                        self.generate_and_add(b.min_x - 1, b.min_y, b.min_z, Direction::West, depth)
                    }
                    Some(Direction::East) => {
                        self.generate_and_add(b.max_x + 1, b.min_y, b.min_z, Direction::East, depth)
                    }
                    Some(_) => self.generate_and_add(b.min_x, b.min_y, b.min_z - 1, Direction::North, depth),
                    None => None,
                };
            }
        }
    }

    fn room_children(&mut self, index: usize, b: BoundingBox, depth: i32) {
        let mut height_space = b.y_span() - 3 - 1;
        if height_space <= 0 {
            height_space = 1;
        }
        // North side.
        let mut pos = 0;
        while pos < b.x_span() {
            pos += self.random.next_int_bounded(b.x_span());
            if pos + 3 > b.x_span() {
                break;
            }
            let y = b.min_y + self.random.next_int_bounded(height_space) + 1;
            if let Some(c) = self.generate_and_add(b.min_x + pos, y, b.min_z - 1, Direction::North, depth) {
                let cb = self.pieces[c].base.bbox;
                self.entrance(
                    index,
                    BoundingBox::new(cb.min_x, cb.min_y, b.min_z, cb.max_x, cb.max_y, b.min_z + 1),
                );
            }
            pos += 4;
        }
        // South side.
        let mut pos = 0;
        while pos < b.x_span() {
            pos += self.random.next_int_bounded(b.x_span());
            if pos + 3 > b.x_span() {
                break;
            }
            let y = b.min_y + self.random.next_int_bounded(height_space) + 1;
            if let Some(c) = self.generate_and_add(b.min_x + pos, y, b.max_z + 1, Direction::South, depth) {
                let cb = self.pieces[c].base.bbox;
                self.entrance(
                    index,
                    BoundingBox::new(cb.min_x, cb.min_y, b.max_z - 1, cb.max_x, cb.max_y, b.max_z),
                );
            }
            pos += 4;
        }
        // West side.
        let mut pos = 0;
        while pos < b.z_span() {
            pos += self.random.next_int_bounded(b.z_span());
            if pos + 3 > b.z_span() {
                break;
            }
            let y = b.min_y + self.random.next_int_bounded(height_space) + 1;
            if let Some(c) = self.generate_and_add(b.min_x - 1, y, b.min_z + pos, Direction::West, depth) {
                let cb = self.pieces[c].base.bbox;
                self.entrance(
                    index,
                    BoundingBox::new(b.min_x, cb.min_y, cb.min_z, b.min_x + 1, cb.max_y, cb.max_z),
                );
            }
            pos += 4;
        }
        // East side.
        let mut pos = 0;
        while pos < b.z_span() {
            pos += self.random.next_int_bounded(b.z_span());
            if pos + 3 > b.z_span() {
                break;
            }
            let y = b.min_y + self.random.next_int_bounded(height_space) + 1;
            if let Some(c) = self.generate_and_add(b.max_x + 1, y, b.min_z + pos, Direction::East, depth) {
                let cb = self.pieces[c].base.bbox;
                self.entrance(
                    index,
                    BoundingBox::new(b.max_x - 1, cb.min_y, cb.min_z, b.max_x, cb.max_y, cb.max_z),
                );
            }
            pos += 4;
        }
    }

    fn entrance(&mut self, index: usize, b: BoundingBox) {
        if let MsKind::Room { entrances } = &mut self.pieces[index].kind {
            entrances.push(b);
        }
    }

    fn corridor_children(&mut self, b: BoundingBox, depth: i32, orientation: Option<Direction>) {
        let end = self.random.next_int_bounded(4);
        if let Some(o) = orientation {
            let y = || b.min_y - 1;
            match o {
                Direction::South => {
                    let dy = self.random.next_int_bounded(3);
                    if end <= 1 {
                        self.generate_and_add(b.min_x, y() + dy, b.max_z + 1, o, depth);
                    } else if end == 2 {
                        self.generate_and_add(b.min_x - 1, y() + dy, b.max_z - 3, Direction::West, depth);
                    } else {
                        self.generate_and_add(b.max_x + 1, y() + dy, b.max_z - 3, Direction::East, depth);
                    }
                }
                Direction::West => {
                    let dy = self.random.next_int_bounded(3);
                    if end <= 1 {
                        self.generate_and_add(b.min_x - 1, y() + dy, b.min_z, o, depth);
                    } else if end == 2 {
                        self.generate_and_add(b.min_x, y() + dy, b.min_z - 1, Direction::North, depth);
                    } else {
                        self.generate_and_add(b.min_x, y() + dy, b.max_z + 1, Direction::South, depth);
                    }
                }
                Direction::East => {
                    let dy = self.random.next_int_bounded(3);
                    if end <= 1 {
                        self.generate_and_add(b.max_x + 1, y() + dy, b.min_z, o, depth);
                    } else if end == 2 {
                        self.generate_and_add(b.max_x - 3, y() + dy, b.min_z - 1, Direction::North, depth);
                    } else {
                        self.generate_and_add(b.max_x - 3, y() + dy, b.max_z + 1, Direction::South, depth);
                    }
                }
                _ => {
                    let dy = self.random.next_int_bounded(3);
                    if end <= 1 {
                        self.generate_and_add(b.min_x, y() + dy, b.min_z - 1, o, depth);
                    } else if end == 2 {
                        self.generate_and_add(b.min_x - 1, y() + dy, b.min_z, Direction::West, depth);
                    } else {
                        self.generate_and_add(b.max_x + 1, y() + dy, b.min_z, Direction::East, depth);
                    }
                }
            }
        }
        if depth < 8 {
            if orientation != Some(Direction::North) && orientation != Some(Direction::South) {
                let mut x = b.min_x + 3;
                while x + 3 <= b.max_x {
                    let s = self.random.next_int_bounded(5);
                    if s == 0 {
                        self.generate_and_add(x, b.min_y, b.min_z - 1, Direction::North, depth + 1);
                    } else if s == 1 {
                        self.generate_and_add(x, b.min_y, b.max_z + 1, Direction::South, depth + 1);
                    }
                    x += 5;
                }
            } else {
                let mut z = b.min_z + 3;
                while z + 3 <= b.max_z {
                    let s = self.random.next_int_bounded(5);
                    if s == 0 {
                        self.generate_and_add(b.min_x - 1, b.min_y, z, Direction::West, depth + 1);
                    } else if s == 1 {
                        self.generate_and_add(b.max_x + 1, b.min_y, z, Direction::East, depth + 1);
                    }
                    z += 5;
                }
            }
        }
    }

    fn crossing_children(&mut self, b: BoundingBox, depth: i32, direction: Direction, two_floored: bool) {
        match direction {
            Direction::South => {
                self.generate_and_add(b.min_x + 1, b.min_y, b.max_z + 1, Direction::South, depth);
                self.generate_and_add(b.min_x - 1, b.min_y, b.min_z + 1, Direction::West, depth);
                self.generate_and_add(b.max_x + 1, b.min_y, b.min_z + 1, Direction::East, depth);
            }
            Direction::West => {
                self.generate_and_add(b.min_x + 1, b.min_y, b.min_z - 1, Direction::North, depth);
                self.generate_and_add(b.min_x + 1, b.min_y, b.max_z + 1, Direction::South, depth);
                self.generate_and_add(b.min_x - 1, b.min_y, b.min_z + 1, Direction::West, depth);
            }
            Direction::East => {
                self.generate_and_add(b.min_x + 1, b.min_y, b.min_z - 1, Direction::North, depth);
                self.generate_and_add(b.min_x + 1, b.min_y, b.max_z + 1, Direction::South, depth);
                self.generate_and_add(b.max_x + 1, b.min_y, b.min_z + 1, Direction::East, depth);
            }
            _ => {
                self.generate_and_add(b.min_x + 1, b.min_y, b.min_z - 1, Direction::North, depth);
                self.generate_and_add(b.min_x - 1, b.min_y, b.min_z + 1, Direction::West, depth);
                self.generate_and_add(b.max_x + 1, b.min_y, b.min_z + 1, Direction::East, depth);
            }
        }
        if two_floored {
            if self.random.next_bool() {
                self.generate_and_add(b.min_x + 1, b.min_y + 3 + 1, b.min_z - 1, Direction::North, depth);
            }
            if self.random.next_bool() {
                self.generate_and_add(b.min_x - 1, b.min_y + 3 + 1, b.min_z + 1, Direction::West, depth);
            }
            if self.random.next_bool() {
                self.generate_and_add(b.max_x + 1, b.min_y + 3 + 1, b.min_z + 1, Direction::East, depth);
            }
            if self.random.next_bool() {
                self.generate_and_add(b.min_x + 1, b.min_y + 3 + 1, b.max_z + 1, Direction::South, depth);
            }
        }
    }
}

/// `MineshaftStructure.findGenerationPoint`.
pub fn find_generation_point<'c>(ctx: &mut GenContext, mesa: bool) -> Option<Stub<'c>> {
    ctx.random.next_double();
    let start = BlockPos::new(ctx.middle_block_x(), 50, ctx.min_block_z());
    let (room_x, room_z) = (ctx.min_block_x() + 2, ctx.min_block_z() + 2);
    let mut builder = PiecesBuilder::default();
    let dy = {
        let random = &mut ctx.random;
        let bbox = BoundingBox::new(
            room_x,
            50,
            room_z,
            room_x + 7 + random.next_int_bounded(6),
            54 + random.next_int_bounded(6),
            room_z + 7 + random.next_int_bounded(6),
        );
        let room = MsPiece {
            base: PieceBase::new(0, bbox, None),
            mesa,
            kind: MsKind::Room {
                entrances: Vec::new(),
            },
            spider_placed: Default::default(),
        };
        let mut g = Gen {
            pieces: vec![room],
            random,
            mesa,
        };
        g.add_children(0);
        for p in g.pieces {
            builder.add(Box::new(p));
        }
        let sea_level = ctx.sea_level();
        if mesa {
            let center = builder.bounding_box().center();
            let surface = ctx.base_height(center.x, center.z, HeightmapKind::WorldSurfaceWg);
            let target = if surface <= sea_level {
                sea_level
            } else {
                crate::providers::random_between_inclusive(&mut ctx.random, sea_level, surface)
            };
            let dy = target - center.y;
            builder.offset_vertically(dy);
            dy
        } else {
            let min_y = ctx.terrain.settings.noise.min_y;
            builder.move_below_sea_level(sea_level, min_y, &mut ctx.random, 10)
        }
    };
    Some(Stub {
        pos: start.offset(0, dy, 0),
        pieces: StubPieces::Built(builder),
    })
}

// --- postProcess ---------------------------------------------------------------------------------

use super::build::PieceCtx;
use chunker_data::blocks::{BlockId, BlockState};

/// `MineshaftStructure.Type` blocks: (wood, planks, fence).
fn type_blocks(ctx: &PieceCtx, mesa: bool) -> (BlockState, BlockState, BlockState) {
    if mesa {
        (
            ctx.bs("dark_oak_log").s(),
            ctx.bs("dark_oak_planks").s(),
            ctx.bs("dark_oak_fence").s(),
        )
    } else {
        (
            ctx.bs("oak_log").s(),
            ctx.bs("oak_planks").s(),
            ctx.bs("oak_fence").s(),
        )
    }
}

impl MsPiece {
    fn post_process_ms(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        if self.is_in_invalid_location(ctx) {
            return;
        }
        let (wood, planks, fence) = type_blocks(ctx, self.mesa);
        let blocks = &ctx.region.env.blocks;
        // `MineShaftPiece.canBeReplaced`
        let keep: Vec<BlockId> = vec![
            blocks.block_of(planks),
            blocks.block_of(wood),
            blocks.block_of(fence),
            ctx.region.env.b.iron_chain,
        ];
        let saved = std::mem::replace(&mut ctx.keep, keep);
        let b = &self.base;
        let cave_air = ctx.bs("cave_air");
        let bb = b.bbox;
        match &self.kind {
            MsKind::Corridor {
                has_rails,
                spider_corridor,
                num_sections,
                ..
            } => {
                let length = num_sections * 5 - 1;
                ctx.generate_box(b, 0, 0, 0, 2, 1, length, cave_air, cave_air, false);
                ctx.generate_maybe_box(
                    b, random, 0.8, 0, 2, 0, 2, 2, length, cave_air, cave_air, false, false,
                );
                if *spider_corridor {
                    ctx.generate_maybe_box(
                        b,
                        random,
                        0.6,
                        0,
                        0,
                        0,
                        2,
                        1,
                        length,
                        ctx.bs("cobweb"),
                        cave_air,
                        false,
                        true,
                    );
                }
                for section in 0..*num_sections {
                    let z = 2 + section * 5;
                    self.place_support(ctx, random, 0, 0, z, 2, 2);
                    for (p, x, dz) in [
                        (0.1, 0, -1),
                        (0.1, 2, -1),
                        (0.1, 0, 1),
                        (0.1, 2, 1),
                        (0.05, 0, -2),
                        (0.05, 2, -2),
                        (0.05, 0, 2),
                        (0.05, 2, 2),
                    ] {
                        self.maybe_place_cobweb(ctx, random, p, x, 2, z + dz);
                    }
                    if random.next_int_bounded(100) == 0 {
                        self.create_chest(ctx, random, 2, 0, z - 1);
                    }
                    if random.next_int_bounded(100) == 0 {
                        self.create_chest(ctx, random, 0, 0, z + 1);
                    }
                    if *spider_corridor && !self.spider_placed.load(std::sync::atomic::Ordering::Relaxed) {
                        let new_z = z - 1 + random.next_int_bounded(3);
                        let pos = b.world_pos(1, 0, new_z);
                        if ctx.chunk_bb.is_inside(pos.x, pos.y, pos.z) && ctx.is_interior(b, 1, 0, new_z) {
                            self.spider_placed
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            let spawner = ctx.bs("spawner").s();
                            ctx.region.set(pos, spawner, 2);
                            if let Some(be) = ctx.region.block_entity(pos)
                                && be.get_str("id") == Some("minecraft:mob_spawner")
                            {
                                crate::feature::behaviour::set_spawner_entity(be, "minecraft:cave_spider");
                            }
                        }
                    }
                }
                for x in 0..=2 {
                    for z in 0..=length {
                        self.set_planks(ctx, planks, x, -1, z);
                    }
                }
                self.double_lower_or_upper_support(ctx, wood, planks, fence, 0, -1, 2);
                if *num_sections > 1 {
                    self.double_lower_or_upper_support(ctx, wood, planks, fence, 0, -1, length - 2);
                }
                if *has_rails {
                    let rail = ctx.bs("rail").with("shape", "north_south");
                    for z in 0..=length {
                        let floor = ctx.get_block(b, 1, -1, z);
                        let f = ctx.region.env.blocks.flags(floor);
                        if !f.is_air() && f.is_solid_render() {
                            let p = if ctx.is_interior(b, 1, 0, z) { 0.7 } else { 0.9 };
                            ctx.maybe_generate_block(b, random, p, 1, 0, z, rail);
                        }
                    }
                }
            }
            MsKind::Crossing { two_floored, .. } => {
                let (x0, y0, z0, x1, y1, z1) = (bb.min_x, bb.min_y, bb.min_z, bb.max_x, bb.max_y, bb.max_z);
                if *two_floored {
                    ctx.generate_box(
                        b,
                        x0 + 1,
                        y0,
                        z0,
                        x1 - 1,
                        y0 + 3 - 1,
                        z1,
                        cave_air,
                        cave_air,
                        false,
                    );
                    ctx.generate_box(
                        b,
                        x0,
                        y0,
                        z0 + 1,
                        x1,
                        y0 + 3 - 1,
                        z1 - 1,
                        cave_air,
                        cave_air,
                        false,
                    );
                    ctx.generate_box(b, x0 + 1, y1 - 2, z0, x1 - 1, y1, z1, cave_air, cave_air, false);
                    ctx.generate_box(b, x0, y1 - 2, z0 + 1, x1, y1, z1 - 1, cave_air, cave_air, false);
                    ctx.generate_box(
                        b,
                        x0 + 1,
                        y0 + 3,
                        z0 + 1,
                        x1 - 1,
                        y0 + 3,
                        z1 - 1,
                        cave_air,
                        cave_air,
                        false,
                    );
                } else {
                    ctx.generate_box(b, x0 + 1, y0, z0, x1 - 1, y1, z1, cave_air, cave_air, false);
                    ctx.generate_box(b, x0, y0, z0 + 1, x1, y1, z1 - 1, cave_air, cave_air, false);
                }
                for (x, z) in [
                    (x0 + 1, z0 + 1),
                    (x0 + 1, z1 - 1),
                    (x1 - 1, z0 + 1),
                    (x1 - 1, z1 - 1),
                ] {
                    // `placeSupportPillar`
                    if !ctx.is_air_at(b, x, y1 + 1, z) {
                        ctx.generate_box(b, x, y0, z, x, y1, z, planks, cave_air, false);
                    }
                }
                let y = y0 - 1;
                for x in x0..=x1 {
                    for z in z0..=z1 {
                        self.set_planks(ctx, planks, x, y, z);
                    }
                }
            }
            MsKind::Room { entrances } => {
                ctx.generate_box(
                    b,
                    bb.min_x,
                    bb.min_y + 1,
                    bb.min_z,
                    bb.max_x,
                    (bb.min_y + 3).min(bb.max_y),
                    bb.max_z,
                    cave_air,
                    cave_air,
                    false,
                );
                for e in entrances {
                    ctx.generate_box(
                        b,
                        e.min_x,
                        e.max_y - 2,
                        e.min_z,
                        e.max_x,
                        e.max_y,
                        e.max_z,
                        cave_air,
                        cave_air,
                        false,
                    );
                }
                ctx.generate_upper_half_sphere(
                    b,
                    bb.min_x,
                    bb.min_y + 4,
                    bb.min_z,
                    bb.max_x,
                    bb.max_y,
                    bb.max_z,
                    cave_air,
                    false,
                );
            }
            MsKind::Stairs => {
                ctx.generate_box(b, 0, 5, 0, 2, 7, 1, cave_air, cave_air, false);
                ctx.generate_box(b, 0, 0, 7, 2, 2, 8, cave_air, cave_air, false);
                for i in 0..5 {
                    ctx.generate_box(
                        b,
                        0,
                        5 - i - if i < 4 { 1 } else { 0 },
                        2 + i,
                        2,
                        7 - i,
                        2 + i,
                        cave_air,
                        cave_air,
                        false,
                    );
                }
            }
        }
        ctx.keep = saved;
    }

    /// `MineShaftPiece.isInInvalidLocation`.
    fn is_in_invalid_location(&self, ctx: &PieceCtx) -> bool {
        let (bb, cb) = (self.base.bbox, ctx.chunk_bb);
        let x0 = (bb.min_x - 1).max(cb.min_x);
        let y0 = (bb.min_y - 1).max(cb.min_y);
        let z0 = (bb.min_z - 1).max(cb.min_z);
        let x1 = (bb.max_x + 1).min(cb.max_x);
        let y1 = (bb.max_y + 1).min(cb.max_y);
        let z1 = (bb.max_z + 1).min(cb.max_z);
        let center = BlockPos::new((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2);
        let env = ctx.region.env;
        if env.mineshaft_blocking().contains(ctx.region.biome(center)) {
            return true;
        }
        let liquid =
            |x: i32, y: i32, z: i32| env.blocks.flags(ctx.region.get(BlockPos::new(x, y, z))).liquid();
        for x in x0..=x1 {
            for z in z0..=z1 {
                if liquid(x, y0, z) || liquid(x, y1, z) {
                    return true;
                }
            }
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                if liquid(x, y, z0) || liquid(x, y, z1) {
                    return true;
                }
            }
        }
        for z in z0..=z1 {
            for y in y0..=y1 {
                if liquid(x0, y, z) || liquid(x1, y, z) {
                    return true;
                }
            }
        }
        false
    }

    /// `MineShaftPiece.setPlanksBlock`.
    fn set_planks(&self, ctx: &mut PieceCtx, planks: BlockState, x: i32, y: i32, z: i32) {
        let b = &self.base;
        if ctx.is_interior(b, x, y, z) {
            let pos = b.world_pos(x, y, z);
            if !ctx.region.flags(pos).is_face_sturdy(1) {
                ctx.region.set(pos, planks, 2);
            }
        }
    }

    /// `MineShaftCorridor.createChest`: a chest minecart on a rail.
    fn create_chest(
        &self,
        ctx: &mut PieceCtx,
        random: &mut dyn RandomSource,
        x: i32,
        y: i32,
        z: i32,
    ) -> bool {
        let b = &self.base;
        let pos = b.world_pos(x, y, z);
        if !(ctx.chunk_bb.is_inside(pos.x, pos.y, pos.z)
            && ctx.region.flags(pos).is_air()
            && !ctx.region.flags(pos.below()).is_air())
        {
            return false;
        }
        let shape = if random.next_bool() {
            "north_south"
        } else {
            "east_west"
        };
        let rail = ctx.bs("rail").with("shape", shape);
        ctx.place_block(b, rail, x, y, z);
        let mut tag = chunker_nbt::Compound::new();
        tag.insert("id", "minecraft:chest_minecart");
        let mut p = chunker_nbt::List::new();
        p.push(pos.x as f64 + 0.5);
        p.push(pos.y as f64 + 0.5);
        p.push(pos.z as f64 + 0.5);
        tag.insert("Pos", p);
        // Saved from `Entity.firstTick`, which is true for a new entity.
        tag.insert("HasTicked", true);
        tag.insert("LootTable", "minecraft:chests/abandoned_mineshaft");
        tag.insert("LootTableSeed", random.next_long());
        ctx.region.add_entity(pos.x, pos.z, tag);
        true
    }

    /// `MineShaftCorridor.placeSupport`.
    fn place_support(
        &self,
        ctx: &mut PieceCtx,
        random: &mut dyn RandomSource,
        x0: i32,
        y0: i32,
        z: i32,
        y1: i32,
        x1: i32,
    ) {
        let b = &self.base;
        // `isSupportingBox`
        if (x0..=x1).any(|x| ctx.is_air_at(b, x, y1 + 1, z)) {
            return;
        }
        let (_, planks, fence) = type_blocks(ctx, self.mesa);
        let blocks = &ctx.region.env.blocks;
        let cave_air = ctx.bs("cave_air");
        let fw = blocks.with_property(fence, "west", "true").unwrap();
        let fe = blocks.with_property(fence, "east", "true").unwrap();
        ctx.generate_box(b, x0, y0, z, x0, y1 - 1, z, fw, cave_air, false);
        ctx.generate_box(b, x1, y0, z, x1, y1 - 1, z, fe, cave_air, false);
        if random.next_int_bounded(4) == 0 {
            ctx.generate_box(b, x0, y1, z, x0, y1, z, planks, cave_air, false);
            ctx.generate_box(b, x1, y1, z, x1, y1, z, planks, cave_air, false);
        } else {
            ctx.generate_box(b, x0, y1, z, x1, y1, z, planks, cave_air, false);
            ctx.maybe_generate_block(
                b,
                random,
                0.05,
                x0 + 1,
                y1,
                z - 1,
                ctx.bs("wall_torch").with("facing", "south"),
            );
            ctx.maybe_generate_block(
                b,
                random,
                0.05,
                x0 + 1,
                y1,
                z + 1,
                ctx.bs("wall_torch").with("facing", "north"),
            );
        }
    }

    /// `MineShaftCorridor.maybePlaceCobWeb`.
    fn maybe_place_cobweb(
        &self,
        ctx: &mut PieceCtx,
        random: &mut dyn RandomSource,
        probability: f32,
        x: i32,
        y: i32,
        z: i32,
    ) {
        let b = &self.base;
        if ctx.is_interior(b, x, y, z)
            && random.next_float() < probability
            && self.has_sturdy_neighbours(ctx, x, y, z, 2)
        {
            ctx.place_block(b, ctx.bs("cobweb"), x, y, z);
        }
    }

    /// `MineShaftCorridor.hasSturdyNeighbours`.
    fn has_sturdy_neighbours(&self, ctx: &PieceCtx, x: i32, y: i32, z: i32, count: i32) -> bool {
        let pos = self.base.world_pos(x, y, z);
        let mut n = 0;
        for d in Direction::ALL {
            let p = pos.relative(d, 1);
            if ctx.chunk_bb.is_inside(p.x, p.y, p.z)
                && ctx
                    .region
                    .flags(p)
                    .is_face_sturdy(d.opposite().data_3d() as usize)
            {
                n += 1;
                if n >= count {
                    return true;
                }
            }
        }
        false
    }

    /// `MineShaftCorridor.placeDoubleLowerOrUpperSupport`.
    fn double_lower_or_upper_support(
        &self,
        ctx: &mut PieceCtx,
        wood: BlockState,
        planks: BlockState,
        fence: BlockState,
        x: i32,
        y: i32,
        z: i32,
    ) {
        let b = &self.base;
        let planks_block = ctx.region.env.blocks.block_of(planks);
        for xx in [x, x + 2] {
            if ctx.region.env.blocks.block_of(ctx.get_block(b, xx, y, z)) == planks_block {
                self.fill_pillar_down_or_chain_up(ctx, wood, fence, xx, y, z);
            }
        }
    }

    /// `MineShaftCorridor.fillPillarDownOrChainUp`.
    fn fill_pillar_down_or_chain_up(
        &self,
        ctx: &mut PieceCtx,
        pillar: BlockState,
        fence: BlockState,
        x: i32,
        y: i32,
        z: i32,
    ) {
        let pos = self.base.world_pos(x, y, z);
        if !ctx.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
            return;
        }
        let env = ctx.region.env;
        let world_y = pos.y;
        let (mut check_below, mut check_above) = (true, true);
        let mut dist = 1;
        while check_below || check_above {
            if check_below {
                let p = BlockPos::new(pos.x, world_y - dist, pos.z);
                let below = ctx.region.get(p);
                let empty =
                    ctx.is_replaceable_by_structures(below) && env.blocks.block_of(below) != env.b.lava;
                if !empty && env.blocks.flags(below).is_face_sturdy(1) {
                    for py in world_y - dist + 1..world_y {
                        ctx.region.set(BlockPos::new(pos.x, py, pos.z), pillar, 2);
                    }
                    return;
                }
                check_below = dist <= 20 && empty && p.y > ctx.region.min_y() + 1;
            }
            if check_above {
                let p = BlockPos::new(pos.x, world_y + dist, pos.z);
                let above = ctx.region.get(p);
                let empty = ctx.is_replaceable_by_structures(above);
                let falling = env.blocks.is_instance(env.blocks.block_of(above), "FallingBlock");
                if !empty && env.blocks.flags(above).supports_center_down() && !falling {
                    ctx.region.set(BlockPos::new(pos.x, world_y + 1, pos.z), fence, 2);
                    let chain = ctx.bs("iron_chain").s();
                    for py in world_y + 2..world_y + dist {
                        ctx.region.set(BlockPos::new(pos.x, py, pos.z), chain, 2);
                    }
                    return;
                }
                check_above = dist <= 50 && empty && p.y < ctx.region.max_y();
            }
            dist += 1;
        }
    }
}
