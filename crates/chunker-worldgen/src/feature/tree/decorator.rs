//! `TreeDecorator`s.

use std::sync::Arc;

use anyhow::{Result, bail};
use chunker_data::blocks::BlockState;
use serde_json::Value;

use super::TreeCtx;
use crate::chunk::HeightmapKind;
use crate::codec::{as_array, f32_field, field, i32_field, opt_i32, str_field, type_name};
use crate::feature::provider::StateProvider;
use crate::feature::registry::Parser;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, BoundingBox, Direction};
use crate::structure::util::shuffle;

pub enum Decorator {
    TrunkVine,
    LeaveVine(f32),
    Cocoa(f32),
    Beehive(f32),
    AlterGround(Arc<StateProvider>),
    AttachedToLeaves {
        probability: f32,
        exclusion_xz: i32,
        exclusion_y: i32,
        provider: Arc<StateProvider>,
        required_empty: i32,
        directions: Vec<Direction>,
    },
    AttachedToLogs {
        probability: f32,
        provider: Arc<StateProvider>,
        directions: Vec<Direction>,
    },
    PlaceOnGround {
        tries: i32,
        radius: i32,
        height: i32,
        provider: Arc<StateProvider>,
    },
    CreakingHeart(f32),
    PaleMoss {
        leaves: f32,
        trunk: f32,
        ground: f32,
        patch: Arc<crate::feature::kinds::Feature>,
    },
    ShelfMushroom(f32),
}

fn directions(json: &Value) -> Result<Vec<Direction>> {
    as_array(field(json, "directions")?)?
        .iter()
        .map(|d| Direction::parse(crate::codec::as_str(d)?))
        .collect()
}

impl Decorator {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let prob = || f32_field(json, "probability");
        Ok(match type_name(json)?.as_str() {
            "trunk_vine" => Decorator::TrunkVine,
            "leave_vine" => Decorator::LeaveVine(prob()?),
            "cocoa" => Decorator::Cocoa(prob()?),
            "beehive" => Decorator::Beehive(prob()?),
            "alter_ground" => Decorator::AlterGround(p.provider(field(json, "provider")?)?),
            "attached_to_leaves" => Decorator::AttachedToLeaves {
                probability: prob()?,
                exclusion_xz: i32_field(json, "exclusion_radius_xz")?,
                exclusion_y: i32_field(json, "exclusion_radius_y")?,
                provider: p.provider(field(json, "block_provider")?)?,
                required_empty: i32_field(json, "required_empty_blocks")?,
                directions: directions(json)?,
            },
            "attached_to_logs" => Decorator::AttachedToLogs {
                probability: prob()?,
                provider: p.provider(field(json, "block_provider")?)?,
                directions: directions(json)?,
            },
            "place_on_ground" => Decorator::PlaceOnGround {
                tries: opt_i32(json, "tries", 128)?,
                radius: opt_i32(json, "radius", 2)?,
                height: opt_i32(json, "height", 1)?,
                provider: p.provider(field(json, "block_state_provider")?)?,
            },
            "creaking_heart" => Decorator::CreakingHeart(prob()?),
            "pale_moss" => Decorator::PaleMoss {
                leaves: f32_field(json, "leaves_probability")?,
                trunk: f32_field(json, "trunk_probability")?,
                ground: f32_field(json, "ground_probability")?,
                // `VegetationFeatures.PALE_MOSS_PATCH`
                patch: p.feature(&Value::String("minecraft:pale_moss_patch".into()))?,
            },
            "shelf_mushroom" => Decorator::ShelfMushroom(prob()?),
            other => bail!("unknown tree decorator {other} ({})", str_field(json, "type")?),
        })
    }
}

/// `TreeDecorator.Context`: the tree's positions as lists stably sorted by y.
pub struct Context<'c, 'a> {
    pub region: &'c mut crate::feature::Region<'a>,
    /// Decorations are recorded for the tree's leaf update (`TreeFeature`), not by fallen trees.
    decorations: Option<&'c mut crate::java::JavaHashSet<BlockPos>>,
    pub logs: Vec<BlockPos>,
    pub leaves: Vec<BlockPos>,
    pub roots: Vec<BlockPos>,
}

impl<'c, 'a> Context<'c, 'a> {
    pub fn new(ctx: &'c mut TreeCtx<'_, 'a>) -> Self {
        let roots = ctx.placed.roots.to_vec();
        let logs = ctx.placed.trunks.to_vec();
        let leaves = ctx.placed.foliage.to_vec();
        Self::from_lists(
            &mut *ctx.region,
            Some(&mut ctx.placed.decorations),
            logs,
            leaves,
            roots,
        )
    }

    pub fn from_lists(
        region: &'c mut crate::feature::Region<'a>,
        decorations: Option<&'c mut crate::java::JavaHashSet<BlockPos>>,
        mut logs: Vec<BlockPos>,
        mut leaves: Vec<BlockPos>,
        mut roots: Vec<BlockPos>,
    ) -> Self {
        logs.sort_by_key(|p| p.y);
        leaves.sort_by_key(|p| p.y);
        roots.sort_by_key(|p| p.y);
        Self {
            region,
            decorations,
            logs,
            leaves,
            roots,
        }
    }

    fn is_air(&self, p: BlockPos) -> bool {
        self.region.flags(p).is_air()
    }

    fn set(&mut self, p: BlockPos, s: BlockState) {
        if let Some(d) = self.decorations.as_mut() {
            d.insert(p);
        }
        self.region.set(p, s, 19);
    }

    fn place_vine(&mut self, p: BlockPos, face: &str) {
        let blocks = &self.region.env.blocks;
        let vine = blocks.block(self.region.env.b.vine).default_state;
        let s = blocks.with_property(vine, face, "true").unwrap();
        self.set(p, s);
    }

    /// `TreeFeature.getLowestTrunkOrRootOfTree`.
    fn lowest_trunk_or_root(&self) -> Vec<BlockPos> {
        if self.roots.is_empty() {
            self.logs.clone()
        } else if !self.logs.is_empty() && self.roots[0].y == self.logs[0].y {
            let mut v = self.logs.clone();
            v.extend(self.roots.iter().copied());
            v
        } else {
            self.roots.clone()
        }
    }
}

fn shuffled(list: &[BlockPos], random: &mut dyn RandomSource) -> Vec<BlockPos> {
    let mut v = list.to_vec();
    shuffle(&mut v, random);
    v
}

impl Decorator {
    pub fn place(&self, c: &mut Context, random: &mut dyn RandomSource) {
        match self {
            Decorator::TrunkVine => {
                for pos in c.logs.clone() {
                    for (d, face) in [
                        ((-1, 0), "east"),
                        ((1, 0), "west"),
                        ((0, -1), "south"),
                        ((0, 1), "north"),
                    ] {
                        if random.next_int_bounded(3) > 0 {
                            let n = pos.offset(d.0, 0, d.1);
                            if c.is_air(n) {
                                c.place_vine(n, face);
                            }
                        }
                    }
                }
            }
            Decorator::LeaveVine(prob) => {
                for pos in c.leaves.clone() {
                    for (d, face) in [
                        ((-1, 0), "east"),
                        ((1, 0), "west"),
                        ((0, -1), "south"),
                        ((0, 1), "north"),
                    ] {
                        if random.next_float() < *prob {
                            let n = pos.offset(d.0, 0, d.1);
                            if c.is_air(n) {
                                c.place_vine(n, face);
                                let mut below = n.below();
                                let mut max = 4;
                                while c.is_air(below) && max > 0 {
                                    c.place_vine(below, face);
                                    below = below.below();
                                    max -= 1;
                                }
                            }
                        }
                    }
                }
            }
            Decorator::Cocoa(prob) => {
                if random.next_float() >= *prob {
                    return;
                }
                if c.logs.is_empty() {
                    return;
                }
                let tree_y = c.logs[0].y;
                let env = c.region.env;
                let cocoa = env.blocks.block(env.b.cocoa).default_state;
                for pos in c.logs.clone() {
                    if pos.y - tree_y > 2 {
                        continue;
                    }
                    for d in Direction::HORIZONTAL {
                        if random.next_float() <= 0.25 {
                            let o = d.opposite();
                            let p = pos.offset(o.step_x(), 0, o.step_z());
                            if c.is_air(p) {
                                let age = random.next_int_bounded(3);
                                let s = env.blocks.with_property(cocoa, "age", &age.to_string()).unwrap();
                                let s = env.blocks.with_property(s, "facing", d.name()).unwrap();
                                c.set(p, s);
                            }
                        }
                    }
                }
            }
            Decorator::Beehive(prob) => {
                if c.logs.is_empty() {
                    return;
                }
                if random.next_float() >= *prob {
                    return;
                }
                let hive_y = if !c.leaves.is_empty() {
                    (c.leaves[0].y - 1).max(c.logs[0].y + 1)
                } else {
                    (c.logs[0].y + 1 + random.next_int_bounded(3)).min(c.logs[c.logs.len() - 1].y)
                };
                // `SPAWN_DIRECTIONS`: horizontal directions except north (opposite of the south facing).
                let spawn = [Direction::East, Direction::South, Direction::West];
                let mut placements: Vec<BlockPos> = c
                    .logs
                    .iter()
                    .filter(|p| p.y == hive_y)
                    .flat_map(|p| spawn.iter().map(move |d| p.relative(*d, 1)))
                    .collect();
                if placements.is_empty() {
                    return;
                }
                shuffle(&mut placements, random);
                let Some(hive) = placements
                    .into_iter()
                    .find(|p| c.is_air(*p) && c.is_air(p.relative(Direction::South, 1)))
                else {
                    return;
                };
                let env = c.region.env;
                let nest = env
                    .blocks
                    .with_property(env.blocks.block(env.b.bee_nest).default_state, "facing", "south")
                    .unwrap();
                c.set(hive, nest);
                if let Some(be) = c.region.block_entity(hive) {
                    let bees = 2 + random.next_int_bounded(2);
                    let mut list = chunker_nbt::List::new();
                    for _ in 0..bees {
                        list.push(crate::feature::behaviour::beehive_occupant(
                            random.next_int_bounded(599),
                        ));
                    }
                    be.insert("bees", list);
                }
            }
            Decorator::AlterGround(provider) => {
                let positions = c.lowest_trunk_or_root();
                if positions.is_empty() {
                    return;
                }
                let min_y = positions[0].y;
                for pos in positions.iter().filter(|p| p.y == min_y) {
                    alter_circle(c, random, provider, pos.offset(-1, 0, -1));
                    alter_circle(c, random, provider, pos.offset(2, 0, -1));
                    alter_circle(c, random, provider, pos.offset(-1, 0, 2));
                    alter_circle(c, random, provider, pos.offset(2, 0, 2));
                    for _ in 0..5 {
                        let placement = random.next_int_bounded(64);
                        let xx = placement % 8;
                        let zz = placement / 8;
                        if xx == 0 || xx == 7 || zz == 0 || zz == 7 {
                            alter_circle(c, random, provider, pos.offset(-3 + xx, 0, -3 + zz));
                        }
                    }
                }
            }
            Decorator::AttachedToLeaves {
                probability,
                exclusion_xz,
                exclusion_y,
                provider,
                required_empty,
                directions,
            } => {
                let mut blacklist = rustc_hash::FxHashSet::default();
                for leaf in shuffled(&c.leaves, random) {
                    let d = directions[random.next_int_bounded(directions.len() as i32) as usize];
                    let placement = leaf.relative(d, 1);
                    if !blacklist.contains(&placement)
                        && random.next_float() < *probability
                        && (1..=*required_empty).all(|i| c.is_air(leaf.relative(d, i)))
                    {
                        for x in -exclusion_xz..=*exclusion_xz {
                            for y in -exclusion_y..=*exclusion_y {
                                for z in -exclusion_xz..=*exclusion_xz {
                                    blacklist.insert(placement.offset(x, y, z));
                                }
                            }
                        }
                        let s = provider.get(c.region, random, placement);
                        c.set(placement, s);
                    }
                }
            }
            Decorator::AttachedToLogs {
                probability,
                provider,
                directions,
            } => {
                for log in shuffled(&c.logs, random) {
                    let d = directions[random.next_int_bounded(directions.len() as i32) as usize];
                    let placement = log.relative(d, 1);
                    if random.next_float() <= *probability && c.is_air(placement) {
                        let s = provider.get(c.region, random, placement);
                        c.set(placement, s);
                    }
                }
            }
            Decorator::PlaceOnGround {
                tries,
                radius,
                height,
                provider,
            } => {
                let positions = c.lowest_trunk_or_root();
                if positions.is_empty() {
                    return;
                }
                let origin = positions[0];
                let min_y = origin.y;
                let (mut min_x, mut max_x, mut min_z, mut max_z) = (origin.x, origin.x, origin.z, origin.z);
                for p in &positions {
                    if p.y == min_y {
                        min_x = min_x.min(p.x);
                        max_x = max_x.max(p.x);
                        min_z = min_z.min(p.z);
                        max_z = max_z.max(p.z);
                    }
                }
                let bb = BoundingBox::new(min_x, min_y, min_z, max_x, min_y, max_z)
                    .inflated_xyz(*radius, *height, *radius);
                for _ in 0..*tries {
                    let x = random.next_int_between_inclusive(bb.min_x, bb.max_x);
                    let y = random.next_int_between_inclusive(bb.min_y, bb.max_y);
                    let z = random.next_int_between_inclusive(bb.min_z, bb.max_z);
                    let pos = BlockPos::new(x, y, z);
                    let above = pos.above();
                    let env = c.region.env;
                    let a = c.region.get(above);
                    if (env.blocks.flags(a).is_air() || env.blocks.block_of(a) == env.b.vine)
                        && c.region.flags(pos).is_solid_render()
                        && c.region
                            .height(HeightmapKind::MotionBlockingNoLeaves, pos.x, pos.z)
                            <= above.y
                    {
                        let s = provider.get(c.region, random, above);
                        c.set(above, s);
                    }
                }
            }
            Decorator::CreakingHeart(prob) => {
                if c.logs.is_empty() || random.next_float() >= *prob {
                    return;
                }
                let mut candidates = c.logs.clone();
                shuffle(&mut candidates, random);
                let env = c.region.env;
                let target = candidates.into_iter().find(|p| {
                    Direction::ALL
                        .iter()
                        .all(|d| env.t.logs.contains(c.region.get(p.relative(*d, 1))))
                });
                if let Some(t) = target {
                    let s = env.blocks.block(env.b.creaking_heart).default_state;
                    let s = env
                        .blocks
                        .with_property(s, "creaking_heart_state", "dormant")
                        .unwrap_or(s);
                    let s = env.blocks.with_property(s, "natural", "true").unwrap_or(s);
                    c.set(t, s);
                }
            }
            Decorator::PaleMoss {
                leaves,
                trunk,
                ground,
                patch,
            } => {
                let logs = shuffled(&c.logs, random);
                if logs.is_empty() {
                    return;
                }
                // `Collections.min` keeps the first of equal elements.
                let mut origin = logs[0];
                for p in &logs[1..] {
                    if p.y < origin.y {
                        origin = *p;
                    }
                }
                if random.next_float() < *ground {
                    patch.place(c.region, random, origin.above());
                }
                for pos in c.logs.clone() {
                    if random.next_float() < *trunk {
                        let down = pos.below();
                        if c.is_air(down) {
                            moss_hanger(c, random, down);
                        }
                    }
                }
                for pos in c.leaves.clone() {
                    if random.next_float() < *leaves {
                        let down = pos.below();
                        if c.is_air(down) {
                            moss_hanger(c, random, down);
                        }
                    }
                }
            }
            Decorator::ShelfMushroom(probability) => shelf_mushroom(c, random, *probability),
        }
    }

    pub fn is_implemented(&self) -> bool {
        true
    }
}

/// `PaleMossDecorator.addMossHanger`.
fn moss_hanger(c: &mut Context, random: &mut dyn RandomSource, mut pos: BlockPos) {
    let env = c.region.env;
    let moss = env.blocks.block(env.b.pale_hanging_moss).default_state;
    let tip = |v: &str| env.blocks.with_property(moss, "tip", v).unwrap();
    while c.is_air(pos.below()) && !(random.next_float() < 0.5) {
        c.set(pos, tip("false"));
        pos = pos.below();
    }
    c.set(pos, tip("true"));
}

/// `ShelfMushroomDecorator.place`.
fn shelf_mushroom(c: &mut Context, random: &mut dyn RandomSource, probability: f32) {
    if random.next_float() >= probability || c.logs.is_empty() {
        return;
    }
    let env = c.region.env;
    let shelf = env.b.shelf_mushroom;
    let logs = c.logs.clone();
    let has_shelf = |c: &Context, p: BlockPos| c.region.is(p, shelf);
    let adjacent_shelf = |c: &Context, p: BlockPos| {
        Direction::HORIZONTAL
            .iter()
            .any(|d| has_shelf(c, p.relative(*d, 1)))
    };
    let replaceable = |c: &Context, p: BlockPos| {
        let water = env.b.water;
        c.region.flags(p).can_be_replaced()
            && !(c.region.is(p, water)
                || c.region.is(p.offset(1, 0, 0), water)
                || c.region.is(p.offset(-1, 0, 0), water)
                || c.region.is(p.offset(0, 0, -1), water)
                || c.region.is(p.offset(0, 0, 1), water))
    };
    let place = |c: &mut Context, random: &mut dyn RandomSource, p: BlockPos, facing: Direction| {
        let age = random.next_int_bounded(2);
        let s = env.blocks.block(shelf).default_state;
        let s = env.blocks.with_property(s, "age", &age.to_string()).unwrap();
        let s = env.blocks.with_property(s, "facing", facing.name()).unwrap();
        c.set(p, s);
    };
    let first = logs[0];
    let last = logs[logs.len() - 1];
    if first.y == last.y {
        // Fallen log: the two sides perpendicular to it.
        let dirs = if first.x != last.x {
            [Direction::North, Direction::South]
        } else {
            [Direction::East, Direction::West]
        };
        for log in &logs {
            for d in dirs {
                if random.next_float() <= 0.25 {
                    let p = log.offset(d.step_x(), 0, d.step_z());
                    if replaceable(c, p) && !adjacent_shelf(c, p) && !adjacent_shelf(c, *log) {
                        place(c, random, p, d);
                    }
                }
            }
        }
    } else {
        let a = Direction::random_horizontal(random);
        let dirs = [a, a.clockwise()];
        let base_y = first.y;
        for log in &logs {
            let dy = log.y - base_y;
            if !(1..=4).contains(&dy) {
                continue;
            }
            for d in dirs {
                if random.next_float() <= 0.25 {
                    let p = log.offset(d.step_x(), 0, d.step_z());
                    if replaceable(c, p) && !has_shelf(c, p.below()) {
                        place(c, random, p, d);
                        break;
                    }
                }
            }
        }
    }
}

/// `AlterGroundDecorator.placeCircle`.
fn alter_circle(c: &mut Context, random: &mut dyn RandomSource, provider: &StateProvider, pos: BlockPos) {
    for xx in -2i32..=2 {
        for zz in -2i32..=2 {
            if xx.abs() != 2 || zz.abs() != 2 {
                let p = pos.offset(xx, 0, zz);
                let mut dy = 2;
                while dy >= -3 {
                    let cursor = p.above_n(dy);
                    if let Some(s) = provider.get_optional(c.region, random, cursor) {
                        c.set(cursor, s);
                        break;
                    }
                    if !c.is_air(cursor) && dy < 0 {
                        break;
                    }
                    dy -= 1;
                }
            }
        }
    }
}
