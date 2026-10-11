//! `MultifaceGrowthFeature` and the `MultifaceSpreader` of glow lichen and sculk veins.

use std::sync::Arc;

use anyhow::Result;
use chunker_data::blocks::{BlockId, BlockState, Fluid};
use serde_json::Value;

use crate::block_set::StateSet;
use crate::codec::{field, opt_bool, opt_f32, opt_i32, str_field};
use crate::feature::Region;
use crate::feature::behaviour::can_attach_to;
use crate::feature::registry::Parser;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};
use crate::structure::util::shuffle;

/// `MultifaceSpreader.SpreadType`.
#[derive(Clone, Copy)]
enum SpreadType {
    SamePosition,
    SamePlane,
    WrapAround,
}

const DEFAULT_ORDER: [SpreadType; 3] = [
    SpreadType::SamePosition,
    SpreadType::SamePlane,
    SpreadType::WrapAround,
];

impl SpreadType {
    fn pos(self, pos: BlockPos, dir: Direction, from: Direction) -> (BlockPos, Direction) {
        match self {
            SpreadType::SamePosition => (pos, dir),
            SpreadType::SamePlane => (pos.relative(dir, 1), from),
            SpreadType::WrapAround => (pos.relative(dir, 1).relative(from, 1), dir.opposite()),
        }
    }
}

/// A `MultifaceBlock` with its spreader config (`DefaultSpreaderConfig` or `SculkVeinSpreaderConfig`).
#[derive(Clone, Copy)]
pub struct Multiface {
    pub block: BlockId,
    sculk_vein: bool,
    /// `SculkVeinBlock.sameSpaceSpreader` (only `SAME_POSITION`).
    same_space: bool,
}

fn face(d: Direction) -> &'static str {
    d.name()
}

impl Multiface {
    pub fn new(region_env: &crate::feature::FeatureEnv, block: BlockId) -> Self {
        Self {
            block,
            sculk_vein: block == region_env.b.sculk_vein,
            same_space: false,
        }
    }

    /// `SculkVeinBlock.getSameSpaceSpreader()`.
    pub fn sculk_vein_same_space(env: &crate::feature::FeatureEnv) -> Self {
        Self {
            block: env.b.sculk_vein,
            sculk_vein: true,
            same_space: true,
        }
    }

    /// `MultifaceBlock.hasFace`.
    pub fn has_face(region: &Region, state: BlockState, d: Direction) -> bool {
        region.env.blocks.get_property(state, face(d)) == Some("true")
    }

    /// `isValidStateForPlacement`.
    fn valid_for_placement(&self, region: &Region, old: BlockState, pos: BlockPos, d: Direction) -> bool {
        let blocks = &region.env.blocks;
        if blocks.block_of(old) == self.block && Self::has_face(region, old, d) {
            return false;
        }
        can_attach_to(region, d, pos.relative(d, 1))
    }

    /// `MultifaceBlock.getStateForPlacement(oldState, level, pos, direction)`.
    pub fn state_for_placement(
        &self,
        region: &Region,
        old: BlockState,
        pos: BlockPos,
        d: Direction,
    ) -> Option<BlockState> {
        if !self.valid_for_placement(region, old, pos, d) {
            return None;
        }
        let blocks = &region.env.blocks;
        let of = blocks.flags(old);
        let base = if blocks.block_of(old) == self.block {
            old
        } else if of.fluid() == Fluid::Water {
            blocks
                .with_property(blocks.block(self.block).default_state, "waterlogged", "true")
                .unwrap()
        } else {
            blocks.block(self.block).default_state
        };
        blocks.with_property(base, face(d), "true")
    }

    fn spread_types(&self) -> &'static [SpreadType] {
        if self.same_space {
            &DEFAULT_ORDER[..1]
        } else {
            &DEFAULT_ORDER
        }
    }

    fn other_block_valid_as_source(&self, region: &Region, state: BlockState) -> bool {
        self.sculk_vein && region.env.blocks.block_of(state) != region.env.b.sculk_vein
    }

    /// `stateCanBeReplaced`.
    fn state_can_be_replaced(
        &self,
        region: &Region,
        source: BlockPos,
        pos: BlockPos,
        d: Direction,
        existing: BlockState,
    ) -> bool {
        let env = region.env;
        let blocks = &env.blocks;
        let default = |existing: BlockState| {
            let f = blocks.flags(existing);
            f.is_air()
                || blocks.block_of(existing) == self.block
                || (blocks.block_of(existing) == env.b.water && f.is_fluid_source())
        };
        if !self.sculk_vein {
            return default(existing);
        }
        let against = blocks.block_of(region.get(pos.relative(d, 1)));
        if against == env.b.sculk
            || against == env.b.sculk_catalyst
            || blocks.name_of(blocks.block(against).default_state).path() == "moving_piston"
        {
            return false;
        }
        if source.dist_manhattan(pos) == 2 {
            let n = source.relative(d.opposite(), 1);
            if crate::feature::behaviour::is_face_sturdy(env, region.get(n), d) {
                return false;
            }
        }
        let f = blocks.flags(existing);
        if f.fluid() != Fluid::Empty && f.fluid() != Fluid::Water {
            return false;
        }
        if env.t.fire.contains(existing) {
            return false;
        }
        f.can_be_replaced() || default(existing)
    }

    fn can_spread_into(&self, region: &Region, source: BlockPos, pos: BlockPos, d: Direction) -> bool {
        let existing = region.get(pos);
        self.state_can_be_replaced(region, source, pos, d, existing)
            && self.valid_for_placement(region, existing, pos, d)
    }

    /// `getSpreadFromFaceTowardDirection`.
    fn spread_from_face_toward(
        &self,
        region: &Region,
        state: BlockState,
        pos: BlockPos,
        start: Direction,
        dir: Direction,
    ) -> Option<(BlockPos, Direction)> {
        if dir.axis() == start.axis() {
            return None;
        }
        if self.other_block_valid_as_source(region, state)
            || (Self::has_face(region, state, start) && !Self::has_face(region, state, dir))
        {
            for t in self.spread_types() {
                let (p, f) = t.pos(pos, dir, start);
                if self.can_spread_into(region, pos, p, f) {
                    return Some((p, f));
                }
            }
        }
        None
    }

    /// `spreadToFace` via `placeBlock`.
    fn spread_to_face(&self, region: &mut Region, pos: BlockPos, d: Direction, post_process: bool) -> bool {
        let old = region.get(pos);
        let Some(s) = self.state_for_placement(region, old, pos, d) else {
            return false;
        };
        if post_process {
            region.mark_post_processing(pos);
        }
        region.set(pos, s, 2)
    }

    /// `spreadFromFaceTowardRandomDirection`.
    pub fn spread_from_face_toward_random(
        &self,
        region: &mut Region,
        state: BlockState,
        pos: BlockPos,
        start: Direction,
        random: &mut dyn RandomSource,
        post_process: bool,
    ) -> bool {
        let mut dirs = Direction::ALL.to_vec();
        shuffle(&mut dirs, random);
        for d in dirs {
            if let Some((p, f)) = self.spread_from_face_toward(region, state, pos, start, d)
                && self.spread_to_face(region, p, f, post_process)
            {
                return true;
            }
        }
        false
    }

    /// `spreadAll`.
    pub fn spread_all(
        &self,
        region: &mut Region,
        state: BlockState,
        pos: BlockPos,
        post_process: bool,
    ) -> i64 {
        let mut n = 0;
        for start in Direction::ALL {
            if !(self.other_block_valid_as_source(region, state) || Self::has_face(region, state, start)) {
                continue;
            }
            for d in Direction::ALL {
                if let Some((p, f)) = self.spread_from_face_toward(region, state, pos, start, d)
                    && self.spread_to_face(region, p, f, post_process)
                {
                    n += 1;
                }
            }
        }
        n
    }
}

/// `MultifaceGrowthFeature`.
pub struct MultifaceGrowth {
    block: BlockId,
    search_range: i32,
    floor: bool,
    ceiling: bool,
    wall: bool,
    chance_of_spreading: f32,
    can_be_placed_on: Arc<StateSet>,
}

impl MultifaceGrowth {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            block: p.block(str_field(json, "block")?)?,
            search_range: opt_i32(json, "search_range", 10)?,
            floor: opt_bool(json, "can_place_on_floor", false)?,
            ceiling: opt_bool(json, "can_place_on_ceiling", false)?,
            wall: opt_bool(json, "can_place_on_wall", false)?,
            chance_of_spreading: opt_f32(json, "chance_of_spreading", 0.5)?,
            can_be_placed_on: p.block_set(field(json, "can_be_placed_on")?)?,
        })
    }

    fn valid_directions(&self) -> Vec<Direction> {
        let mut v = Vec::with_capacity(6);
        if self.ceiling {
            v.push(Direction::Up);
        }
        if self.floor {
            v.push(Direction::Down);
        }
        if self.wall {
            v.extend(Direction::HORIZONTAL);
        }
        v
    }

    fn air_or_water(region: &Region, s: BlockState) -> bool {
        region.env.blocks.flags(s).is_air() || region.env.blocks.block_of(s) == region.env.b.water
    }

    fn place_growth(
        &self,
        m: &Multiface,
        region: &mut Region,
        pos: BlockPos,
        old: BlockState,
        random: &mut dyn RandomSource,
        dirs: &[Direction],
    ) -> bool {
        for &d in dirs {
            let n = region.get(pos.relative(d, 1));
            if self.can_be_placed_on.contains(n) {
                let Some(new) = m.state_for_placement(region, old, pos, d) else {
                    return false;
                };
                region.set_and_update(pos, new);
                region.mark_post_processing(pos);
                if random.next_float() < self.chance_of_spreading {
                    m.spread_from_face_toward_random(region, new, pos, d, random, true);
                }
                return true;
            }
        }
        false
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if !Self::air_or_water(region, region.get(origin)) {
            return false;
        }
        let m = Multiface::new(region.env, self.block);
        let mut dirs = self.valid_directions();
        shuffle(&mut dirs, random);
        let here = region.get(origin);
        if self.place_growth(&m, region, origin, here, random, &dirs) {
            return true;
        }
        for &search in &dirs {
            let mut except: Vec<Direction> = self
                .valid_directions()
                .into_iter()
                .filter(|d| *d != search.opposite())
                .collect();
            shuffle(&mut except, random);
            for _ in 0..self.search_range {
                // Vanilla resets the cursor to `origin + direction` each step (`setWithOffset(origin, ...)`).
                let pos = origin.relative(search, 1);
                let s = region.get(pos);
                if !Self::air_or_water(region, s) && region.env.blocks.block_of(s) != self.block {
                    break;
                }
                if self.place_growth(&m, region, pos, s, random, &except) {
                    return true;
                }
            }
        }
        false
    }
}
