//! Small feature types: blocks, selectors, columns, disks, springs, lakes, freezing.

use std::sync::Arc;

use anyhow::{Context, Result};
use chunker_data::blocks::{BlockState, Fluid};
use serde_json::Value;

use crate::biome::BiomeId;
use crate::block_set::StateSet;
use crate::chunk::HeightmapKind;
use crate::codec::{as_array, f32_field, field, i32_field, opt_bool, opt_i32, str_field};
use crate::feature::Region;
use crate::feature::placement;
use crate::feature::predicate::BlockPredicate;
use crate::feature::provider::StateProvider;
use crate::feature::registry::{Parser, PlacedFeature};
use crate::providers::IntProvider;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

/// `SimpleBlockFeature`.
pub struct SimpleBlock {
    pub to_place: Arc<StateProvider>,
    pub schedule_tick: bool,
}

impl SimpleBlock {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            to_place: p.provider(field(json, "to_place")?)?,
            schedule_tick: opt_bool(json, "schedule_tick", false)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let Some(state) = self.to_place.get_optional(region, random, origin) else {
            return false;
        };
        if !crate::feature::behaviour::can_survive(region, state, origin) {
            return false;
        }
        let env = region.env;
        let blocks = &env.blocks;
        let block = blocks.block_of(state);
        if blocks.is_instance(block, "DoublePlantBlock") {
            let above = region.get(origin.above());
            let af = blocks.flags(above);
            if !af.is_air()
                && (blocks.flags(state).fluid() != af.fluid()
                    || blocks.flags(state).fluid_amount() != af.fluid_amount()
                    || !af.can_be_replaced())
            {
                return false;
            }
            place_double_plant(region, state, origin, 2);
        } else if blocks.is_instance(block, "MossyCarpetBlock") {
            crate::feature::behaviour::place_mossy_carpet(region, origin, 2);
        } else {
            region.set(origin, state, 2);
        }
        if self.schedule_tick {
            let b = region.block_of(origin);
            region.schedule_block_tick(origin, b);
        }
        true
    }
}

/// `DoublePlantBlock.placeAt`.
pub fn place_double_plant(region: &mut Region, state: BlockState, lower: BlockPos, flags: u32) {
    let blocks = &region.env.blocks;
    let lower_state = copy_waterlogged(
        region,
        lower,
        blocks.with_property(state, "half", "lower").unwrap_or(state),
    );
    let upper = lower.above();
    let upper_state = copy_waterlogged(
        region,
        upper,
        blocks.with_property(state, "half", "upper").unwrap_or(state),
    );
    region.set(lower, lower_state, flags);
    region.set(upper, upper_state, flags);
}

/// `DoublePlantBlock.copyWaterloggedFrom`.
pub fn copy_waterlogged(region: &Region, pos: BlockPos, state: BlockState) -> BlockState {
    let blocks = &region.env.blocks;
    if blocks.has_property(state, "waterlogged") {
        let water = region.fluid(pos).is_water();
        blocks
            .with_property(state, "waterlogged", if water { "true" } else { "false" })
            .unwrap_or(state)
    } else {
        state
    }
}

/// `RandomSelectorFeature`.
pub struct RandomSelector {
    pub features: Vec<(Arc<PlacedFeature>, f32)>,
    pub default: Arc<PlacedFeature>,
}

impl RandomSelector {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let mut features = Vec::new();
        for f in as_array(field(json, "features")?)? {
            features.push((p.placed(field(f, "feature")?)?, f32_field(f, "chance")?));
        }
        Ok(Self {
            features,
            default: p.placed(field(json, "default")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        for (f, chance) in &self.features {
            if random.next_float() < *chance {
                return placement::place(region, f, None, random, origin);
            }
        }
        placement::place(region, &self.default, None, random, origin)
    }
}

/// `BlockColumnFeature`.
pub struct BlockColumn {
    pub layers: Vec<(IntProvider, Arc<StateProvider>)>,
    pub direction: Direction,
    pub allowed: BlockPredicate,
    pub prioritize_tip: bool,
}

impl BlockColumn {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let mut layers = Vec::new();
        for l in as_array(field(json, "layers")?)? {
            layers.push((
                IntProvider::parse(field(l, "height")?)?,
                p.provider(field(l, "provider")?)?,
            ));
        }
        Ok(Self {
            layers,
            direction: Direction::parse(str_field(json, "direction")?)?,
            allowed: BlockPredicate::parse(p, field(json, "allowed_placement")?)?,
            prioritize_tip: crate::codec::as_bool(field(json, "prioritize_tip")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let mut heights: Vec<i32> = Vec::with_capacity(self.layers.len());
        let mut total = 0;
        for (h, _) in &self.layers {
            let v = h.sample(random);
            heights.push(v);
            total += v;
        }
        if total == 0 {
            return false;
        }
        let mut next = origin.relative(self.direction, 1);
        for y in 0..total {
            if !self.allowed.test(region, next) {
                truncate(&mut heights, total, y, self.prioritize_tip);
                break;
            }
            next = next.relative(self.direction, 1);
        }
        let mut pos = origin;
        for (i, (_, provider)) in self.layers.iter().enumerate() {
            for _ in 0..heights[i] {
                let s = provider.get(region, random, pos);
                region.set(pos, s, 2);
                pos = pos.relative(self.direction, 1);
            }
        }
        true
    }
}

fn truncate(heights: &mut [i32], total: i32, new_height: i32, prioritize_tip: bool) {
    let mut remove = total - new_height;
    let n = heights.len() as i32;
    let (step, start, end) = if prioritize_tip {
        (1, 0, n)
    } else {
        (-1, n - 1, -1)
    };
    let mut i = start;
    while i != end && remove > 0 {
        let this = heights[i as usize];
        let r = this.min(remove);
        remove -= r;
        heights[i as usize] -= r;
        i += step;
    }
}

/// `DiskFeature`.
pub struct Disk {
    pub provider: Arc<StateProvider>,
    pub target: BlockPredicate,
    pub radius: IntProvider,
    pub half_height: i32,
}

impl Disk {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            provider: p.provider(field(json, "state_provider")?)?,
            target: BlockPredicate::parse(p, field(json, "target")?)?,
            radius: IntProvider::parse(field(json, "radius")?)?,
            half_height: i32_field(json, "half_height")?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let mut placed_any = false;
        let top = origin.y + self.half_height;
        let bottom = origin.y - self.half_height - 1;
        let r = self.radius.sample(random);
        // `BlockPos.betweenClosed`: x fastest, then y, then z.
        for z in origin.z - r..=origin.z + r {
            for x in origin.x - r..=origin.x + r {
                let xd = x - origin.x;
                let zd = z - origin.z;
                if xd * xd + zd * zd <= r * r {
                    placed_any |= self.place_column(region, random, top, bottom, x, z);
                }
            }
        }
        placed_any
    }

    fn place_column(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        top: i32,
        bottom: i32,
        x: i32,
        z: i32,
    ) -> bool {
        let mut placed_any = false;
        let mut placed_above = false;
        let mut y = top;
        while y > bottom {
            let pos = BlockPos::new(x, y, z);
            if self.target.test(region, pos) {
                if let Some(state) = self.provider.get_optional(region, random, pos) {
                    region.set(pos, state, 2);
                    if !placed_above {
                        mark_above_for_post_processing(region, pos);
                    }
                    placed_any = true;
                    placed_above = true;
                }
            } else {
                placed_above = false;
            }
            y -= 1;
        }
        placed_any
    }
}

/// `Feature.markAboveForPostProcessing`.
pub fn mark_above_for_post_processing(region: &mut Region, place: BlockPos) {
    let mut pos = place;
    for _ in 0..2 {
        pos = pos.above();
        if region.flags(pos).is_air() {
            return;
        }
        region.mark_post_processing(pos);
    }
}

/// `SpringFeature`.
pub struct Spring {
    pub fluid: Fluid,
    pub block_state: BlockState,
    pub requires_block_below: bool,
    pub rock_count: i32,
    pub hole_count: i32,
    pub valid: Arc<StateSet>,
}

impl Spring {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let state = field(json, "state")?;
        let id = str_field(state, "id")?;
        let fluid = Fluid::from_id(id).with_context(|| format!("unknown fluid {id}"))?;
        // `FluidState.createLegacyBlock`: sources become level 0 liquid blocks.
        let block = match fluid {
            Fluid::Water | Fluid::FlowingWater => "water",
            Fluid::Lava | Fluid::FlowingLava => "lava",
            Fluid::Empty => "air",
        };
        let mut block_state = p.reg.blocks.vanilla_default(block);
        if matches!(fluid, Fluid::FlowingWater | Fluid::FlowingLava) {
            let props = state.get("properties");
            let amount: i32 = props
                .and_then(|p| p.get("level"))
                .and_then(|v| v.as_str())
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(8);
            let falling = props.and_then(|p| p.get("falling")).and_then(|v| v.as_str()) == Some("true");
            let level = 8 - amount.min(8) + if falling { 8 } else { 0 };
            block_state = p
                .reg
                .blocks
                .with_property(block_state, "level", &level.to_string())
                .context("liquid level")?;
        }
        Ok(Self {
            fluid,
            block_state,
            requires_block_below: opt_bool(json, "requires_block_below", true)?,
            rock_count: opt_i32(json, "rock_count", 4)?,
            hole_count: opt_i32(json, "hole_count", 1)?,
            valid: p.block_set(field(json, "valid_blocks")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, origin: BlockPos) -> bool {
        let valid = |r: &Region, p: BlockPos| self.valid.contains(r.get(p));
        if !valid(region, origin.above()) {
            return false;
        }
        if self.requires_block_below && !valid(region, origin.below()) {
            return false;
        }
        let current = region.get(origin);
        if !region.env.blocks.flags(current).is_air() && !self.valid.contains(current) {
            return false;
        }
        let neighbours = [
            origin.offset(-1, 0, 0),
            origin.offset(1, 0, 0),
            origin.offset(0, 0, -1),
            origin.offset(0, 0, 1),
            origin.below(),
        ];
        let rocks = neighbours.iter().filter(|p| valid(region, **p)).count() as i32;
        let holes = neighbours.iter().filter(|p| region.is_empty_block(**p)).count() as i32;
        if rocks == self.rock_count && holes == self.hole_count {
            region.set(origin, self.block_state, 2);
            region.schedule_fluid_tick(origin, self.fluid);
            true
        } else {
            false
        }
    }
}

/// `Biome.shouldFreeze(level, pos, checkNeighbors)`.
pub fn should_freeze(region: &Region, biome: BiomeId, pos: BlockPos, check_neighbors: bool) -> bool {
    let env = region.env;
    let data = env.biomes.data(biome);
    if data.warm_enough_to_rain(pos.x, pos.y, pos.z, env.sea_level) {
        return false;
    }
    if !region.is_outside_build_height(pos.y) && region.block_light(pos) < 10 {
        let state = region.get(pos);
        let f = env.blocks.flags(state);
        if f.fluid() == Fluid::Water && env.blocks.block_of(state) == env.b.water {
            if !check_neighbors {
                return true;
            }
            let surrounded = [
                pos.offset(-1, 0, 0),
                pos.offset(1, 0, 0),
                pos.offset(0, 0, -1),
                pos.offset(0, 0, 1),
            ]
            .iter()
            .all(|p| region.fluid(*p).is_water());
            if !surrounded {
                return true;
            }
        }
    }
    false
}

/// `Biome.shouldSnow(level, pos)`.
pub fn should_snow(region: &Region, biome: BiomeId, pos: BlockPos) -> bool {
    let env = region.env;
    let data = env.biomes.data(biome);
    if !data.climate.has_precipitation || !data.cold_enough_to_snow(pos.x, pos.y, pos.z, env.sea_level) {
        return false;
    }
    if !region.is_outside_build_height(pos.y) && region.block_light(pos) < 10 {
        let state = region.get(pos);
        let snow = env.blocks.block(env.b.snow).default_state;
        if (env.blocks.flags(state).is_air() || env.blocks.block_of(state) == env.b.snow)
            && crate::feature::behaviour::can_survive(region, snow, pos)
        {
            return true;
        }
    }
    false
}

/// `SnowAndFreezeFeature`.
pub fn freeze_top_layer(region: &mut Region, origin: BlockPos) -> bool {
    let env = region.env;
    let ice = env.blocks.block(env.b.ice).default_state;
    let snow = env.blocks.block(env.b.snow).default_state;
    for dx in 0..16 {
        for dz in 0..16 {
            let x = origin.x + dx;
            let z = origin.z + dz;
            let y = region.height(HeightmapKind::MotionBlocking, x, z);
            let top = BlockPos::new(x, y, z);
            let below = top.below();
            let biome = region.biome(top);
            if should_freeze(region, biome, below, false) {
                region.set(below, ice, 2);
            }
            if should_snow(region, biome, top) {
                region.set(top, snow, 2);
                let below_state = region.get(below);
                if env.blocks.has_property(below_state, "snowy") {
                    let s = env.blocks.with_property(below_state, "snowy", "true").unwrap();
                    region.set(below, s, 2);
                }
            }
        }
    }
    true
}

/// `LakeFeature`.
pub struct Lake {
    pub fluid: Arc<StateProvider>,
    pub barrier: Arc<StateProvider>,
    pub can_place_feature: BlockPredicate,
    pub can_replace_with_air_or_fluid: BlockPredicate,
    pub can_replace_with_barrier: BlockPredicate,
}

impl Lake {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            fluid: p.provider(field(json, "fluid")?)?,
            barrier: p.provider(field(json, "barrier")?)?,
            can_place_feature: BlockPredicate::parse(p, field(json, "can_place_feature")?)?,
            can_replace_with_air_or_fluid: BlockPredicate::parse(
                p,
                field(json, "can_replace_with_air_or_fluid")?,
            )?,
            can_replace_with_barrier: BlockPredicate::parse(p, field(json, "can_replace_with_barrier")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if origin.y <= region.min_y() + 4 {
            return false;
        }
        let origin = origin.offset(-8, -4, -8);
        let mut grid = [false; 2048];
        let spots = random.next_int_bounded(4) + 4;
        for _ in 0..spots {
            let xr = random.next_double() * 6.0 + 3.0;
            let yr = random.next_double() * 4.0 + 2.0;
            let zr = random.next_double() * 6.0 + 3.0;
            let xp = random.next_double() * (16.0 - xr - 2.0) + 1.0 + xr / 2.0;
            let yp = random.next_double() * (8.0 - yr - 4.0) + 2.0 + yr / 2.0;
            let zp = random.next_double() * (16.0 - zr - 2.0) + 1.0 + zr / 2.0;
            for xx in 1..15 {
                for zz in 1..15 {
                    for yy in 1..7 {
                        let xd = (xx as f64 - xp) / (xr / 2.0);
                        let yd = (yy as f64 - yp) / (yr / 2.0);
                        let zd = (zz as f64 - zp) / (zr / 2.0);
                        let d = xd * xd + yd * yd + zd * zd;
                        if d < 1.0 {
                            grid[((xx * 16 + zz) * 8 + yy) as usize] = true;
                        }
                    }
                }
            }
        }
        let g = |x: i32, z: i32, y: i32| grid[((x * 16 + z) * 8 + y) as usize];
        let edge = |x: i32, z: i32, y: i32| {
            !g(x, z, y)
                && (x < 15 && g(x + 1, z, y)
                    || x > 0 && g(x - 1, z, y)
                    || z < 15 && g(x, z + 1, y)
                    || z > 0 && g(x, z - 1, y)
                    || y < 7 && g(x, z, y + 1)
                    || y > 0 && g(x, z, y - 1))
        };
        let fluid = self.fluid.get(region, random, origin);
        let blocks = &region.env.blocks;
        for xx in 0..16 {
            for zz in 0..16 {
                for yy in 0..8 {
                    if edge(xx, zz, yy) {
                        let p = origin.offset(xx, yy, zz);
                        let s = region.get(p);
                        let f = blocks.flags(s);
                        if yy >= 4 && f.liquid() {
                            return false;
                        }
                        if yy < 4 && !f.is_solid() && s != fluid {
                            return false;
                        }
                        if !self.can_place_feature.test(region, p) {
                            return false;
                        }
                    }
                }
            }
        }
        let cave_air = region.env.b.cave_air_state;
        for xx in 0..16 {
            for zz in 0..16 {
                for yy in 0..8 {
                    if g(xx, zz, yy) {
                        let p = origin.offset(xx, yy, zz);
                        if self.can_replace_with_air_or_fluid.test(region, p) {
                            let place_air = yy >= 4;
                            region.set(p, if place_air { cave_air } else { fluid }, 2);
                            if place_air {
                                region.schedule_block_tick(p, region.env.b.cave_air);
                                mark_above_for_post_processing(region, p);
                            }
                        }
                    }
                }
            }
        }
        let barrier = self.barrier.get(region, random, origin);
        if !blocks.flags(barrier).is_air() {
            for xx in 0..16 {
                for zz in 0..16 {
                    for yy in 0..8 {
                        if edge(xx, zz, yy) && (yy < 4 || random.next_int_bounded(2) != 0) {
                            let p = origin.offset(xx, yy, zz);
                            if blocks.flags(region.get(p)).is_solid()
                                && self.can_replace_with_barrier.test(region, p)
                            {
                                region.set(p, barrier, 2);
                                mark_above_for_post_processing(region, p);
                            }
                        }
                    }
                }
            }
        }
        if blocks.flags(fluid).fluid().is_water() {
            let ice = blocks.block(region.env.b.ice).default_state;
            for xx in 0..16 {
                for zz in 0..16 {
                    let p = origin.offset(xx, 4, zz);
                    let biome = region.biome(p);
                    if should_freeze(region, biome, p, false)
                        && self.can_replace_with_air_or_fluid.test(region, p)
                    {
                        region.set(p, ice, 2);
                    }
                }
            }
        }
        true
    }
}

/// `BlockReplacement`.
pub struct Replacement {
    pub target: crate::feature::rule::RuleTest,
    pub state: BlockState,
}

impl Replacement {
    pub fn parse_list(p: &Parser, json: &Value) -> Result<Vec<Self>> {
        as_array(json)?
            .iter()
            .map(|t| {
                Ok(Self {
                    target: crate::feature::rule::RuleTest::parse(p, field(t, "target")?)?,
                    state: p.state(field(t, "state")?)?,
                })
            })
            .collect()
    }
}
