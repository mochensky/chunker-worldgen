//! `OreFeature` and `ScatteredOreFeature`.

use anyhow::Result;
use serde_json::Value;

use super::basic::Replacement;
use crate::chunk::HeightmapKind;
use crate::codec::{f32_field, field, i32_field};
use crate::feature::Region;
use crate::feature::registry::Parser;
use crate::math;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

pub struct Ore {
    pub targets: Vec<Replacement>,
    pub size: i32,
    pub discard_chance_on_air_exposure: f32,
}

impl Ore {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            targets: Replacement::parse_list(p, field(json, "targets")?)?,
            size: i32_field(json, "size")?,
            discard_chance_on_air_exposure: f32_field(json, "discard_chance_on_air_exposure")?,
        })
    }

    /// `AbstractOreFeature.canPlaceOre`.
    fn can_place(
        &self,
        region: &Region,
        state: chunker_data::blocks::BlockState,
        random: &mut dyn RandomSource,
        target: &Replacement,
        pos: BlockPos,
    ) -> bool {
        if !target.target.test(&region.env.blocks, state, pos, random) {
            return false;
        }
        let d = self.discard_chance_on_air_exposure;
        let skip_air_check = if d <= 0.0 {
            true
        } else if d >= 1.0 {
            false
        } else {
            random.next_float() >= d
        };
        skip_air_check
            || !Direction::ALL
                .iter()
                .any(|dir| region.flags(pos.relative(*dir, 1)).is_air())
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let dir = random.next_float() * std::f32::consts::PI;
        let spread_xy = self.size as f32 / 8.0;
        let max_radius = math::ceil(((self.size as f32 / 16.0 * 2.0 + 1.0) / 2.0) as f64);
        let x0 = origin.x as f64 + crate::math::trig::sin(dir as f64) * spread_xy as f64;
        let x1 = origin.x as f64 - crate::math::trig::sin(dir as f64) * spread_xy as f64;
        let z0 = origin.z as f64 + crate::math::trig::cos(dir as f64) * spread_xy as f64;
        let z1 = origin.z as f64 - crate::math::trig::cos(dir as f64) * spread_xy as f64;
        let y0 = (origin.y + random.next_int_bounded(3) - 2) as f64;
        let y1 = (origin.y + random.next_int_bounded(3) - 2) as f64;
        let ceil_spread = math::ceil(spread_xy as f64);
        let x_start = origin.x - ceil_spread - max_radius;
        let y_start = origin.y - 2 - max_radius;
        let z_start = origin.z - ceil_spread - max_radius;
        let size_xz = 2 * (ceil_spread + max_radius);
        let size_y = 2 * (2 + max_radius);
        for xp in x_start..=x_start + size_xz {
            for zp in z_start..=z_start + size_xz {
                if y_start <= region.height(HeightmapKind::OceanFloorWg, xp, zp) {
                    return self.do_place(
                        region, random, x0, x1, z0, z1, y0, y1, x_start, y_start, z_start, size_xz, size_y,
                    );
                }
            }
        }
        false
    }

    fn do_place(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        x0: f64,
        x1: f64,
        z0: f64,
        z1: f64,
        y0: f64,
        y1: f64,
        x_start: i32,
        y_start: i32,
        z_start: i32,
        size_xz: i32,
        size_y: i32,
    ) -> bool {
        let size = self.size as usize;
        let mut placed = 0;
        let mut tested = vec![false; (size_xz * size_y * size_xz).max(0) as usize + 1];
        let mut data = vec![0.0f64; size * 4];
        for i in 0..size {
            let step = i as f32 / self.size as f32;
            let xx = math::lerp_f64(step as f64, x0, x1);
            let yy = math::lerp_f64(step as f64, y0, y1);
            let zz = math::lerp_f64(step as f64, z0, z1);
            let ss = random.next_double() * self.size as f64 / 16.0;
            let r = ((math::mth_sin((std::f32::consts::PI * step) as f64) + 1.0) as f64 * ss + 1.0) / 2.0;
            data[i * 4] = xx;
            data[i * 4 + 1] = yy;
            data[i * 4 + 2] = zz;
            data[i * 4 + 3] = r;
        }
        for i1 in 0..size.saturating_sub(1) {
            if data[i1 * 4 + 3] > 0.0 {
                for i2 in i1 + 1..size {
                    if data[i2 * 4 + 3] > 0.0 {
                        let dx = data[i1 * 4] - data[i2 * 4];
                        let dy = data[i1 * 4 + 1] - data[i2 * 4 + 1];
                        let dz = data[i1 * 4 + 2] - data[i2 * 4 + 2];
                        let dr = data[i1 * 4 + 3] - data[i2 * 4 + 3];
                        if dr * dr > dx * dx + dy * dy + dz * dz {
                            if dr > 0.0 {
                                data[i2 * 4 + 3] = -1.0;
                            } else {
                                data[i1 * 4 + 3] = -1.0;
                            }
                        }
                    }
                }
            }
        }
        for i in 0..size {
            let r = data[i * 4 + 3];
            if r < 0.0 {
                continue;
            }
            let xx = data[i * 4];
            let yy = data[i * 4 + 1];
            let zz = data[i * 4 + 2];
            let x_min = math::floor(xx - r).max(x_start);
            let y_min = math::floor(yy - r).max(y_start);
            let z_min = math::floor(zz - r).max(z_start);
            let x_max = math::floor(xx + r).max(x_min);
            let y_max = math::floor(yy + r).max(y_min);
            let z_max = math::floor(zz + r).max(z_min);
            for x in x_min..=x_max {
                let xd = (x as f64 + 0.5 - xx) / r;
                if xd * xd >= 1.0 {
                    continue;
                }
                for y in y_min..=y_max {
                    let yd = (y as f64 + 0.5 - yy) / r;
                    if xd * xd + yd * yd >= 1.0 {
                        continue;
                    }
                    for z in z_min..=z_max {
                        let zd = (z as f64 + 0.5 - zz) / r;
                        if xd * xd + yd * yd + zd * zd < 1.0 && !region.is_outside_build_height(y) {
                            let idx =
                                (x - x_start + (y - y_start) * size_xz + (z - z_start) * size_xz * size_y)
                                    as usize;
                            if tested[idx] {
                                continue;
                            }
                            tested[idx] = true;
                            let pos = BlockPos::new(x, y, z);
                            if region.ensure_can_write(pos) {
                                let state = region.get(pos);
                                for t in &self.targets {
                                    if self.can_place(region, state, random, t, pos) {
                                        region.set_section_raw(pos, t.state);
                                        placed += 1;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        placed > 0
    }

    /// `ScatteredOreFeature.place`.
    pub fn place_scattered(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        origin: BlockPos,
    ) -> bool {
        let tries = random.next_int_bounded(self.size + 1);
        for i in 0..tries {
            let max = i.min(7);
            let mut axis = || math::round_f32((random.next_float() - random.next_float()) * max as f32);
            let xd = axis();
            let yd = axis();
            let zd = axis();
            let pos = origin.offset(xd, yd, zd);
            let state = region.get(pos);
            for t in &self.targets {
                if self.can_place(region, state, random, t, pos) {
                    region.set(pos, t.state, 2);
                    break;
                }
            }
        }
        true
    }
}
