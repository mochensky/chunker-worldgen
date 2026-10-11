//! `SpeleothemFeature`, `SpeleothemClusterFeature`, `LargeDripstoneFeature` and `SpeleothemUtils`
//! (pointed dripstone and sulfur spikes).

use std::sync::Arc;

use anyhow::Result;
use chunker_data::blocks::{BlockId, BlockState};
use serde_json::Value;

use super::cave::Column;
use crate::block_set::StateSet;
use crate::chunk::HeightmapKind;
use crate::codec::{f32_field, field, i32_field, opt_f32, opt_i32};
use crate::feature::Region;
use crate::feature::registry::Parser;
use crate::math;
use crate::providers::{FloatProvider, IntProvider, random_between_inclusive};
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

fn empty_or_water(region: &Region, s: BlockState) -> bool {
    region.env.blocks.flags(s).is_air() || region.env.blocks.block_of(s) == region.env.b.water
}

fn empty_or_water_or_lava(region: &Region, s: BlockState) -> bool {
    empty_or_water(region, s) || region.env.blocks.block_of(s) == region.env.b.lava
}

/// `SpeleothemUtils.isBase`.
fn is_base(region: &Region, s: BlockState, base: BlockId, replaceable: &StateSet) -> bool {
    region.env.blocks.block_of(s) == base || replaceable.contains(s)
}

/// `SpeleothemUtils.getSpeleothemHeight`.
fn speleothem_height(mut xz_distance: f64, radius: f64, scale: f64, bluntness: f64) -> f64 {
    if xz_distance < bluntness {
        xz_distance = bluntness;
    }
    let r = xz_distance / radius * 0.384;
    // TODO(precision): `Math.pow`/`Math.log` are HotSpot intrinsics; the CRT may differ by an ulp.
    let part1 = 0.75 * r.powf(1.3333333333333333);
    let part2 = r.powf(0.6666666666666666);
    let part3 = 0.3333333333333333 * r.ln();
    let h = scale * (part1 - part2 - part3);
    let h = math::max_f64(h, 0.0);
    h / 0.384 * radius
}

/// `SpeleothemUtils.isCircleMostlyEmbeddedInStone`.
fn circle_embedded(region: &Region, center: BlockPos, xz_radius: i32) -> bool {
    if empty_or_water_or_lava(region, region.get(center)) {
        return false;
    }
    let increment = 6.0f32 / xz_radius as f32;
    let mut angle = 0.0f32;
    while angle < (std::f64::consts::PI * 2.0) as f32 {
        let dx = (math::mth_cos(angle as f64) * xz_radius as f32) as i32;
        let dz = (math::mth_sin(angle as f64) * xz_radius as f32) as i32;
        if empty_or_water_or_lava(region, region.get(center.offset(dx, 0, dz))) {
            return false;
        }
        angle += increment;
    }
    true
}

/// `SpeleothemUtils.growSpeleothem`.
fn grow(
    region: &mut Region,
    start: BlockPos,
    tip: Direction,
    height: i32,
    merged: bool,
    base: BlockId,
    pointed: BlockId,
    replaceable: &StateSet,
) {
    if !is_base(
        region,
        region.get(start.relative(tip.opposite(), 1)),
        base,
        replaceable,
    ) {
        return;
    }
    let blocks = &region.env.blocks;
    let pointed_default = blocks.block(pointed).default_state;
    let make = |thickness: &str| {
        let s = blocks
            .with_property(pointed_default, "vertical_direction", tip.name())
            .unwrap();
        blocks.with_property(s, "thickness", thickness).unwrap()
    };
    // `buildBaseToTipColumn`
    let mut column = Vec::new();
    if height >= 3 {
        column.push(make("base"));
        for _ in 0..height - 3 {
            column.push(make("middle"));
        }
    }
    if height >= 2 {
        column.push(make("frustum"));
    }
    if height >= 1 {
        column.push(make(if merged { "tip_merge" } else { "tip" }));
    }
    let mut pos = start;
    for mut s in column {
        if blocks.block_of(s) == pointed {
            let water = region.fluid(pos).is_water();
            s = blocks
                .with_property(s, "waterlogged", if water { "true" } else { "false" })
                .unwrap();
        }
        region.set(pos, s, 2);
        pos = pos.relative(tip, 1);
    }
}

/// `SpeleothemUtils.placeBaseBlockIfPossible`.
fn place_base(region: &mut Region, pos: BlockPos, base: BlockId, replaceable: &StateSet) -> bool {
    if replaceable.contains(region.get(pos)) {
        let s = region.env.blocks.block(base).default_state;
        region.set(pos, s, 2);
        true
    } else {
        false
    }
}

/// `SpeleothemFeature`.
pub struct Speleothem {
    base: BlockId,
    pointed: BlockId,
    replaceable: Arc<StateSet>,
    taller: f32,
    directional_spread: f32,
    spread2: f32,
    spread3: f32,
}

impl Speleothem {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let blocks = &p.reg.blocks;
        Ok(Self {
            base: blocks.block_of(p.state(field(json, "base_block")?)?),
            pointed: blocks.block_of(p.state(field(json, "pointed_block")?)?),
            replaceable: p.block_set(field(json, "replaceable_blocks")?)?,
            taller: opt_f32(json, "chance_of_taller_generation", 0.2)?,
            directional_spread: opt_f32(json, "chance_of_directional_spread", 0.7)?,
            spread2: opt_f32(json, "chance_of_spread_radius2", 0.5)?,
            spread3: opt_f32(json, "chance_of_spread_radius3", 0.5)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let above = is_base(region, region.get(origin.above()), self.base, &self.replaceable);
        let below = is_base(region, region.get(origin.below()), self.base, &self.replaceable);
        let tip = if above && below {
            if random.next_bool() {
                Direction::Down
            } else {
                Direction::Up
            }
        } else if above {
            Direction::Down
        } else if below {
            Direction::Up
        } else {
            return false;
        };
        let root = origin.relative(tip.opposite(), 1);
        // `createPatchOfBaseBlocks`
        place_base(region, root, self.base, &self.replaceable);
        for d in Direction::HORIZONTAL {
            if random.next_float() <= self.directional_spread {
                let p1 = root.relative(d, 1);
                place_base(region, p1, self.base, &self.replaceable);
                if random.next_float() <= self.spread2 {
                    let p2 = p1.relative(Direction::ALL[random.next_int_bounded(6) as usize], 1);
                    place_base(region, p2, self.base, &self.replaceable);
                    if random.next_float() <= self.spread3 {
                        let p3 = p2.relative(Direction::ALL[random.next_int_bounded(6) as usize], 1);
                        place_base(region, p3, self.base, &self.replaceable);
                    }
                }
            }
        }
        let height = if random.next_float() < self.taller
            && empty_or_water(region, region.get(origin.relative(tip, 1)))
        {
            2
        } else {
            1
        };
        grow(
            region,
            origin,
            tip,
            height,
            false,
            self.base,
            self.pointed,
            &self.replaceable,
        );
        true
    }
}

/// `SpeleothemClusterFeature`.
pub struct SpeleothemCluster {
    base: BlockId,
    pointed: BlockId,
    replaceable: Arc<StateSet>,
    search_range: i32,
    height: IntProvider,
    radius: IntProvider,
    max_height_diff: i32,
    height_deviation: i32,
    layer_thickness: IntProvider,
    density: FloatProvider,
    wetness: FloatProvider,
    chance_at_max_distance: f32,
    max_distance_from_edge: i32,
    max_distance_from_center: i32,
}

impl SpeleothemCluster {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let blocks = &p.reg.blocks;
        Ok(Self {
            base: blocks.block_of(p.state(field(json, "base_block")?)?),
            pointed: blocks.block_of(p.state(field(json, "pointed_block")?)?),
            replaceable: p.block_set(field(json, "replaceable_blocks")?)?,
            search_range: i32_field(json, "floor_to_ceiling_search_range")?,
            height: IntProvider::parse(field(json, "height")?)?,
            radius: IntProvider::parse(field(json, "radius")?)?,
            max_height_diff: i32_field(json, "max_stalagmite_stalactite_height_diff")?,
            height_deviation: i32_field(json, "height_deviation")?,
            layer_thickness: IntProvider::parse(field(json, "speleothem_block_layer_thickness")?)?,
            density: FloatProvider::parse(field(json, "density")?)?,
            wetness: FloatProvider::parse(field(json, "wetness")?)?,
            chance_at_max_distance: f32_field(json, "chance_of_speleothem_at_max_distance_from_center")?,
            max_distance_from_edge: i32_field(json, "max_distance_from_edge_affecting_chance_of_speleothem")?,
            max_distance_from_center: i32_field(json, "max_distance_from_center_affecting_height_bias")?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if !empty_or_water(region, region.get(origin)) {
            return false;
        }
        let height = self.height.sample(random);
        let wetness = self.wetness.sample(random);
        let density = self.density.sample(random);
        let xr = self.radius.sample(random);
        let zr = self.radius.sample(random);
        for dx in -xr..=xr {
            for dz in -zr..=zr {
                let edge = (xr - dx.abs()).min(zr - dz.abs());
                let chance = math::clamped_map_f32(
                    edge as f32,
                    0.0,
                    self.max_distance_from_edge as f32,
                    self.chance_at_max_distance,
                    1.0,
                ) as f64;
                self.column(
                    region,
                    random,
                    origin.offset(dx, 0, dz),
                    dx,
                    dz,
                    wetness,
                    chance,
                    height,
                    density,
                );
            }
        }
        true
    }

    fn is_lava(region: &Region, p: BlockPos) -> bool {
        region.is(p, region.env.b.lava)
    }

    fn adjacent_to_water_ok(region: &Region, p: BlockPos) -> bool {
        let s = region.get(p);
        region.env.t.base_stone_overworld.contains(s) || region.env.blocks.flags(s).fluid().is_water()
    }

    fn can_place_pool(&self, region: &Region, p: BlockPos) -> bool {
        let id = region.block_of(p);
        let b = &region.env.b;
        if id == b.water || id == self.base || id == self.pointed {
            return false;
        }
        if region.fluid(p.above()).is_water() {
            return false;
        }
        Direction::HORIZONTAL
            .iter()
            .all(|d| Self::adjacent_to_water_ok(region, p.relative(*d, 1)))
            && Self::adjacent_to_water_ok(region, p.below())
    }

    fn replace_with_base(&self, region: &mut Region, first: BlockPos, count: i32, d: Direction) {
        let mut p = first;
        for _ in 0..count {
            if !place_base(region, p, self.base, &self.replaceable) {
                return;
            }
            p = p.relative(d, 1);
        }
    }

    fn speleothem_height(
        &self,
        random: &mut dyn RandomSource,
        dx: i32,
        dz: i32,
        density: f32,
        max_height: i32,
    ) -> i32 {
        if random.next_float() > density {
            return 0;
        }
        let distance = dx.abs() + dz.abs();
        let mean = math::clamped_map_f64(
            distance as f64,
            0.0,
            self.max_distance_from_center as f64,
            max_height as f64 / 2.0,
            0.0,
        ) as f32;
        // `ClampedNormalFloat.sample(random, mean, deviation, 0, maxHeight)`
        let v = math::clamp_f32(
            mean + random.next_gaussian() as f32 * self.height_deviation as f32,
            0.0,
            max_height as f32,
        );
        v as i32
    }

    fn column(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        pos: BlockPos,
        dx: i32,
        dz: i32,
        wetness: f32,
        chance: f64,
        cluster_height: i32,
        density: f32,
    ) {
        let r: &Region = region;
        let Some(base_column) = Column::scan(r, pos, self.search_range, &|s| empty_or_water(r, s), &|s| {
            !empty_or_water(r, s)
        }) else {
            return;
        };
        let ceiling = base_column.ceiling;
        let base_floor = base_column.floor;
        if ceiling.is_none() && base_floor.is_none() {
            return;
        }
        let at = |y: i32| BlockPos::new(pos.x, y, pos.z);
        let want_pool = random.next_float() < wetness;
        let column = match base_floor {
            Some(f) if want_pool && self.can_place_pool(region, at(f)) => {
                let w = region.env.blocks.block(region.env.b.water).default_state;
                region.set(at(f), w, 2);
                Column {
                    floor: Some(f - 1),
                    ceiling,
                }
            }
            _ => base_column,
        };
        let floor = column.floor;
        let want_stalactite = random.next_double() < chance;
        let stalactite_height = match ceiling {
            Some(c) if want_stalactite && !Self::is_lava(region, at(c)) => {
                let thickness = self.layer_thickness.sample(random);
                self.replace_with_base(region, at(c), thickness, Direction::Up);
                let max = match floor {
                    Some(f) => cluster_height.min(c - f),
                    None => cluster_height,
                };
                self.speleothem_height(random, dx, dz, density, max)
            }
            _ => 0,
        };
        let want_stalagmite = random.next_double() < chance;
        let stalagmite_height = match floor {
            Some(f) if want_stalagmite && !Self::is_lava(region, at(f)) => {
                let thickness = self.layer_thickness.sample(random);
                self.replace_with_base(region, at(f), thickness, Direction::Down);
                if ceiling.is_some() {
                    0.max(
                        stalactite_height
                            + random_between_inclusive(random, -self.max_height_diff, self.max_height_diff),
                    )
                } else {
                    self.speleothem_height(random, dx, dz, density, cluster_height)
                }
            }
            _ => 0,
        };
        let (actual_stalactite, actual_stalagmite) = match (ceiling, floor) {
            (Some(c), Some(f)) if c - stalactite_height <= f + stalagmite_height => {
                let lowest_bottom = (c - stalactite_height).max(f + 1);
                let highest_top = (f + stalagmite_height).min(c - 1);
                let bottom = random_between_inclusive(random, lowest_bottom, highest_top + 1);
                (c - bottom, bottom - 1 - f)
            }
            _ => (stalactite_height, stalagmite_height),
        };
        let merge = random.next_bool()
            && actual_stalactite > 0
            && actual_stalagmite > 0
            && column
                .height()
                .is_some_and(|h| actual_stalactite + actual_stalagmite == h);
        if let Some(c) = ceiling {
            grow(
                region,
                at(c - 1),
                Direction::Down,
                actual_stalactite,
                merge,
                self.base,
                self.pointed,
                &self.replaceable,
            );
        }
        if let Some(f) = floor {
            grow(
                region,
                at(f + 1),
                Direction::Up,
                actual_stalagmite,
                merge,
                self.base,
                self.pointed,
                &self.replaceable,
            );
        }
    }
}

/// `LargeDripstoneFeature`.
pub struct LargeDripstone {
    replaceable: Arc<StateSet>,
    search_range: i32,
    column_radius: IntProvider,
    height_scale: FloatProvider,
    max_radius_ratio: f32,
    stalactite_bluntness: FloatProvider,
    stalagmite_bluntness: FloatProvider,
    wind_speed: FloatProvider,
    min_radius_for_wind: i32,
    min_bluntness_for_wind: f32,
}

struct Dripstone {
    root: BlockPos,
    pointing_up: bool,
    radius: i32,
    bluntness: f64,
    scale: f64,
}

struct Wind {
    origin_y: i32,
    speed: Option<(f64, f64)>,
    max_offset: i32,
}

impl Wind {
    fn offset(&self, p: BlockPos) -> BlockPos {
        let Some((sx, sz)) = self.speed else { return p };
        let dy = (self.origin_y - p.y) as f64;
        let dx = math::clamp_i32(math::floor(sx * dy), -self.max_offset, self.max_offset);
        let dz = math::clamp_i32(math::floor(sz * dy), -self.max_offset, self.max_offset);
        p.offset(dx, 0, dz)
    }
}

impl Dripstone {
    fn height_at(&self, r: f32) -> i32 {
        speleothem_height(r as f64, self.radius as f64, self.scale, self.bluntness) as i32
    }

    fn move_into_stone(&mut self, region: &Region, wind: &Wind) -> bool {
        while self.radius > 1 {
            let mut p = self.root;
            let tries = 10.min(self.height_at(0.0));
            for _ in 0..tries {
                if region.is(p, region.env.b.lava) {
                    return false;
                }
                if circle_embedded(region, wind.offset(p), self.radius) {
                    self.root = p;
                    return true;
                }
                p = p.relative(
                    if self.pointing_up {
                        Direction::Down
                    } else {
                        Direction::Up
                    },
                    1,
                );
            }
            self.radius /= 2;
        }
        false
    }

    fn place(&self, region: &mut Region, random: &mut dyn RandomSource, wind: &Wind) {
        let block = region
            .env
            .blocks
            .block(region.env.b.dripstone_block)
            .default_state;
        for dx in -self.radius..=self.radius {
            for dz in -self.radius..=self.radius {
                let current = math::sqrt_f32((dx * dx + dz * dz) as f32);
                if current > self.radius as f32 {
                    continue;
                }
                let mut height = self.height_at(current);
                if height <= 0 {
                    continue;
                }
                if (random.next_float() as f64) < 0.2 {
                    let f = random.next_float() * (1.0 - 0.8) + 0.8;
                    height = (height as f32 * f) as i32;
                }
                let mut pos = self.root.offset(dx, 0, dz);
                let mut out_of_stone = false;
                let max_y = if self.pointing_up {
                    region.height(HeightmapKind::WorldSurfaceWg, pos.x, pos.z)
                } else {
                    i32::MAX
                };
                let mut i = 0;
                while i < height && pos.y < max_y {
                    let w = wind.offset(pos);
                    let s = region.get(w);
                    if empty_or_water_or_lava(region, s) {
                        out_of_stone = true;
                        region.set(w, block, 2);
                    } else if out_of_stone && region.env.t.base_stone_overworld.contains(s) {
                        break;
                    }
                    pos = pos.relative(
                        if self.pointing_up {
                            Direction::Up
                        } else {
                            Direction::Down
                        },
                        1,
                    );
                    i += 1;
                }
            }
        }
    }
}

impl LargeDripstone {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            replaceable: p.block_set(field(json, "replaceable_blocks")?)?,
            search_range: opt_i32(json, "floor_to_ceiling_search_range", 30)?,
            column_radius: IntProvider::parse(field(json, "column_radius")?)?,
            height_scale: FloatProvider::parse(field(json, "height_scale")?)?,
            max_radius_ratio: f32_field(json, "max_column_radius_to_cave_height_ratio")?,
            stalactite_bluntness: FloatProvider::parse(field(json, "stalactite_bluntness")?)?,
            stalagmite_bluntness: FloatProvider::parse(field(json, "stalagmite_bluntness")?)?,
            wind_speed: FloatProvider::parse(field(json, "wind_speed")?)?,
            min_radius_for_wind: i32_field(json, "min_radius_for_wind")?,
            min_bluntness_for_wind: f32_field(json, "min_bluntness_for_wind")?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if !empty_or_water(region, region.get(origin)) {
            return false;
        }
        let dripstone = region.env.b.dripstone_block;
        let r: &Region = region;
        let edge = |s: BlockState| {
            is_base(r, s, dripstone, &self.replaceable) || r.env.blocks.block_of(s) == r.env.b.lava
        };
        let Some(column) = Column::scan(r, origin, self.search_range, &|s| empty_or_water(r, s), &edge)
        else {
            return false;
        };
        let (Some(floor), Some(ceiling)) = (column.floor, column.ceiling) else {
            return false;
        };
        let column_height = ceiling - floor - 1;
        if column_height < 4 {
            return false;
        }
        let by_height = (column_height as f32 * self.max_radius_ratio) as i32;
        let max_radius = math::clamp_i32(
            by_height,
            self.column_radius.min_inclusive(),
            self.column_radius.max_inclusive(),
        );
        let radius = random_between_inclusive(random, self.column_radius.min_inclusive(), max_radius);
        let mut stalactite = Dripstone {
            root: BlockPos::new(origin.x, ceiling - 1, origin.z),
            pointing_up: false,
            radius,
            bluntness: self.stalactite_bluntness.sample(random) as f64,
            scale: self.height_scale.sample(random) as f64,
        };
        let mut stalagmite = Dripstone {
            root: BlockPos::new(origin.x, floor + 1, origin.z),
            pointing_up: true,
            radius,
            bluntness: self.stalagmite_bluntness.sample(random) as f64,
            scale: self.height_scale.sample(random) as f64,
        };
        let suitable = |d: &Dripstone| {
            d.radius >= self.min_radius_for_wind && d.bluntness >= self.min_bluntness_for_wind as f64
        };
        let wind = if suitable(&stalactite) && suitable(&stalagmite) {
            let speed = self.wind_speed.sample(random);
            let direction = random.next_float() * (std::f32::consts::PI - 0.0) + 0.0;
            Wind {
                origin_y: origin.y,
                speed: Some((
                    (math::mth_cos(direction as f64) * speed) as f64,
                    (math::mth_sin(direction as f64) * speed) as f64,
                )),
                max_offset: 16 - radius,
            }
        } else {
            Wind {
                origin_y: 0,
                speed: None,
                max_offset: 0,
            }
        };
        let a = stalactite.move_into_stone(region, &wind);
        let b = stalagmite.move_into_stone(region, &wind);
        if a {
            stalactite.place(region, random, &wind);
        }
        if b {
            stalagmite.place(region, random, &wind);
        }
        true
    }
}
