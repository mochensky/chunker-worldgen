//! World carvers (`CaveWorldCarver`, `CanyonWorldCarver`) and `CarvingMask`.

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::codec::{self, field};
use crate::math::{abs_f32, floor, mth_cos, mth_sin};
use crate::providers::{FloatProvider, HeightProvider, IntProvider, random_between_f32};
use crate::random::{AnyRandom, LegacyRandom, RandomSource};
use crate::terrain::material::GenerationContext;

const PI_F: f32 = std::f32::consts::PI;

/// `CarvingMask`: carved positions of one chunk, indexed `y - minY + (z + (x << 4)) * height`.
pub struct CarvingMask {
    pub min_y: i32,
    pub max_y: i32,
    height: i32,
    bits: Vec<u64>,
}

impl CarvingMask {
    pub fn new(min_y: i32, max_y: i32) -> Self {
        let height = max_y - min_y + 1;
        Self {
            min_y,
            max_y,
            height,
            bits: vec![0; ((256 * height) as usize).div_ceil(64)],
        }
    }

    #[inline]
    pub fn carve(&mut self, x: i32, y: i32, z: i32) {
        let i = (y - self.min_y + (z + (x << 4)) * self.height) as usize;
        self.bits[i >> 6] |= 1 << (i & 63);
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|w| *w == 0)
    }

    fn get(&self, i: usize) -> bool {
        self.bits[i >> 6] & (1 << (i & 63)) != 0
    }

    /// `CarvingMask.visit`: contiguous runs of set bits, split per column.
    pub fn visit(&self, mut visitor: impl FnMut(i32, i32, i32, i32)) {
        let total = (256 * self.height) as usize;
        let mut i = 0usize;
        while i < total {
            if !self.get(i) {
                i += 1;
                continue;
            }
            let start = i;
            while i < total && self.get(i) {
                i += 1;
            }
            let end = i - 1;
            let h = self.height as usize;
            for column in start / h..=end / h {
                let cx = (column >> 4 & 15) as i32;
                let cz = (column & 15) as i32;
                let base = column * h;
                let bottom = start.saturating_sub(base) as i32 + self.min_y;
                let top = (end - base).min(h - 1) as i32 + self.min_y;
                visitor(cx, cz, bottom, top);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct CanyonShape {
    distance_factor: FloatProvider,
    thickness: FloatProvider,
    width_smoothness: i32,
    horizontal_radius_factor: FloatProvider,
    vertical_radius_default_factor: f32,
    vertical_radius_center_factor: f32,
    y_scale: FloatProvider,
}

#[derive(Clone, Debug)]
pub enum Carver {
    Cave {
        probability: f32,
        y: HeightProvider,
        count: IntProvider,
        thickness: FloatProvider,
        weird_thickness_bias: bool,
        room_vertical_radius_multiplier: FloatProvider,
        horizontal_radius_multiplier: FloatProvider,
        vertical_radius_multiplier: FloatProvider,
        start_vertical_radius_multiplier: FloatProvider,
        floor_level: FloatProvider,
    },
    Canyon {
        probability: f32,
        y: HeightProvider,
        vertical_rotation: FloatProvider,
        shape: CanyonShape,
    },
}

/// `WorldCarver.carveEllipsoid`.
fn carve_ellipsoid(
    chunk_x: i32,
    chunk_z: i32,
    x: f64,
    y: f64,
    z: f64,
    horizontal_radius: f64,
    vertical_radius: f64,
    mask: &mut CarvingMask,
    skip: &dyn Fn(f64, f64, f64, i32) -> bool,
) {
    let center_x = ((chunk_x << 4) + 8) as f64;
    let center_z = ((chunk_z << 4) + 8) as f64;
    let max_delta = 16.0 + horizontal_radius * 2.0;
    if (x - center_x).abs() > max_delta || (z - center_z).abs() > max_delta {
        return;
    }
    let min_bx = chunk_x << 4;
    let min_bz = chunk_z << 4;
    let min_xi = (floor(x - horizontal_radius) - min_bx - 1).max(0);
    let max_xi = (floor(x + horizontal_radius) - min_bx).min(15);
    let min_y = (floor(y - vertical_radius) - 1).max(mask.min_y);
    let max_y = (floor(y + vertical_radius) + 1).min(mask.max_y);
    let min_zi = (floor(z - horizontal_radius) - min_bz - 1).max(0);
    let max_zi = (floor(z + horizontal_radius) - min_bz).min(15);
    for xi in min_xi..=max_xi {
        let wx = min_bx + xi;
        let xd = (wx as f64 + 0.5 - x) / horizontal_radius;
        for zi in min_zi..=max_zi {
            let wz = min_bz + zi;
            let zd = (wz as f64 + 0.5 - z) / horizontal_radius;
            if xd * xd + zd * zd >= 1.0 {
                continue;
            }
            let mut wy = max_y;
            while wy > min_y {
                let yd = (wy as f64 - 0.5 - y) / vertical_radius;
                if !skip(xd, yd, zd, wy) {
                    mask.carve(xi, wy, zi);
                }
                wy -= 1;
            }
        }
    }
}

/// `WorldCarver.canReach`.
fn can_reach(chunk_x: i32, chunk_z: i32, x: f64, z: f64, step: i32, total: i32, thickness: f32) -> bool {
    let xd = x - ((chunk_x << 4) + 8) as f64;
    let zd = z - ((chunk_z << 4) + 8) as f64;
    let remaining = (total - step) as f64;
    let rr = (thickness + 2.0 + 16.0) as f64;
    xd * xd + zd * zd - remaining * remaining <= rr * rr
}

impl Carver {
    pub fn parse(json: &Value) -> Result<Carver> {
        let ty = codec::type_name(json)?;
        let fp = |n: &str| FloatProvider::parse(field(json, n)?).with_context(|| n.to_owned());
        Ok(match ty.as_str() {
            "cave" => Carver::Cave {
                probability: codec::f32_field(json, "probability")?,
                y: HeightProvider::parse(field(json, "y")?)?,
                count: IntProvider::parse(field(json, "count")?)?,
                thickness: fp("thickness")?,
                weird_thickness_bias: codec::opt_bool(json, "weird_thickness_bias", false)?,
                room_vertical_radius_multiplier: fp("room_vertical_radius_multiplier")?,
                horizontal_radius_multiplier: fp("horizontal_radius_multiplier")?,
                vertical_radius_multiplier: fp("vertical_radius_multiplier")?,
                start_vertical_radius_multiplier: match json.get("start_vertical_radius_multiplier") {
                    Some(v) => FloatProvider::parse(v)?,
                    None => FloatProvider::Constant(1.0),
                },
                floor_level: fp("floor_level")?,
            },
            "canyon" => {
                let s = field(json, "shape")?;
                let sp = |n: &str| FloatProvider::parse(field(s, n)?).with_context(|| n.to_owned());
                Carver::Canyon {
                    probability: codec::f32_field(json, "probability")?,
                    y: HeightProvider::parse(field(json, "y")?)?,
                    vertical_rotation: fp("vertical_rotation")?,
                    shape: CanyonShape {
                        distance_factor: sp("distance_factor")?,
                        thickness: sp("thickness")?,
                        width_smoothness: codec::i32_field(s, "width_smoothness")?,
                        horizontal_radius_factor: sp("horizontal_radius_factor")?,
                        vertical_radius_default_factor: codec::f32_field(
                            s,
                            "vertical_radius_default_factor",
                        )?,
                        vertical_radius_center_factor: codec::f32_field(s, "vertical_radius_center_factor")?,
                        y_scale: sp("y_scale")?,
                    },
                }
            }
            other => bail!("unknown carver type {other}"),
        })
    }

    /// `isStartChunk`.
    pub fn is_start_chunk(&self, random: &mut dyn RandomSource) -> bool {
        let p = match self {
            Carver::Cave { probability, .. } | Carver::Canyon { probability, .. } => *probability,
        };
        random.next_float() <= p
    }

    /// `carve(context, random, chunkPos, sourceChunkPos, output)`.
    pub fn carve(
        &self,
        ctx: &GenerationContext,
        random: &mut dyn RandomSource,
        chunk_x: i32,
        chunk_z: i32,
        source_x: i32,
        source_z: i32,
        mask: &mut CarvingMask,
    ) {
        match self {
            Carver::Cave {
                y,
                count,
                thickness,
                weird_thickness_bias,
                room_vertical_radius_multiplier,
                horizontal_radius_multiplier,
                vertical_radius_multiplier,
                start_vertical_radius_multiplier,
                floor_level,
                ..
            } => {
                let max_distance = (4 * 2 - 1) << 4;
                let caves = count.sample(random);
                for _ in 0..caves {
                    let x = ((source_x << 4) + random.next_int_bounded(16)) as f64;
                    let yy = y.sample(random, ctx) as f64;
                    let z = ((source_z << 4) + random.next_int_bounded(16)) as f64;
                    let hrm = horizontal_radius_multiplier.sample(random) as f64;
                    let vrm = vertical_radius_multiplier.sample(random) as f64;
                    let svrm = start_vertical_radius_multiplier.sample(random) as f64;
                    let floor_level = floor_level.sample(random) as f64;
                    let skip = move |xd: f64, yd: f64, zd: f64, _y: i32| -> bool {
                        if yd <= floor_level {
                            true
                        } else {
                            xd * xd + yd * yd + zd * zd >= 1.0
                        }
                    };
                    let mut tunnels = 1;
                    if random.next_int_bounded(4) == 0 {
                        let y_scale = room_vertical_radius_multiplier.sample(random) as f64;
                        let t = 1.0 + random.next_float() * 6.0;
                        let hr = 1.5 + (mth_sin((PI_F / 2.0) as f64) * t) as f64;
                        let vr = hr * y_scale;
                        carve_ellipsoid(chunk_x, chunk_z, x + 1.0, yy, z, hr, vr, mask, &skip);
                        tunnels += random.next_int_bounded(4);
                    }
                    for _ in 0..tunnels {
                        let hrot = random.next_float() * (std::f64::consts::PI * 2.0) as f32;
                        let vrot = (random.next_float() - 0.5) / 4.0;
                        let mut t = thickness.sample(random);
                        if *weird_thickness_bias && random.next_int_bounded(10) == 0 {
                            t *= random.next_float() * random.next_float() * 3.0 + 1.0;
                        }
                        let distance = max_distance - random.next_int_bounded(max_distance / 4);
                        let seed = random.next_long();
                        create_tunnel(
                            chunk_x, chunk_z, seed, x, yy, z, hrm, vrm, t, hrot, vrot, 0, distance, svrm,
                            mask, &skip,
                        );
                    }
                }
            }
            Carver::Canyon {
                y,
                vertical_rotation,
                shape,
                ..
            } => {
                let max_distance = (4 * 2 - 1) * 16;
                let x = ((source_x << 4) + random.next_int_bounded(16)) as f64;
                let yy = y.sample(random, ctx);
                let z = ((source_z << 4) + random.next_int_bounded(16)) as f64;
                let hrot = random.next_float() * (std::f64::consts::PI * 2.0) as f32;
                let vrot = vertical_rotation.sample(random);
                let y_scale = shape.y_scale.sample(random) as f64;
                let thickness = shape.thickness.sample(random);
                let distance = (max_distance as f32 * shape.distance_factor.sample(random)) as i32;
                let seed = random.next_long();
                canyon_carve(
                    ctx, shape, chunk_x, chunk_z, seed, x, yy as f64, z, thickness, hrot, vrot, 0, distance,
                    y_scale, mask,
                );
            }
        }
    }
}

fn create_tunnel(
    chunk_x: i32,
    chunk_z: i32,
    seed: i64,
    mut x: f64,
    mut y: f64,
    mut z: f64,
    hrm: f64,
    vrm: f64,
    thickness: f32,
    mut hrot: f32,
    mut vrot: f32,
    step: i32,
    dist: i32,
    y_scale: f64,
    mask: &mut CarvingMask,
    skip: &dyn Fn(f64, f64, f64, i32) -> bool,
) {
    let mut random = AnyRandom::Legacy(LegacyRandom::new(seed));
    let split = random.next_int_bounded(dist / 2) + dist / 4;
    let steep = random.next_int_bounded(6) == 0;
    let mut y_rota = 0.0f32;
    let mut x_rota = 0.0f32;
    for current in step..dist {
        let hr = 1.5 + (mth_sin((PI_F * current as f32 / dist as f32) as f64) * thickness) as f64;
        let vr = hr * y_scale;
        let cos_x = mth_cos(vrot as f64);
        x += (mth_cos(hrot as f64) * cos_x) as f64;
        y += mth_sin(vrot as f64) as f64;
        z += (mth_sin(hrot as f64) * cos_x) as f64;
        vrot *= if steep { 0.92 } else { 0.7 };
        vrot += x_rota * 0.1;
        hrot += y_rota * 0.1;
        x_rota *= 0.9;
        y_rota *= 0.75;
        x_rota += (random.next_float() - random.next_float()) * random.next_float() * 2.0;
        y_rota += (random.next_float() - random.next_float()) * random.next_float() * 4.0;
        if current == split && thickness > 1.0 {
            let s1 = random.next_long();
            let t1 = random.next_float() * 0.5 + 0.5;
            create_tunnel(
                chunk_x,
                chunk_z,
                s1,
                x,
                y,
                z,
                hrm,
                vrm,
                t1,
                hrot - PI_F / 2.0,
                vrot / 3.0,
                current,
                dist,
                1.0,
                mask,
                skip,
            );
            let s2 = random.next_long();
            let t2 = random.next_float() * 0.5 + 0.5;
            create_tunnel(
                chunk_x,
                chunk_z,
                s2,
                x,
                y,
                z,
                hrm,
                vrm,
                t2,
                hrot + PI_F / 2.0,
                vrot / 3.0,
                current,
                dist,
                1.0,
                mask,
                skip,
            );
            return;
        }
        if random.next_int_bounded(4) != 0 {
            if !can_reach(chunk_x, chunk_z, x, z, current, dist, thickness) {
                return;
            }
            carve_ellipsoid(chunk_x, chunk_z, x, y, z, hr * hrm, vr * vrm, mask, skip);
        }
    }
}

fn canyon_carve(
    ctx: &GenerationContext,
    shape: &CanyonShape,
    chunk_x: i32,
    chunk_z: i32,
    seed: i64,
    mut x: f64,
    mut y: f64,
    mut z: f64,
    thickness: f32,
    mut hrot: f32,
    mut vrot: f32,
    step: i32,
    distance: i32,
    y_scale: f64,
    mask: &mut CarvingMask,
) {
    let mut random = AnyRandom::Legacy(LegacyRandom::new(seed));
    let depth = ctx.height;
    let mut widths = vec![0.0f32; depth as usize];
    let mut width = 1.0f32;
    for yi in 0..depth {
        if yi == 0 || random.next_int_bounded(shape.width_smoothness) == 0 {
            width = 1.0 + random.next_float() * random.next_float();
        }
        widths[yi as usize] = width * width;
    }
    let min_gen_y = ctx.min_y;
    let skip = |xd: f64, yd: f64, zd: f64, wy: i32| -> bool {
        let yi = wy - min_gen_y;
        (xd * xd + zd * zd) * widths[(yi - 1) as usize] as f64 + yd * yd / 6.0 >= 1.0
    };
    let mut y_rota = 0.0f32;
    let mut x_rota = 0.0f32;
    for current in step..distance {
        let mut hr = 1.5 + (mth_sin((current as f32 * PI_F / distance as f32) as f64) * thickness) as f64;
        let mut vr = hr * y_scale;
        hr *= shape.horizontal_radius_factor.sample(&mut random) as f64;
        // updateVerticalRadius(random, verticalRadius, (float) distance, (float) currentStep)
        let multiplier = 1.0 - abs_f32(0.5 - current as f32 / distance as f32) * 2.0;
        let factor = shape.vertical_radius_default_factor + shape.vertical_radius_center_factor * multiplier;
        vr = factor as f64 * vr * random_between_f32(&mut random, 0.75, 1.0) as f64;
        let xc = mth_cos(vrot as f64);
        let xs = mth_sin(vrot as f64);
        x += (mth_cos(hrot as f64) * xc) as f64;
        y += xs as f64;
        z += (mth_sin(hrot as f64) * xc) as f64;
        vrot *= 0.7;
        vrot += x_rota * 0.05;
        hrot += y_rota * 0.05;
        x_rota *= 0.8;
        y_rota *= 0.5;
        x_rota += (random.next_float() - random.next_float()) * random.next_float() * 2.0;
        y_rota += (random.next_float() - random.next_float()) * random.next_float() * 4.0;
        if random.next_int_bounded(4) != 0 {
            if !can_reach(chunk_x, chunk_z, x, z, current, distance, thickness) {
                return;
            }
            carve_ellipsoid(chunk_x, chunk_z, x, y, z, hr, vr, mask, &skip);
        }
    }
}
