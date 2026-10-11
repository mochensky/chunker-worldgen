//! `WoodlandMansionStructure` + `WoodlandMansionPieces` (grid layout, template pieces, NBT).

use chunker_data::Identifier;
use chunker_nbt::Compound;

use super::geom::{BlockPos, Direction, Mirror, Rotation};
use super::start::{PieceBase, PiecesBuilder};
use super::template::zero_position_with_transform;
use super::util::shuffle;
use super::{GenContext, Stub};
use crate::piece_boilerplate;
use crate::random::RandomSource;

/// `WoodlandMansionPieces.WoodlandMansionPiece`.
pub struct MansionPiece {
    pub base: PieceBase,
    pub template_name: String,
    pub template_pos: BlockPos,
    pub rotation: Rotation,
    pub mirror: Mirror,
}

impl super::start::Piece for MansionPiece {
    piece_boilerplate!();
    fn type_id(&self) -> &'static str {
        "minecraft:wmp"
    }
    fn save_extra(&self, tag: &mut Compound) {
        tag.insert("TPX", self.template_pos.x);
        tag.insert("TPY", self.template_pos.y);
        tag.insert("TPZ", self.template_pos.z);
        tag.insert("Template", self.template_name.as_str());
        tag.insert("Rot", self.rotation.name());
        tag.insert("Mi", self.mirror.name());
    }
    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.move_by(dx, dy, dz);
        self.template_pos = self.template_pos.offset(dx, dy, dz);
    }
    fn post_process_ported(&self) -> bool {
        true
    }
    /// `TemplateStructurePiece.postProcess` with `WoodlandMansionPiece.makeSettings` / `handleDataMarker`.
    fn post_process(&self, ctx: &mut super::build::PieceCtx, random: &mut dyn RandomSource) {
        let b = &ctx.region.env.b;
        let mut settings = super::place::PlaceSettings {
            ignore_entities: true,
            rotation: self.rotation,
            mirror: self.mirror,
            processors: vec![std::sync::Arc::new(super::processor::Processor::BlockIgnore(
                vec![b.structure_block],
            ))],
            ..Default::default()
        };
        let id = Identifier::vanilla(&format!("woodland_mansion/{}", self.template_name));
        let rotation = self.rotation;
        super::templated::template_post_process(
            ctx,
            random,
            &id,
            self.template_pos,
            &mut settings,
            &mut |ctx, random, marker, pos| {
                let env = ctx.region.env;
                if marker.starts_with("Chest") {
                    let mut chest = env.blocks.block(env.b.chest).default_state;
                    let facing = match marker {
                        "ChestWest" => Some(Direction::West),
                        "ChestEast" => Some(Direction::East),
                        "ChestSouth" => Some(Direction::South),
                        "ChestNorth" => Some(Direction::North),
                        _ => None,
                    };
                    if let Some(f) = facing {
                        chest = env
                            .blocks
                            .with_property(chest, "facing", rotation.rotate(f).name())
                            .unwrap();
                    }
                    ctx.create_chest_at(random, pos, "minecraft:chests/woodland_mansion", Some(chest));
                    return;
                }
                let count = match marker {
                    "Mage" | "Warrior" => 1,
                    // `level.getRandom()` is the region's random.
                    "Group of Allays" => ctx.region.random.next_int_bounded(3) + 1,
                    _ => return,
                };
                let id = match marker {
                    "Mage" => "minecraft:evoker",
                    "Warrior" => "minecraft:vindicator",
                    _ => "minecraft:allay",
                };
                for _ in 0..count {
                    let tag = super::templated::structure_mob(id, pos);
                    crate::entity::spawn_mob(ctx.region, tag);
                    ctx.region.set(pos, chunker_data::blocks::BlockState::AIR, 2);
                }
            },
        );
    }
}

const CORRIDOR: i32 = 1;
const ROOM: i32 = 2;
const START_ROOM: i32 = 3;
const BLOCKED: i32 = 5;
const ROOM_1X1: i32 = 65536;
const ROOM_1X2: i32 = 131072;
const ROOM_2X2: i32 = 262144;
const ROOM_ORIGIN_FLAG: i32 = 1048576;
const ROOM_DOOR_FLAG: i32 = 2097152;
const ROOM_STAIRS_FLAG: i32 = 4194304;
const ROOM_CORRIDOR_FLAG: i32 = 8388608;
const ROOM_TYPE_MASK: i32 = 983040;
const ROOM_ID_MASK: i32 = 65535;

#[derive(Clone)]
struct SimpleGrid {
    grid: Vec<i32>,
    width: i32,
    height: i32,
    outside: i32,
}

impl SimpleGrid {
    fn new(width: i32, height: i32, outside: i32) -> Self {
        Self {
            grid: vec![0; (width * height) as usize],
            width,
            height,
            outside,
        }
    }
    fn set(&mut self, x: i32, y: i32, v: i32) {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            self.grid[(x * self.height + y) as usize] = v;
        }
    }
    fn set_box(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, v: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set(x, y, v);
            }
        }
    }
    fn get(&self, x: i32, y: i32) -> i32 {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            self.grid[(x * self.height + y) as usize]
        } else {
            self.outside
        }
    }
    fn setif(&mut self, x: i32, y: i32, if_value: i32, v: i32) {
        if self.get(x, y) == if_value {
            self.set(x, y, v);
        }
    }
    fn edges_to(&self, x: i32, y: i32, v: i32) -> bool {
        self.get(x - 1, y) == v
            || self.get(x + 1, y) == v
            || self.get(x, y + 1) == v
            || self.get(x, y - 1) == v
    }
}

fn is_house(g: &SimpleGrid, x: i32, y: i32) -> bool {
    matches!(g.get(x, y), 1..=4)
}

fn step(d: Direction) -> (i32, i32) {
    let (x, _, z) = d.step();
    (x, z)
}

struct MansionGrid {
    base: SimpleGrid,
    third: SimpleGrid,
    floors: [SimpleGrid; 3],
    entrance_x: i32,
    entrance_y: i32,
}

impl MansionGrid {
    fn new(random: &mut dyn RandomSource) -> Self {
        let (ex, ey) = (7, 4);
        let mut base = SimpleGrid::new(11, 11, BLOCKED);
        base.set_box(ex, ey, ex + 1, ey + 1, START_ROOM);
        base.set_box(ex - 1, ey, ex - 1, ey + 1, ROOM);
        base.set_box(ex + 2, ey - 2, ex + 3, ey + 3, BLOCKED);
        base.set_box(ex + 1, ey - 2, ex + 1, ey - 1, CORRIDOR);
        base.set_box(ex + 1, ey + 2, ex + 1, ey + 3, CORRIDOR);
        base.set(ex - 1, ey - 1, CORRIDOR);
        base.set(ex - 1, ey + 2, CORRIDOR);
        base.set_box(0, 0, 11, 1, BLOCKED);
        base.set_box(0, 9, 11, 11, BLOCKED);
        recursive_corridor(random, &mut base, ex, ey - 2, Direction::West, 6);
        recursive_corridor(random, &mut base, ex, ey + 3, Direction::West, 6);
        recursive_corridor(random, &mut base, ex - 2, ey - 1, Direction::West, 3);
        recursive_corridor(random, &mut base, ex - 2, ey + 2, Direction::West, 3);
        while clean_edges(&mut base) {}
        let mut floors = [
            SimpleGrid::new(11, 11, BLOCKED),
            SimpleGrid::new(11, 11, BLOCKED),
            SimpleGrid::new(11, 11, BLOCKED),
        ];
        identify_rooms(random, &base, &mut floors[0]);
        identify_rooms(random, &base, &mut floors[1]);
        floors[0].set_box(ex + 1, ey, ex + 1, ey + 1, ROOM_CORRIDOR_FLAG);
        floors[1].set_box(ex + 1, ey, ex + 1, ey + 1, ROOM_CORRIDOR_FLAG);
        let third = SimpleGrid::new(base.width, base.height, BLOCKED);
        let mut g = Self {
            base,
            third,
            floors,
            entrance_x: ex,
            entrance_y: ey,
        };
        g.setup_third_floor(random);
        let third = g.third.clone();
        identify_rooms(random, &third, &mut g.floors[2]);
        g
    }

    fn is_room_id(&self, x: i32, y: i32, floor: usize, room_id: i32) -> bool {
        (self.floors[floor].get(x, y) & ROOM_ID_MASK) == room_id
    }

    fn room_1x2_direction(&self, x: i32, y: i32, floor: usize, room_id: i32) -> Option<Direction> {
        Direction::HORIZONTAL.into_iter().find(|&d| {
            let (dx, dz) = step(d);
            self.is_room_id(x + dx, y + dz, floor, room_id)
        })
    }

    fn setup_third_floor(&mut self, random: &mut dyn RandomSource) {
        let mut potential = Vec::new();
        for y in 0..self.third.height {
            for x in 0..self.third.width {
                let data = self.floors[1].get(x, y);
                if data & ROOM_TYPE_MASK == ROOM_1X2 && data & ROOM_DOOR_FLAG == ROOM_DOOR_FLAG {
                    potential.push((x, y));
                }
            }
        }
        let (w, h) = (self.third.width, self.third.height);
        if potential.is_empty() {
            self.third.set_box(0, 0, w, h, BLOCKED);
            return;
        }
        let (rx, ry) = potential[random.next_int_bounded(potential.len() as i32) as usize];
        let data = self.floors[1].get(rx, ry);
        self.floors[1].set(rx, ry, data | ROOM_STAIRS_FLAG);
        let dir = self
            .room_1x2_direction(rx, ry, 1, data & ROOM_ID_MASK)
            .expect("1x2 room without partner");
        let (dx, dz) = step(dir);
        let (end_x, end_y) = (rx + dx, ry + dz);
        for y in 0..h {
            for x in 0..w {
                if !is_house(&self.base, x, y) {
                    self.third.set(x, y, BLOCKED);
                } else if x == rx && y == ry {
                    self.third.set(x, y, START_ROOM);
                } else if x == end_x && y == end_y {
                    self.third.set(x, y, START_ROOM);
                    self.floors[2].set(x, y, ROOM_CORRIDOR_FLAG);
                }
            }
        }
        let corridors: Vec<Direction> = Direction::HORIZONTAL
            .into_iter()
            .filter(|&d| {
                let (dx, dz) = step(d);
                self.third.get(end_x + dx, end_y + dz) == 0
            })
            .collect();
        if corridors.is_empty() {
            self.third.set_box(0, 0, w, h, BLOCKED);
            self.floors[1].set(rx, ry, data);
        } else {
            let cd = corridors[random.next_int_bounded(corridors.len() as i32) as usize];
            let (cx, cz) = step(cd);
            recursive_corridor(random, &mut self.third, end_x + cx, end_y + cz, cd, 4);
            while clean_edges(&mut self.third) {}
        }
    }
}

fn recursive_corridor(
    random: &mut dyn RandomSource,
    grid: &mut SimpleGrid,
    x: i32,
    y: i32,
    heading: Direction,
    depth: i32,
) {
    if depth <= 0 {
        return;
    }
    let (hx, hz) = step(heading);
    grid.set(x, y, CORRIDOR);
    grid.setif(x + hx, y + hz, 0, CORRIDOR);
    for _ in 0..8 {
        let next = Direction::from_2d(random.next_int_bounded(4));
        if next != heading.opposite() && (next != Direction::East || !random.next_bool()) {
            let (nx, ny) = (x + hx, y + hz);
            let (sx, sz) = step(next);
            if grid.get(nx + sx, ny + sz) == 0 && grid.get(nx + sx * 2, ny + sz * 2) == 0 {
                recursive_corridor(random, grid, x + hx + sx, y + hz + sz, next, depth - 1);
                break;
            }
        }
    }
    let (cw, ccw) = (step(heading.clockwise()), step(heading.counter_clockwise()));
    grid.setif(x + cw.0, y + cw.1, 0, ROOM);
    grid.setif(x + ccw.0, y + ccw.1, 0, ROOM);
    grid.setif(x + hx + cw.0, y + hz + cw.1, 0, ROOM);
    grid.setif(x + hx + ccw.0, y + hz + ccw.1, 0, ROOM);
    grid.setif(x + hx * 2, y + hz * 2, 0, ROOM);
    grid.setif(x + cw.0 * 2, y + cw.1 * 2, 0, ROOM);
    grid.setif(x + ccw.0 * 2, y + ccw.1 * 2, 0, ROOM);
}

fn clean_edges(grid: &mut SimpleGrid) -> bool {
    let mut touched = false;
    for y in 0..grid.height {
        for x in 0..grid.width {
            if grid.get(x, y) != 0 {
                continue;
            }
            let direct = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .filter(|(dx, dy)| is_house(grid, x + dx, y + dy))
                .count();
            if direct >= 3 {
                grid.set(x, y, ROOM);
                touched = true;
            } else if direct == 2 {
                let diagonal = [(1, 1), (-1, 1), (1, -1), (-1, -1)]
                    .iter()
                    .filter(|(dx, dy)| is_house(grid, x + dx, y + dy))
                    .count();
                if diagonal <= 1 {
                    grid.set(x, y, ROOM);
                    touched = true;
                }
            }
        }
    }
    touched
}

fn identify_rooms(random: &mut dyn RandomSource, from: &SimpleGrid, rooms: &mut SimpleGrid) {
    let mut positions = Vec::new();
    for y in 0..from.height {
        for x in 0..from.width {
            if from.get(x, y) == ROOM {
                positions.push((x, y));
            }
        }
    }
    shuffle(&mut positions, random);
    let mut room_id = 10;
    for (xx, y) in positions {
        if rooms.get(xx, y) != 0 {
            continue;
        }
        let (mut x0, mut x1, mut y0, mut y1) = (xx, xx, y, y);
        let mut kind = ROOM_1X1;
        let free = |rooms: &SimpleGrid, x: i32, y: i32| rooms.get(x, y) == 0 && from.get(x, y) == ROOM;
        if rooms.get(xx + 1, y) == 0
            && rooms.get(xx, y + 1) == 0
            && rooms.get(xx + 1, y + 1) == 0
            && from.get(xx + 1, y) == ROOM
            && from.get(xx, y + 1) == ROOM
            && from.get(xx + 1, y + 1) == ROOM
        {
            x1 = xx + 1;
            y1 = y + 1;
            kind = ROOM_2X2;
        } else if rooms.get(xx - 1, y) == 0
            && rooms.get(xx, y + 1) == 0
            && rooms.get(xx - 1, y + 1) == 0
            && from.get(xx - 1, y) == ROOM
            && from.get(xx, y + 1) == ROOM
            && from.get(xx - 1, y + 1) == ROOM
        {
            x0 = xx - 1;
            y1 = y + 1;
            kind = ROOM_2X2;
        } else if rooms.get(xx - 1, y) == 0
            && rooms.get(xx, y - 1) == 0
            && rooms.get(xx - 1, y - 1) == 0
            && from.get(xx - 1, y) == ROOM
            && from.get(xx, y - 1) == ROOM
            && from.get(xx - 1, y - 1) == ROOM
        {
            x0 = xx - 1;
            y0 = y - 1;
            kind = ROOM_2X2;
        } else if free(rooms, xx + 1, y) {
            x1 = xx + 1;
            kind = ROOM_1X2;
        } else if free(rooms, xx, y + 1) {
            y1 = y + 1;
            kind = ROOM_1X2;
        } else if free(rooms, xx - 1, y) {
            x0 = xx - 1;
            kind = ROOM_1X2;
        } else if free(rooms, xx, y - 1) {
            y0 = y - 1;
            kind = ROOM_1X2;
        }
        let mut door_x = if random.next_bool() { x0 } else { x1 };
        let mut door_y = if random.next_bool() { y0 } else { y1 };
        let mut door_flag = ROOM_DOOR_FLAG;
        if !from.edges_to(door_x, door_y, CORRIDOR) {
            door_x = if door_x == x0 { x1 } else { x0 };
            door_y = if door_y == y0 { y1 } else { y0 };
            if !from.edges_to(door_x, door_y, CORRIDOR) {
                door_y = if door_y == y0 { y1 } else { y0 };
                if !from.edges_to(door_x, door_y, CORRIDOR) {
                    door_x = if door_x == x0 { x1 } else { x0 };
                    door_y = if door_y == y0 { y1 } else { y0 };
                    if !from.edges_to(door_x, door_y, CORRIDOR) {
                        door_flag = 0;
                        door_x = x0;
                        door_y = y0;
                    }
                }
            }
        }
        for ry in y0..=y1 {
            for rx in x0..=x1 {
                if rx == door_x && ry == door_y {
                    rooms.set(rx, ry, ROOM_ORIGIN_FLAG | door_flag | kind | room_id);
                } else {
                    rooms.set(rx, ry, kind | room_id);
                }
            }
        }
        room_id += 1;
    }
}

/// `FloorRoomCollection` (0 = first floor, otherwise second/third).
#[derive(Clone, Copy)]
struct Rooms(usize);

impl Rooms {
    fn n(random: &mut dyn RandomSource, bound: i32) -> i32 {
        random.next_int_bounded(bound) + 1
    }
    fn get_1x1(self, r: &mut dyn RandomSource) -> String {
        if self.0 == 0 {
            format!("1x1_a{}", Self::n(r, 5))
        } else {
            format!("1x1_b{}", Self::n(r, 5))
        }
    }
    fn get_1x1_secret(self, r: &mut dyn RandomSource) -> String {
        format!("1x1_as{}", Self::n(r, 4))
    }
    fn get_1x2_side(self, r: &mut dyn RandomSource, stairs: bool) -> String {
        match (self.0, stairs) {
            (0, _) => format!("1x2_a{}", Self::n(r, 9)),
            (_, true) => "1x2_c_stairs".into(),
            _ => format!("1x2_c{}", Self::n(r, 4)),
        }
    }
    fn get_1x2_front(self, r: &mut dyn RandomSource, stairs: bool) -> String {
        match (self.0, stairs) {
            (0, _) => format!("1x2_b{}", Self::n(r, 5)),
            (_, true) => "1x2_d_stairs".into(),
            _ => format!("1x2_d{}", Self::n(r, 5)),
        }
    }
    fn get_1x2_secret(self, r: &mut dyn RandomSource) -> String {
        if self.0 == 0 {
            format!("1x2_s{}", Self::n(r, 2))
        } else {
            format!("1x2_se{}", Self::n(r, 1))
        }
    }
    fn get_2x2(self, r: &mut dyn RandomSource) -> String {
        if self.0 == 0 {
            format!("2x2_a{}", Self::n(r, 4))
        } else {
            format!("2x2_b{}", Self::n(r, 5))
        }
    }
}

struct Placer<'a, 'c> {
    ctx: &'a GenContext<'c>,
    random: &'a mut dyn RandomSource,
    pieces: Vec<MansionPiece>,
    start_x: i32,
    start_y: i32,
}

struct PlacementData {
    rotation: Rotation,
    position: BlockPos,
    wall_type: &'static str,
}

impl Placer<'_, '_> {
    fn add(&mut self, name: &str, pos: BlockPos, rotation: Rotation) {
        self.add_mirrored(name, pos, rotation, Mirror::None);
    }

    fn add_mirrored(&mut self, name: &str, pos: BlockPos, rotation: Rotation, mirror: Mirror) {
        let id = Identifier::vanilla(&format!("woodland_mansion/{name}"));
        let bbox = self
            .ctx
            .structures
            .templates
            .get(&id)
            .bounding_box(pos, rotation, BlockPos::ZERO, mirror);
        self.pieces.push(MansionPiece {
            base: PieceBase::new(0, bbox, Some(Direction::North)),
            template_name: name.to_owned(),
            template_pos: pos,
            rotation,
            mirror,
        });
    }

    fn grid_pos(&self, origin: BlockPos, rotation: Rotation, x: i32, y: i32, east_extra: i32) -> BlockPos {
        origin
            .relative(rotation.rotate(Direction::South), 8 + (y - self.start_y) * 8)
            .relative(
                rotation.rotate(Direction::East),
                east_extra + (x - self.start_x) * 8,
            )
    }

    fn create(&mut self, origin: BlockPos, rotation: Rotation, mansion: &MansionGrid) {
        let mut data = PlacementData {
            rotation,
            position: origin,
            wall_type: "wall_flat",
        };
        self.entrance(&mut data);
        let mut second = PlacementData {
            rotation: data.rotation,
            position: data.position.above_n(8),
            wall_type: "wall_window",
        };
        let base = &mansion.base;
        let third = &mansion.third;
        self.start_x = mansion.entrance_x + 1;
        self.start_y = mansion.entrance_y + 1;
        let (end_x, end_y) = (mansion.entrance_x + 1, mansion.entrance_y);
        self.traverse_outer_walls(
            &mut data,
            base,
            Direction::South,
            self.start_x,
            self.start_y,
            end_x,
            end_y,
        );
        self.traverse_outer_walls(
            &mut second,
            base,
            Direction::South,
            self.start_x,
            self.start_y,
            end_x,
            end_y,
        );
        let mut third_data = PlacementData {
            rotation: data.rotation,
            position: data.position.above_n(19),
            wall_type: "wall_window",
        };
        'outer: for y in 0..third.height {
            for x in (0..third.width).rev() {
                if is_house(third, x, y) {
                    third_data.position = third_data
                        .position
                        .relative(rotation.rotate(Direction::South), 8 + (y - self.start_y) * 8);
                    third_data.position = third_data
                        .position
                        .relative(rotation.rotate(Direction::East), (x - self.start_x) * 8);
                    self.traverse_wall_piece(&mut third_data);
                    self.traverse_outer_walls(&mut third_data, third, Direction::South, x, y, x, y);
                    break 'outer;
                }
            }
        }
        self.create_roof(origin.above_n(16), rotation, base, Some(third));
        self.create_roof(origin.above_n(27), rotation, third, None);

        for floor in 0..3usize {
            let floor_origin = origin.above_n(8 * floor as i32 + if floor == 2 { 3 } else { 0 });
            let rooms = &mansion.floors[floor];
            let grid = if floor == 2 { third } else { base };
            let south_piece = if floor == 0 {
                "carpet_south_1"
            } else {
                "carpet_south_2"
            };
            let west_piece = if floor == 0 {
                "carpet_west_1"
            } else {
                "carpet_west_2"
            };
            for y in 0..grid.height {
                for xx in 0..grid.width {
                    if grid.get(xx, y) != CORRIDOR {
                        continue;
                    }
                    let pos = self.grid_pos(floor_origin, rotation, xx, y, 0);
                    self.add("corridor_floor", pos, rotation);
                    if grid.get(xx, y - 1) == CORRIDOR
                        || (rooms.get(xx, y - 1) & ROOM_CORRIDOR_FLAG) == ROOM_CORRIDOR_FLAG
                    {
                        self.add(
                            "carpet_north",
                            pos.relative(rotation.rotate(Direction::East), 1).above(),
                            rotation,
                        );
                    }
                    if grid.get(xx + 1, y) == CORRIDOR
                        || (rooms.get(xx + 1, y) & ROOM_CORRIDOR_FLAG) == ROOM_CORRIDOR_FLAG
                    {
                        let p = pos
                            .relative(rotation.rotate(Direction::South), 1)
                            .relative(rotation.rotate(Direction::East), 5)
                            .above();
                        self.add("carpet_east", p, rotation);
                    }
                    if grid.get(xx, y + 1) == CORRIDOR
                        || (rooms.get(xx, y + 1) & ROOM_CORRIDOR_FLAG) == ROOM_CORRIDOR_FLAG
                    {
                        let p = pos
                            .relative(rotation.rotate(Direction::South), 5)
                            .relative(rotation.rotate(Direction::West), 1);
                        self.add(south_piece, p, rotation);
                    }
                    if grid.get(xx - 1, y) == CORRIDOR
                        || (rooms.get(xx - 1, y) & ROOM_CORRIDOR_FLAG) == ROOM_CORRIDOR_FLAG
                    {
                        let p = pos
                            .relative(rotation.rotate(Direction::West), 1)
                            .relative(rotation.rotate(Direction::North), 1);
                        self.add(west_piece, p, rotation);
                    }
                }
            }
            let wall_piece = if floor == 0 {
                "indoors_wall_1"
            } else {
                "indoors_wall_2"
            };
            let door_piece = if floor == 0 {
                "indoors_door_1"
            } else {
                "indoors_door_2"
            };
            for y in 0..grid.height {
                for xxx in 0..grid.width {
                    let mut third_start = floor == 2 && grid.get(xxx, y) == START_ROOM;
                    if grid.get(xxx, y) != ROOM && !third_start {
                        continue;
                    }
                    let room_data = rooms.get(xxx, y);
                    let room_type = room_data & ROOM_TYPE_MASK;
                    let room_id = room_data & ROOM_ID_MASK;
                    third_start = third_start && (room_data & ROOM_CORRIDOR_FLAG) == ROOM_CORRIDOR_FLAG;
                    let mut door_dirs = Vec::new();
                    if room_data & ROOM_DOOR_FLAG == ROOM_DOOR_FLAG {
                        for d in Direction::HORIZONTAL {
                            let (dx, dz) = step(d);
                            if grid.get(xxx + dx, y + dz) == CORRIDOR {
                                door_dirs.push(d);
                            }
                        }
                    }
                    let door_dir = if !door_dirs.is_empty() {
                        Some(door_dirs[self.random.next_int_bounded(door_dirs.len() as i32) as usize])
                    } else if room_data & ROOM_ORIGIN_FLAG == ROOM_ORIGIN_FLAG {
                        Some(Direction::Up)
                    } else {
                        None
                    };
                    let room_pos = self.grid_pos(floor_origin, rotation, xxx, y, -1);
                    if is_house(grid, xxx - 1, y) && !mansion.is_room_id(xxx - 1, y, floor, room_id) {
                        self.add(
                            if door_dir == Some(Direction::West) {
                                door_piece
                            } else {
                                wall_piece
                            },
                            room_pos,
                            rotation,
                        );
                    }
                    if grid.get(xxx + 1, y) == CORRIDOR && !third_start {
                        let p = room_pos.relative(rotation.rotate(Direction::East), 8);
                        self.add(
                            if door_dir == Some(Direction::East) {
                                door_piece
                            } else {
                                wall_piece
                            },
                            p,
                            rotation,
                        );
                    }
                    if is_house(grid, xxx, y + 1) && !mansion.is_room_id(xxx, y + 1, floor, room_id) {
                        let p = room_pos
                            .relative(rotation.rotate(Direction::South), 7)
                            .relative(rotation.rotate(Direction::East), 7);
                        let name = if door_dir == Some(Direction::South) {
                            door_piece
                        } else {
                            wall_piece
                        };
                        self.add(name, p, rotation.then(Rotation::Clockwise90));
                    }
                    if grid.get(xxx, y - 1) == CORRIDOR && !third_start {
                        let p = room_pos
                            .relative(rotation.rotate(Direction::North), 1)
                            .relative(rotation.rotate(Direction::East), 7);
                        let name = if door_dir == Some(Direction::North) {
                            door_piece
                        } else {
                            wall_piece
                        };
                        self.add(name, p, rotation.then(Rotation::Clockwise90));
                    }
                    let collection = Rooms(floor);
                    if room_type == ROOM_1X1 {
                        self.add_room_1x1(room_pos, rotation, door_dir, collection);
                    } else if room_type == ROOM_1X2 && door_dir.is_some() {
                        let room_dir = mansion.room_1x2_direction(xxx, y, floor, room_id);
                        let stairs = room_data & ROOM_STAIRS_FLAG == ROOM_STAIRS_FLAG;
                        self.add_room_1x2(
                            room_pos,
                            rotation,
                            room_dir,
                            door_dir.unwrap(),
                            collection,
                            stairs,
                        );
                    } else if room_type == ROOM_2X2
                        && let Some(door) = door_dir
                        && door != Direction::Up
                    {
                        let mut room_dir = door.clockwise();
                        let (dx, dz) = step(room_dir);
                        if !mansion.is_room_id(xxx + dx, y + dz, floor, room_id) {
                            room_dir = room_dir.opposite();
                        }
                        self.add_room_2x2(room_pos, rotation, room_dir, door, collection);
                    } else if room_type == ROOM_2X2 && door_dir == Some(Direction::Up) {
                        let p = room_pos.relative(rotation.rotate(Direction::East), 1);
                        self.add_mirrored("2x2_s1", p, rotation, Mirror::None);
                    }
                }
            }
        }
    }

    fn traverse_outer_walls(
        &mut self,
        data: &mut PlacementData,
        grid: &SimpleGrid,
        mut dir: Direction,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) {
        let (mut gx, mut gy) = (start_x, start_y);
        let start_dir = dir;
        loop {
            let (dx, dz) = step(dir);
            if !is_house(grid, gx + dx, gy + dz) {
                self.traverse_turn(data);
                dir = dir.clockwise();
                if gx != end_x || gy != end_y || start_dir != dir {
                    self.traverse_wall_piece(data);
                }
            } else {
                let (cx, cz) = step(dir.counter_clockwise());
                if is_house(grid, gx + dx + cx, gy + dz + cz) {
                    self.traverse_inner_turn(data);
                    gx += dx;
                    gy += dz;
                    dir = dir.counter_clockwise();
                } else {
                    gx += dx;
                    gy += dz;
                    if gx != end_x || gy != end_y || start_dir != dir {
                        self.traverse_wall_piece(data);
                    }
                }
            }
            if !(gx != end_x || gy != end_y || start_dir != dir) {
                break;
            }
        }
    }

    fn create_roof(
        &mut self,
        roof_origin: BlockPos,
        rotation: Rotation,
        grid: &SimpleGrid,
        above: Option<&SimpleGrid>,
    ) {
        let r = |d: Direction| rotation.rotate(d);
        for y in 0..grid.height {
            for x in 0..grid.width {
                let position = self.grid_pos(roof_origin, rotation, x, y, 0);
                let is_above = above.is_some_and(|a| is_house(a, x, y));
                if is_house(grid, x, y) && !is_above {
                    self.add("roof", position.above_n(3), rotation);
                    if !is_house(grid, x + 1, y) {
                        self.add("roof_front", position.relative(r(Direction::East), 6), rotation);
                    }
                    if !is_house(grid, x - 1, y) {
                        let p = position
                            .relative(r(Direction::East), 0)
                            .relative(r(Direction::South), 7);
                        self.add("roof_front", p, rotation.then(Rotation::Clockwise180));
                    }
                    if !is_house(grid, x, y - 1) {
                        self.add(
                            "roof_front",
                            position.relative(r(Direction::West), 1),
                            rotation.then(Rotation::CounterClockwise90),
                        );
                    }
                    if !is_house(grid, x, y + 1) {
                        let p = position
                            .relative(r(Direction::East), 6)
                            .relative(r(Direction::South), 6);
                        self.add("roof_front", p, rotation.then(Rotation::Clockwise90));
                    }
                }
            }
        }
        if let Some(above) = above {
            for y in 0..grid.height {
                for xx in 0..grid.width {
                    let v = self.grid_pos(roof_origin, rotation, xx, y, 0);
                    if !(is_house(grid, xx, y) && is_house(above, xx, y)) {
                        continue;
                    }
                    if !is_house(grid, xx + 1, y) {
                        self.add("small_wall", v.relative(r(Direction::East), 7), rotation);
                    }
                    if !is_house(grid, xx - 1, y) {
                        let p = v.relative(r(Direction::West), 1).relative(r(Direction::South), 6);
                        self.add("small_wall", p, rotation.then(Rotation::Clockwise180));
                    }
                    if !is_house(grid, xx, y - 1) {
                        let p = v.relative(r(Direction::West), 0).relative(r(Direction::North), 1);
                        self.add("small_wall", p, rotation.then(Rotation::CounterClockwise90));
                    }
                    if !is_house(grid, xx, y + 1) {
                        let p = v.relative(r(Direction::East), 6).relative(r(Direction::South), 7);
                        self.add("small_wall", p, rotation.then(Rotation::Clockwise90));
                    }
                    if !is_house(grid, xx + 1, y) {
                        if !is_house(grid, xx, y - 1) {
                            let p = v.relative(r(Direction::East), 7).relative(r(Direction::North), 2);
                            self.add("small_wall_corner", p, rotation);
                        }
                        if !is_house(grid, xx, y + 1) {
                            let p = v.relative(r(Direction::East), 8).relative(r(Direction::South), 7);
                            self.add("small_wall_corner", p, rotation.then(Rotation::Clockwise90));
                        }
                    }
                    if !is_house(grid, xx - 1, y) {
                        if !is_house(grid, xx, y - 1) {
                            let p = v.relative(r(Direction::West), 2).relative(r(Direction::North), 1);
                            self.add(
                                "small_wall_corner",
                                p,
                                rotation.then(Rotation::CounterClockwise90),
                            );
                        }
                        if !is_house(grid, xx, y + 1) {
                            let p = v.relative(r(Direction::West), 1).relative(r(Direction::South), 8);
                            self.add("small_wall_corner", p, rotation.then(Rotation::Clockwise180));
                        }
                    }
                }
            }
        }
        for y in 0..grid.height {
            for xxx in 0..grid.width {
                let v = self.grid_pos(roof_origin, rotation, xxx, y, 0);
                let is_above = above.is_some_and(|a| is_house(a, xxx, y));
                if !(is_house(grid, xxx, y) && !is_above) {
                    continue;
                }
                if !is_house(grid, xxx + 1, y) {
                    let p2 = v.relative(r(Direction::East), 6);
                    if !is_house(grid, xxx, y + 1) {
                        self.add("roof_corner", p2.relative(r(Direction::South), 6), rotation);
                    } else if is_house(grid, xxx + 1, y + 1) {
                        self.add("roof_inner_corner", p2.relative(r(Direction::South), 5), rotation);
                    }
                    if !is_house(grid, xxx, y - 1) {
                        self.add("roof_corner", p2, rotation.then(Rotation::CounterClockwise90));
                    } else if is_house(grid, xxx + 1, y - 1) {
                        let p3 = v.relative(r(Direction::East), 9).relative(r(Direction::North), 2);
                        self.add("roof_inner_corner", p3, rotation.then(Rotation::Clockwise90));
                    }
                }
                if !is_house(grid, xxx - 1, y) {
                    let p2 = v.relative(r(Direction::East), 0).relative(r(Direction::South), 0);
                    if !is_house(grid, xxx, y + 1) {
                        self.add(
                            "roof_corner",
                            p2.relative(r(Direction::South), 6),
                            rotation.then(Rotation::Clockwise90),
                        );
                    } else if is_house(grid, xxx - 1, y + 1) {
                        let p3 = p2
                            .relative(r(Direction::South), 8)
                            .relative(r(Direction::West), 3);
                        self.add(
                            "roof_inner_corner",
                            p3,
                            rotation.then(Rotation::CounterClockwise90),
                        );
                    }
                    if !is_house(grid, xxx, y - 1) {
                        self.add("roof_corner", p2, rotation.then(Rotation::Clockwise180));
                    } else if is_house(grid, xxx - 1, y - 1) {
                        self.add(
                            "roof_inner_corner",
                            p2.relative(r(Direction::South), 1),
                            rotation.then(Rotation::Clockwise180),
                        );
                    }
                }
            }
        }
    }

    fn entrance(&mut self, data: &mut PlacementData) {
        let west = data.rotation.rotate(Direction::West);
        self.add("entrance", data.position.relative(west, 9), data.rotation);
        data.position = data.position.relative(data.rotation.rotate(Direction::South), 16);
    }

    fn traverse_wall_piece(&mut self, data: &mut PlacementData) {
        let p = data.position.relative(data.rotation.rotate(Direction::East), 7);
        self.add(data.wall_type, p, data.rotation);
        data.position = data.position.relative(data.rotation.rotate(Direction::South), 8);
    }

    fn traverse_turn(&mut self, data: &mut PlacementData) {
        data.position = data.position.relative(data.rotation.rotate(Direction::South), -1);
        self.add("wall_corner", data.position, data.rotation);
        data.position = data.position.relative(data.rotation.rotate(Direction::South), -7);
        data.position = data.position.relative(data.rotation.rotate(Direction::West), -6);
        data.rotation = data.rotation.then(Rotation::Clockwise90);
    }

    fn traverse_inner_turn(&mut self, data: &mut PlacementData) {
        data.position = data.position.relative(data.rotation.rotate(Direction::South), 6);
        data.position = data.position.relative(data.rotation.rotate(Direction::East), 8);
        data.rotation = data.rotation.then(Rotation::CounterClockwise90);
    }

    fn add_room_1x1(
        &mut self,
        room_pos: BlockPos,
        rotation: Rotation,
        door: Option<Direction>,
        rooms: Rooms,
    ) {
        let mut rot = Rotation::None;
        let mut name = rooms.get_1x1(self.random);
        if door != Some(Direction::East) {
            match door {
                Some(Direction::North) => rot = rot.then(Rotation::CounterClockwise90),
                Some(Direction::West) => rot = rot.then(Rotation::Clockwise180),
                Some(Direction::South) => rot = rot.then(Rotation::Clockwise90),
                _ => name = rooms.get_1x1_secret(self.random),
            }
        }
        let o = zero_position_with_transform(BlockPos::new(1, 0, 0), Mirror::None, rot, 7, 7);
        rot = rot.then(rotation);
        let o = rotate_pos(o, rotation);
        self.add(&name, room_pos.offset(o.x, 0, o.z), rot);
    }

    fn add_room_1x2(
        &mut self,
        room_pos: BlockPos,
        rotation: Rotation,
        room_dir: Option<Direction>,
        door: Direction,
        rooms: Rooms,
        stairs: bool,
    ) {
        use Direction::*;
        let r = |d: Direction| rotation.rotate(d);
        match (door, room_dir) {
            (East, Some(South)) => {
                let n = rooms.get_1x2_side(self.random, stairs);
                self.add(&n, room_pos.relative(r(East), 1), rotation);
            }
            (East, Some(North)) => {
                let n = rooms.get_1x2_side(self.random, stairs);
                self.add_mirrored(
                    &n,
                    room_pos.relative(r(East), 1).relative(r(South), 6),
                    rotation,
                    Mirror::LeftRight,
                );
            }
            (West, Some(North)) => {
                let n = rooms.get_1x2_side(self.random, stairs);
                self.add(
                    &n,
                    room_pos.relative(r(East), 7).relative(r(South), 6),
                    rotation.then(Rotation::Clockwise180),
                );
            }
            (West, Some(South)) => {
                let n = rooms.get_1x2_side(self.random, stairs);
                self.add_mirrored(&n, room_pos.relative(r(East), 7), rotation, Mirror::FrontBack);
            }
            (South, Some(East)) => {
                let n = rooms.get_1x2_side(self.random, stairs);
                self.add_mirrored(
                    &n,
                    room_pos.relative(r(East), 1),
                    rotation.then(Rotation::Clockwise90),
                    Mirror::LeftRight,
                );
            }
            (South, Some(West)) => {
                let n = rooms.get_1x2_side(self.random, stairs);
                self.add(
                    &n,
                    room_pos.relative(r(East), 7),
                    rotation.then(Rotation::Clockwise90),
                );
            }
            (North, Some(West)) => {
                let n = rooms.get_1x2_side(self.random, stairs);
                let p = room_pos.relative(r(East), 7).relative(r(South), 6);
                self.add_mirrored(&n, p, rotation.then(Rotation::Clockwise90), Mirror::FrontBack);
            }
            (North, Some(East)) => {
                let n = rooms.get_1x2_side(self.random, stairs);
                self.add(
                    &n,
                    room_pos.relative(r(East), 1).relative(r(South), 6),
                    rotation.then(Rotation::CounterClockwise90),
                );
            }
            (South, Some(North)) => {
                let n = rooms.get_1x2_front(self.random, stairs);
                self.add(&n, room_pos.relative(r(East), 1).relative(r(North), 8), rotation);
            }
            (North, Some(South)) => {
                let n = rooms.get_1x2_front(self.random, stairs);
                self.add(
                    &n,
                    room_pos.relative(r(East), 7).relative(r(South), 14),
                    rotation.then(Rotation::Clockwise180),
                );
            }
            (West, Some(East)) => {
                let n = rooms.get_1x2_front(self.random, stairs);
                self.add(
                    &n,
                    room_pos.relative(r(East), 15),
                    rotation.then(Rotation::Clockwise90),
                );
            }
            (East, Some(West)) => {
                let n = rooms.get_1x2_front(self.random, stairs);
                self.add(
                    &n,
                    room_pos.relative(r(West), 7).relative(r(South), 6),
                    rotation.then(Rotation::CounterClockwise90),
                );
            }
            (Up, Some(East)) => {
                let n = rooms.get_1x2_secret(self.random);
                self.add(
                    &n,
                    room_pos.relative(r(East), 15),
                    rotation.then(Rotation::Clockwise90),
                );
            }
            (Up, Some(South)) => {
                let n = rooms.get_1x2_secret(self.random);
                self.add(&n, room_pos.relative(r(East), 1).relative(r(North), 0), rotation);
            }
            _ => {}
        }
    }

    fn add_room_2x2(
        &mut self,
        room_pos: BlockPos,
        rotation: Rotation,
        room_dir: Direction,
        door: Direction,
        rooms: Rooms,
    ) {
        use Direction::*;
        let (mut east, mut south, mut rot, mut mirror) = (0, 0, rotation, Mirror::None);
        match (door, room_dir) {
            (East, South) => east = -7,
            (East, North) => {
                east = -7;
                south = 6;
                mirror = Mirror::LeftRight;
            }
            (North, East) => {
                east = 1;
                south = 14;
                rot = rotation.then(Rotation::CounterClockwise90);
            }
            (North, West) => {
                east = 7;
                south = 14;
                rot = rotation.then(Rotation::CounterClockwise90);
                mirror = Mirror::LeftRight;
            }
            (South, West) => {
                east = 7;
                south = -8;
                rot = rotation.then(Rotation::Clockwise90);
            }
            (South, East) => {
                east = 1;
                south = -8;
                rot = rotation.then(Rotation::Clockwise90);
                mirror = Mirror::LeftRight;
            }
            (West, North) => {
                east = 15;
                south = 6;
                rot = rotation.then(Rotation::Clockwise180);
            }
            (West, South) => {
                east = 15;
                mirror = Mirror::FrontBack;
            }
            _ => {}
        }
        let p = room_pos
            .relative(rotation.rotate(East), east)
            .relative(rotation.rotate(South), south);
        let n = rooms.get_2x2(self.random);
        self.add_mirrored(&n, p, rot, mirror);
    }
}

/// `BlockPos.rotate(rotation)` (around the origin).
fn rotate_pos(p: BlockPos, rotation: Rotation) -> BlockPos {
    match rotation {
        Rotation::Clockwise90 => BlockPos::new(-p.z, p.y, p.x),
        Rotation::Clockwise180 => BlockPos::new(-p.x, p.y, -p.z),
        Rotation::CounterClockwise90 => BlockPos::new(p.z, p.y, -p.x),
        Rotation::None => p,
    }
}

/// `WoodlandMansionStructure.findGenerationPoint`.
pub fn find_generation_point<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    let (bx, bz) = (ctx.min_block_x() + 7, ctx.min_block_z() + 7);
    if !ctx.could_valid_biome_exist_in_terrain_column(bx, bz) {
        return None;
    }
    let rotation = Rotation::random(&mut ctx.random);
    // `getLowestYIn5by5Box`
    let (ox, oz) = match rotation {
        Rotation::Clockwise90 => (-5, 5),
        Rotation::Clockwise180 => (-5, -5),
        Rotation::CounterClockwise90 => (5, -5),
        Rotation::None => (5, 5),
    };
    let start = BlockPos::new(bx, ctx.lowest_y_at(bx, bz, ox, oz), bz);
    if start.y < 60 {
        return None;
    }
    Some(Stub::lazy(
        start,
        move |ctx: &mut GenContext, b: &mut PiecesBuilder| {
            let mut random = std::mem::replace(&mut ctx.random, super::util::legacy_worldgen_random());
            let grid = MansionGrid::new(&mut random);
            let pieces = {
                let mut placer = Placer {
                    ctx,
                    random: &mut random,
                    pieces: Vec::new(),
                    start_x: 0,
                    start_y: 0,
                };
                placer.create(start, rotation, &grid);
                placer.pieces
            };
            ctx.random = random;
            for p in pieces {
                b.add(Box::new(p));
            }
        },
    ))
}
