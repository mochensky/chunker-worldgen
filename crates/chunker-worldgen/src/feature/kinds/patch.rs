//! `VegetationPatchFeature`, `WaterloggedVegetationPatchFeature` and `RootSystemFeature`.

use std::sync::Arc;

use anyhow::{Result, bail};
use chunker_data::blocks::BlockState;
use serde_json::Value;

use crate::block_set::StateSet;
use crate::chunk::HeightmapKind;
use crate::codec::{f32_field, field, i32_field, str_field};
use crate::feature::behaviour::{can_survive, is_face_sturdy};
use crate::feature::predicate::BlockPredicate;
use crate::feature::provider::StateProvider;
use crate::feature::registry::{Parser, PlacedFeature};
use crate::feature::{Region, placement};
use crate::java::JavaHashSet;
use crate::providers::IntProvider;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

pub struct VegetationPatch {
    waterlogged: bool,
    replaceable: Arc<StateSet>,
    ground: Arc<StateProvider>,
    vegetation: Arc<PlacedFeature>,
    /// `CaveSurface.getDirection()`: down for floors, up for ceilings.
    inwards: Direction,
    depth: IntProvider,
    extra_bottom_block_chance: f32,
    vertical_range: i32,
    vegetation_chance: f32,
    xz_radius: IntProvider,
    extra_edge_column_chance: f32,
}

impl VegetationPatch {
    pub fn parse(p: &Parser, json: &Value, waterlogged: bool) -> Result<Self> {
        let inwards = match str_field(json, "surface")? {
            "floor" => Direction::Down,
            "ceiling" => Direction::Up,
            other => bail!("unknown cave surface {other}"),
        };
        Ok(Self {
            waterlogged,
            replaceable: p.block_set(field(json, "replaceable")?)?,
            ground: p.provider(field(json, "ground_state")?)?,
            vegetation: p.placed(field(json, "vegetation_feature")?)?,
            inwards,
            depth: IntProvider::parse(field(json, "depth")?)?,
            extra_bottom_block_chance: f32_field(json, "extra_bottom_block_chance")?,
            vertical_range: i32_field(json, "vertical_range")?,
            vegetation_chance: f32_field(json, "vegetation_chance")?,
            xz_radius: IntProvider::parse(field(json, "xz_radius")?)?,
            extra_edge_column_chance: f32_field(json, "extra_edge_column_chance")?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let xr = self.xz_radius.sample(random) + 1;
        let zr = self.xz_radius.sample(random) + 1;
        let mut surface = self.ground_patch(region, random, origin, xr, zr);
        if self.waterlogged {
            let mut water = JavaHashSet::new();
            for p in surface.iter() {
                if !self.exposed(region, p) {
                    water.insert(p);
                }
            }
            let w = region.env.blocks.block(region.env.b.water).default_state;
            for p in water.iter() {
                region.set(p, w, 2);
            }
            surface = water;
        }
        // `distributeVegetation`
        for p in surface.iter() {
            if self.vegetation_chance > 0.0 && random.next_float() < self.vegetation_chance {
                self.place_vegetation(region, random, p);
            }
        }
        !surface.is_empty()
    }

    fn exposed(&self, region: &Region, p: BlockPos) -> bool {
        [
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
            Direction::Down,
        ]
        .iter()
        .any(|d| !is_face_sturdy(region.env, region.get(p.relative(*d, 1)), d.opposite()))
    }

    fn place_vegetation(&self, region: &mut Region, random: &mut dyn RandomSource, p: BlockPos) -> bool {
        let outwards = self.inwards.opposite();
        if !self.waterlogged {
            return placement::place(region, &self.vegetation, None, random, p.relative(outwards, 1));
        }
        // `WaterloggedVegetationPatchFeature.placeVegetation(level, ..., pos)` passes `pos.below()` to the base.
        let base = p.below();
        if placement::place(region, &self.vegetation, None, random, base.relative(outwards, 1)) {
            let placed = region.get(p);
            let blocks = &region.env.blocks;
            if blocks.get_property(placed, "waterlogged") == Some("false") {
                let s = blocks.with_property(placed, "waterlogged", "true").unwrap();
                region.set(p, s, 2);
            }
            true
        } else {
            false
        }
    }

    fn ground_patch(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        xr: i32,
        zr: i32,
    ) -> JavaHashSet<BlockPos> {
        let inwards = self.inwards;
        let outwards = inwards.opposite();
        let mut surface = JavaHashSet::new();
        for dx in -xr..=xr {
            let x_edge = dx == -xr || dx == xr;
            for dz in -zr..=zr {
                let z_edge = dz == -zr || dz == zr;
                let edge = x_edge || z_edge;
                let corner = x_edge && z_edge;
                let edge_not_corner = edge && !corner;
                if corner {
                    continue;
                }
                if edge_not_corner
                    && !(self.extra_edge_column_chance != 0.0
                        && random.next_float() <= self.extra_edge_column_chance)
                {
                    continue;
                }
                let mut pos = origin.offset(dx, 0, dz);
                let mut offset = 0;
                while region.flags(pos).is_air() && offset < self.vertical_range {
                    pos = pos.relative(inwards, 1);
                    offset += 1;
                }
                let mut offset = 0;
                while !region.flags(pos).is_air() && offset < self.vertical_range {
                    pos = pos.relative(outwards, 1);
                    offset += 1;
                }
                let below = pos.relative(inwards, 1);
                if region.is_empty_block(pos) && is_face_sturdy(region.env, region.get(below), outwards) {
                    let depth = self.depth.sample(random)
                        + if self.extra_bottom_block_chance > 0.0
                            && random.next_float() < self.extra_bottom_block_chance
                        {
                            1
                        } else {
                            0
                        };
                    if self.place_ground(region, random, below, depth) {
                        surface.insert(below);
                    }
                }
            }
        }
        surface
    }

    fn place_ground(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        start: BlockPos,
        depth: i32,
    ) -> bool {
        let mut pos = start;
        for i in 0..depth {
            let to_place = self.ground.get(region, random, pos);
            let below = region.get(pos);
            let blocks = &region.env.blocks;
            // Vanilla does not advance when the block is already the target block.
            if blocks.block_of(to_place) != blocks.block_of(below) {
                if !self.replaceable.contains(below) {
                    return i != 0;
                }
                region.set(pos, to_place, 2);
                pos = pos.relative(self.inwards, 1);
            }
        }
        true
    }
}

/// `RootSystemFeature` (azalea trees over rooted dirt, sulfur springs).
pub struct RootSystem {
    tree: Arc<PlacedFeature>,
    required_vertical_space: i32,
    level_test_distance: i32,
    max_level_deviation: i32,
    root_radius: i32,
    root_replaceable: Arc<StateSet>,
    root_state: Arc<StateProvider>,
    root_placement_attempts: i32,
    root_column_max_height: i32,
    hanging_root_radius: i32,
    hanging_roots_vertical_span: i32,
    hanging_root_state: Arc<StateProvider>,
    hanging_root_placement_attempts: i32,
    allowed_vertical_water: i32,
    allowed_tree_position: BlockPredicate,
}

impl RootSystem {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let i = |n: &str| i32_field(json, n);
        Ok(Self {
            tree: p.placed(field(json, "feature")?)?,
            required_vertical_space: i("required_vertical_space_for_tree")?,
            level_test_distance: i("level_test_distance")?,
            max_level_deviation: i("max_level_deviation")?,
            root_radius: i("root_radius")?,
            root_replaceable: p.block_set(field(json, "root_replaceable")?)?,
            root_state: p.provider(field(json, "root_state_provider")?)?,
            root_placement_attempts: i("root_placement_attempts")?,
            root_column_max_height: i("root_column_max_height")?,
            hanging_root_radius: i("hanging_root_radius")?,
            hanging_roots_vertical_span: i("hanging_roots_vertical_span")?,
            hanging_root_state: p.provider(field(json, "hanging_root_state_provider")?)?,
            hanging_root_placement_attempts: i("hanging_root_placement_attempts")?,
            allowed_vertical_water: i("allowed_vertical_water_for_tree")?,
            allowed_tree_position: BlockPredicate::parse(p, field(json, "allowed_tree_position")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if !region.flags(origin).is_air() {
            return false;
        }
        if let Some(working) = self.dirt_and_tree(region, random, origin) {
            self.hanging_roots(region, random, origin, working);
        }
        true
    }

    fn space_for_tree(&self, region: &Region, pos: BlockPos) -> bool {
        let mut up = pos;
        for i in 1..=self.required_vertical_space {
            up = up.above();
            let f = region.flags(up);
            if !f.is_air() && !(i + 1 <= self.allowed_vertical_water && f.fluid().is_water()) {
                return false;
            }
        }
        if self.level_test_distance > 0 {
            for ix in 0..4 {
                let corner = pos.relative(Direction::from_2d(ix), self.level_test_distance);
                let below = region.flags(corner.offset(0, -self.max_level_deviation, 0));
                let above = region.flags(corner.offset(0, self.max_level_deviation, 0));
                if below.is_air() || !above.is_air() {
                    return false;
                }
            }
        }
        true
    }

    /// `placeDirtAndTree`; returns the working position on success.
    fn dirt_and_tree(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        pos: BlockPos,
    ) -> Option<BlockPos> {
        let mut working = pos;
        for y in 0..self.root_column_max_height {
            working = working.above();
            if region.height(HeightmapKind::WorldSurface, working.x, working.z) < working.y {
                return None;
            }
            if self.allowed_tree_position.test(region, working) && self.space_for_tree(region, working) {
                let below = working.below();
                let bf = region.flags(below);
                if bf.fluid().is_lava() || !bf.is_solid() {
                    return None;
                }
                if placement::place(region, &self.tree, None, random, working) {
                    self.place_dirt(region, random, pos, pos.y + y);
                    return Some(working);
                }
            }
        }
        None
    }

    fn place_dirt(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos, target: i32) {
        for y in origin.y..target {
            for _ in 0..self.root_placement_attempts {
                let dx =
                    random.next_int_bounded(self.root_radius) - random.next_int_bounded(self.root_radius);
                let dz =
                    random.next_int_bounded(self.root_radius) - random.next_int_bounded(self.root_radius);
                let p = BlockPos::new(origin.x + dx, y, origin.z + dz);
                if self.root_replaceable.contains(region.get(p)) {
                    let s = self.root_state.get(region, random, p);
                    region.set(p, s, 2);
                }
            }
        }
    }

    fn hanging_roots(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        pos: BlockPos,
        _working: BlockPos,
    ) {
        for _ in 0..self.hanging_root_placement_attempts {
            let r = self.hanging_root_radius;
            let v = self.hanging_roots_vertical_span;
            let dx = random.next_int_bounded(r) - random.next_int_bounded(r);
            let dy = random.next_int_bounded(v) - random.next_int_bounded(v);
            let dz = random.next_int_bounded(r) - random.next_int_bounded(r);
            let p = pos.offset(dx, dy, dz);
            if region.is_empty_block(p) {
                let target: BlockState = self.hanging_root_state.get(region, random, p);
                if can_survive(region, target, p)
                    && is_face_sturdy(region.env, region.get(p.above()), Direction::Down)
                {
                    region.set(p, target, 2);
                }
            }
        }
    }
}
