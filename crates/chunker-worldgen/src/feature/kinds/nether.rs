//! Nether feature types: deltas, replace blobs, neighbour spread, stepped columns, huge fungi.

use std::sync::Arc;

use anyhow::Result;
use chunker_data::blocks::{BlockId, BlockState};
use serde_json::Value;

use crate::block_set::StateSet;
use crate::codec::field;
use crate::feature::Region;
use crate::feature::predicate::BlockPredicate;
use crate::feature::provider::StateProvider;
use crate::feature::registry::Parser;
use crate::providers::{IntProvider, mth_next_int};
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

/// `BlockPos.manhattanOrdered`: positions by increasing Manhattan distance, each z mirrored.
fn manhattan_ordered(
    origin: BlockPos,
    reach_x: i32,
    reach_y: i32,
    reach_z: i32,
    max_depth: i32,
) -> impl Iterator<Item = BlockPos> {
    let (mut depth, mut max_x, mut max_y, mut x, mut y) = (0, 0, 0, 0, 0);
    let mut mirror: Option<BlockPos> = None;
    let mut done = false;
    std::iter::from_fn(move || {
        if let Some(p) = mirror.take() {
            return Some(BlockPos::new(p.x, p.y, origin.z - (p.z - origin.z)));
        }
        if done {
            return None;
        }
        loop {
            if y > max_y {
                x += 1;
                if x > max_x {
                    depth += 1;
                    if depth > max_depth {
                        done = true;
                        return None;
                    }
                    max_x = reach_x.min(depth);
                    x = -max_x;
                }
                max_y = reach_y.min(depth - x.abs());
                y = -max_y;
            }
            let (xx, yy) = (x, y);
            y += 1;
            let zz = depth - xx.abs() - yy.abs();
            if zz <= reach_z {
                let p = BlockPos::new(origin.x + xx, origin.y + yy, origin.z + zz);
                if zz != 0 {
                    mirror = Some(p);
                }
                return Some(p);
            }
        }
    })
}

/// `BlockPos.withinBoxByManhattanDistance`.
fn within_box_by_manhattan(origin: BlockPos, rx: i32, ry: i32, rz: i32) -> impl Iterator<Item = BlockPos> {
    manhattan_ordered(origin, rx, ry, rz, rx + ry + rz)
}

/// `Vec3i.distManhattan` (sums as floats like vanilla; exact for block distances).
fn dist_manhattan(a: BlockPos, b: BlockPos) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs() + (a.z - b.z).abs()
}

/// `DeltaFeature`.
pub struct Delta {
    contents: BlockState,
    rim: BlockState,
    size: IntProvider,
    rim_size: IntProvider,
}

impl Delta {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            contents: p.state(field(json, "contents")?)?,
            rim: p.state(field(json, "rim")?)?,
            size: IntProvider::parse(field(json, "size")?)?,
            rim_size: IntProvider::parse(field(json, "rim_size")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let mut any = false;
        let spawn_rim = random.next_double() < 0.9;
        let rim_x = if spawn_rim {
            self.rim_size.sample(random)
        } else {
            0
        };
        let rim_z = if spawn_rim {
            self.rim_size.sample(random)
        } else {
            0
        };
        let has_rim = spawn_rim && rim_x != 0 && rim_z != 0;
        let radius_x = self.size.sample(random);
        let radius_z = self.size.sample(random);
        let limit = radius_x.max(radius_z);
        for pos in within_box_by_manhattan(origin, radius_x, 0, radius_z) {
            if dist_manhattan(pos, origin) > limit {
                break;
            }
            if self.is_clear(region, pos) {
                if has_rim {
                    any = true;
                    region.set_and_update(pos, self.rim);
                }
                let offset = pos.offset(rim_x, 0, rim_z);
                if self.is_clear(region, offset) {
                    any = true;
                    region.set_and_update(offset, self.contents);
                }
            }
        }
        any
    }

    fn is_clear(&self, region: &Region, pos: BlockPos) -> bool {
        let b = &region.env.b;
        let block = region.block_of(pos);
        if block == region.env.blocks.block_of(self.contents) {
            return false;
        }
        let cannot_replace = [
            b.bedrock,
            b.nether_bricks,
            b.nether_brick_fence,
            b.nether_brick_stairs,
            b.nether_wart,
            b.chest,
            b.spawner,
        ];
        if cannot_replace.contains(&block) {
            return false;
        }
        Direction::ALL
            .iter()
            .all(|d| region.flags(pos.relative(*d, 1)).is_air() == (*d == Direction::Up))
    }
}

/// `ReplaceBlobsFeature`.
pub struct ReplaceBlobs {
    target: BlockId,
    state: BlockState,
    radius: IntProvider,
}

impl ReplaceBlobs {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let target = p.state(field(json, "target")?)?;
        Ok(Self {
            target: p.reg.blocks.block_of(target),
            state: p.state(field(json, "state")?)?,
            radius: IntProvider::parse(field(json, "radius")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let min_y = region.min_y();
        let mut cursor = BlockPos::new(origin.x, origin.y.clamp(min_y + 1, region.max_y()), origin.z);
        let center = loop {
            if cursor.y <= min_y + 1 {
                return false;
            }
            if region.is(cursor, self.target) {
                break cursor;
            }
            cursor = cursor.below();
        };
        let rx = self.radius.sample(random);
        let ry = self.radius.sample(random);
        let rz = self.radius.sample(random);
        let max = rx.max(ry.max(rz));
        let mut any = false;
        for pos in within_box_by_manhattan(center, rx, ry, rz) {
            if dist_manhattan(pos, center) > max {
                break;
            }
            if region.is(pos, self.target) {
                region.set_and_update(pos, self.state);
                any = true;
            }
        }
        any
    }
}

/// `RandomNeighborSpreadFeature`.
pub struct RandomNeighborSpread {
    block: Arc<StateProvider>,
    accepted: Arc<StateSet>,
    can_replace: BlockPredicate,
    attempts: IntProvider,
    xz_offset: IntProvider,
    y_offset: IntProvider,
}

impl RandomNeighborSpread {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            block: p.provider(field(json, "block")?)?,
            accepted: p.block_set(field(json, "accepted_neighbors")?)?,
            can_replace: BlockPredicate::parse(p, field(json, "can_replace")?)?,
            attempts: IntProvider::parse(field(json, "attempts")?)?,
            xz_offset: IntProvider::parse(field(json, "xz_offset")?)?,
            y_offset: IntProvider::parse(field(json, "y_offset")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let s = self.block.get(region, random, origin);
        region.set(origin, s, 2);
        let attempts = self.attempts.sample(random);
        for _ in 0..attempts {
            let dx = self.xz_offset.sample(random);
            let dy = self.y_offset.sample(random);
            let dz = self.xz_offset.sample(random);
            let pos = origin.offset(dx, dy, dz);
            if self.can_replace.test(region, pos) {
                let mut neighbours = 0;
                for d in Direction::ALL {
                    if self.accepted.contains(region.get(pos.relative(d, 1))) {
                        neighbours += 1;
                    }
                    if neighbours > 1 {
                        break;
                    }
                }
                if neighbours == 1 {
                    let s = self.block.get(region, random, pos);
                    region.set(pos, s, 2);
                }
            }
        }
        true
    }
}

/// `SteppedColumnClusterFeature` (basalt columns).
pub struct SteppedColumnCluster {
    block: Arc<StateProvider>,
    continue_through: BlockPredicate,
    can_replace: BlockPredicate,
    cannot_place_on: Arc<StateSet>,
    cluster_reach: IntProvider,
    column_count: IntProvider,
    column_reach: IntProvider,
    height: IntProvider,
}

impl SteppedColumnCluster {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            block: p.provider(field(json, "block")?)?,
            continue_through: BlockPredicate::parse(p, field(json, "continue_through")?)?,
            can_replace: BlockPredicate::parse(p, field(json, "can_replace")?)?,
            cannot_place_on: p.block_set(field(json, "cannot_place_on")?)?,
            cluster_reach: IntProvider::parse(field(json, "cluster_reach")?)?,
            column_count: IntProvider::parse(field(json, "column_count")?)?,
            column_reach: IntProvider::parse(field(json, "column_reach")?)?,
            height: IntProvider::parse(field(json, "height")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if !self.can_place_at(region, origin) {
            return false;
        }
        let column_height = self.height.sample(random);
        let reach = column_height.min(self.cluster_reach.sample(random));
        let count = self.column_count.sample(random);
        let mut placed = false;
        // `BlockPos.randomBetweenClosed` over a flat square.
        let width = 2 * reach + 1;
        for _ in 0..count {
            let x = origin.x - reach + random.next_int_bounded(width);
            let y = origin.y + random.next_int_bounded(1);
            let z = origin.z - reach + random.next_int_bounded(width);
            let pos = BlockPos::new(x, y, z);
            let blocks_y = column_height - dist_manhattan(pos, origin);
            if blocks_y >= 0 {
                let column_reach = self.column_reach.sample(random);
                placed |= self.place_column(region, random, pos, blocks_y, column_reach);
            }
        }
        placed
    }

    fn place_column(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        height: i32,
        reach: i32,
    ) -> bool {
        let mut any = false;
        // `BlockPos.betweenClosed`: x fastest, then z (y is fixed).
        for z in origin.z - reach..=origin.z + reach {
            for x in origin.x - reach..=origin.x + reach {
                let pos = BlockPos::new(x, origin.y, z);
                let step_limit = dist_manhattan(pos, origin);
                let column = if self.can_replace.test(region, pos) {
                    self.find_surface(region, pos, step_limit)
                } else {
                    self.find_air(region, pos, step_limit)
                };
                let Some(mut cursor) = column else { continue };
                let mut blocks_y = height - step_limit / 2;
                while blocks_y >= 0 {
                    if self.can_replace.test(region, cursor) {
                        let s = self.block.get(region, random, cursor);
                        region.set_and_update(cursor, s);
                        cursor = cursor.above();
                        any = true;
                    } else {
                        if !self.continue_through.test(region, cursor) {
                            break;
                        }
                        cursor = cursor.above();
                    }
                    blocks_y -= 1;
                }
            }
        }
        any
    }

    fn find_surface(&self, region: &Region, mut cursor: BlockPos, mut limit: i32) -> Option<BlockPos> {
        while cursor.y > region.min_y() + 1 && limit > 0 {
            limit -= 1;
            if self.can_place_at(region, cursor) {
                return Some(cursor);
            }
            cursor = cursor.below();
        }
        None
    }

    fn can_place_at(&self, region: &Region, pos: BlockPos) -> bool {
        if !self.can_replace.test(region, pos) {
            return false;
        }
        let below = region.get(pos.below());
        !region.env.blocks.flags(below).is_air() && !self.cannot_place_on.contains(below)
    }

    fn find_air(&self, region: &Region, mut cursor: BlockPos, mut limit: i32) -> Option<BlockPos> {
        while cursor.y <= region.max_y() && limit > 0 {
            limit -= 1;
            let s = region.get(cursor);
            if self.cannot_place_on.contains(s) {
                return None;
            }
            if region.env.blocks.flags(s).is_air() {
                return Some(cursor);
            }
            cursor = cursor.above();
        }
        None
    }
}

/// `HugeFungusFeature`.
pub struct HugeFungus {
    valid_base: BlockId,
    stem: BlockState,
    hat: BlockState,
    decor: BlockState,
    replaceable: BlockPredicate,
    planted: bool,
}

impl HugeFungus {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let base = p.state(field(json, "valid_base_block")?)?;
        Ok(Self {
            valid_base: p.reg.blocks.block_of(base),
            stem: p.state(field(json, "stem_state")?)?,
            hat: p.state(field(json, "hat_state")?)?,
            decor: p.state(field(json, "decor_state")?)?,
            replaceable: BlockPredicate::parse(p, field(json, "replaceable_blocks")?)?,
            planted: json.get("planted").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if !region.is(origin.below(), self.valid_base) {
            return false;
        }
        let mut total = mth_next_int(random, 4, 13);
        if random.next_int_bounded(12) == 0 {
            total *= 2;
        }
        if !self.planted && origin.y + total + 1 >= region.env.gen_depth {
            return false;
        }
        let huge = !self.planted && random.next_float() < 0.06;
        region.set(origin, BlockState::AIR, 260);
        self.place_stem(region, random, origin, total, huge);
        self.place_hat(region, random, origin, total, huge);
        true
    }

    fn is_replaceable(&self, region: &Region, pos: BlockPos, check_plants: bool) -> bool {
        region.flags(pos).can_be_replaced() || (check_plants && self.replaceable.test(region, pos))
    }

    /// `WorldGenRegion.destroyBlock(pos, true)` without drops (only used for planted fungi).
    fn destroy(region: &mut Region, pos: BlockPos) {
        if !region.flags(pos).is_air() {
            // Planted fungi only come from bone meal; source fluids are enough here.
            let env = region.env;
            let fluid = region.fluid(pos);
            let state = if fluid.is_water() {
                env.blocks.block(env.b.water).default_state
            } else if fluid.is_lava() {
                env.blocks.block(env.b.lava).default_state
            } else {
                BlockState::AIR
            };
            region.set(pos, state, 3);
        }
    }

    fn place_stem(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        total: i32,
        huge: bool,
    ) {
        let r: i32 = if huge { 1 } else { 0 };
        for dx in -r..=r {
            for dz in -r..=r {
                let corner = huge && dx.abs() == r && dz.abs() == r;
                for dy in 0..total {
                    let pos = origin.offset(dx, dy, dz);
                    if self.is_replaceable(region, pos, true) {
                        if self.planted {
                            if !region.flags(pos.below()).is_air() {
                                Self::destroy(region, pos);
                            }
                            region.set_and_update(pos, self.stem);
                        } else if corner {
                            if random.next_float() < 0.1 {
                                region.set_and_update(pos, self.stem);
                            }
                        } else {
                            region.set_and_update(pos, self.stem);
                        }
                    }
                }
            }
        }
    }

    fn place_hat(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        total: i32,
        huge: bool,
    ) {
        let vines = region.env.blocks.block_of(self.hat) == region.env.b.nether_wart_block;
        let hat_height = (random.next_int_bounded(1 + total / 3) + 5).min(total);
        let start = total - hat_height;
        for dy in start..=total {
            let mut radius = if dy < total - random.next_int_bounded(3) {
                2
            } else {
                1
            };
            if hat_height > 8 && dy < start + 4 {
                radius = 3;
            }
            if huge {
                radius += 1;
            }
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let edge_x = dx == -radius || dx == radius;
                    let edge_z = dz == -radius || dz == radius;
                    let inside = !edge_x && !edge_z && dy != total;
                    let corner = edge_x && edge_z;
                    let bottom = dy < start + 3;
                    let pos = origin.offset(dx, dy, dz);
                    if !self.is_replaceable(region, pos, false) {
                        continue;
                    }
                    if self.planted && !region.flags(pos.below()).is_air() {
                        Self::destroy(region, pos);
                    }
                    if bottom {
                        if !inside {
                            self.place_drop(region, random, pos, vines);
                        }
                    } else if inside {
                        self.place_hat_block(region, random, pos, 0.1, 0.2, if vines { 0.1 } else { 0.0 });
                    } else if corner {
                        self.place_hat_block(region, random, pos, 0.01, 0.7, if vines { 0.083 } else { 0.0 });
                    } else {
                        self.place_hat_block(
                            region,
                            random,
                            pos,
                            5.0E-4,
                            0.98,
                            if vines { 0.07 } else { 0.0 },
                        );
                    }
                }
            }
        }
    }

    fn place_hat_block(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        pos: BlockPos,
        decor: f32,
        hat: f32,
        vines: f32,
    ) {
        if random.next_float() < decor {
            region.set_and_update(pos, self.decor);
        } else if random.next_float() < hat {
            region.set_and_update(pos, self.hat);
            if random.next_float() < vines {
                weeping_vines(region, random, pos);
            }
        }
    }

    fn place_drop(&self, region: &mut Region, random: &mut dyn RandomSource, pos: BlockPos, vines: bool) {
        if region.is(pos.below(), region.env.blocks.block_of(self.hat)) {
            region.set_and_update(pos, self.hat);
        } else if (random.next_float() as f64) < 0.15 {
            region.set_and_update(pos, self.hat);
            if vines && random.next_int_bounded(11) == 0 {
                weeping_vines(region, random, pos);
            }
        }
    }
}

/// `HugeFungusFeature.tryPlaceWeepingVines`.
fn weeping_vines(region: &mut Region, random: &mut dyn RandomSource, hat: BlockPos) {
    let start = hat.below();
    if !region.is_empty_block(start) {
        return;
    }
    let mut goal = mth_next_int(random, 1, 5);
    if random.next_int_bounded(7) == 0 {
        goal *= 2;
    }
    let env = region.env;
    let mut pos = start;
    for h in 0..=goal {
        if region.is_empty_block(pos) {
            if h == goal || !region.is_empty_block(pos.below()) {
                let age = mth_next_int(random, 23, 25);
                let s = env.blocks.block(env.b.weeping_vines).default_state;
                let s = env.blocks.with_property(s, "age", &age.to_string()).unwrap();
                region.set(pos, s, 2);
                break;
            }
            region.set(pos, env.blocks.block(env.b.weeping_vines_plant).default_state, 2);
        }
        pos = pos.below();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manhattan_order_starts_at_origin_and_mirrors_z() {
        let o = BlockPos::new(0, 0, 0);
        let v: Vec<_> = manhattan_ordered(o, 1, 1, 1, 3)
            .take(8)
            .map(|p| (p.x, p.y, p.z))
            .collect();
        assert_eq!(
            v,
            vec![
                (0, 0, 0),
                (-1, 0, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
                (0, 1, 0),
                (1, 0, 0),
                (-1, -1, 0)
            ]
        );
        assert_eq!(manhattan_ordered(o, 1, 1, 1, 3).count(), 27);
    }
}
