use crate::math::{floor_div, floor_mod};

/// `DensityVolume`: a regular 3D grid of block positions. Buffers are indexed y-fastest:
/// `index = y + (x + z * size_x) * size_y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DensityVolume {
    pub size_x: i32,
    pub size_y: i32,
    pub size_z: i32,
    pub min_x: i32,
    pub min_y: i32,
    pub min_z: i32,
    pub step_x: i32,
    pub step_y: i32,
    pub step_z: i32,
}

impl DensityVolume {
    pub fn new(
        size_x: i32,
        size_y: i32,
        size_z: i32,
        min_x: i32,
        min_y: i32,
        min_z: i32,
        step_x: i32,
        step_y: i32,
        step_z: i32,
    ) -> Self {
        assert!(
            size_x > 0 && size_y > 0 && size_z > 0,
            "Size must be positive, was: {size_x}x{size_y}x{size_z}"
        );
        assert!(step_x > 0 && step_y > 0 && step_z > 0, "Step must be positive");
        Self {
            size_x,
            size_y,
            size_z,
            min_x,
            min_y,
            min_z,
            step_x,
            step_y,
            step_z,
        }
    }

    pub fn blocks(size_x: i32, size_y: i32, size_z: i32, min_x: i32, min_y: i32, min_z: i32) -> Self {
        Self::new(size_x, size_y, size_z, min_x, min_y, min_z, 1, 1, 1)
    }

    #[inline(always)]
    pub fn index(&self, x: i32, y: i32, z: i32) -> usize {
        (y + (x + z * self.size_x) * self.size_y) as usize
    }

    #[inline(always)]
    pub fn block_x(&self, x: i32) -> i32 {
        self.min_x + x * self.step_x
    }

    #[inline(always)]
    pub fn block_y(&self, y: i32) -> i32 {
        self.min_y + y * self.step_y
    }

    #[inline(always)]
    pub fn block_z(&self, z: i32) -> i32 {
        self.min_z + z * self.step_z
    }

    pub fn max_x(&self) -> i32 {
        self.min_x + self.size_x * self.step_x - 1
    }

    pub fn max_y(&self) -> i32 {
        self.min_y + self.size_y * self.step_y - 1
    }

    pub fn max_z(&self) -> i32 {
        self.min_z + self.size_z * self.step_z - 1
    }

    #[inline(always)]
    pub fn size(&self) -> usize {
        (self.size_x * self.size_y * self.size_z) as usize
    }

    fn contains_relative(&self, rx: i32, ry: i32, rz: i32) -> bool {
        rx >= 0
            && ry >= 0
            && rz >= 0
            && rx < self.size_x * self.step_x
            && ry < self.size_y * self.step_y
            && rz < self.size_z * self.step_z
            && floor_mod(rx, self.step_x) == 0
            && floor_mod(ry, self.step_y) == 0
            && floor_mod(rz, self.step_z) == 0
    }

    /// `indexOfBlock`: buffer index of a block position, `None` if not on the grid.
    pub fn index_of_block(&self, bx: i32, by: i32, bz: i32) -> Option<usize> {
        let rx = bx - self.min_x;
        let ry = by - self.min_y;
        let rz = bz - self.min_z;
        if self.step_x == 1 && self.step_y == 1 && self.step_z == 1 {
            if rx >= 0 && ry >= 0 && rz >= 0 && rx < self.size_x && ry < self.size_y && rz < self.size_z {
                return Some(self.index(rx, ry, rz));
            }
        } else if self.contains_relative(rx, ry, rz) {
            return Some(self.index(
                floor_div(rx, self.step_x),
                floor_div(ry, self.step_y),
                floor_div(rz, self.step_z),
            ));
        }
        None
    }

    pub fn intersects_box(
        &self,
        min_x: i32,
        min_y: i32,
        min_z: i32,
        max_x: i32,
        max_y: i32,
        max_z: i32,
    ) -> bool {
        self.max_x() >= min_x
            && self.min_x <= max_x
            && self.max_z() >= min_z
            && self.min_z <= max_z
            && self.max_y() >= min_y
            && self.min_y <= max_y
    }
}
