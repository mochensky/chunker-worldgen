//! `SculkPatchFeature` with the world generation `SculkSpreader` and the `SculkBehaviour` of sculk
//! and sculk veins.

use anyhow::Result;
use chunker_data::blocks::BlockState;
use serde_json::Value;

use super::multiface::Multiface;
use crate::codec::i32_field;
use crate::feature::Region;
use crate::feature::behaviour::{can_attach_to, is_face_sturdy};
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};
use crate::structure::util::shuffle;

/// `SculkSpreader.createWorldGenSpreader()` constants.
const GROWTH_SPAWN_COST: i32 = 50;
const NO_GROWTH_RADIUS: i32 = 1;
const CHARGE_DECAY_RATE: i32 = 5;
const ADDITIONAL_DECAY_RATE: i32 = 10;

pub struct SculkPatch {
    charge_count: i32,
    amount_per_charge: i32,
    spread_attempts: i32,
    growth_rounds: i32,
    spread_rounds: i32,
}

#[derive(Clone)]
struct Cursor {
    pos: BlockPos,
    charge: i32,
    update_delay: i32,
    decay_delay: i32,
    /// `facings` (`None` until the cursor first moves onto a sculk block).
    facings: Option<Vec<Direction>>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Behaviour {
    Default,
    Sculk,
    Vein,
}

fn behaviour(region: &Region, s: BlockState) -> Behaviour {
    let id = region.env.blocks.block_of(s);
    if id == region.env.b.sculk {
        Behaviour::Sculk
    } else if id == region.env.b.sculk_vein {
        Behaviour::Vein
    } else {
        Behaviour::Default
    }
}

fn has_face(region: &Region, s: BlockState, d: Direction) -> bool {
    Multiface::has_face(region, s, d)
}

/// `MultifaceBlock.availableFaces` (EnumSet order).
fn available_faces(region: &Region, s: BlockState) -> Vec<Direction> {
    Direction::ALL
        .into_iter()
        .filter(|d| has_face(region, s, *d))
        .collect()
}

impl SculkPatch {
    pub fn parse(json: &Value) -> Result<Self> {
        Ok(Self {
            charge_count: i32_field(json, "charge_count")?,
            amount_per_charge: i32_field(json, "amount_per_charge")?,
            spread_attempts: i32_field(json, "spread_attempts")?,
            growth_rounds: i32_field(json, "growth_rounds")?,
            spread_rounds: i32_field(json, "spread_rounds")?,
        })
    }

    fn can_spread_from(region: &Region, origin: BlockPos) -> bool {
        let start = region.get(origin);
        if behaviour(region, start) != Behaviour::Default {
            return true;
        }
        let f = region.env.blocks.flags(start);
        if !f.is_air() && !(region.env.blocks.block_of(start) == region.env.b.water && f.is_fluid_source()) {
            return false;
        }
        Direction::ALL
            .iter()
            .any(|d| region.flags(origin.relative(*d, 1)).is_collision_full_block())
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if !Self::can_spread_from(region, origin) {
            return false;
        }
        let mut cursors: Vec<Cursor> = Vec::new();
        for round in 0..self.spread_rounds + self.growth_rounds {
            for _ in 0..self.charge_count {
                // `addCursors(origin, amountPerCharge)`
                let mut charge = self.amount_per_charge;
                while charge > 0 {
                    let c = charge.min(1000);
                    if cursors.len() < 32 {
                        cursors.push(Cursor {
                            pos: origin,
                            charge: c,
                            update_delay: 0,
                            decay_delay: 1,
                            facings: None,
                        });
                    }
                    charge -= c;
                }
            }
            let spread_veins = round < self.spread_rounds;
            for _ in 0..self.spread_attempts {
                update_cursors(region, &mut cursors, origin, random, spread_veins);
            }
            cursors.clear();
        }
        true
    }
}

/// `SculkSpreader.updateCursors` (world generation: no merging).
fn update_cursors(
    region: &mut Region,
    cursors: &mut Vec<Cursor>,
    origin: BlockPos,
    random: &mut dyn RandomSource,
    spread_veins: bool,
) {
    if cursors.is_empty() {
        return;
    }
    let mut processed = Vec::with_capacity(cursors.len());
    for mut c in std::mem::take(cursors) {
        let chessboard = (c.pos.x - origin.x)
            .abs()
            .max((c.pos.y - origin.y).abs())
            .max((c.pos.z - origin.z).abs());
        if chessboard > 1024 {
            continue;
        }
        update_cursor(region, &mut c, origin, random, spread_veins);
        if c.charge > 0 {
            processed.push(c);
        }
    }
    *cursors = processed;
}

fn update_cursor(
    region: &mut Region,
    c: &mut Cursor,
    origin: BlockPos,
    random: &mut dyn RandomSource,
    spread_veins: bool,
) {
    if c.charge <= 0 {
        return;
    }
    if c.update_delay > 0 {
        c.update_delay -= 1;
        return;
    }
    let mut state = region.get(c.pos);
    let mut beh = behaviour(region, state);
    if spread_veins && attempt_spread_vein(region, beh, c.pos, state, c.facings.as_deref()) {
        // `canChangeBlockStateOnSpread`: false only for sculk.
        if beh != Behaviour::Sculk {
            state = region.get(c.pos);
            beh = behaviour(region, state);
        }
    }
    c.charge = attempt_use_charge(region, beh, c, origin, random, spread_veins);
    if c.charge <= 0 {
        on_discharged(region, beh, state, c.pos, random);
        return;
    }
    match valid_movement_pos(region, c.pos, random, origin) {
        Some(to) => {
            on_discharged(region, beh, state, c.pos, random);
            c.pos = to;
            state = region.get(to);
        }
        None => {
            on_discharged(region, beh, state, c.pos, random);
            c.charge = 0;
            return;
        }
    }
    let moved_beh = behaviour(region, state);
    if moved_beh != Behaviour::Default {
        c.facings = Some(available_faces(region, state));
    }
    // `updateDecayDelay` / `getSculkSpreadDelay` of the behaviour before moving.
    c.decay_delay = if beh == Behaviour::Default {
        (c.decay_delay - 1).max(0)
    } else {
        1
    };
    c.update_delay = 1;
}

fn attempt_spread_vein(
    region: &mut Region,
    beh: Behaviour,
    pos: BlockPos,
    state: BlockState,
    facings: Option<&[Direction]>,
) -> bool {
    let env = region.env;
    if beh == Behaviour::Default {
        match facings {
            None => {
                let here = region.get(pos);
                return Multiface::sculk_vein_same_space(env).spread_all(region, here, pos, true) > 0;
            }
            Some(f) if !f.is_empty() => {
                let fl = env.blocks.flags(state);
                if !fl.is_air() && fl.fluid() != chunker_data::blocks::Fluid::Water {
                    return false;
                }
                return regrow(region, pos, state, f);
            }
            Some(_) => {}
        }
    }
    Multiface::new(env, env.b.sculk_vein).spread_all(region, state, pos, true) > 0
}

/// `SculkVeinBlock.regrow`.
fn regrow(region: &mut Region, pos: BlockPos, existing: BlockState, faces: &[Direction]) -> bool {
    let env = region.env;
    let blocks = &env.blocks;
    let mut s = blocks.block(env.b.sculk_vein).default_state;
    let mut any = false;
    for &f in faces {
        if can_attach_to(region, f, pos.relative(f, 1)) {
            s = blocks.with_property(s, f.name(), "true").unwrap();
            any = true;
        }
    }
    if !any {
        return false;
    }
    if blocks.flags(existing).fluid() != chunker_data::blocks::Fluid::Empty {
        s = blocks.with_property(s, "waterlogged", "true").unwrap();
    }
    region.set_and_update(pos, s);
    true
}

fn attempt_use_charge(
    region: &mut Region,
    beh: Behaviour,
    c: &Cursor,
    origin: BlockPos,
    random: &mut dyn RandomSource,
    spread_veins: bool,
) -> i32 {
    match beh {
        Behaviour::Default => {
            if c.decay_delay > 0 {
                c.charge
            } else {
                0
            }
        }
        Behaviour::Sculk => sculk_use_charge(region, c, origin, random),
        Behaviour::Vein => {
            if spread_veins && attempt_place_sculk(region, c.pos, random) {
                c.charge - 1
            } else if random.next_int_bounded(CHARGE_DECAY_RATE) == 0 {
                crate::math::floor_f32(c.charge as f32 * 0.5)
            } else {
                c.charge
            }
        }
    }
}

fn sculk_use_charge(region: &mut Region, c: &Cursor, origin: BlockPos, random: &mut dyn RandomSource) -> i32 {
    let charge = c.charge;
    if charge == 0 || random.next_int_bounded(CHARGE_DECAY_RATE) != 0 {
        return charge;
    }
    let pos = c.pos;
    let dist_sqr = |a: BlockPos, b: BlockPos| {
        let (x, y, z) = ((a.x - b.x) as f64, (a.y - b.y) as f64, (a.z - b.z) as f64);
        x * x + y * y + z * z
    };
    let close = dist_sqr(pos, origin) < (NO_GROWTH_RADIUS * NO_GROWTH_RADIUS) as f64;
    if !close && can_place_growth(region, pos) {
        if random.next_int_bounded(GROWTH_SPAWN_COST) < charge {
            let at = pos.above();
            let env = region.env;
            let blocks = &env.blocks;
            let mut s = if random.next_int_bounded(11) == 0 {
                blocks
                    .with_property(
                        blocks.block(env.b.sculk_shrieker).default_state,
                        "can_summon",
                        "true",
                    )
                    .unwrap()
            } else {
                blocks.block(env.b.sculk_sensor).default_state
            };
            if blocks.has_property(s, "waterlogged") && region.fluid(at) != chunker_data::blocks::Fluid::Empty
            {
                s = blocks.with_property(s, "waterlogged", "true").unwrap();
            }
            region.set_and_update(at, s);
        }
        return 0.max(charge - GROWTH_SPAWN_COST);
    }
    if random.next_int_bounded(ADDITIONAL_DECAY_RATE) != 0 {
        return charge;
    }
    let penalty = if close {
        1
    } else {
        // `getDecayPenalty`
        let outer = crate::math::square_f32(dist_sqr(pos, origin).sqrt() as f32 - NO_GROWTH_RADIUS as f32);
        let max_reach = (24 - NO_GROWTH_RADIUS) * (24 - NO_GROWTH_RADIUS);
        let factor = 1.0f32.min(outer / max_reach as f32);
        1.max((charge as f32 * factor * 0.5) as i32)
    };
    charge - penalty
}

/// `SculkBlock.canPlaceGrowth`.
fn can_place_growth(region: &Region, pos: BlockPos) -> bool {
    let above = region.get(pos.above());
    let env = region.env;
    let af = env.blocks.flags(above);
    if !(af.is_air()
        || (env.blocks.block_of(above) == env.b.water && af.fluid() == chunker_data::blocks::Fluid::Water))
    {
        return false;
    }
    let mut matched = 0;
    for x in -4..=4 {
        for y in 0..=2 {
            for z in -4..=4 {
                if env
                    .t
                    .sculk_growth_inhibitors
                    .contains(region.get(pos.offset(x, y, z)))
                {
                    matched += 1;
                    if matched > 2 {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// `SculkVeinBlock.attemptPlaceSculk`.
fn attempt_place_sculk(region: &mut Region, pos: BlockPos, random: &mut dyn RandomSource) -> bool {
    let env = region.env;
    let state = region.get(pos);
    let mut dirs = Direction::ALL.to_vec();
    shuffle(&mut dirs, random);
    for support in dirs {
        if !has_face(region, state, support) {
            continue;
        }
        let sp = pos.relative(support, 1);
        if !env.t.sculk_replaceable_world_gen.contains(region.get(sp)) {
            continue;
        }
        let sculk = env.blocks.block(env.b.sculk).default_state;
        region.set_and_update(sp, sculk);
        Multiface::new(env, env.b.sculk_vein).spread_all(region, sculk, sp, true);
        let skip = support.opposite();
        for d in Direction::ALL {
            if d == skip {
                continue;
            }
            let vp = sp.relative(d, 1);
            let vs = region.get(vp);
            if env.blocks.block_of(vs) == env.b.sculk_vein {
                on_discharged(region, Behaviour::Vein, vs, vp, random);
            }
        }
        return true;
    }
    false
}

/// `onDischarged` (only sculk veins do something).
fn on_discharged(
    region: &mut Region,
    beh: Behaviour,
    mut state: BlockState,
    pos: BlockPos,
    _random: &mut dyn RandomSource,
) {
    let env = region.env;
    if beh != Behaviour::Vein || env.blocks.block_of(state) != env.b.sculk_vein {
        return;
    }
    let blocks = &env.blocks;
    for d in Direction::ALL {
        if has_face(region, state, d) && region.is(pos.relative(d, 1), env.b.sculk) {
            state = blocks.with_property(state, d.name(), "false").unwrap();
        }
    }
    if !Direction::ALL.iter().any(|d| has_face(region, state, *d)) {
        state = if region.fluid(pos) == chunker_data::blocks::Fluid::Empty {
            BlockState::AIR
        } else {
            blocks.block(env.b.water).default_state
        };
    }
    region.set_and_update(pos, state);
}

/// `ChargeCursor.getValidMovementPos`.
fn valid_movement_pos(
    region: &Region,
    pos: BlockPos,
    random: &mut dyn RandomSource,
    origin: BlockPos,
) -> Option<BlockPos> {
    let env = region.env;
    // `NON_CORNER_NEIGHBOURS` in `betweenClosed` order (x fastest, then y, then z).
    let mut offsets = Vec::with_capacity(18);
    for z in -1..=1i32 {
        for y in -1..=1i32 {
            for x in -1..=1i32 {
                if (x == 0 || y == 0 || z == 0) && !(x == 0 && y == 0 && z == 0) {
                    offsets.push((x, y, z));
                }
            }
        }
    }
    shuffle(&mut offsets, random);
    let mut found = pos;
    for (x, y, z) in offsets {
        let n = pos.offset(x, y, z);
        let dx = origin.x - n.x;
        let dz = origin.z - n.z;
        if dx * dx + dz * dz > 144 {
            continue;
        }
        let s = region.get(n);
        if behaviour(region, s) == Behaviour::Default || !movement_unobstructed(region, pos, n) {
            continue;
        }
        found = n;
        // `SculkVeinBlock.hasSubstrateAccess`
        if env.blocks.block_of(s) == env.b.sculk_vein
            && Direction::ALL.iter().any(|d| {
                has_face(region, s, *d) && env.t.sculk_replaceable.contains(region.get(n.relative(*d, 1)))
            })
        {
            break;
        }
    }
    if found == pos { None } else { Some(found) }
}

fn movement_unobstructed(region: &Region, from: BlockPos, to: BlockPos) -> bool {
    if from.dist_manhattan(to) == 1 {
        return true;
    }
    let delta = to.sub(from);
    let dx = Direction::from_axis(crate::structure::geom::Axis::X, delta.x >= 0);
    let dy = Direction::from_axis(crate::structure::geom::Axis::Y, delta.y >= 0);
    let dz = Direction::from_axis(crate::structure::geom::Axis::Z, delta.z >= 0);
    let free = |d: Direction| {
        let t = from.relative(d, 1);
        !is_face_sturdy(region.env, region.get(t), d.opposite())
    };
    if delta.x == 0 {
        free(dy) || free(dz)
    } else if delta.y == 0 {
        free(dx) || free(dz)
    } else {
        free(dx) || free(dy)
    }
}
