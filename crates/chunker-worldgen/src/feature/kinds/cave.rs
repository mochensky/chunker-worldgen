//! Cave and underwater features: `Column` scans, underwater magma.

use anyhow::Result;
use chunker_data::blocks::BlockState;
use serde_json::Value;

use crate::codec::{f32_field, i32_field};
use crate::feature::Region;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

/// `Column`: an optional floor and ceiling (exclusive edges).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub floor: Option<i32>,
    pub ceiling: Option<i32>,
}

impl Column {
    /// `Column.getHeight` (only for ranges).
    pub fn height(&self) -> Option<i32> {
        match (self.floor, self.ceiling) {
            (Some(f), Some(c)) => Some(c - f - 1),
            _ => None,
        }
    }

    /// `Column.scan(level, pos, searchRange, insideColumn, validEdge)`.
    pub fn scan(
        region: &Region,
        pos: BlockPos,
        search_range: i32,
        inside: &dyn Fn(BlockState) -> bool,
        edge: &dyn Fn(BlockState) -> bool,
    ) -> Option<Column> {
        if !inside(region.get(pos)) {
            return None;
        }
        let scan = |dir: Direction| -> Option<i32> {
            let mut p = pos;
            let mut i = 1;
            while i < search_range && inside(region.get(p)) {
                p = p.relative(dir, 1);
                i += 1;
            }
            if edge(region.get(p)) { Some(p.y) } else { None }
        };
        let ceiling = scan(Direction::Up);
        let floor = scan(Direction::Down);
        Some(Column { floor, ceiling })
    }
}

/// `UnderwaterMagmaFeature`.
pub struct UnderwaterMagma {
    floor_search_range: i32,
    radius: i32,
    probability: f32,
}

impl UnderwaterMagma {
    pub fn parse(json: &Value) -> Result<Self> {
        Ok(Self {
            floor_search_range: i32_field(json, "floor_search_range")?,
            radius: i32_field(json, "placement_radius_around_floor")?,
            probability: f32_field(json, "placement_probability_per_valid_position")?,
        })
    }

    fn visible_from_outside(region: &Region, pos: BlockPos, covered: Direction) -> bool {
        !region.flags(pos).face_occlusion_full(covered.data_3d() as usize)
    }

    fn valid(region: &Region, pos: BlockPos) -> bool {
        let s = region.get(pos);
        let water_or_air =
            region.env.blocks.block_of(s) == region.env.b.water || region.env.blocks.flags(s).is_air();
        if water_or_air || Self::visible_from_outside(region, pos.below(), Direction::Up) {
            return false;
        }
        !Direction::HORIZONTAL
            .iter()
            .any(|d| Self::visible_from_outside(region, pos.relative(*d, 1), d.opposite()))
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let water = region.env.b.water;
        let blocks = &region.env.blocks;
        let column = Column::scan(
            region,
            origin,
            self.floor_search_range,
            &|s| blocks.block_of(s) == water,
            &|s| blocks.block_of(s) != water,
        );
        let Some(floor) = column.and_then(|c| c.floor) else {
            return false;
        };
        let f = BlockPos::new(origin.x, floor, origin.z);
        let r = self.radius;
        let magma = blocks.block(region.env.b.magma_block).default_state;
        let mut placed = 0;
        for z in f.z - r..=f.z + r {
            for y in f.y - r..=f.y + r {
                for x in f.x - r..=f.x + r {
                    let p = BlockPos::new(x, y, z);
                    if random.next_float() < self.probability && Self::valid(region, p) {
                        region.set(p, magma, 2);
                        placed += 1;
                    }
                }
            }
        }
        placed > 0
    }
}
