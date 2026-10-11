//! `TrunkPlacer`s.

use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::Value;

use super::{Attachment, TreeCtx};
use crate::block_set::StateSet;
use crate::codec::{f32_field, field, i32_field, opt_i32, type_name};
use crate::feature::registry::Parser;
use crate::math;
use crate::providers::IntProvider;
use crate::random::RandomSource;
use crate::structure::geom::{Axis, BlockPos, Direction};

pub enum Kind {
    Straight,
    Forking,
    Giant,
    MegaJungle,
    DarkOak,
    Fancy,
    Bending {
        min_height_for_leaves: i32,
        bend_length: IntProvider,
    },
    UpwardsBranching {
        extra_branch_steps: IntProvider,
        branch_probability: f32,
        extra_branch_length: IntProvider,
        can_grow_through: Arc<StateSet>,
    },
    Cherry {
        branch_count: IntProvider,
        branch_horizontal_length: IntProvider,
        branch_start: (i32, i32),
        branch_end_offset: IntProvider,
    },
    Poplar {
        trunk_height_above_branches: IntProvider,
        branch_amount: IntProvider,
    },
}

pub struct TrunkPlacer {
    base_height: i32,
    rand_a: i32,
    rand_b: i32,
    kind: Kind,
}

impl TrunkPlacer {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let int = |n: &str| IntProvider::parse(field(json, n)?);
        let kind = match type_name(json)?.as_str() {
            "straight_trunk_placer" => Kind::Straight,
            "forking_trunk_placer" => Kind::Forking,
            "giant_trunk_placer" => Kind::Giant,
            "mega_jungle_trunk_placer" => Kind::MegaJungle,
            "dark_oak_trunk_placer" => Kind::DarkOak,
            "fancy_trunk_placer" => Kind::Fancy,
            "bending_trunk_placer" => Kind::Bending {
                min_height_for_leaves: opt_i32(json, "min_height_for_leaves", 1)?,
                bend_length: int("bend_length")?,
            },
            "upwards_branching_trunk_placer" => Kind::UpwardsBranching {
                extra_branch_steps: int("extra_branch_steps")?,
                branch_probability: f32_field(json, "place_branch_per_log_probability")?,
                extra_branch_length: int("extra_branch_length")?,
                can_grow_through: p.block_set(field(json, "can_grow_through")?)?,
            },
            "cherry_trunk_placer" => {
                let start = field(json, "branch_start_offset_from_top")?;
                Kind::Cherry {
                    branch_count: int("branch_count")?,
                    branch_horizontal_length: int("branch_horizontal_length")?,
                    branch_start: (
                        i32_field(start, "min_inclusive")?,
                        i32_field(start, "max_inclusive")?,
                    ),
                    branch_end_offset: int("branch_end_offset_from_top")?,
                }
            }
            "poplar_trunk_placer" => Kind::Poplar {
                trunk_height_above_branches: int("trunk_height_above_branches")?,
                branch_amount: int("branch_amount")?,
            },
            other => bail!("unknown trunk placer {other}"),
        };
        Ok(Self {
            base_height: i32_field(json, "base_height")?,
            rand_a: i32_field(json, "height_rand_a")?,
            rand_b: i32_field(json, "height_rand_b")?,
            kind,
        })
    }

    /// `getTreeHeight`.
    pub fn tree_height(&self, random: &mut dyn RandomSource) -> i32 {
        self.base_height + random.next_int_bounded(self.rand_a + 1) + random.next_int_bounded(self.rand_b + 1)
    }

    fn valid_tree_pos(&self, ctx: &TreeCtx, p: BlockPos) -> bool {
        ctx.valid_tree_pos(p)
            || matches!(&self.kind, Kind::UpwardsBranching { can_grow_through, .. } if can_grow_through.contains(ctx.region.get(p)))
    }

    /// `isFree`.
    pub fn is_free(&self, ctx: &TreeCtx, p: BlockPos) -> bool {
        self.valid_tree_pos(ctx, p) || ctx.region.env.t.logs.contains(ctx.region.get(p))
    }

    /// `placeLog` with an optional axis modifier.
    fn place_log(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        p: BlockPos,
        axis: Option<Axis>,
    ) -> bool {
        if !self.valid_tree_pos(ctx, p) {
            return false;
        }
        let mut s = ctx.tree.trunk_provider.get(ctx.region, random, p);
        if let Some(a) = axis {
            s = ctx.with_axis(s, a);
        }
        ctx.set_trunk(p, s);
        true
    }

    fn place_log_if_free(&self, ctx: &mut TreeCtx, random: &mut dyn RandomSource, p: BlockPos) {
        if self.is_free(ctx, p) {
            self.place_log(ctx, random, p, None);
        }
    }

    /// `placeBelowTrunkBlock`.
    fn place_below_trunk(ctx: &mut TreeCtx, random: &mut dyn RandomSource, p: BlockPos) {
        if let Some(s) = ctx.tree.below_trunk_provider.get_optional(ctx.region, random, p) {
            ctx.set_trunk(p, s);
        }
    }

    /// `placeTrunk`.
    pub fn place_trunk(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        tree_height: i32,
        origin: BlockPos,
    ) -> Vec<Attachment> {
        match &self.kind {
            Kind::Straight => {
                Self::place_below_trunk(ctx, random, origin.below());
                for y in 0..tree_height {
                    self.place_log(ctx, random, origin.above_n(y), None);
                }
                vec![Attachment::new(origin.above_n(tree_height), 0, false)]
            }
            Kind::Forking => self.forking(ctx, random, tree_height, origin),
            Kind::Giant => self.giant(ctx, random, tree_height, origin),
            Kind::MegaJungle => {
                let mut attachments = self.giant(ctx, random, tree_height, origin);
                let mut branch_height = tree_height - 2 - random.next_int_bounded(4);
                while branch_height > tree_height / 2 {
                    let angle = random.next_float() * (std::f64::consts::PI * 2.0) as f32;
                    let mut bx = 0;
                    let mut bz = 0;
                    for b in 0..5 {
                        bx = (1.5f32 + math::mth_cos(angle as f64) * b as f32) as i32;
                        bz = (1.5f32 + math::mth_sin(angle as f64) * b as f32) as i32;
                        let p = origin.offset(bx, branch_height - 3 + b / 2, bz);
                        self.place_log(ctx, random, p, None);
                    }
                    attachments.push(Attachment::new(origin.offset(bx, branch_height, bz), -2, false));
                    branch_height -= 2 + random.next_int_bounded(4);
                }
                attachments
            }
            Kind::DarkOak => self.dark_oak(ctx, random, tree_height, origin),
            Kind::Fancy => self.fancy(ctx, random, tree_height, origin),
            Kind::Bending {
                min_height_for_leaves,
                bend_length,
            } => {
                let direction = Direction::random_horizontal(random);
                let log_height = tree_height - 1;
                let mut pos = origin;
                Self::place_below_trunk(ctx, random, pos.below());
                let mut points = Vec::new();
                for i in 0..=log_height {
                    if i + 1 >= log_height + random.next_int_bounded(2) {
                        pos = pos.relative(direction, 1);
                    }
                    if ctx.valid_tree_pos(pos) {
                        self.place_log(ctx, random, pos, None);
                    }
                    if i >= *min_height_for_leaves {
                        points.push(Attachment::new(pos, 0, false));
                    }
                    pos = pos.above();
                }
                let len = bend_length.sample(random);
                for _ in 0..=len {
                    if ctx.valid_tree_pos(pos) {
                        self.place_log(ctx, random, pos, None);
                    }
                    points.push(Attachment::new(pos, 0, false));
                    pos = pos.relative(direction, 1);
                }
                points
            }
            Kind::UpwardsBranching {
                extra_branch_steps,
                branch_probability,
                extra_branch_length,
                ..
            } => {
                let mut attachments = Vec::new();
                let mut log = origin;
                for h in 0..tree_height {
                    let current = origin.y + h;
                    log = BlockPos::new(origin.x, current, origin.z);
                    if self.place_log(ctx, random, log, None)
                        && h < tree_height - 1
                        && random.next_float() < *branch_probability
                    {
                        let dir = Direction::random_horizontal(random);
                        let len = extra_branch_length.sample(random);
                        let branch_pos = 0.max(len - extra_branch_length.sample(random) - 1);
                        let steps = extra_branch_steps.sample(random);
                        log = self.upwards_branch(
                            ctx,
                            random,
                            tree_height,
                            &mut attachments,
                            log,
                            current,
                            dir,
                            branch_pos,
                            steps,
                        );
                    }
                    if h == tree_height - 1 {
                        log = BlockPos::new(origin.x, current + 1, origin.z);
                        attachments.push(Attachment::new(log, 0, false));
                    }
                }
                let _ = log;
                attachments
            }
            Kind::Cherry {
                branch_count,
                branch_horizontal_length,
                branch_start,
                branch_end_offset,
            } => {
                Self::place_below_trunk(ctx, random, origin.below());
                let uniform = |random: &mut dyn RandomSource, min: i32, max: i32| {
                    random.next_int_bounded(max - min + 1) + min
                };
                let first = 0.max(tree_height - 1 + uniform(random, branch_start.0, branch_start.1));
                let mut second = 0.max(tree_height - 1 + uniform(random, branch_start.0, branch_start.1 - 1));
                if second >= first {
                    second += 1;
                }
                let count = branch_count.sample(random);
                let middle = count == 3;
                let both = count >= 2;
                let trunk_height = if middle {
                    tree_height
                } else if both {
                    first.max(second) + 1
                } else {
                    first + 1
                };
                for y in 0..trunk_height {
                    self.place_log(ctx, random, origin.above_n(y), None);
                }
                let mut attachments = Vec::new();
                if middle {
                    attachments.push(Attachment::new(origin.above_n(trunk_height), 0, false));
                }
                let dir = Direction::random_horizontal(random);
                attachments.push(self.cherry_branch(
                    ctx,
                    random,
                    tree_height,
                    origin,
                    dir,
                    first,
                    first < trunk_height - 1,
                    branch_horizontal_length,
                    branch_end_offset,
                ));
                if both {
                    attachments.push(self.cherry_branch(
                        ctx,
                        random,
                        tree_height,
                        origin,
                        dir.opposite(),
                        second,
                        second < trunk_height - 1,
                        branch_horizontal_length,
                        branch_end_offset,
                    ));
                }
                attachments
            }
            Kind::Poplar {
                trunk_height_above_branches,
                branch_amount,
            } => {
                Self::place_below_trunk(ctx, random, origin.below());
                let up_to = tree_height - trunk_height_above_branches.sample(random);
                for y in 0..tree_height {
                    self.place_log(ctx, random, origin.above_n(y), None);
                    // `Direction.allShuffled(random)` without the vertical directions.
                    let mut all = Direction::ALL.to_vec();
                    crate::structure::util::shuffle(&mut all, random);
                    let directions: Vec<Direction> = all.into_iter().filter(|d| d.is_horizontal()).collect();
                    if up_to - 1 == y {
                        let n = branch_amount.sample(random);
                        for x in 0..n {
                            let d = directions[x as usize];
                            self.place_log(ctx, random, origin.above_n(y).relative(d, 1), Some(d.axis()));
                        }
                    }
                }
                vec![Attachment::new(origin.above_n(up_to), 0, false)]
            }
        }
    }

    fn giant(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        tree_height: i32,
        origin: BlockPos,
    ) -> Vec<Attachment> {
        let below = origin.below();
        Self::place_below_trunk(ctx, random, below);
        Self::place_below_trunk(ctx, random, below.offset(1, 0, 0));
        Self::place_below_trunk(ctx, random, below.offset(0, 0, 1));
        Self::place_below_trunk(ctx, random, below.offset(1, 0, 1));
        for hh in 0..tree_height {
            self.place_log_if_free(ctx, random, origin.offset(0, hh, 0));
            if hh < tree_height - 1 {
                self.place_log_if_free(ctx, random, origin.offset(1, hh, 0));
                self.place_log_if_free(ctx, random, origin.offset(1, hh, 1));
                self.place_log_if_free(ctx, random, origin.offset(0, hh, 1));
            }
        }
        vec![Attachment::new(origin.above_n(tree_height), 0, true)]
    }

    fn forking(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        tree_height: i32,
        origin: BlockPos,
    ) -> Vec<Attachment> {
        Self::place_below_trunk(ctx, random, origin.below());
        let mut attachments = Vec::new();
        let lean_dir = Direction::random_horizontal(random);
        let lean_height = tree_height - random.next_int_bounded(4) - 1;
        let mut lean_steps = 3 - random.next_int_bounded(3);
        let mut tx = origin.x;
        let mut tz = origin.z;
        let mut ey = None;
        for yo in 0..tree_height {
            let yy = origin.y + yo;
            if yo >= lean_height && lean_steps > 0 {
                tx += lean_dir.step_x();
                tz += lean_dir.step_z();
                lean_steps -= 1;
            }
            if self.place_log(ctx, random, BlockPos::new(tx, yy, tz), None) {
                ey = Some(yy + 1);
            }
        }
        if let Some(y) = ey {
            attachments.push(Attachment::new(BlockPos::new(tx, y, tz), 1, false));
        }
        tx = origin.x;
        tz = origin.z;
        let branch_dir = Direction::random_horizontal(random);
        if branch_dir != lean_dir {
            let branch_pos = lean_height - random.next_int_bounded(2) - 1;
            let mut branch_steps = 1 + random.next_int_bounded(3);
            ey = None;
            let mut yo = branch_pos;
            while yo < tree_height && branch_steps > 0 {
                if yo >= 1 {
                    let yy = origin.y + yo;
                    tx += branch_dir.step_x();
                    tz += branch_dir.step_z();
                    if self.place_log(ctx, random, BlockPos::new(tx, yy, tz), None) {
                        ey = Some(yy + 1);
                    }
                }
                yo += 1;
                branch_steps -= 1;
            }
            if let Some(y) = ey {
                attachments.push(Attachment::new(BlockPos::new(tx, y, tz), 0, false));
            }
        }
        attachments
    }

    fn dark_oak(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        tree_height: i32,
        origin: BlockPos,
    ) -> Vec<Attachment> {
        let mut attachments = Vec::new();
        let below = origin.below();
        Self::place_below_trunk(ctx, random, below);
        Self::place_below_trunk(ctx, random, below.offset(1, 0, 0));
        Self::place_below_trunk(ctx, random, below.offset(0, 0, 1));
        Self::place_below_trunk(ctx, random, below.offset(1, 0, 1));
        let lean_dir = Direction::random_horizontal(random);
        let lean_height = tree_height - random.next_int_bounded(4);
        let mut lean_steps = 2 - random.next_int_bounded(3);
        let (x, y, z) = (origin.x, origin.y, origin.z);
        let mut tx = x;
        let mut tz = z;
        let ey = y + tree_height - 1;
        for dy in 0..tree_height {
            if dy >= lean_height && lean_steps > 0 {
                tx += lean_dir.step_x();
                tz += lean_dir.step_z();
                lean_steps -= 1;
            }
            let p = BlockPos::new(tx, y + dy, tz);
            if ctx.is_air_or_leaves(p) {
                self.place_log(ctx, random, p, None);
                self.place_log(ctx, random, p.offset(1, 0, 0), None);
                self.place_log(ctx, random, p.offset(0, 0, 1), None);
                self.place_log(ctx, random, p.offset(1, 0, 1), None);
            }
        }
        attachments.push(Attachment::new(BlockPos::new(tx, ey, tz), 0, true));
        for ox in -1..=2 {
            for oz in -1..=2 {
                if (!(0..=1).contains(&ox) || !(0..=1).contains(&oz)) && random.next_int_bounded(3) <= 0 {
                    let length = random.next_int_bounded(3) + 2;
                    for by in 0..length {
                        self.place_log(ctx, random, BlockPos::new(x + ox, ey - by - 1, z + oz), None);
                    }
                    attachments.push(Attachment::new(BlockPos::new(x + ox, ey, z + oz), 0, false));
                }
            }
        }
        attachments
    }

    fn fancy(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        tree_height: i32,
        origin: BlockPos,
    ) -> Vec<Attachment> {
        let height = tree_height + 2;
        let trunk_height = math::floor(height as f64 * 0.618);
        Self::place_below_trunk(ctx, random, origin.below());
        let ratio = 1.0 * height as f64 / 13.0;
        let clusters_per_y = 1.min(math::floor(1.382 + ratio * ratio));
        let trunk_top = origin.y + trunk_height;
        let mut relative_y = height - 5;
        let mut foliage: Vec<(Attachment, i32)> =
            vec![(Attachment::new(origin.above_n(relative_y), 0, false), trunk_top)];
        while relative_y >= 0 {
            let shape = tree_shape(height, relative_y);
            if shape >= 0.0 {
                for _ in 0..clusters_per_y {
                    let radius = 1.0 * shape as f64 * (random.next_float() as f64 + 0.328);
                    let angle = (random.next_float() * 2.0) as f64 * std::f64::consts::PI;
                    let x = radius * crate::math::trig::sin(angle) + 0.5;
                    let z = radius * crate::math::trig::cos(angle) + 0.5;
                    let check_start = origin.offset(math::floor(x), relative_y - 1, math::floor(z));
                    let check_end = check_start.above_n(5);
                    if self.make_limb(ctx, random, check_start, check_end, false) {
                        let dx = origin.x - check_start.x;
                        let dz = origin.z - check_start.z;
                        let branch_height =
                            check_start.y as f64 - ((dx * dx + dz * dz) as f64).sqrt() * 0.381;
                        let branch_top = if branch_height > trunk_top as f64 {
                            trunk_top
                        } else {
                            branch_height as i32
                        };
                        let base = BlockPos::new(origin.x, branch_top, origin.z);
                        if self.make_limb(ctx, random, base, check_start, false) {
                            foliage.push((Attachment::new(check_start, 0, false), base.y));
                        }
                    }
                }
            }
            relative_y -= 1;
        }
        self.make_limb(ctx, random, origin, origin.above_n(trunk_height), true);
        // `makeBranches`
        for (attachment, branch_base) in &foliage {
            let base = BlockPos::new(origin.x, *branch_base, origin.z);
            if base != attachment.pos && trim_branches(height, branch_base - origin.y) {
                self.make_limb(ctx, random, base, attachment.pos, true);
            }
        }
        foliage
            .into_iter()
            .filter(|(_, b)| trim_branches(height, b - origin.y))
            .map(|(a, _)| a)
            .collect()
    }

    /// `FancyTrunkPlacer.makeLimb`.
    fn make_limb(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        start: BlockPos,
        end: BlockPos,
        do_place: bool,
    ) -> bool {
        if !do_place && start == end {
            return true;
        }
        let delta = end.sub(start);
        let steps = delta.x.abs().max(delta.y.abs().max(delta.z.abs()));
        let dx = delta.x as f32 / steps as f32;
        let dy = delta.y as f32 / steps as f32;
        let dz = delta.z as f32 / steps as f32;
        for i in 0..=steps {
            let p = start.offset(
                math::floor_f32(0.5 + i as f32 * dx),
                math::floor_f32(0.5 + i as f32 * dy),
                math::floor_f32(0.5 + i as f32 * dz),
            );
            if do_place {
                let axis = log_axis(start, p);
                self.place_log(ctx, random, p, Some(axis));
            } else if !self.is_free(ctx, p) {
                return false;
            }
        }
        true
    }

    fn upwards_branch(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        tree_height: i32,
        attachments: &mut Vec<Attachment>,
        log: BlockPos,
        current_height: i32,
        dir: Direction,
        branch_pos: i32,
        mut steps: i32,
    ) -> BlockPos {
        let mut height_along = current_height + branch_pos;
        let mut log_x = log.x;
        let mut log_z = log.z;
        let mut index = branch_pos;
        let mut last = log;
        while index < tree_height && steps > 0 {
            if index >= 1 {
                let placement = current_height + index;
                log_x += dir.step_x();
                log_z += dir.step_z();
                height_along = placement;
                last = BlockPos::new(log_x, placement, log_z);
                if self.place_log(ctx, random, last, None) {
                    height_along = placement + 1;
                }
                attachments.push(Attachment::new(last, 0, false));
            }
            index += 1;
            steps -= 1;
        }
        if height_along - current_height > 1 {
            let f = BlockPos::new(log_x, height_along, log_z);
            attachments.push(Attachment::new(f, 0, false));
            attachments.push(Attachment::new(f.offset(0, -2, 0), 0, false));
        }
        last
    }

    fn cherry_branch(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        tree_height: i32,
        origin: BlockPos,
        dir: Direction,
        offset: i32,
        middle_continues: bool,
        horizontal_length: &IntProvider,
        end_offset: &IntProvider,
    ) -> Attachment {
        let mut log = origin.above_n(offset);
        let end_y = tree_height - 1 + end_offset.sample(random);
        let extend = middle_continues || end_y < offset;
        let distance = horizontal_length.sample(random) + if extend { 1 } else { 0 };
        let end = origin.relative(dir, distance).above_n(end_y);
        let steps = if extend { 2 } else { 1 };
        for _ in 0..steps {
            log = log.relative(dir, 1);
            self.place_log(ctx, random, log, Some(dir.axis()));
        }
        let vertical = if end.y > log.y {
            Direction::Up
        } else {
            Direction::Down
        };
        loop {
            let d = log.dist_manhattan(end);
            if d == 0 {
                return Attachment::new(end.above(), 0, false);
            }
            let chance = (end.y - log.y).abs() as f32 / d as f32;
            let grow_vertically = random.next_float() < chance;
            log = log.relative(if grow_vertically { vertical } else { dir }, 1);
            self.place_log(
                ctx,
                random,
                log,
                if grow_vertically { None } else { Some(dir.axis()) },
            );
        }
    }
}

fn log_axis(start: BlockPos, p: BlockPos) -> Axis {
    let xd = (p.x - start.x).abs();
    let zd = (p.z - start.z).abs();
    let m = xd.max(zd);
    if m > 0 {
        if xd == m { Axis::X } else { Axis::Z }
    } else {
        Axis::Y
    }
}

fn trim_branches(height: i32, local_y: i32) -> bool {
    local_y as f64 >= height as f64 * 0.2
}

/// `FancyTrunkPlacer.treeShape`.
fn tree_shape(height: i32, y: i32) -> f32 {
    if (y as f32) < height as f32 * 0.3 {
        return -1.0;
    }
    let radius = height as f32 / 2.0;
    let adjacent = radius - y as f32;
    let mut distance = math::sqrt_f32(radius * radius - adjacent * adjacent);
    if adjacent == 0.0 {
        distance = radius;
    } else if adjacent.abs() >= radius {
        return 0.0;
    }
    distance * 0.5
}
