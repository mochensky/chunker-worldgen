//! `BoundingBox`, `Direction`, `Rotation`, `Mirror` and block positions as used by structures.

use crate::random::RandomSource;

/// `BlockPos` / `Vec3i`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const ZERO: BlockPos = BlockPos { x: 0, y: 0, z: 0 };

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(
            self.x.wrapping_add(dx),
            self.y.wrapping_add(dy),
            self.z.wrapping_add(dz),
        )
    }

    pub fn add(self, o: BlockPos) -> Self {
        self.offset(o.x, o.y, o.z)
    }

    pub fn sub(self, o: BlockPos) -> Self {
        self.offset(-o.x, -o.y, -o.z)
    }

    pub fn relative(self, d: Direction, n: i32) -> Self {
        let (dx, dy, dz) = d.step();
        self.offset(dx * n, dy * n, dz * n)
    }

    pub fn above(self) -> Self {
        self.offset(0, 1, 0)
    }

    pub fn above_n(self, n: i32) -> Self {
        self.offset(0, n, 0)
    }

    pub fn below(self) -> Self {
        self.offset(0, -1, 0)
    }

    /// `BlockPos.asLong`.
    pub fn as_long(self) -> i64 {
        crate::density::context::pack_block_pos(self.x, self.y, self.z)
    }

    pub fn dist_manhattan(self, o: BlockPos) -> i32 {
        (self.x - o.x).abs() + (self.y - o.y).abs() + (self.z - o.z).abs()
    }
}

/// `Direction` (3D ids in vanilla order: down, up, north, south, west, east).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Direction {
    pub const ALL: [Direction; 6] = [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ];
    /// `Direction.Plane.HORIZONTAL` order.
    pub const HORIZONTAL: [Direction; 4] = [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
    ];
    /// `BY_2D_DATA`: south, west, north, east.
    pub const BY_2D: [Direction; 4] = [
        Direction::South,
        Direction::West,
        Direction::North,
        Direction::East,
    ];

    pub fn step(self) -> (i32, i32, i32) {
        match self {
            Direction::Down => (0, -1, 0),
            Direction::Up => (0, 1, 0),
            Direction::North => (0, 0, -1),
            Direction::South => (0, 0, 1),
            Direction::West => (-1, 0, 0),
            Direction::East => (1, 0, 0),
        }
    }

    pub fn axis(self) -> Axis {
        match self {
            Direction::Down | Direction::Up => Axis::Y,
            Direction::North | Direction::South => Axis::Z,
            Direction::West | Direction::East => Axis::X,
        }
    }

    /// `get3DDataValue`.
    pub fn data_3d(self) -> i32 {
        self as i32
    }

    /// `get2DDataValue` (-1 for vertical directions).
    pub fn data_2d(self) -> i32 {
        match self {
            Direction::South => 0,
            Direction::West => 1,
            Direction::North => 2,
            Direction::East => 3,
            _ => -1,
        }
    }

    /// `Direction.from3DDataValue`.
    pub fn from_3d(v: i32) -> Direction {
        Self::ALL[v.rem_euclid(6) as usize]
    }

    pub fn from_2d(v: i32) -> Direction {
        Self::BY_2D[(v.rem_euclid(4)) as usize]
    }

    pub fn opposite(self) -> Direction {
        match self {
            Direction::Down => Direction::Up,
            Direction::Up => Direction::Down,
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::West => Direction::East,
            Direction::East => Direction::West,
        }
    }

    pub fn clockwise(self) -> Direction {
        match self {
            Direction::North => Direction::East,
            Direction::East => Direction::South,
            Direction::South => Direction::West,
            Direction::West => Direction::North,
            d => panic!("unable to get Y-rotated facing of {d:?}"),
        }
    }

    pub fn counter_clockwise(self) -> Direction {
        match self {
            Direction::North => Direction::West,
            Direction::West => Direction::South,
            Direction::South => Direction::East,
            Direction::East => Direction::North,
            d => panic!("unable to get CCW facing of {d:?}"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Direction::Down => "down",
            Direction::Up => "up",
            Direction::North => "north",
            Direction::South => "south",
            Direction::West => "west",
            Direction::East => "east",
        }
    }

    pub fn by_name(s: &str) -> Option<Direction> {
        Self::ALL.iter().copied().find(|d| d.name() == s)
    }

    /// `Direction.Plane.HORIZONTAL.getRandomDirection(random)`.
    pub fn random_horizontal(random: &mut dyn RandomSource) -> Direction {
        Self::HORIZONTAL[random.next_int_bounded(4) as usize]
    }

    /// `Direction.CODEC` (lower case names).
    pub fn parse(s: &str) -> anyhow::Result<Direction> {
        Self::by_name(s).ok_or_else(|| anyhow::anyhow!("unknown direction {s}"))
    }

    pub fn step_x(self) -> i32 {
        self.step().0
    }

    pub fn step_y(self) -> i32 {
        self.step().1
    }

    pub fn step_z(self) -> i32 {
        self.step().2
    }

    pub fn is_horizontal(self) -> bool {
        self.axis() != Axis::Y
    }

    /// `getAxisDirection() == POSITIVE`.
    pub fn is_positive(self) -> bool {
        matches!(self, Direction::Up | Direction::South | Direction::East)
    }

    /// `Direction.fromAxisAndDirection`.
    pub fn from_axis(axis: Axis, positive: bool) -> Direction {
        match (axis, positive) {
            (Axis::X, true) => Direction::East,
            (Axis::X, false) => Direction::West,
            (Axis::Y, true) => Direction::Up,
            (Axis::Y, false) => Direction::Down,
            (Axis::Z, true) => Direction::South,
            (Axis::Z, false) => Direction::North,
        }
    }
}

impl Axis {
    pub fn name(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Y => "y",
            Axis::Z => "z",
        }
    }
}

/// `Rotation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rotation {
    None,
    Clockwise90,
    Clockwise180,
    CounterClockwise90,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [
        Rotation::None,
        Rotation::Clockwise90,
        Rotation::Clockwise180,
        Rotation::CounterClockwise90,
    ];

    /// `Rotation.getRandom` (`Util.getRandom(values())`).
    pub fn random(random: &mut dyn RandomSource) -> Rotation {
        Self::ALL[random.next_int_bounded(4) as usize]
    }

    /// `Rotation.getShuffled` (`Util.shuffledCopy(values())`).
    pub fn shuffled(random: &mut dyn RandomSource) -> Vec<Rotation> {
        let mut v = Self::ALL.to_vec();
        crate::structure::util::shuffle(&mut v, random);
        v
    }

    /// Serialized enum name (`NONE`, `CLOCKWISE_90`, ...), used by piece NBT.
    pub fn name(self) -> &'static str {
        match self {
            Rotation::None => "NONE",
            Rotation::Clockwise90 => "CLOCKWISE_90",
            Rotation::Clockwise180 => "CLOCKWISE_180",
            Rotation::CounterClockwise90 => "COUNTERCLOCKWISE_90",
        }
    }

    /// `getRotated`.
    pub fn then(self, other: Rotation) -> Rotation {
        Self::ALL[((self as usize) + (other as usize)) % 4]
    }

    pub fn rotate(self, d: Direction) -> Direction {
        if d.axis() == Axis::Y {
            return d;
        }
        match self {
            Rotation::None => d,
            Rotation::Clockwise90 => d.clockwise(),
            Rotation::Clockwise180 => d.opposite(),
            Rotation::CounterClockwise90 => d.counter_clockwise(),
        }
    }

    /// `rotate(int rotation, int count)` for 16-step rotations (signs, skulls, banners).
    pub fn rotate_16(self, value: i32, count: i32) -> i32 {
        match self {
            Rotation::None => value,
            Rotation::Clockwise90 => (value + count / 4).rem_euclid(count),
            Rotation::Clockwise180 => (value + count / 2).rem_euclid(count),
            Rotation::CounterClockwise90 => (value + count * 3 / 4).rem_euclid(count),
        }
    }
}

/// `Mirror`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mirror {
    None,
    LeftRight,
    FrontBack,
}

impl Mirror {
    pub fn name(self) -> &'static str {
        match self {
            Mirror::None => "NONE",
            Mirror::LeftRight => "LEFT_RIGHT",
            Mirror::FrontBack => "FRONT_BACK",
        }
    }

    /// `getRotation(Direction)`.
    pub fn rotation_of(self, d: Direction) -> Rotation {
        let axis = d.axis();
        match self {
            Mirror::FrontBack if axis == Axis::X => Rotation::Clockwise180,
            Mirror::LeftRight if axis == Axis::Z => Rotation::Clockwise180,
            _ => Rotation::None,
        }
    }

    pub fn mirror(self, d: Direction) -> Direction {
        match self {
            Mirror::FrontBack if d.axis() == Axis::X => d.opposite(),
            Mirror::LeftRight if d.axis() == Axis::Z => d.opposite(),
            _ => d,
        }
    }

    /// `mirror(int rotation, int count)`.
    pub fn mirror_16(self, value: i32, count: i32) -> i32 {
        let half = count / 2;
        let rel = if value > half { value - count } else { value };
        match self {
            Mirror::FrontBack => (count - rel).rem_euclid(count),
            Mirror::LeftRight => (half - rel).rem_euclid(count),
            Mirror::None => value,
        }
    }
}

/// `BoundingBox` (inclusive bounds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoundingBox {
    pub min_x: i32,
    pub min_y: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub max_z: i32,
}

impl BoundingBox {
    /// `new BoundingBox(...)`: inverted bounds are swapped like vanilla does.
    pub fn new(min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Self {
        Self {
            min_x: min_x.min(max_x),
            min_y: min_y.min(max_y),
            min_z: min_z.min(max_z),
            max_x: min_x.max(max_x),
            max_y: min_y.max(max_y),
            max_z: min_z.max(max_z),
        }
    }

    pub fn of_pos(p: BlockPos) -> Self {
        Self::new(p.x, p.y, p.z, p.x, p.y, p.z)
    }

    pub fn from_corners(a: BlockPos, b: BlockPos) -> Self {
        Self::new(
            a.x.min(b.x),
            a.y.min(b.y),
            a.z.min(b.z),
            a.x.max(b.x),
            a.y.max(b.y),
            a.z.max(b.z),
        )
    }

    pub fn infinite() -> Self {
        Self::new(i32::MIN, i32::MIN, i32::MIN, i32::MAX, i32::MAX, i32::MAX)
    }

    /// `BoundingBox.orientBox`.
    pub fn orient_box(
        foot_x: i32,
        foot_y: i32,
        foot_z: i32,
        off_x: i32,
        off_y: i32,
        off_z: i32,
        width: i32,
        height: i32,
        depth: i32,
        direction: Direction,
    ) -> Self {
        match direction {
            Direction::North => Self::new(
                foot_x + off_x,
                foot_y + off_y,
                foot_z - depth + 1 + off_z,
                foot_x + width - 1 + off_x,
                foot_y + height - 1 + off_y,
                foot_z + off_z,
            ),
            Direction::West => Self::new(
                foot_x - depth + 1 + off_z,
                foot_y + off_y,
                foot_z + off_x,
                foot_x + off_z,
                foot_y + height - 1 + off_y,
                foot_z + width - 1 + off_x,
            ),
            Direction::East => Self::new(
                foot_x + off_z,
                foot_y + off_y,
                foot_z + off_x,
                foot_x + depth - 1 + off_z,
                foot_y + height - 1 + off_y,
                foot_z + width - 1 + off_x,
            ),
            _ => Self::new(
                foot_x + off_x,
                foot_y + off_y,
                foot_z + off_z,
                foot_x + width - 1 + off_x,
                foot_y + height - 1 + off_y,
                foot_z + depth - 1 + off_z,
            ),
        }
    }

    pub fn intersects(&self, o: &BoundingBox) -> bool {
        self.max_x >= o.min_x
            && self.min_x <= o.max_x
            && self.max_z >= o.min_z
            && self.min_z <= o.max_z
            && self.max_y >= o.min_y
            && self.min_y <= o.max_y
    }

    pub fn intersects_xz(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        self.max_x >= min_x && self.min_x <= max_x && self.max_z >= min_z && self.min_z <= max_z
    }

    /// `encapsulate(BoundingBox)` (mutating).
    pub fn encapsulate(&mut self, o: &BoundingBox) {
        self.min_x = self.min_x.min(o.min_x);
        self.min_y = self.min_y.min(o.min_y);
        self.min_z = self.min_z.min(o.min_z);
        self.max_x = self.max_x.max(o.max_x);
        self.max_y = self.max_y.max(o.max_y);
        self.max_z = self.max_z.max(o.max_z);
    }

    pub fn encapsulate_pos(&mut self, p: BlockPos) {
        self.encapsulate(&BoundingBox::of_pos(p));
    }

    /// `move` (mutating).
    pub fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.min_x += dx;
        self.min_y += dy;
        self.min_z += dz;
        self.max_x += dx;
        self.max_y += dy;
        self.max_z += dz;
    }

    pub fn moved(&self, dx: i32, dy: i32, dz: i32) -> BoundingBox {
        let mut b = *self;
        b.move_by(dx, dy, dz);
        b
    }

    pub fn inflated_by(&self, n: i32) -> BoundingBox {
        self.inflated_xyz(n, n, n)
    }

    pub fn inflated_xyz(&self, x: i32, y: i32, z: i32) -> BoundingBox {
        BoundingBox::new(
            self.min_x - x,
            self.min_y - y,
            self.min_z - z,
            self.max_x + x,
            self.max_y + y,
            self.max_z + z,
        )
    }

    pub fn is_inside(&self, x: i32, y: i32, z: i32) -> bool {
        x >= self.min_x
            && x <= self.max_x
            && z >= self.min_z
            && z <= self.max_z
            && y >= self.min_y
            && y <= self.max_y
    }

    pub fn contains(&self, p: BlockPos) -> bool {
        self.is_inside(p.x, p.y, p.z)
    }

    pub fn x_span(&self) -> i32 {
        self.max_x - self.min_x + 1
    }

    pub fn y_span(&self) -> i32 {
        self.max_y - self.min_y + 1
    }

    pub fn z_span(&self) -> i32 {
        self.max_z - self.min_z + 1
    }

    pub fn center(&self) -> BlockPos {
        BlockPos::new(
            self.min_x + (self.max_x - self.min_x + 1) / 2,
            self.min_y + (self.max_y - self.min_y + 1) / 2,
            self.min_z + (self.max_z - self.min_z + 1) / 2,
        )
    }

    /// The `BoundingBox.CODEC` form (int array).
    pub fn to_array(&self) -> Vec<i32> {
        vec![
            self.min_x, self.min_y, self.min_z, self.max_x, self.max_y, self.max_z,
        ]
    }

    /// `encapsulatingBoxes`.
    /// `BoundingBox.encapsulatingPositions`.
    pub fn encapsulating_positions(positions: &[BlockPos]) -> Option<BoundingBox> {
        let mut it = positions.iter();
        let first = *it.next()?;
        let mut b = BoundingBox::of_pos(first);
        for p in it {
            b.encapsulate_pos(*p);
        }
        Some(b)
    }

    pub fn encapsulating<'a>(mut boxes: impl Iterator<Item = &'a BoundingBox>) -> Option<BoundingBox> {
        let mut result = *boxes.next()?;
        for b in boxes {
            result.encapsulate(b);
        }
        Some(result)
    }
}
