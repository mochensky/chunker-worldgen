//! `OceanMonumentPieces`: the building's room graph (built from the start random like vanilla's
//! `MonumentBuilding` constructor) and `postProcess` of the building and all its rooms. Room bodies
//! are translated statement by statement from vanilla.

use chunker_data::blocks::BlockState;

use super::build::PieceCtx;
use super::geom::{BoundingBox, Direction};
use super::start::PieceBase;
use crate::random::RandomSource;

/// `OceanMonumentPieces.RoomDefinition`.
#[derive(Clone, Debug)]
pub struct RoomDef {
    pub index: i32,
    connections: [Option<usize>; 6],
    has_opening: [bool; 6],
    claimed: bool,
    is_source: bool,
    scan_index: i32,
}

/// Room definitions (an arena; vanilla links objects).
#[derive(Clone, Debug, Default)]
pub struct Rooms {
    pub defs: Vec<RoomDef>,
}

impl Rooms {
    fn add(&mut self, index: i32) -> usize {
        self.defs.push(RoomDef {
            index,
            connections: [None; 6],
            has_opening: [false; 6],
            claimed: false,
            is_source: false,
            scan_index: 0,
        });
        self.defs.len() - 1
    }

    fn set_connection(&mut self, a: usize, d: Direction, b: usize) {
        self.defs[a].connections[d.data_3d() as usize] = Some(b);
        self.defs[b].connections[d.opposite().data_3d() as usize] = Some(a);
    }

    fn update_openings(&mut self, a: usize) {
        for i in 0..6 {
            self.defs[a].has_opening[i] = self.defs[a].connections[i].is_some();
        }
    }

    fn find_source(&mut self, a: usize, scan: i32) -> bool {
        if self.defs[a].is_source {
            return true;
        }
        self.defs[a].scan_index = scan;
        for i in 0..6 {
            if let Some(c) = self.defs[a].connections[i]
                && self.defs[a].has_opening[i]
                && self.defs[c].scan_index != scan
                && self.find_source(c, scan)
            {
                return true;
            }
        }
        false
    }

    pub fn conn(&self, a: usize, d: Direction) -> Option<usize> {
        self.defs[a].connections[d.data_3d() as usize]
    }

    pub fn open(&self, a: usize, d: Direction) -> bool {
        self.defs[a].has_opening[d.data_3d() as usize]
    }

    pub fn count_openings(&self, a: usize) -> i32 {
        self.defs[a].has_opening.iter().filter(|o| **o).count() as i32
    }

    fn claim(&mut self, a: usize) {
        self.defs[a].claimed = true;
    }

    fn claimed_at(&self, a: usize, d: Direction) -> bool {
        self.defs[self.conn(a, d).unwrap()].claimed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomKind {
    Entry,
    Core,
    DoubleX,
    DoubleXY,
    DoubleY,
    DoubleYZ,
    DoubleZ,
    Simple,
    SimpleTop,
    Wing,
    Penthouse,
}

/// A child piece of the building.
#[derive(Clone, Debug)]
pub struct Room {
    pub kind: RoomKind,
    pub base: PieceBase,
    /// Room definition (unused for wings and the penthouse).
    pub def: usize,
    /// `mainDesign` (simple rooms, wings).
    pub design: i32,
}

/// The building's room graph and child pieces.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub rooms: Rooms,
    pub children: Vec<Room>,
}

fn room_index(x: i32, y: i32, z: i32) -> i32 {
    y * 25 + z * 5 + x
}

/// `OceanMonumentPiece.makeBoundingBox(orientation, definition, w, h, d)`.
fn room_box(d: Direction, index: i32, w: i32, h: i32, depth: i32) -> BoundingBox {
    let (rx, rz, ry) = (index % 5, index / 5 % 5, index / 25);
    let mut bb = PieceBase::make_bounding_box(0, 0, 0, d, w * 8, h * 4, depth * 8);
    match d {
        Direction::North => bb.move_by(rx * 8, ry * 4, -(rz + depth) * 8 + 1),
        Direction::South => bb.move_by(rx * 8, ry * 4, rz * 8),
        Direction::West => bb.move_by(-(rz + depth) * 8 + 1, ry * 4, rx * 8),
        _ => bb.move_by(rz * 8, ry * 4, rx * 8),
    }
    bb
}

/// `MonumentBuilding(random, west, north, direction)` after `super(...)`: the room graph and children.
pub fn build_layout(random: &mut dyn RandomSource, building: &PieceBase) -> Layout {
    let d = building.orientation.expect("monument orientation");
    let mut rooms = Rooms::default();
    // `generateRoomGraph`
    let mut grid: [Option<usize>; 75] = [None; 75];
    for (y, xs, zs) in [(0, 0..5, 0..4), (1, 0..5, 0..4), (2, 1..4, 0..2)] {
        for x in xs.clone() {
            for z in zs.clone() {
                let pos = room_index(x, y, z);
                grid[pos as usize] = Some(rooms.add(pos));
            }
        }
    }
    let source = grid[room_index(2, 0, 0) as usize].unwrap();
    for x in 0..5 {
        for z in 0..5 {
            for y in 0..3 {
                let pos = room_index(x, y, z);
                let Some(here) = grid[pos as usize] else { continue };
                for dir in Direction::ALL {
                    let (nx, ny, nz) = (x + dir.step_x(), y + dir.step_y(), z + dir.step_z());
                    if (0..5).contains(&nx)
                        && (0..5).contains(&nz)
                        && (0..3).contains(&ny)
                        && let Some(n) = grid[room_index(nx, ny, nz) as usize]
                    {
                        if nz == z {
                            rooms.set_connection(here, dir, n);
                        } else {
                            rooms.set_connection(here, dir.opposite(), n);
                        }
                    }
                }
            }
        }
    }
    let roof = rooms.add(1003);
    let left_wing = rooms.add(1001);
    let right_wing = rooms.add(1002);
    rooms.set_connection(grid[room_index(2, 2, 0) as usize].unwrap(), Direction::Up, roof);
    rooms.set_connection(
        grid[room_index(0, 1, 0) as usize].unwrap(),
        Direction::South,
        left_wing,
    );
    rooms.set_connection(
        grid[room_index(4, 1, 0) as usize].unwrap(),
        Direction::South,
        right_wing,
    );
    for r in [roof, left_wing, right_wing] {
        rooms.claim(r);
    }
    rooms.defs[source].is_source = true;
    let core = grid[room_index(random.next_int_bounded(4), 0, 2) as usize].unwrap();
    rooms.claim(core);
    let c = |r: &Rooms, a: usize, dir: Direction| r.conn(a, dir).unwrap();
    let e = c(&rooms, core, Direction::East);
    let n = c(&rooms, core, Direction::North);
    let en = c(&rooms, e, Direction::North);
    for r in [
        e,
        n,
        en,
        c(&rooms, core, Direction::Up),
        c(&rooms, e, Direction::Up),
        c(&rooms, n, Direction::Up),
        c(&rooms, en, Direction::Up),
    ] {
        rooms.claim(r);
    }
    let mut defs: Vec<usize> = Vec::new();
    for g in grid.iter().flatten() {
        rooms.update_openings(*g);
        defs.push(*g);
    }
    rooms.update_openings(roof);
    super::util::shuffle(&mut defs, random);
    let mut scan = 1;
    for &def in &defs {
        let (mut close, mut attempts) = (0, 0);
        while close < 2 && attempts < 5 {
            attempts += 1;
            let f = random.next_int_bounded(6) as usize;
            if rooms.defs[def].has_opening[f] {
                let of = Direction::from_3d(f as i32).opposite().data_3d() as usize;
                let other = rooms.defs[def].connections[f].unwrap();
                rooms.defs[def].has_opening[f] = false;
                rooms.defs[other].has_opening[of] = false;
                let first = rooms.find_source(def, scan);
                scan += 1;
                let ok = first && {
                    let second = rooms.find_source(other, scan);
                    scan += 1;
                    second
                };
                if ok {
                    close += 1;
                } else {
                    rooms.defs[def].has_opening[f] = true;
                    rooms.defs[other].has_opening[of] = true;
                }
            }
        }
    }
    defs.push(roof);
    defs.push(left_wing);
    defs.push(right_wing);
    // The constructor: entry and core rooms, then the fitters in vanilla order.
    rooms.claim(source);
    let mut children = vec![
        Room {
            kind: RoomKind::Entry,
            base: PieceBase::new(1, room_box(d, rooms.defs[source].index, 1, 1, 1), Some(d)),
            def: source,
            design: 0,
        },
        Room {
            kind: RoomKind::Core,
            base: PieceBase::new(1, room_box(d, rooms.defs[core].index, 2, 2, 2), Some(d)),
            def: core,
            design: 0,
        },
    ];
    for &def in &defs {
        if rooms.defs[def].claimed || rooms.defs[def].index >= 75 {
            continue;
        }
        let open = |r: &Rooms, dir: Direction| r.open(def, dir);
        let up_ok = |r: &Rooms, a: usize| r.open(a, Direction::Up) && !r.claimed_at(a, Direction::Up);
        let (kind, size) = if open(&rooms, Direction::East)
            && !rooms.claimed_at(def, Direction::East)
            && up_ok(&rooms, def)
            && up_ok(&rooms, c(&rooms, def, Direction::East))
        {
            let east = c(&rooms, def, Direction::East);
            for r in [
                def,
                east,
                c(&rooms, def, Direction::Up),
                c(&rooms, east, Direction::Up),
            ] {
                rooms.claim(r);
            }
            (RoomKind::DoubleXY, (2, 2, 1))
        } else if open(&rooms, Direction::North)
            && !rooms.claimed_at(def, Direction::North)
            && up_ok(&rooms, def)
            && up_ok(&rooms, c(&rooms, def, Direction::North))
        {
            let north = c(&rooms, def, Direction::North);
            for r in [
                def,
                north,
                c(&rooms, def, Direction::Up),
                c(&rooms, north, Direction::Up),
            ] {
                rooms.claim(r);
            }
            (RoomKind::DoubleYZ, (1, 2, 2))
        } else if open(&rooms, Direction::North) && !rooms.claimed_at(def, Direction::North) {
            rooms.claim(def);
            rooms.claim(c(&rooms, def, Direction::North));
            (RoomKind::DoubleZ, (1, 1, 2))
        } else if open(&rooms, Direction::East) && !rooms.claimed_at(def, Direction::East) {
            rooms.claim(def);
            rooms.claim(c(&rooms, def, Direction::East));
            (RoomKind::DoubleX, (2, 1, 1))
        } else if up_ok(&rooms, def) {
            rooms.claim(def);
            rooms.claim(c(&rooms, def, Direction::Up));
            (RoomKind::DoubleY, (1, 2, 1))
        } else if !open(&rooms, Direction::West)
            && !open(&rooms, Direction::East)
            && !open(&rooms, Direction::North)
            && !open(&rooms, Direction::South)
            && !open(&rooms, Direction::Up)
        {
            rooms.claim(def);
            (RoomKind::SimpleTop, (1, 1, 1))
        } else {
            rooms.claim(def);
            (RoomKind::Simple, (1, 1, 1))
        };
        let design = if kind == RoomKind::Simple {
            random.next_int_bounded(3)
        } else {
            0
        };
        let bb = room_box(d, rooms.defs[def].index, size.0, size.1, size.2);
        children.push(Room {
            kind,
            base: PieceBase::new(1, bb, Some(d)),
            def,
            design,
        });
    }
    let offset = building.world_pos(9, 0, 22);
    for ch in &mut children {
        ch.base.bbox.move_by(offset.x, offset.y, offset.z);
    }
    let corners = |a: (i32, i32, i32), b2: (i32, i32, i32)| {
        BoundingBox::from_corners(
            building.world_pos(a.0, a.1, a.2),
            building.world_pos(b2.0, b2.1, b2.2),
        )
    };
    let wing_random = random.next_int();
    children.push(Room {
        kind: RoomKind::Wing,
        base: PieceBase::new(1, corners((1, 1, 1), (23, 8, 21)), Some(d)),
        def: usize::MAX,
        design: wing_random & 1,
    });
    children.push(Room {
        kind: RoomKind::Wing,
        base: PieceBase::new(1, corners((34, 1, 1), (56, 8, 21)), Some(d)),
        def: usize::MAX,
        design: wing_random.wrapping_add(1) & 1,
    });
    children.push(Room {
        kind: RoomKind::Penthouse,
        base: PieceBase::new(1, corners((22, 13, 22), (35, 17, 35)), Some(d)),
        def: usize::MAX,
        design: 0,
    });
    Layout { rooms, children }
}

/// `MonumentBuilding.postProcess`.
pub fn post_process_building(
    ctx: &mut PieceCtx,
    b: &PieceBase,
    random: &mut dyn RandomSource,
    layout: &Layout,
) {
    let p = palette(ctx);
    let water_height = ctx.region.env.sea_level.max(64) - b.bbox.min_y;
    water_box(ctx, b, 0, 0, 0, 58, water_height, 58);
    generate_wing(ctx, b, false, 0);
    generate_wing(ctx, b, true, 33);
    generate_entrance_archs(ctx, b);
    generate_entrance_wall(ctx, b);
    generate_roof_piece(ctx, b);
    generate_lower_wall(ctx, b);
    generate_middle_wall(ctx, b);
    generate_upper_wall(ctx, b);
    for pillar_x in 0..7 {
        let mut pillar_z = 0;
        while pillar_z < 7 {
            if pillar_z == 0 && pillar_x == 3 {
                pillar_z = 6;
            }
            let (bx, bz) = (pillar_x * 9, pillar_z * 9);
            for w in 0..4 {
                for dd in 0..4 {
                    ctx.place_block(b, p.base_light, bx + w, 0, bz + dd);
                    ctx.fill_column_down(b, p.base_light, bx + w, -1, bz + dd);
                }
            }
            if pillar_x != 0 && pillar_x != 6 {
                pillar_z += 6;
            } else {
                pillar_z += 1;
            }
        }
    }
    for i in 0..5 {
        water_box(ctx, b, -1 - i, i * 2, -1 - i, -1 - i, 23, 58 + i);
        water_box(ctx, b, 58 + i, i * 2, -1 - i, 58 + i, 23, 58 + i);
        water_box(ctx, b, -i, i * 2, -1 - i, 57 + i, 23, -1 - i);
        water_box(ctx, b, -i, i * 2, 58 + i, 57 + i, 23, 58 + i);
    }
    let rooms = &layout.rooms;
    for ch in &layout.children {
        if !ch.base.bbox.intersects(&ctx.chunk_bb) {
            continue;
        }
        let cb = &ch.base;
        match ch.kind {
            RoomKind::Entry => entry_room(ctx, cb, rooms, ch.def),
            RoomKind::Core => core_room(ctx, cb),
            RoomKind::DoubleX => double_x_room(ctx, cb, rooms, ch.def),
            RoomKind::DoubleXY => double_xy_room(ctx, cb, rooms, ch.def),
            RoomKind::DoubleY => double_y_room(ctx, cb, rooms, ch.def),
            RoomKind::DoubleYZ => double_yz_room(ctx, cb, rooms, ch.def),
            RoomKind::DoubleZ => double_z_room(ctx, cb, rooms, ch.def),
            RoomKind::Simple => simple_room(ctx, cb, random, rooms, ch.def, ch.design),
            RoomKind::SimpleTop => simple_top_room(ctx, cb, random, rooms, ch.def),
            RoomKind::Wing => wing_room(ctx, cb, ch.design),
            RoomKind::Penthouse => penthouse(ctx, cb),
        }
    }
}

/// `BASE_GRAY`, `BASE_LIGHT`, `BASE_BLACK`, `LAMP_BLOCK`, `FILL_BLOCK`.
#[derive(Clone, Copy)]
struct Palette {
    base_gray: BlockState,
    base_light: BlockState,
    base_black: BlockState,
    lamp: BlockState,
}

fn palette(ctx: &PieceCtx) -> Palette {
    Palette {
        base_gray: ctx.bs("prismarine").s(),
        base_light: ctx.bs("prismarine_bricks").s(),
        base_black: ctx.bs("dark_prismarine").s(),
        lamp: ctx.bs("sea_lantern").s(),
    }
}

/// `generateWaterBox`: water below sea level, air above, keeping ice and water.
fn water_box(ctx: &mut PieceCtx, b: &PieceBase, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
    let env = ctx.region.env;
    let keep = [env.b.ice, env.b.packed_ice, env.b.blue_ice, env.b.water];
    let fill = ctx.bs("water").s();
    for y in y0..=y1 {
        for x in x0..=x1 {
            for z in z0..=z1 {
                let block = ctx.get_block(b, x, y, z);
                if keep.contains(&env.blocks.block_of(block)) {
                    continue;
                }
                if b.world_y(y) >= env.sea_level && block != fill {
                    ctx.place_block(b, BlockState::AIR, x, y, z);
                } else {
                    ctx.place_block(b, fill, x, y, z);
                }
            }
        }
    }
}

/// `generateDefaultFloor`.
fn default_floor(ctx: &mut PieceCtx, b: &PieceBase, x: i32, z: i32, down_opening: bool) {
    let Palette {
        base_gray: gray,
        base_light: light,
        ..
    } = palette(ctx);
    if down_opening {
        solid_box(ctx, b, x, 0, z, x + 2, 0, z + 7, gray);
        solid_box(ctx, b, x + 5, 0, z, x + 7, 0, z + 7, gray);
        solid_box(ctx, b, x + 3, 0, z, x + 4, 0, z + 2, gray);
        solid_box(ctx, b, x + 3, 0, z + 5, x + 4, 0, z + 7, gray);
        solid_box(ctx, b, x + 3, 0, z + 2, x + 4, 0, z + 2, light);
        solid_box(ctx, b, x + 3, 0, z + 5, x + 4, 0, z + 5, light);
        solid_box(ctx, b, x + 2, 0, z + 3, x + 2, 0, z + 4, light);
        solid_box(ctx, b, x + 5, 0, z + 3, x + 5, 0, z + 4, light);
    } else {
        solid_box(ctx, b, x, 0, z, x + 7, 0, z + 7, gray);
    }
}

/// `generateBox` with one state for the edge and the inside, not skipping air.
fn solid_box(
    ctx: &mut PieceCtx,
    b: &PieceBase,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
    state: BlockState,
) {
    ctx.generate_box(b, x0, y0, z0, x1, y1, z1, state, state, false);
}

/// `generateBoxOnFillOnly`.
fn fill_only(
    ctx: &mut PieceCtx,
    b: &PieceBase,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
    target: BlockState,
) {
    let fill = ctx.bs("water").s();
    for y in y0..=y1 {
        for x in x0..=x1 {
            for z in z0..=z1 {
                if ctx.get_block(b, x, y, z) == fill {
                    ctx.place_block(b, target, x, y, z);
                }
            }
        }
    }
}

/// `chunkIntersects`.
fn chunk_intersects(ctx: &mut PieceCtx, b: &PieceBase, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
    let (wx0, wz0, wx1, wz1) = (
        b.world_x(x0, z0),
        b.world_z(x0, z0),
        b.world_x(x1, z1),
        b.world_z(x1, z1),
    );
    ctx.chunk_bb
        .intersects_xz(wx0.min(wx1), wz0.min(wz1), wx0.max(wx1), wz0.max(wz1))
}

/// `spawnElder` (elder guardians make themselves persistent in their constructor).
fn spawn_elder(ctx: &mut PieceCtx, b: &PieceBase, x: i32, y: i32, z: i32) {
    let pos = b.world_pos(x, y, z);
    if ctx.chunk_bb.is_inside(pos.x, pos.y, pos.z) {
        let tag = super::templated::structure_mob("minecraft:elder_guardian", pos);
        crate::entity::spawn_mob(ctx.region, tag);
    }
}

fn generate_wing(ctx: &mut PieceCtx, b: &PieceBase, is_flipped: bool, xoff: i32) {
    let p = palette(ctx);
    if chunk_intersects(ctx, b, xoff, 0, xoff + 23, 20) {
        solid_box(ctx, b, xoff, 0, 0, xoff + 24, 0, 20, p.base_gray);
        water_box(ctx, b, xoff, 1, 0, xoff + 24, 10, 20);

        for i in 0..4 {
            solid_box(ctx, b, xoff + i, i + 1, i, xoff + i, i + 1, 20, p.base_light);
            solid_box(
                ctx,
                b,
                xoff + i + 7,
                i + 5,
                i + 7,
                xoff + i + 7,
                i + 5,
                20,
                p.base_light,
            );
            solid_box(
                ctx,
                b,
                xoff + 17 - i,
                i + 5,
                i + 7,
                xoff + 17 - i,
                i + 5,
                20,
                p.base_light,
            );
            solid_box(
                ctx,
                b,
                xoff + 24 - i,
                i + 1,
                i,
                xoff + 24 - i,
                i + 1,
                20,
                p.base_light,
            );
            solid_box(
                ctx,
                b,
                xoff + i + 1,
                i + 1,
                i,
                xoff + 23 - i,
                i + 1,
                i,
                p.base_light,
            );
            solid_box(
                ctx,
                b,
                xoff + i + 8,
                i + 5,
                i + 7,
                xoff + 16 - i,
                i + 5,
                i + 7,
                p.base_light,
            );
        }

        solid_box(ctx, b, xoff + 4, 4, 4, xoff + 6, 4, 20, p.base_gray);
        solid_box(ctx, b, xoff + 7, 4, 4, xoff + 17, 4, 6, p.base_gray);
        solid_box(ctx, b, xoff + 18, 4, 4, xoff + 20, 4, 20, p.base_gray);
        solid_box(ctx, b, xoff + 11, 8, 11, xoff + 13, 8, 20, p.base_gray);
        ctx.place_block(b, p.base_light, xoff + 12, 9, 12);
        ctx.place_block(b, p.base_light, xoff + 12, 9, 15);
        ctx.place_block(b, p.base_light, xoff + 12, 9, 18);
        let left_pos = xoff + if is_flipped { 19 } else { 5 };
        let right_pos = xoff + if is_flipped { 5 } else { 19 };

        for z in (5..=20).rev().step_by(3) {
            ctx.place_block(b, p.base_light, left_pos, 5, z);
        }

        for z in (7..=19).rev().step_by(3) {
            ctx.place_block(b, p.base_light, right_pos, 5, z);
        }

        for i in 0..4 {
            let pos = if is_flipped {
                xoff + 24 - (17 - i * 3)
            } else {
                xoff + 17 - i * 3
            };
            ctx.place_block(b, p.base_light, pos, 5, 5);
        }

        ctx.place_block(b, p.base_light, right_pos, 5, 5);
        solid_box(ctx, b, xoff + 11, 1, 12, xoff + 13, 7, 12, p.base_gray);
        solid_box(ctx, b, xoff + 12, 1, 11, xoff + 12, 7, 13, p.base_gray);
    }
}

fn generate_entrance_archs(ctx: &mut PieceCtx, b: &PieceBase) {
    let p = palette(ctx);
    if chunk_intersects(ctx, b, 22, 5, 35, 17) {
        water_box(ctx, b, 25, 0, 0, 32, 8, 20);

        for i in 0..4 {
            solid_box(ctx, b, 24, 2, 5 + i * 4, 24, 4, 5 + i * 4, p.base_light);
            solid_box(ctx, b, 22, 4, 5 + i * 4, 23, 4, 5 + i * 4, p.base_light);
            ctx.place_block(b, p.base_light, 25, 5, 5 + i * 4);
            ctx.place_block(b, p.base_light, 26, 6, 5 + i * 4);
            ctx.place_block(b, p.lamp, 26, 5, 5 + i * 4);
            solid_box(ctx, b, 33, 2, 5 + i * 4, 33, 4, 5 + i * 4, p.base_light);
            solid_box(ctx, b, 34, 4, 5 + i * 4, 35, 4, 5 + i * 4, p.base_light);
            ctx.place_block(b, p.base_light, 32, 5, 5 + i * 4);
            ctx.place_block(b, p.base_light, 31, 6, 5 + i * 4);
            ctx.place_block(b, p.lamp, 31, 5, 5 + i * 4);
            solid_box(ctx, b, 27, 6, 5 + i * 4, 30, 6, 5 + i * 4, p.base_gray);
        }
    }
}

fn generate_entrance_wall(ctx: &mut PieceCtx, b: &PieceBase) {
    let p = palette(ctx);
    if chunk_intersects(ctx, b, 15, 20, 42, 21) {
        solid_box(ctx, b, 15, 0, 21, 42, 0, 21, p.base_gray);
        water_box(ctx, b, 26, 1, 21, 31, 3, 21);
        solid_box(ctx, b, 21, 12, 21, 36, 12, 21, p.base_gray);
        solid_box(ctx, b, 17, 11, 21, 40, 11, 21, p.base_gray);
        solid_box(ctx, b, 16, 10, 21, 41, 10, 21, p.base_gray);
        solid_box(ctx, b, 15, 7, 21, 42, 9, 21, p.base_gray);
        solid_box(ctx, b, 16, 6, 21, 41, 6, 21, p.base_gray);
        solid_box(ctx, b, 17, 5, 21, 40, 5, 21, p.base_gray);
        solid_box(ctx, b, 21, 4, 21, 36, 4, 21, p.base_gray);
        solid_box(ctx, b, 22, 3, 21, 26, 3, 21, p.base_gray);
        solid_box(ctx, b, 31, 3, 21, 35, 3, 21, p.base_gray);
        solid_box(ctx, b, 23, 2, 21, 25, 2, 21, p.base_gray);
        solid_box(ctx, b, 32, 2, 21, 34, 2, 21, p.base_gray);
        solid_box(ctx, b, 28, 4, 20, 29, 4, 21, p.base_light);
        ctx.place_block(b, p.base_light, 27, 3, 21);
        ctx.place_block(b, p.base_light, 30, 3, 21);
        ctx.place_block(b, p.base_light, 26, 2, 21);
        ctx.place_block(b, p.base_light, 31, 2, 21);
        ctx.place_block(b, p.base_light, 25, 1, 21);
        ctx.place_block(b, p.base_light, 32, 1, 21);

        for i in 0..7 {
            ctx.place_block(b, p.base_black, 28 - i, 6 + i, 21);
            ctx.place_block(b, p.base_black, 29 + i, 6 + i, 21);
        }

        for i in 0..4 {
            ctx.place_block(b, p.base_black, 28 - i, 9 + i, 21);
            ctx.place_block(b, p.base_black, 29 + i, 9 + i, 21);
        }

        ctx.place_block(b, p.base_black, 28, 12, 21);
        ctx.place_block(b, p.base_black, 29, 12, 21);

        for i in 0..3 {
            ctx.place_block(b, p.base_black, 22 - i * 2, 8, 21);
            ctx.place_block(b, p.base_black, 22 - i * 2, 9, 21);
            ctx.place_block(b, p.base_black, 35 + i * 2, 8, 21);
            ctx.place_block(b, p.base_black, 35 + i * 2, 9, 21);
        }

        water_box(ctx, b, 15, 13, 21, 42, 15, 21);
        water_box(ctx, b, 15, 1, 21, 15, 6, 21);
        water_box(ctx, b, 16, 1, 21, 16, 5, 21);
        water_box(ctx, b, 17, 1, 21, 20, 4, 21);
        water_box(ctx, b, 21, 1, 21, 21, 3, 21);
        water_box(ctx, b, 22, 1, 21, 22, 2, 21);
        water_box(ctx, b, 23, 1, 21, 24, 1, 21);
        water_box(ctx, b, 42, 1, 21, 42, 6, 21);
        water_box(ctx, b, 41, 1, 21, 41, 5, 21);
        water_box(ctx, b, 37, 1, 21, 40, 4, 21);
        water_box(ctx, b, 36, 1, 21, 36, 3, 21);
        water_box(ctx, b, 33, 1, 21, 34, 1, 21);
        water_box(ctx, b, 35, 1, 21, 35, 2, 21);
    }
}

fn generate_roof_piece(ctx: &mut PieceCtx, b: &PieceBase) {
    let p = palette(ctx);
    if chunk_intersects(ctx, b, 21, 21, 36, 36) {
        solid_box(ctx, b, 21, 0, 22, 36, 0, 36, p.base_gray);
        water_box(ctx, b, 21, 1, 22, 36, 23, 36);

        for i in 0..4 {
            solid_box(
                ctx,
                b,
                21 + i,
                13 + i,
                21 + i,
                36 - i,
                13 + i,
                21 + i,
                p.base_light,
            );
            solid_box(
                ctx,
                b,
                21 + i,
                13 + i,
                36 - i,
                36 - i,
                13 + i,
                36 - i,
                p.base_light,
            );
            solid_box(
                ctx,
                b,
                21 + i,
                13 + i,
                22 + i,
                21 + i,
                13 + i,
                35 - i,
                p.base_light,
            );
            solid_box(
                ctx,
                b,
                36 - i,
                13 + i,
                22 + i,
                36 - i,
                13 + i,
                35 - i,
                p.base_light,
            );
        }

        solid_box(ctx, b, 25, 16, 25, 32, 16, 32, p.base_gray);
        solid_box(ctx, b, 25, 17, 25, 25, 19, 25, p.base_light);
        solid_box(ctx, b, 32, 17, 25, 32, 19, 25, p.base_light);
        solid_box(ctx, b, 25, 17, 32, 25, 19, 32, p.base_light);
        solid_box(ctx, b, 32, 17, 32, 32, 19, 32, p.base_light);
        ctx.place_block(b, p.base_light, 26, 20, 26);
        ctx.place_block(b, p.base_light, 27, 21, 27);
        ctx.place_block(b, p.lamp, 27, 20, 27);
        ctx.place_block(b, p.base_light, 26, 20, 31);
        ctx.place_block(b, p.base_light, 27, 21, 30);
        ctx.place_block(b, p.lamp, 27, 20, 30);
        ctx.place_block(b, p.base_light, 31, 20, 31);
        ctx.place_block(b, p.base_light, 30, 21, 30);
        ctx.place_block(b, p.lamp, 30, 20, 30);
        ctx.place_block(b, p.base_light, 31, 20, 26);
        ctx.place_block(b, p.base_light, 30, 21, 27);
        ctx.place_block(b, p.lamp, 30, 20, 27);
        solid_box(ctx, b, 28, 21, 27, 29, 21, 27, p.base_gray);
        solid_box(ctx, b, 27, 21, 28, 27, 21, 29, p.base_gray);
        solid_box(ctx, b, 28, 21, 30, 29, 21, 30, p.base_gray);
        solid_box(ctx, b, 30, 21, 28, 30, 21, 29, p.base_gray);
    }
}

fn generate_lower_wall(ctx: &mut PieceCtx, b: &PieceBase) {
    let p = palette(ctx);
    if chunk_intersects(ctx, b, 0, 21, 6, 58) {
        solid_box(ctx, b, 0, 0, 21, 6, 0, 57, p.base_gray);
        water_box(ctx, b, 0, 1, 21, 6, 7, 57);
        solid_box(ctx, b, 4, 4, 21, 6, 4, 53, p.base_gray);

        for i in 0..4 {
            solid_box(ctx, b, i, i + 1, 21, i, i + 1, 57 - i, p.base_light);
        }

        for z in (23..53).step_by(3) {
            ctx.place_block(b, p.base_light, 5, 5, z);
        }

        ctx.place_block(b, p.base_light, 5, 5, 52);

        for i in 0..4 {
            solid_box(ctx, b, i, i + 1, 21, i, i + 1, 57 - i, p.base_light);
        }

        solid_box(ctx, b, 4, 1, 52, 6, 3, 52, p.base_gray);
        solid_box(ctx, b, 5, 1, 51, 5, 3, 53, p.base_gray);
    }

    if chunk_intersects(ctx, b, 51, 21, 58, 58) {
        solid_box(ctx, b, 51, 0, 21, 57, 0, 57, p.base_gray);
        water_box(ctx, b, 51, 1, 21, 57, 7, 57);
        solid_box(ctx, b, 51, 4, 21, 53, 4, 53, p.base_gray);

        for i in 0..4 {
            solid_box(ctx, b, 57 - i, i + 1, 21, 57 - i, i + 1, 57 - i, p.base_light);
        }

        for z in (23..53).step_by(3) {
            ctx.place_block(b, p.base_light, 52, 5, z);
        }

        ctx.place_block(b, p.base_light, 52, 5, 52);
        solid_box(ctx, b, 51, 1, 52, 53, 3, 52, p.base_gray);
        solid_box(ctx, b, 52, 1, 51, 52, 3, 53, p.base_gray);
    }

    if chunk_intersects(ctx, b, 0, 51, 57, 57) {
        solid_box(ctx, b, 7, 0, 51, 50, 0, 57, p.base_gray);
        water_box(ctx, b, 7, 1, 51, 50, 10, 57);

        for i in 0..4 {
            solid_box(ctx, b, i + 1, i + 1, 57 - i, 56 - i, i + 1, 57 - i, p.base_light);
        }
    }
}

fn generate_middle_wall(ctx: &mut PieceCtx, b: &PieceBase) {
    let p = palette(ctx);
    if chunk_intersects(ctx, b, 7, 21, 13, 50) {
        solid_box(ctx, b, 7, 0, 21, 13, 0, 50, p.base_gray);
        water_box(ctx, b, 7, 1, 21, 13, 10, 50);
        solid_box(ctx, b, 11, 8, 21, 13, 8, 53, p.base_gray);

        for i in 0..4 {
            solid_box(ctx, b, i + 7, i + 5, 21, i + 7, i + 5, 54, p.base_light);
        }

        for z in (21..=45).step_by(3) {
            ctx.place_block(b, p.base_light, 12, 9, z);
        }
    }

    if chunk_intersects(ctx, b, 44, 21, 50, 54) {
        solid_box(ctx, b, 44, 0, 21, 50, 0, 50, p.base_gray);
        water_box(ctx, b, 44, 1, 21, 50, 10, 50);
        solid_box(ctx, b, 44, 8, 21, 46, 8, 53, p.base_gray);

        for i in 0..4 {
            solid_box(ctx, b, 50 - i, i + 5, 21, 50 - i, i + 5, 54, p.base_light);
        }

        for z in (21..=45).step_by(3) {
            ctx.place_block(b, p.base_light, 45, 9, z);
        }
    }

    if chunk_intersects(ctx, b, 8, 44, 49, 54) {
        solid_box(ctx, b, 14, 0, 44, 43, 0, 50, p.base_gray);
        water_box(ctx, b, 14, 1, 44, 43, 10, 50);

        for x in (12..=45).step_by(3) {
            ctx.place_block(b, p.base_light, x, 9, 45);
            ctx.place_block(b, p.base_light, x, 9, 52);
            if x == 12 || x == 18 || x == 24 || x == 33 || x == 39 || x == 45 {
                ctx.place_block(b, p.base_light, x, 9, 47);
                ctx.place_block(b, p.base_light, x, 9, 50);
                ctx.place_block(b, p.base_light, x, 10, 45);
                ctx.place_block(b, p.base_light, x, 10, 46);
                ctx.place_block(b, p.base_light, x, 10, 51);
                ctx.place_block(b, p.base_light, x, 10, 52);
                ctx.place_block(b, p.base_light, x, 11, 47);
                ctx.place_block(b, p.base_light, x, 11, 50);
                ctx.place_block(b, p.base_light, x, 12, 48);
                ctx.place_block(b, p.base_light, x, 12, 49);
            }
        }

        for i in 0..3 {
            solid_box(ctx, b, 8 + i, 5 + i, 54, 49 - i, 5 + i, 54, p.base_gray);
        }

        solid_box(ctx, b, 11, 8, 54, 46, 8, 54, p.base_light);
        solid_box(ctx, b, 14, 8, 44, 43, 8, 53, p.base_gray);
    }
}

fn generate_upper_wall(ctx: &mut PieceCtx, b: &PieceBase) {
    let p = palette(ctx);
    if chunk_intersects(ctx, b, 14, 21, 20, 43) {
        solid_box(ctx, b, 14, 0, 21, 20, 0, 43, p.base_gray);
        water_box(ctx, b, 14, 1, 22, 20, 14, 43);
        solid_box(ctx, b, 18, 12, 22, 20, 12, 39, p.base_gray);
        solid_box(ctx, b, 18, 12, 21, 20, 12, 21, p.base_light);

        for i in 0..4 {
            solid_box(ctx, b, i + 14, i + 9, 21, i + 14, i + 9, 43 - i, p.base_light);
        }

        for z in (23..=39).step_by(3) {
            ctx.place_block(b, p.base_light, 19, 13, z);
        }
    }

    if chunk_intersects(ctx, b, 37, 21, 43, 43) {
        solid_box(ctx, b, 37, 0, 21, 43, 0, 43, p.base_gray);
        water_box(ctx, b, 37, 1, 22, 43, 14, 43);
        solid_box(ctx, b, 37, 12, 22, 39, 12, 39, p.base_gray);
        solid_box(ctx, b, 37, 12, 21, 39, 12, 21, p.base_light);

        for i in 0..4 {
            solid_box(ctx, b, 43 - i, i + 9, 21, 43 - i, i + 9, 43 - i, p.base_light);
        }

        for z in (23..=39).step_by(3) {
            ctx.place_block(b, p.base_light, 38, 13, z);
        }
    }

    if chunk_intersects(ctx, b, 15, 37, 42, 43) {
        solid_box(ctx, b, 21, 0, 37, 36, 0, 43, p.base_gray);
        water_box(ctx, b, 21, 1, 37, 36, 14, 43);
        solid_box(ctx, b, 21, 12, 37, 36, 12, 39, p.base_gray);

        for i in 0..4 {
            solid_box(ctx, b, 15 + i, i + 9, 43 - i, 42 - i, i + 9, 43 - i, p.base_light);
        }

        for x in (21..=36).step_by(3) {
            ctx.place_block(b, p.base_light, x, 13, 38);
        }
    }
}

fn core_room(ctx: &mut PieceCtx, b: &PieceBase) {
    let p = palette(ctx);
    fill_only(ctx, b, 1, 8, 0, 14, 8, 14, p.base_gray);
    solid_box(ctx, b, 0, 7, 0, 0, 7, 15, p.base_light);
    solid_box(ctx, b, 15, 7, 0, 15, 7, 15, p.base_light);
    solid_box(ctx, b, 1, 7, 0, 15, 7, 0, p.base_light);
    solid_box(ctx, b, 1, 7, 15, 14, 7, 15, p.base_light);

    for yx in 1..=6 {
        let block = if yx == 2 || yx == 6 {
            p.base_gray
        } else {
            p.base_light
        };

        for x in (0..=15).step_by(15) {
            solid_box(ctx, b, x, yx, 0, x, yx, 1, block);
            solid_box(ctx, b, x, yx, 6, x, yx, 9, block);
            solid_box(ctx, b, x, yx, 14, x, yx, 15, block);
        }

        solid_box(ctx, b, 1, yx, 0, 1, yx, 0, block);
        solid_box(ctx, b, 6, yx, 0, 9, yx, 0, block);
        solid_box(ctx, b, 14, yx, 0, 14, yx, 0, block);
        solid_box(ctx, b, 1, yx, 15, 14, yx, 15, block);
    }

    solid_box(ctx, b, 6, 3, 6, 9, 6, 9, p.base_black);
    let gold = ctx.bs("gold_block").s();
    solid_box(ctx, b, 7, 4, 7, 8, 5, 8, gold);

    for yx in (3..=6).step_by(3) {
        for x in (6..=9).step_by(3) {
            ctx.place_block(b, p.lamp, x, yx, 6);
            ctx.place_block(b, p.lamp, x, yx, 9);
        }
    }

    solid_box(ctx, b, 5, 1, 6, 5, 2, 6, p.base_light);
    solid_box(ctx, b, 5, 1, 9, 5, 2, 9, p.base_light);
    solid_box(ctx, b, 10, 1, 6, 10, 2, 6, p.base_light);
    solid_box(ctx, b, 10, 1, 9, 10, 2, 9, p.base_light);
    solid_box(ctx, b, 6, 1, 5, 6, 2, 5, p.base_light);
    solid_box(ctx, b, 9, 1, 5, 9, 2, 5, p.base_light);
    solid_box(ctx, b, 6, 1, 10, 6, 2, 10, p.base_light);
    solid_box(ctx, b, 9, 1, 10, 9, 2, 10, p.base_light);
    solid_box(ctx, b, 5, 2, 5, 5, 6, 5, p.base_light);
    solid_box(ctx, b, 5, 2, 10, 5, 6, 10, p.base_light);
    solid_box(ctx, b, 10, 2, 5, 10, 6, 5, p.base_light);
    solid_box(ctx, b, 10, 2, 10, 10, 6, 10, p.base_light);
    solid_box(ctx, b, 5, 7, 1, 5, 7, 6, p.base_light);
    solid_box(ctx, b, 10, 7, 1, 10, 7, 6, p.base_light);
    solid_box(ctx, b, 5, 7, 9, 5, 7, 14, p.base_light);
    solid_box(ctx, b, 10, 7, 9, 10, 7, 14, p.base_light);
    solid_box(ctx, b, 1, 7, 5, 6, 7, 5, p.base_light);
    solid_box(ctx, b, 1, 7, 10, 6, 7, 10, p.base_light);
    solid_box(ctx, b, 9, 7, 5, 14, 7, 5, p.base_light);
    solid_box(ctx, b, 9, 7, 10, 14, 7, 10, p.base_light);
    solid_box(ctx, b, 2, 1, 2, 2, 1, 3, p.base_light);
    solid_box(ctx, b, 3, 1, 2, 3, 1, 2, p.base_light);
    solid_box(ctx, b, 13, 1, 2, 13, 1, 3, p.base_light);
    solid_box(ctx, b, 12, 1, 2, 12, 1, 2, p.base_light);
    solid_box(ctx, b, 2, 1, 12, 2, 1, 13, p.base_light);
    solid_box(ctx, b, 3, 1, 13, 3, 1, 13, p.base_light);
    solid_box(ctx, b, 13, 1, 12, 13, 1, 13, p.base_light);
    solid_box(ctx, b, 12, 1, 13, 12, 1, 13, p.base_light);
}

fn double_x_room(ctx: &mut PieceCtx, b: &PieceBase, rooms: &Rooms, def: usize) {
    let p = palette(ctx);
    let east = rooms.conn(def, Direction::East).unwrap();
    let west = def;
    if rooms.defs[def].index / 25 > 0 {
        default_floor(ctx, b, 8, 0, rooms.open(east, Direction::Down));
        default_floor(ctx, b, 0, 0, rooms.open(west, Direction::Down));
    }

    if rooms.conn(west, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 4, 1, 7, 4, 6, p.base_gray);
    }

    if rooms.conn(east, Direction::Up).is_none() {
        fill_only(ctx, b, 8, 4, 1, 14, 4, 6, p.base_gray);
    }

    solid_box(ctx, b, 0, 3, 0, 0, 3, 7, p.base_light);
    solid_box(ctx, b, 15, 3, 0, 15, 3, 7, p.base_light);
    solid_box(ctx, b, 1, 3, 0, 15, 3, 0, p.base_light);
    solid_box(ctx, b, 1, 3, 7, 14, 3, 7, p.base_light);
    solid_box(ctx, b, 0, 2, 0, 0, 2, 7, p.base_gray);
    solid_box(ctx, b, 15, 2, 0, 15, 2, 7, p.base_gray);
    solid_box(ctx, b, 1, 2, 0, 15, 2, 0, p.base_gray);
    solid_box(ctx, b, 1, 2, 7, 14, 2, 7, p.base_gray);
    solid_box(ctx, b, 0, 1, 0, 0, 1, 7, p.base_light);
    solid_box(ctx, b, 15, 1, 0, 15, 1, 7, p.base_light);
    solid_box(ctx, b, 1, 1, 0, 15, 1, 0, p.base_light);
    solid_box(ctx, b, 1, 1, 7, 14, 1, 7, p.base_light);
    solid_box(ctx, b, 5, 1, 0, 10, 1, 4, p.base_light);
    solid_box(ctx, b, 6, 2, 0, 9, 2, 3, p.base_gray);
    solid_box(ctx, b, 5, 3, 0, 10, 3, 4, p.base_light);
    ctx.place_block(b, p.lamp, 6, 2, 3);
    ctx.place_block(b, p.lamp, 9, 2, 3);
    if rooms.open(west, Direction::South) {
        water_box(ctx, b, 3, 1, 0, 4, 2, 0);
    }

    if rooms.open(west, Direction::North) {
        water_box(ctx, b, 3, 1, 7, 4, 2, 7);
    }

    if rooms.open(west, Direction::West) {
        water_box(ctx, b, 0, 1, 3, 0, 2, 4);
    }

    if rooms.open(east, Direction::South) {
        water_box(ctx, b, 11, 1, 0, 12, 2, 0);
    }

    if rooms.open(east, Direction::North) {
        water_box(ctx, b, 11, 1, 7, 12, 2, 7);
    }

    if rooms.open(east, Direction::East) {
        water_box(ctx, b, 15, 1, 3, 15, 2, 4);
    }
}

fn double_xy_room(ctx: &mut PieceCtx, b: &PieceBase, rooms: &Rooms, def: usize) {
    let p = palette(ctx);
    let east = rooms.conn(def, Direction::East).unwrap();
    let west = def;
    let west_up = rooms.conn(west, Direction::Up).unwrap();
    let east_up = rooms.conn(east, Direction::Up).unwrap();
    if rooms.defs[def].index / 25 > 0 {
        default_floor(ctx, b, 8, 0, rooms.open(east, Direction::Down));
        default_floor(ctx, b, 0, 0, rooms.open(west, Direction::Down));
    }

    if rooms.conn(west_up, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 8, 1, 7, 8, 6, p.base_gray);
    }

    if rooms.conn(east_up, Direction::Up).is_none() {
        fill_only(ctx, b, 8, 8, 1, 14, 8, 6, p.base_gray);
    }

    for y in 1..=7 {
        let block = if y == 2 || y == 6 {
            p.base_gray
        } else {
            p.base_light
        };

        solid_box(ctx, b, 0, y, 0, 0, y, 7, block);
        solid_box(ctx, b, 15, y, 0, 15, y, 7, block);
        solid_box(ctx, b, 1, y, 0, 15, y, 0, block);
        solid_box(ctx, b, 1, y, 7, 14, y, 7, block);
    }

    solid_box(ctx, b, 2, 1, 3, 2, 7, 4, p.base_light);
    solid_box(ctx, b, 3, 1, 2, 4, 7, 2, p.base_light);
    solid_box(ctx, b, 3, 1, 5, 4, 7, 5, p.base_light);
    solid_box(ctx, b, 13, 1, 3, 13, 7, 4, p.base_light);
    solid_box(ctx, b, 11, 1, 2, 12, 7, 2, p.base_light);
    solid_box(ctx, b, 11, 1, 5, 12, 7, 5, p.base_light);
    solid_box(ctx, b, 5, 1, 3, 5, 3, 4, p.base_light);
    solid_box(ctx, b, 10, 1, 3, 10, 3, 4, p.base_light);
    solid_box(ctx, b, 5, 7, 2, 10, 7, 5, p.base_light);
    solid_box(ctx, b, 5, 5, 2, 5, 7, 2, p.base_light);
    solid_box(ctx, b, 10, 5, 2, 10, 7, 2, p.base_light);
    solid_box(ctx, b, 5, 5, 5, 5, 7, 5, p.base_light);
    solid_box(ctx, b, 10, 5, 5, 10, 7, 5, p.base_light);
    ctx.place_block(b, p.base_light, 6, 6, 2);
    ctx.place_block(b, p.base_light, 9, 6, 2);
    ctx.place_block(b, p.base_light, 6, 6, 5);
    ctx.place_block(b, p.base_light, 9, 6, 5);
    solid_box(ctx, b, 5, 4, 3, 6, 4, 4, p.base_light);
    solid_box(ctx, b, 9, 4, 3, 10, 4, 4, p.base_light);
    ctx.place_block(b, p.lamp, 5, 4, 2);
    ctx.place_block(b, p.lamp, 5, 4, 5);
    ctx.place_block(b, p.lamp, 10, 4, 2);
    ctx.place_block(b, p.lamp, 10, 4, 5);
    if rooms.open(west, Direction::South) {
        water_box(ctx, b, 3, 1, 0, 4, 2, 0);
    }

    if rooms.open(west, Direction::North) {
        water_box(ctx, b, 3, 1, 7, 4, 2, 7);
    }

    if rooms.open(west, Direction::West) {
        water_box(ctx, b, 0, 1, 3, 0, 2, 4);
    }

    if rooms.open(east, Direction::South) {
        water_box(ctx, b, 11, 1, 0, 12, 2, 0);
    }

    if rooms.open(east, Direction::North) {
        water_box(ctx, b, 11, 1, 7, 12, 2, 7);
    }

    if rooms.open(east, Direction::East) {
        water_box(ctx, b, 15, 1, 3, 15, 2, 4);
    }

    if rooms.open(west_up, Direction::South) {
        water_box(ctx, b, 3, 5, 0, 4, 6, 0);
    }

    if rooms.open(west_up, Direction::North) {
        water_box(ctx, b, 3, 5, 7, 4, 6, 7);
    }

    if rooms.open(west_up, Direction::West) {
        water_box(ctx, b, 0, 5, 3, 0, 6, 4);
    }

    if rooms.open(east_up, Direction::South) {
        water_box(ctx, b, 11, 5, 0, 12, 6, 0);
    }

    if rooms.open(east_up, Direction::North) {
        water_box(ctx, b, 11, 5, 7, 12, 6, 7);
    }

    if rooms.open(east_up, Direction::East) {
        water_box(ctx, b, 15, 5, 3, 15, 6, 4);
    }
}

fn double_y_room(ctx: &mut PieceCtx, b: &PieceBase, rooms: &Rooms, def: usize) {
    let p = palette(ctx);
    if rooms.defs[def].index / 25 > 0 {
        default_floor(ctx, b, 0, 0, rooms.open(def, Direction::Down));
    }

    let above = rooms.conn(def, Direction::Up).unwrap();
    if rooms.conn(above, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 8, 1, 6, 8, 6, p.base_gray);
    }

    solid_box(ctx, b, 0, 4, 0, 0, 4, 7, p.base_light);
    solid_box(ctx, b, 7, 4, 0, 7, 4, 7, p.base_light);
    solid_box(ctx, b, 1, 4, 0, 6, 4, 0, p.base_light);
    solid_box(ctx, b, 1, 4, 7, 6, 4, 7, p.base_light);
    solid_box(ctx, b, 2, 4, 1, 2, 4, 2, p.base_light);
    solid_box(ctx, b, 1, 4, 2, 1, 4, 2, p.base_light);
    solid_box(ctx, b, 5, 4, 1, 5, 4, 2, p.base_light);
    solid_box(ctx, b, 6, 4, 2, 6, 4, 2, p.base_light);
    solid_box(ctx, b, 2, 4, 5, 2, 4, 6, p.base_light);
    solid_box(ctx, b, 1, 4, 5, 1, 4, 5, p.base_light);
    solid_box(ctx, b, 5, 4, 5, 5, 4, 6, p.base_light);
    solid_box(ctx, b, 6, 4, 5, 6, 4, 5, p.base_light);
    let mut definition = def;

    for y in (1..=5).step_by(4) {
        let z = 0;
        if rooms.open(definition, Direction::South) {
            solid_box(ctx, b, 2, y, z, 2, y + 2, z, p.base_light);
            solid_box(ctx, b, 5, y, z, 5, y + 2, z, p.base_light);
            solid_box(ctx, b, 3, y + 2, z, 4, y + 2, z, p.base_light);
        } else {
            solid_box(ctx, b, 0, y, z, 7, y + 2, z, p.base_light);
            solid_box(ctx, b, 0, y + 1, z, 7, y + 1, z, p.base_gray);
        }

        let z = 7;
        if rooms.open(definition, Direction::North) {
            solid_box(ctx, b, 2, y, z, 2, y + 2, z, p.base_light);
            solid_box(ctx, b, 5, y, z, 5, y + 2, z, p.base_light);
            solid_box(ctx, b, 3, y + 2, z, 4, y + 2, z, p.base_light);
        } else {
            solid_box(ctx, b, 0, y, z, 7, y + 2, z, p.base_light);
            solid_box(ctx, b, 0, y + 1, z, 7, y + 1, z, p.base_gray);
        }

        let x = 0;
        if rooms.open(definition, Direction::West) {
            solid_box(ctx, b, x, y, 2, x, y + 2, 2, p.base_light);
            solid_box(ctx, b, x, y, 5, x, y + 2, 5, p.base_light);
            solid_box(ctx, b, x, y + 2, 3, x, y + 2, 4, p.base_light);
        } else {
            solid_box(ctx, b, x, y, 0, x, y + 2, 7, p.base_light);
            solid_box(ctx, b, x, y + 1, 0, x, y + 1, 7, p.base_gray);
        }

        let x = 7;
        if rooms.open(definition, Direction::East) {
            solid_box(ctx, b, x, y, 2, x, y + 2, 2, p.base_light);
            solid_box(ctx, b, x, y, 5, x, y + 2, 5, p.base_light);
            solid_box(ctx, b, x, y + 2, 3, x, y + 2, 4, p.base_light);
        } else {
            solid_box(ctx, b, x, y, 0, x, y + 2, 7, p.base_light);
            solid_box(ctx, b, x, y + 1, 0, x, y + 1, 7, p.base_gray);
        }

        definition = above;
    }
}

fn double_yz_room(ctx: &mut PieceCtx, b: &PieceBase, rooms: &Rooms, def: usize) {
    let p = palette(ctx);
    let north = rooms.conn(def, Direction::North).unwrap();
    let south = def;
    let north_up = rooms.conn(north, Direction::Up).unwrap();
    let south_up = rooms.conn(south, Direction::Up).unwrap();
    if rooms.defs[def].index / 25 > 0 {
        default_floor(ctx, b, 0, 8, rooms.open(north, Direction::Down));
        default_floor(ctx, b, 0, 0, rooms.open(south, Direction::Down));
    }

    if rooms.conn(south_up, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 8, 1, 6, 8, 7, p.base_gray);
    }

    if rooms.conn(north_up, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 8, 8, 6, 8, 14, p.base_gray);
    }

    for y in 1..=7 {
        let block = if y == 2 || y == 6 {
            p.base_gray
        } else {
            p.base_light
        };

        solid_box(ctx, b, 0, y, 0, 0, y, 15, block);
        solid_box(ctx, b, 7, y, 0, 7, y, 15, block);
        solid_box(ctx, b, 1, y, 0, 6, y, 0, block);
        solid_box(ctx, b, 1, y, 15, 6, y, 15, block);
    }

    for y in 1..=7 {
        let block = if y == 2 || y == 6 { p.lamp } else { p.base_black };

        solid_box(ctx, b, 3, y, 7, 4, y, 8, block);
    }

    if rooms.open(south, Direction::South) {
        water_box(ctx, b, 3, 1, 0, 4, 2, 0);
    }

    if rooms.open(south, Direction::East) {
        water_box(ctx, b, 7, 1, 3, 7, 2, 4);
    }

    if rooms.open(south, Direction::West) {
        water_box(ctx, b, 0, 1, 3, 0, 2, 4);
    }

    if rooms.open(north, Direction::North) {
        water_box(ctx, b, 3, 1, 15, 4, 2, 15);
    }

    if rooms.open(north, Direction::West) {
        water_box(ctx, b, 0, 1, 11, 0, 2, 12);
    }

    if rooms.open(north, Direction::East) {
        water_box(ctx, b, 7, 1, 11, 7, 2, 12);
    }

    if rooms.open(south_up, Direction::South) {
        water_box(ctx, b, 3, 5, 0, 4, 6, 0);
    }

    if rooms.open(south_up, Direction::East) {
        water_box(ctx, b, 7, 5, 3, 7, 6, 4);
        solid_box(ctx, b, 5, 4, 2, 6, 4, 5, p.base_light);
        solid_box(ctx, b, 6, 1, 2, 6, 3, 2, p.base_light);
        solid_box(ctx, b, 6, 1, 5, 6, 3, 5, p.base_light);
    }

    if rooms.open(south_up, Direction::West) {
        water_box(ctx, b, 0, 5, 3, 0, 6, 4);
        solid_box(ctx, b, 1, 4, 2, 2, 4, 5, p.base_light);
        solid_box(ctx, b, 1, 1, 2, 1, 3, 2, p.base_light);
        solid_box(ctx, b, 1, 1, 5, 1, 3, 5, p.base_light);
    }

    if rooms.open(north_up, Direction::North) {
        water_box(ctx, b, 3, 5, 15, 4, 6, 15);
    }

    if rooms.open(north_up, Direction::West) {
        water_box(ctx, b, 0, 5, 11, 0, 6, 12);
        solid_box(ctx, b, 1, 4, 10, 2, 4, 13, p.base_light);
        solid_box(ctx, b, 1, 1, 10, 1, 3, 10, p.base_light);
        solid_box(ctx, b, 1, 1, 13, 1, 3, 13, p.base_light);
    }

    if rooms.open(north_up, Direction::East) {
        water_box(ctx, b, 7, 5, 11, 7, 6, 12);
        solid_box(ctx, b, 5, 4, 10, 6, 4, 13, p.base_light);
        solid_box(ctx, b, 6, 1, 10, 6, 3, 10, p.base_light);
        solid_box(ctx, b, 6, 1, 13, 6, 3, 13, p.base_light);
    }
}

fn double_z_room(ctx: &mut PieceCtx, b: &PieceBase, rooms: &Rooms, def: usize) {
    let p = palette(ctx);
    let north = rooms.conn(def, Direction::North).unwrap();
    let south = def;
    if rooms.defs[def].index / 25 > 0 {
        default_floor(ctx, b, 0, 8, rooms.open(north, Direction::Down));
        default_floor(ctx, b, 0, 0, rooms.open(south, Direction::Down));
    }

    if rooms.conn(south, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 4, 1, 6, 4, 7, p.base_gray);
    }

    if rooms.conn(north, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 4, 8, 6, 4, 14, p.base_gray);
    }

    solid_box(ctx, b, 0, 3, 0, 0, 3, 15, p.base_light);
    solid_box(ctx, b, 7, 3, 0, 7, 3, 15, p.base_light);
    solid_box(ctx, b, 1, 3, 0, 7, 3, 0, p.base_light);
    solid_box(ctx, b, 1, 3, 15, 6, 3, 15, p.base_light);
    solid_box(ctx, b, 0, 2, 0, 0, 2, 15, p.base_gray);
    solid_box(ctx, b, 7, 2, 0, 7, 2, 15, p.base_gray);
    solid_box(ctx, b, 1, 2, 0, 7, 2, 0, p.base_gray);
    solid_box(ctx, b, 1, 2, 15, 6, 2, 15, p.base_gray);
    solid_box(ctx, b, 0, 1, 0, 0, 1, 15, p.base_light);
    solid_box(ctx, b, 7, 1, 0, 7, 1, 15, p.base_light);
    solid_box(ctx, b, 1, 1, 0, 7, 1, 0, p.base_light);
    solid_box(ctx, b, 1, 1, 15, 6, 1, 15, p.base_light);
    solid_box(ctx, b, 1, 1, 1, 1, 1, 2, p.base_light);
    solid_box(ctx, b, 6, 1, 1, 6, 1, 2, p.base_light);
    solid_box(ctx, b, 1, 3, 1, 1, 3, 2, p.base_light);
    solid_box(ctx, b, 6, 3, 1, 6, 3, 2, p.base_light);
    solid_box(ctx, b, 1, 1, 13, 1, 1, 14, p.base_light);
    solid_box(ctx, b, 6, 1, 13, 6, 1, 14, p.base_light);
    solid_box(ctx, b, 1, 3, 13, 1, 3, 14, p.base_light);
    solid_box(ctx, b, 6, 3, 13, 6, 3, 14, p.base_light);
    solid_box(ctx, b, 2, 1, 6, 2, 3, 6, p.base_light);
    solid_box(ctx, b, 5, 1, 6, 5, 3, 6, p.base_light);
    solid_box(ctx, b, 2, 1, 9, 2, 3, 9, p.base_light);
    solid_box(ctx, b, 5, 1, 9, 5, 3, 9, p.base_light);
    solid_box(ctx, b, 3, 2, 6, 4, 2, 6, p.base_light);
    solid_box(ctx, b, 3, 2, 9, 4, 2, 9, p.base_light);
    solid_box(ctx, b, 2, 2, 7, 2, 2, 8, p.base_light);
    solid_box(ctx, b, 5, 2, 7, 5, 2, 8, p.base_light);
    ctx.place_block(b, p.lamp, 2, 2, 5);
    ctx.place_block(b, p.lamp, 5, 2, 5);
    ctx.place_block(b, p.lamp, 2, 2, 10);
    ctx.place_block(b, p.lamp, 5, 2, 10);
    ctx.place_block(b, p.base_light, 2, 3, 5);
    ctx.place_block(b, p.base_light, 5, 3, 5);
    ctx.place_block(b, p.base_light, 2, 3, 10);
    ctx.place_block(b, p.base_light, 5, 3, 10);
    if rooms.open(south, Direction::South) {
        water_box(ctx, b, 3, 1, 0, 4, 2, 0);
    }

    if rooms.open(south, Direction::East) {
        water_box(ctx, b, 7, 1, 3, 7, 2, 4);
    }

    if rooms.open(south, Direction::West) {
        water_box(ctx, b, 0, 1, 3, 0, 2, 4);
    }

    if rooms.open(north, Direction::North) {
        water_box(ctx, b, 3, 1, 15, 4, 2, 15);
    }

    if rooms.open(north, Direction::West) {
        water_box(ctx, b, 0, 1, 11, 0, 2, 12);
    }

    if rooms.open(north, Direction::East) {
        water_box(ctx, b, 7, 1, 11, 7, 2, 12);
    }
}

fn entry_room(ctx: &mut PieceCtx, b: &PieceBase, rooms: &Rooms, def: usize) {
    let p = palette(ctx);
    solid_box(ctx, b, 0, 3, 0, 2, 3, 7, p.base_light);
    solid_box(ctx, b, 5, 3, 0, 7, 3, 7, p.base_light);
    solid_box(ctx, b, 0, 2, 0, 1, 2, 7, p.base_light);
    solid_box(ctx, b, 6, 2, 0, 7, 2, 7, p.base_light);
    solid_box(ctx, b, 0, 1, 0, 0, 1, 7, p.base_light);
    solid_box(ctx, b, 7, 1, 0, 7, 1, 7, p.base_light);
    solid_box(ctx, b, 0, 1, 7, 7, 3, 7, p.base_light);
    solid_box(ctx, b, 1, 1, 0, 2, 3, 0, p.base_light);
    solid_box(ctx, b, 5, 1, 0, 6, 3, 0, p.base_light);
    if rooms.open(def, Direction::North) {
        water_box(ctx, b, 3, 1, 7, 4, 2, 7);
    }

    if rooms.open(def, Direction::West) {
        water_box(ctx, b, 0, 1, 3, 1, 2, 4);
    }

    if rooms.open(def, Direction::East) {
        water_box(ctx, b, 6, 1, 3, 7, 2, 4);
    }
}

fn penthouse(ctx: &mut PieceCtx, b: &PieceBase) {
    let p = palette(ctx);
    solid_box(ctx, b, 2, -1, 2, 11, -1, 11, p.base_light);
    solid_box(ctx, b, 0, -1, 0, 1, -1, 11, p.base_gray);
    solid_box(ctx, b, 12, -1, 0, 13, -1, 11, p.base_gray);
    solid_box(ctx, b, 2, -1, 0, 11, -1, 1, p.base_gray);
    solid_box(ctx, b, 2, -1, 12, 11, -1, 13, p.base_gray);
    solid_box(ctx, b, 0, 0, 0, 0, 0, 13, p.base_light);
    solid_box(ctx, b, 13, 0, 0, 13, 0, 13, p.base_light);
    solid_box(ctx, b, 1, 0, 0, 12, 0, 0, p.base_light);
    solid_box(ctx, b, 1, 0, 13, 12, 0, 13, p.base_light);

    for i in (2..=11).step_by(3) {
        ctx.place_block(b, p.lamp, 0, 0, i);
        ctx.place_block(b, p.lamp, 13, 0, i);
        ctx.place_block(b, p.lamp, i, 0, 0);
    }

    solid_box(ctx, b, 2, 0, 3, 4, 0, 9, p.base_light);
    solid_box(ctx, b, 9, 0, 3, 11, 0, 9, p.base_light);
    solid_box(ctx, b, 4, 0, 9, 9, 0, 11, p.base_light);
    ctx.place_block(b, p.base_light, 5, 0, 8);
    ctx.place_block(b, p.base_light, 8, 0, 8);
    ctx.place_block(b, p.base_light, 10, 0, 10);
    ctx.place_block(b, p.base_light, 3, 0, 10);
    solid_box(ctx, b, 3, 0, 3, 3, 0, 7, p.base_black);
    solid_box(ctx, b, 10, 0, 3, 10, 0, 7, p.base_black);
    solid_box(ctx, b, 6, 0, 10, 7, 0, 10, p.base_black);
    for x in [3, 10] {
        for z in (2..=8).step_by(3) {
            solid_box(ctx, b, x, 0, z, x, 2, z, p.base_light);
        }
    }

    solid_box(ctx, b, 5, 0, 10, 5, 2, 10, p.base_light);
    solid_box(ctx, b, 8, 0, 10, 8, 2, 10, p.base_light);
    solid_box(ctx, b, 6, -1, 7, 7, -1, 8, p.base_black);
    water_box(ctx, b, 6, -1, 3, 7, -1, 4);
    spawn_elder(ctx, b, 6, 1, 6);
}

fn simple_room(
    ctx: &mut PieceCtx,
    b: &PieceBase,
    random: &mut dyn RandomSource,
    rooms: &Rooms,
    def: usize,
    design: i32,
) {
    let p = palette(ctx);
    if rooms.defs[def].index / 25 > 0 {
        default_floor(ctx, b, 0, 0, rooms.open(def, Direction::Down));
    }

    if rooms.conn(def, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 4, 1, 6, 4, 6, p.base_gray);
    }

    let center_pillar = design != 0
        && random.next_bool()
        && !rooms.open(def, Direction::Down)
        && !rooms.open(def, Direction::Up)
        && rooms.count_openings(def) > 1;
    if design == 0 {
        solid_box(ctx, b, 0, 1, 0, 2, 1, 2, p.base_light);
        solid_box(ctx, b, 0, 3, 0, 2, 3, 2, p.base_light);
        solid_box(ctx, b, 0, 2, 0, 0, 2, 2, p.base_gray);
        solid_box(ctx, b, 1, 2, 0, 2, 2, 0, p.base_gray);
        ctx.place_block(b, p.lamp, 1, 2, 1);
        solid_box(ctx, b, 5, 1, 0, 7, 1, 2, p.base_light);
        solid_box(ctx, b, 5, 3, 0, 7, 3, 2, p.base_light);
        solid_box(ctx, b, 7, 2, 0, 7, 2, 2, p.base_gray);
        solid_box(ctx, b, 5, 2, 0, 6, 2, 0, p.base_gray);
        ctx.place_block(b, p.lamp, 6, 2, 1);
        solid_box(ctx, b, 0, 1, 5, 2, 1, 7, p.base_light);
        solid_box(ctx, b, 0, 3, 5, 2, 3, 7, p.base_light);
        solid_box(ctx, b, 0, 2, 5, 0, 2, 7, p.base_gray);
        solid_box(ctx, b, 1, 2, 7, 2, 2, 7, p.base_gray);
        ctx.place_block(b, p.lamp, 1, 2, 6);
        solid_box(ctx, b, 5, 1, 5, 7, 1, 7, p.base_light);
        solid_box(ctx, b, 5, 3, 5, 7, 3, 7, p.base_light);
        solid_box(ctx, b, 7, 2, 5, 7, 2, 7, p.base_gray);
        solid_box(ctx, b, 5, 2, 7, 6, 2, 7, p.base_gray);
        ctx.place_block(b, p.lamp, 6, 2, 6);
        if rooms.open(def, Direction::South) {
            solid_box(ctx, b, 3, 3, 0, 4, 3, 0, p.base_light);
        } else {
            solid_box(ctx, b, 3, 3, 0, 4, 3, 1, p.base_light);
            solid_box(ctx, b, 3, 2, 0, 4, 2, 0, p.base_gray);
            solid_box(ctx, b, 3, 1, 0, 4, 1, 1, p.base_light);
        }

        if rooms.open(def, Direction::North) {
            solid_box(ctx, b, 3, 3, 7, 4, 3, 7, p.base_light);
        } else {
            solid_box(ctx, b, 3, 3, 6, 4, 3, 7, p.base_light);
            solid_box(ctx, b, 3, 2, 7, 4, 2, 7, p.base_gray);
            solid_box(ctx, b, 3, 1, 6, 4, 1, 7, p.base_light);
        }

        if rooms.open(def, Direction::West) {
            solid_box(ctx, b, 0, 3, 3, 0, 3, 4, p.base_light);
        } else {
            solid_box(ctx, b, 0, 3, 3, 1, 3, 4, p.base_light);
            solid_box(ctx, b, 0, 2, 3, 0, 2, 4, p.base_gray);
            solid_box(ctx, b, 0, 1, 3, 1, 1, 4, p.base_light);
        }

        if rooms.open(def, Direction::East) {
            solid_box(ctx, b, 7, 3, 3, 7, 3, 4, p.base_light);
        } else {
            solid_box(ctx, b, 6, 3, 3, 7, 3, 4, p.base_light);
            solid_box(ctx, b, 7, 2, 3, 7, 2, 4, p.base_gray);
            solid_box(ctx, b, 6, 1, 3, 7, 1, 4, p.base_light);
        }
    } else if design == 1 {
        solid_box(ctx, b, 2, 1, 2, 2, 3, 2, p.base_light);
        solid_box(ctx, b, 2, 1, 5, 2, 3, 5, p.base_light);
        solid_box(ctx, b, 5, 1, 5, 5, 3, 5, p.base_light);
        solid_box(ctx, b, 5, 1, 2, 5, 3, 2, p.base_light);
        ctx.place_block(b, p.lamp, 2, 2, 2);
        ctx.place_block(b, p.lamp, 2, 2, 5);
        ctx.place_block(b, p.lamp, 5, 2, 5);
        ctx.place_block(b, p.lamp, 5, 2, 2);
        solid_box(ctx, b, 0, 1, 0, 1, 3, 0, p.base_light);
        solid_box(ctx, b, 0, 1, 1, 0, 3, 1, p.base_light);
        solid_box(ctx, b, 0, 1, 7, 1, 3, 7, p.base_light);
        solid_box(ctx, b, 0, 1, 6, 0, 3, 6, p.base_light);
        solid_box(ctx, b, 6, 1, 7, 7, 3, 7, p.base_light);
        solid_box(ctx, b, 7, 1, 6, 7, 3, 6, p.base_light);
        solid_box(ctx, b, 6, 1, 0, 7, 3, 0, p.base_light);
        solid_box(ctx, b, 7, 1, 1, 7, 3, 1, p.base_light);
        ctx.place_block(b, p.base_gray, 1, 2, 0);
        ctx.place_block(b, p.base_gray, 0, 2, 1);
        ctx.place_block(b, p.base_gray, 1, 2, 7);
        ctx.place_block(b, p.base_gray, 0, 2, 6);
        ctx.place_block(b, p.base_gray, 6, 2, 7);
        ctx.place_block(b, p.base_gray, 7, 2, 6);
        ctx.place_block(b, p.base_gray, 6, 2, 0);
        ctx.place_block(b, p.base_gray, 7, 2, 1);
        if !rooms.open(def, Direction::South) {
            solid_box(ctx, b, 1, 3, 0, 6, 3, 0, p.base_light);
            solid_box(ctx, b, 1, 2, 0, 6, 2, 0, p.base_gray);
            solid_box(ctx, b, 1, 1, 0, 6, 1, 0, p.base_light);
        }

        if !rooms.open(def, Direction::North) {
            solid_box(ctx, b, 1, 3, 7, 6, 3, 7, p.base_light);
            solid_box(ctx, b, 1, 2, 7, 6, 2, 7, p.base_gray);
            solid_box(ctx, b, 1, 1, 7, 6, 1, 7, p.base_light);
        }

        if !rooms.open(def, Direction::West) {
            solid_box(ctx, b, 0, 3, 1, 0, 3, 6, p.base_light);
            solid_box(ctx, b, 0, 2, 1, 0, 2, 6, p.base_gray);
            solid_box(ctx, b, 0, 1, 1, 0, 1, 6, p.base_light);
        }

        if !rooms.open(def, Direction::East) {
            solid_box(ctx, b, 7, 3, 1, 7, 3, 6, p.base_light);
            solid_box(ctx, b, 7, 2, 1, 7, 2, 6, p.base_gray);
            solid_box(ctx, b, 7, 1, 1, 7, 1, 6, p.base_light);
        }
    } else if design == 2 {
        solid_box(ctx, b, 0, 1, 0, 0, 1, 7, p.base_light);
        solid_box(ctx, b, 7, 1, 0, 7, 1, 7, p.base_light);
        solid_box(ctx, b, 1, 1, 0, 6, 1, 0, p.base_light);
        solid_box(ctx, b, 1, 1, 7, 6, 1, 7, p.base_light);
        solid_box(ctx, b, 0, 2, 0, 0, 2, 7, p.base_black);
        solid_box(ctx, b, 7, 2, 0, 7, 2, 7, p.base_black);
        solid_box(ctx, b, 1, 2, 0, 6, 2, 0, p.base_black);
        solid_box(ctx, b, 1, 2, 7, 6, 2, 7, p.base_black);
        solid_box(ctx, b, 0, 3, 0, 0, 3, 7, p.base_light);
        solid_box(ctx, b, 7, 3, 0, 7, 3, 7, p.base_light);
        solid_box(ctx, b, 1, 3, 0, 6, 3, 0, p.base_light);
        solid_box(ctx, b, 1, 3, 7, 6, 3, 7, p.base_light);
        solid_box(ctx, b, 0, 1, 3, 0, 2, 4, p.base_black);
        solid_box(ctx, b, 7, 1, 3, 7, 2, 4, p.base_black);
        solid_box(ctx, b, 3, 1, 0, 4, 2, 0, p.base_black);
        solid_box(ctx, b, 3, 1, 7, 4, 2, 7, p.base_black);
        if rooms.open(def, Direction::South) {
            water_box(ctx, b, 3, 1, 0, 4, 2, 0);
        }

        if rooms.open(def, Direction::North) {
            water_box(ctx, b, 3, 1, 7, 4, 2, 7);
        }

        if rooms.open(def, Direction::West) {
            water_box(ctx, b, 0, 1, 3, 0, 2, 4);
        }

        if rooms.open(def, Direction::East) {
            water_box(ctx, b, 7, 1, 3, 7, 2, 4);
        }
    }

    if center_pillar {
        solid_box(ctx, b, 3, 1, 3, 4, 1, 4, p.base_light);
        solid_box(ctx, b, 3, 2, 3, 4, 2, 4, p.base_gray);
        solid_box(ctx, b, 3, 3, 3, 4, 3, 4, p.base_light);
    }
}

fn simple_top_room(
    ctx: &mut PieceCtx,
    b: &PieceBase,
    random: &mut dyn RandomSource,
    rooms: &Rooms,
    def: usize,
) {
    let p = palette(ctx);
    if rooms.defs[def].index / 25 > 0 {
        default_floor(ctx, b, 0, 0, rooms.open(def, Direction::Down));
    }

    if rooms.conn(def, Direction::Up).is_none() {
        fill_only(ctx, b, 1, 4, 1, 6, 4, 6, p.base_gray);
    }

    for x in 1..=6 {
        for z in 1..=6 {
            if random.next_int_bounded(3) != 0 {
                let y0 = 2 + if random.next_int_bounded(4) == 0 { 0 } else { 1 };
                let wet_sponge = ctx.bs("wet_sponge").s();
                solid_box(ctx, b, x, y0, z, x, 3, z, wet_sponge);
            }
        }
    }

    solid_box(ctx, b, 0, 1, 0, 0, 1, 7, p.base_light);
    solid_box(ctx, b, 7, 1, 0, 7, 1, 7, p.base_light);
    solid_box(ctx, b, 1, 1, 0, 6, 1, 0, p.base_light);
    solid_box(ctx, b, 1, 1, 7, 6, 1, 7, p.base_light);
    solid_box(ctx, b, 0, 2, 0, 0, 2, 7, p.base_black);
    solid_box(ctx, b, 7, 2, 0, 7, 2, 7, p.base_black);
    solid_box(ctx, b, 1, 2, 0, 6, 2, 0, p.base_black);
    solid_box(ctx, b, 1, 2, 7, 6, 2, 7, p.base_black);
    solid_box(ctx, b, 0, 3, 0, 0, 3, 7, p.base_light);
    solid_box(ctx, b, 7, 3, 0, 7, 3, 7, p.base_light);
    solid_box(ctx, b, 1, 3, 0, 6, 3, 0, p.base_light);
    solid_box(ctx, b, 1, 3, 7, 6, 3, 7, p.base_light);
    solid_box(ctx, b, 0, 1, 3, 0, 2, 4, p.base_black);
    solid_box(ctx, b, 7, 1, 3, 7, 2, 4, p.base_black);
    solid_box(ctx, b, 3, 1, 0, 4, 2, 0, p.base_black);
    solid_box(ctx, b, 3, 1, 7, 4, 2, 7, p.base_black);
    if rooms.open(def, Direction::South) {
        water_box(ctx, b, 3, 1, 0, 4, 2, 0);
    }
}

fn wing_room(ctx: &mut PieceCtx, b: &PieceBase, design: i32) {
    let p = palette(ctx);
    if design == 0 {
        for i in 0..4 {
            solid_box(ctx, b, 10 - i, 3 - i, 20 - i, 12 + i, 3 - i, 20, p.base_light);
        }

        solid_box(ctx, b, 7, 0, 6, 15, 0, 16, p.base_light);
        solid_box(ctx, b, 6, 0, 6, 6, 3, 20, p.base_light);
        solid_box(ctx, b, 16, 0, 6, 16, 3, 20, p.base_light);
        solid_box(ctx, b, 7, 1, 7, 7, 1, 20, p.base_light);
        solid_box(ctx, b, 15, 1, 7, 15, 1, 20, p.base_light);
        solid_box(ctx, b, 7, 1, 6, 9, 3, 6, p.base_light);
        solid_box(ctx, b, 13, 1, 6, 15, 3, 6, p.base_light);
        solid_box(ctx, b, 8, 1, 7, 9, 1, 7, p.base_light);
        solid_box(ctx, b, 13, 1, 7, 14, 1, 7, p.base_light);
        solid_box(ctx, b, 9, 0, 5, 13, 0, 5, p.base_light);
        solid_box(ctx, b, 10, 0, 7, 12, 0, 7, p.base_black);
        solid_box(ctx, b, 8, 0, 10, 8, 0, 12, p.base_black);
        solid_box(ctx, b, 14, 0, 10, 14, 0, 12, p.base_black);

        for z in (7..=18).rev().step_by(3) {
            ctx.place_block(b, p.lamp, 6, 3, z);
            ctx.place_block(b, p.lamp, 16, 3, z);
        }

        ctx.place_block(b, p.lamp, 10, 0, 10);
        ctx.place_block(b, p.lamp, 12, 0, 10);
        ctx.place_block(b, p.lamp, 10, 0, 12);
        ctx.place_block(b, p.lamp, 12, 0, 12);
        ctx.place_block(b, p.lamp, 8, 3, 6);
        ctx.place_block(b, p.lamp, 14, 3, 6);
        ctx.place_block(b, p.base_light, 4, 2, 4);
        ctx.place_block(b, p.lamp, 4, 1, 4);
        ctx.place_block(b, p.base_light, 4, 0, 4);
        ctx.place_block(b, p.base_light, 18, 2, 4);
        ctx.place_block(b, p.lamp, 18, 1, 4);
        ctx.place_block(b, p.base_light, 18, 0, 4);
        ctx.place_block(b, p.base_light, 4, 2, 18);
        ctx.place_block(b, p.lamp, 4, 1, 18);
        ctx.place_block(b, p.base_light, 4, 0, 18);
        ctx.place_block(b, p.base_light, 18, 2, 18);
        ctx.place_block(b, p.lamp, 18, 1, 18);
        ctx.place_block(b, p.base_light, 18, 0, 18);
        ctx.place_block(b, p.base_light, 9, 7, 20);
        ctx.place_block(b, p.base_light, 13, 7, 20);
        solid_box(ctx, b, 6, 0, 21, 7, 4, 21, p.base_light);
        solid_box(ctx, b, 15, 0, 21, 16, 4, 21, p.base_light);
        spawn_elder(ctx, b, 11, 2, 16);
    } else if design == 1 {
        solid_box(ctx, b, 9, 3, 18, 13, 3, 20, p.base_light);
        solid_box(ctx, b, 9, 0, 18, 9, 2, 18, p.base_light);
        solid_box(ctx, b, 13, 0, 18, 13, 2, 18, p.base_light);
        for x in [9, 13] {
            ctx.place_block(b, p.base_light, x, 6, 20);
            ctx.place_block(b, p.lamp, x, 5, 20);
            ctx.place_block(b, p.base_light, x, 4, 20);
        }

        solid_box(ctx, b, 7, 3, 7, 15, 3, 14, p.base_light);
        for x in [10, 12] {
            solid_box(ctx, b, x, 0, 10, x, 6, 10, p.base_light);
            solid_box(ctx, b, x, 0, 12, x, 6, 12, p.base_light);
            ctx.place_block(b, p.lamp, x, 0, 10);
            ctx.place_block(b, p.lamp, x, 0, 12);
            ctx.place_block(b, p.lamp, x, 4, 10);
            ctx.place_block(b, p.lamp, x, 4, 12);
        }

        for x in [8, 14] {
            solid_box(ctx, b, x, 0, 7, x, 2, 7, p.base_light);
            solid_box(ctx, b, x, 0, 14, x, 2, 14, p.base_light);
        }

        solid_box(ctx, b, 8, 3, 8, 8, 3, 13, p.base_black);
        solid_box(ctx, b, 14, 3, 8, 14, 3, 13, p.base_black);
        spawn_elder(ctx, b, 11, 5, 13);
    }
}
