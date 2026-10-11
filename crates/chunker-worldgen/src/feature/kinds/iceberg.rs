//! `IcebergFeature`.

use chunker_data::blocks::BlockState;

use crate::feature::Region;
use crate::math;
use crate::random::RandomSource;
use crate::structure::geom::BlockPos;

pub struct Iceberg {
    pub state: BlockState,
}

/// `Math.pow(x, 2.0)` is exact (`x * x`).
fn sq(x: f64) -> f64 {
    x * x
}

impl Iceberg {
    fn is_iceberg(region: &Region, s: BlockState) -> bool {
        let b = &region.env.b;
        let id = region.env.blocks.block_of(s);
        id == b.packed_ice || id == b.snow_block || id == b.blue_ice
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let origin = BlockPos::new(origin.x, region.env.sea_level, origin.z);
        let snow_on_top = random.next_double() > 0.7;
        let angle = random.next_double() * 2.0 * std::f64::consts::PI;
        let ellipse_a = 11 - random.next_int_bounded(5);
        let ellipse_c = 3 + random.next_int_bounded(3);
        let is_ellipse = random.next_double() > 0.7;
        let mut over = if is_ellipse {
            random.next_int_bounded(6) + 6
        } else {
            random.next_int_bounded(15) + 3
        };
        if !is_ellipse && random.next_double() > 0.9 {
            over += random.next_int_bounded(19) + 7;
        }
        let under = (over + random.next_int_bounded(11)).min(18);
        let width = (over + random.next_int_bounded(7) - random.next_int_bounded(5)).min(11);
        let a = if is_ellipse { ellipse_a } else { 11 };
        for xo in -a..a {
            for zo in -a..a {
                for y_off in 0..over {
                    let radius = if is_ellipse {
                        radius_ellipse(y_off, over, width)
                    } else {
                        radius_round(random, y_off, over, width)
                    };
                    if is_ellipse || xo < radius {
                        self.block(
                            region,
                            random,
                            origin,
                            over,
                            xo,
                            y_off,
                            zo,
                            radius,
                            a,
                            is_ellipse,
                            ellipse_c,
                            angle,
                            snow_on_top,
                        );
                    }
                }
            }
        }
        self.smooth(region, origin, width, over, is_ellipse, ellipse_a);
        for xo in -a..a {
            for zo in -a..a {
                let mut y_off = -1;
                while y_off > -under {
                    let new_a = if is_ellipse {
                        math::ceil((a as f32 * (1.0 - sq(y_off as f64) as f32 / (under as f32 * 8.0))) as f64)
                    } else {
                        a
                    };
                    let radius = radius_steep(random, -y_off, under, width);
                    if xo < radius {
                        self.block(
                            region,
                            random,
                            origin,
                            under,
                            xo,
                            y_off,
                            zo,
                            radius,
                            new_a,
                            is_ellipse,
                            ellipse_c,
                            angle,
                            snow_on_top,
                        );
                    }
                    y_off -= 1;
                }
            }
        }
        let cut_out = if is_ellipse {
            random.next_double() > 0.1
        } else {
            random.next_double() > 0.7
        };
        if cut_out {
            self.cut_out(
                random, region, width, over, origin, is_ellipse, ellipse_a, angle, ellipse_c,
            );
        }
        true
    }

    fn cut_out(
        &self,
        random: &mut dyn RandomSource,
        region: &mut Region,
        width: i32,
        height: i32,
        origin: BlockPos,
        is_ellipse: bool,
        ellipse_a: i32,
        shape_angle: f64,
        ellipse_c: i32,
    ) {
        let sign_x = if random.next_bool() { -1 } else { 1 };
        let sign_z = if random.next_bool() { -1 } else { 1 };
        let mut x_off = random.next_int_bounded((width / 2 - 2).max(1));
        if random.next_bool() {
            x_off = width / 2 + 1 - random.next_int_bounded((width - width / 2 - 1).max(1));
        }
        let mut z_off = random.next_int_bounded((width / 2 - 2).max(1));
        if random.next_bool() {
            z_off = width / 2 + 1 - random.next_int_bounded((width - width / 2 - 1).max(1));
        }
        if is_ellipse {
            x_off = random.next_int_bounded((ellipse_a - 5).max(1));
            z_off = x_off;
        }
        let local = BlockPos::new(sign_x * x_off, 0, sign_z * z_off);
        let angle = if is_ellipse {
            shape_angle + std::f64::consts::FRAC_PI_2
        } else {
            random.next_double() * 2.0 * std::f64::consts::PI
        };
        for y_off in 0..height - 3 {
            let radius = radius_round(random, y_off, height, width);
            self.carve(
                radius, y_off, origin, region, false, angle, local, ellipse_a, ellipse_c,
            );
        }
        let mut y_off = -1;
        // The bound draws a new random number on every check.
        while y_off > -height + random.next_int_bounded(5) {
            let radius = radius_steep(random, -y_off, height, width);
            self.carve(
                radius, y_off, origin, region, true, angle, local, ellipse_a, ellipse_c,
            );
            y_off -= 1;
        }
    }

    fn carve(
        &self,
        radius: i32,
        y_off: i32,
        origin: BlockPos,
        region: &mut Region,
        under_water: bool,
        angle: f64,
        local: BlockPos,
        ellipse_a: i32,
        ellipse_c: i32,
    ) {
        let a = radius + 1 + ellipse_a / 3;
        let c = (radius - 3).min(3) + ellipse_c / 2 - 1;
        let env = region.env;
        for xo in -a..a {
            for zo in -a..a {
                if ellipse_distance(xo, zo, local, a, c, angle) < 0.0 {
                    let p = origin.offset(xo, y_off, zo);
                    let s = region.get(p);
                    if Self::is_iceberg(region, s) || env.blocks.block_of(s) == env.b.snow_block {
                        if under_water {
                            region.set_and_update(p, env.blocks.block(env.b.water).default_state);
                        } else {
                            region.set_and_update(p, BlockState::AIR);
                            if region.is(p.above(), env.b.snow) {
                                region.set_and_update(p.above(), BlockState::AIR);
                            }
                        }
                    }
                }
            }
        }
    }

    fn block(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        height: i32,
        xo: i32,
        y_off: i32,
        zo: i32,
        radius: i32,
        a: i32,
        is_ellipse: bool,
        ellipse_c: i32,
        angle: f64,
        snow_on_top: bool,
    ) {
        let dist = if is_ellipse {
            ellipse_distance(
                xo,
                zo,
                BlockPos::ZERO,
                a,
                ellipse_c_at(y_off, height, ellipse_c),
                angle,
            )
        } else {
            circle_distance(xo, zo, radius, random)
        };
        // `if (signedDist < 0.0)`: NaN distances (zero-width ellipse rows) place nothing.
        if !(dist < 0.0) {
            return;
        }
        let p = origin.offset(xo, y_off, zo);
        let compare = if is_ellipse {
            -0.5
        } else {
            (-6 - random.next_int_bounded(3)) as f64
        };
        if dist > compare && random.next_double() > 0.9 {
            return;
        }
        // `setIcebergBlock`
        let env = region.env;
        let s = region.get(p);
        let id = env.blocks.block_of(s);
        let b = &env.b;
        if env.blocks.flags(s).is_air() || id == b.snow_block || id == b.ice || id == b.water {
            let h_diff = height - y_off;
            let randomness = !is_ellipse || random.next_double() > 0.05;
            let divisor = if is_ellipse { 3 } else { 2 };
            if snow_on_top
                && id != b.water
                && (h_diff as f64)
                    <= (random.next_int_bounded(1.max(height / divisor)) as f64) + height as f64 * 0.6
                && randomness
            {
                region.set_and_update(p, env.blocks.block(b.snow_block).default_state);
            } else {
                region.set_and_update(p, self.state);
            }
        }
    }

    fn smooth(
        &self,
        region: &mut Region,
        origin: BlockPos,
        width: i32,
        height: i32,
        is_ellipse: bool,
        ellipse_a: i32,
    ) {
        let a = if is_ellipse { ellipse_a } else { width / 2 };
        let snow = region.env.b.snow;
        for x in -a..=a {
            for z in -a..=a {
                for y_off in 0..=height {
                    let p = origin.offset(x, y_off, z);
                    let s = region.get(p);
                    let iceberg = Self::is_iceberg(region, s);
                    if !(iceberg || region.env.blocks.block_of(s) == snow) {
                        continue;
                    }
                    if region.flags(p.below()).is_air() {
                        region.set_and_update(p, BlockState::AIR);
                        region.set_and_update(p.above(), BlockState::AIR);
                    } else if iceberg {
                        let sides = [
                            p.offset(-1, 0, 0),
                            p.offset(1, 0, 0),
                            p.offset(0, 0, -1),
                            p.offset(0, 0, 1),
                        ];
                        let counter = sides
                            .iter()
                            .filter(|q| !Self::is_iceberg(region, region.get(**q)))
                            .count();
                        if counter >= 3 {
                            region.set_and_update(p, BlockState::AIR);
                        }
                    }
                }
            }
        }
    }
}

fn ellipse_c_at(y_off: i32, height: i32, c: i32) -> i32 {
    if y_off > 0 && height - y_off <= 3 {
        c - (4 - (height - y_off))
    } else {
        c
    }
}

fn circle_distance(xo: i32, zo: i32, radius: i32, random: &mut dyn RandomSource) -> f64 {
    let off = 10.0 * math::clamp_f32(random.next_float(), 0.2, 0.8) / radius as f32;
    off as f64 + sq(xo as f64) + sq(zo as f64) - sq(radius as f64)
}

fn ellipse_distance(xo: i32, zo: i32, origin: BlockPos, a: i32, c: i32, angle: f64) -> f64 {
    let dx = (xo - origin.x) as f64;
    let dz = (zo - origin.z) as f64;
    sq((dx * crate::math::trig::cos(angle) - dz * crate::math::trig::sin(angle)) / a as f64)
        + sq((dx * crate::math::trig::sin(angle) + dz * crate::math::trig::cos(angle)) / c as f64)
        - 1.0
}

fn radius_round(random: &mut dyn RandomSource, y_off: i32, height: i32, width: i32) -> i32 {
    let k = 3.5f32 - random.next_float();
    let mut scale = (1.0 - sq(y_off as f64) as f32 / (height as f32 * k)) * width as f32;
    if height > 15 + random.next_int_bounded(5) {
        let temp = if y_off < 3 + random.next_int_bounded(6) {
            y_off / 2
        } else {
            y_off
        };
        scale = (1.0 - temp as f32 / (height as f32 * k * 0.4)) * width as f32;
    }
    math::ceil((scale / 2.0) as f64)
}

fn radius_ellipse(y_off: i32, height: i32, width: i32) -> i32 {
    let scale = (1.0 - sq(y_off as f64) as f32 / (height as f32 * 1.0)) * width as f32;
    math::ceil((scale / 2.0) as f64)
}

fn radius_steep(random: &mut dyn RandomSource, y_off: i32, height: i32, width: i32) -> i32 {
    let k = 1.0f32 + random.next_float() / 2.0;
    let scale = (1.0 - y_off as f32 / (height as f32 * k)) * width as f32;
    math::ceil((scale / 2.0) as f64)
}
