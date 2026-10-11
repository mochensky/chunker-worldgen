//! `Aquifer` (disabled and noise based).

use chunker_data::blocks::BlockState;
use rustc_hash::FxHashMap;

use crate::density::sampler::SamplerRef;
use crate::density::{DensityVolume, SamplerContext};
use crate::math::{clamp_f64, clamped_map_f64, floor_div, map_f64, quantize};
use crate::random::{PositionalRandomFactory, RandomSource};

/// `DimensionType.WAY_BELOW_MIN_Y`.
pub const WAY_BELOW_MIN_Y: i32 = -2032 << 4;

/// Block ids the aquifer logic needs to compare against.
#[derive(Clone, Copy)]
pub struct FluidBlocks {
    pub air: BlockState,
    pub water_default: BlockState,
    pub lava_default: BlockState,
}

/// `Aquifer.FluidStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidStatus {
    pub fluid_level: i32,
    pub fluid_type: BlockState,
}

impl FluidStatus {
    #[inline(always)]
    pub fn at(&self, y: i32, air: BlockState) -> BlockState {
        if y < self.fluid_level {
            self.fluid_type
        } else {
            air
        }
    }
}

/// `NoiseBasedChunkGenerator.createFluidPicker`.
#[derive(Clone, Copy)]
pub struct FluidPicker {
    pub lava: FluidStatus,
    pub sea: FluidStatus,
    pub sea_level: i32,
}

impl FluidPicker {
    pub fn new(sea_level: i32, default_fluid: BlockState, lava: BlockState) -> Self {
        Self {
            lava: FluidStatus {
                fluid_level: -54,
                fluid_type: lava,
            },
            sea: FluidStatus {
                fluid_level: sea_level,
                fluid_type: default_fluid,
            },
            sea_level,
        }
    }

    #[inline(always)]
    pub fn compute(&self, _x: i32, y: i32, _z: i32) -> FluidStatus {
        if y < (-54i32).min(self.sea_level) {
            self.lava
        } else {
            self.sea
        }
    }
}

/// Samplers of `Aquifer.Config`.
#[derive(Clone)]
pub struct AquiferSamplers {
    pub barrier: SamplerRef,
    pub floodedness: SamplerRef,
    pub spread: SamplerRef,
    pub lava: SamplerRef,
    pub exclusion: SamplerRef,
    pub surface_level: SamplerRef,
}

pub enum Aquifer {
    Disabled(FluidPicker),
    Noise(Box<NoiseBasedAquifer>),
}

pub struct NoiseBasedAquifer {
    s: AquiferSamplers,
    random: PositionalRandomFactory,
    cache: Vec<Option<FluidStatus>>,
    location_cache: Vec<i64>,
    picker: FluidPicker,
    should_schedule_fluid_update: bool,
    skip_sampling_above_y: i32,
    min_grid_x: i32,
    min_grid_y: i32,
    min_grid_z: i32,
    grid_size_x: i32,
    grid_size_z: i32,
    surface_level_cache: FxHashMap<i64, i32>,
    blocks: FluidBlocks,
    reg: std::sync::Arc<chunker_data::blocks::BlockRegistry>,
}

const SURFACE_SAMPLING_OFFSETS: [[i32; 2]; 13] = [
    [0, 0],
    [-2, -1],
    [-1, -1],
    [0, -1],
    [1, -1],
    [-3, 0],
    [-2, 0],
    [-1, 0],
    [1, 0],
    [-2, 1],
    [-1, 1],
    [0, 1],
    [1, 1],
];

fn grid_x(c: i32) -> i32 {
    c >> 4
}
fn from_grid_x(g: i32, o: i32) -> i32 {
    (g << 4) + o
}
fn grid_y(c: i32) -> i32 {
    floor_div(c, 12)
}
fn from_grid_y(g: i32, o: i32) -> i32 {
    g * 12 + o
}

/// `BlockPos.asLong` / `getX/Y/Z`.
fn pack(x: i32, y: i32, z: i32) -> i64 {
    crate::density::context::pack_block_pos(x, y, z)
}
/// x is the top 26 bits, z the next 26, y the low 12 (all sign-extended).
fn unpack_x(p: i64) -> i32 {
    (p >> 38) as i32
}
fn unpack_y(p: i64) -> i32 {
    (p << 52 >> 52) as i32
}
fn unpack_z(p: i64) -> i32 {
    (p << 26 >> 38) as i32
}

/// `ChunkPos.pack(x, z)`.
fn chunk_pos_pack(x: i32, z: i32) -> i64 {
    (x as i64 & 0xFFFF_FFFF) | ((z as i64 & 0xFFFF_FFFF) << 32)
}

fn similarity(d1: i32, d2: i32) -> f64 {
    1.0 - (d2 - d1) as f64 / 25.0
}

impl Aquifer {
    /// `computeSubstance`: `None` means "solid" (use the default block).
    #[inline]
    pub fn compute_substance(
        &mut self,
        ctx: &mut SamplerContext,
        x: i32,
        y: i32,
        z: i32,
        density: f64,
    ) -> Option<BlockState> {
        match self {
            Aquifer::Disabled(p) => {
                if density > 0.0 {
                    None
                } else {
                    Some(p.compute(x, y, z).at(y, BlockState::AIR))
                }
            }
            Aquifer::Noise(a) => a.compute_substance(ctx, x, y, z, density),
        }
    }

    pub fn should_schedule_fluid_update(&self) -> bool {
        match self {
            Aquifer::Disabled(_) => false,
            Aquifer::Noise(a) => a.should_schedule_fluid_update,
        }
    }
}

impl NoiseBasedAquifer {
    pub fn new(
        ctx: &mut SamplerContext,
        samplers: AquiferSamplers,
        random: PositionalRandomFactory,
        volume: &DensityVolume,
        picker: FluidPicker,
        blocks: FluidBlocks,
        reg: std::sync::Arc<chunker_data::blocks::BlockRegistry>,
    ) -> Self {
        let min_grid_x = grid_x(volume.min_x - 5);
        let max_grid_x = grid_x(volume.max_x() - 5) + 1;
        let grid_size_x = max_grid_x - min_grid_x + 1;
        let min_grid_y = grid_y(volume.min_y + 1) - 1;
        let max_grid_y = grid_y(volume.max_y() + 1) + 1;
        let grid_size_y = max_grid_y - min_grid_y + 1;
        let min_grid_z = grid_x(volume.min_z - 5);
        let max_grid_z = grid_x(volume.max_z() - 5) + 1;
        let grid_size_z = max_grid_z - min_grid_z + 1;
        let total = (grid_size_x * grid_size_y * grid_size_z) as usize;
        let mut a = Self {
            s: samplers,
            random,
            cache: vec![None; total],
            location_cache: vec![i64::MAX; total],
            picker,
            should_schedule_fluid_update: false,
            skip_sampling_above_y: 0,
            min_grid_x,
            min_grid_y,
            min_grid_z,
            grid_size_x,
            grid_size_z,
            surface_level_cache: FxHashMap::default(),
            blocks,
            reg,
        };
        let max_surface = a.max_surface_level(
            ctx,
            from_grid_x(min_grid_x, 0),
            from_grid_x(min_grid_z, 0),
            from_grid_x(max_grid_x, 9),
            from_grid_x(max_grid_z, 9),
        );
        let max_adjusted = max_surface + 8;
        let skip_grid_y = grid_y(max_adjusted + 12) + 1;
        a.skip_sampling_above_y = from_grid_y(skip_grid_y, 11) - 1;
        a
    }

    fn surface_level(&mut self, ctx: &mut SamplerContext, bx: i32, bz: i32) -> i32 {
        let qx = (bx >> 2) << 2;
        let qz = (bz >> 2) << 2;
        let key = chunk_pos_pack(qx, qz);
        if let Some(v) = self.surface_level_cache.get(&key) {
            return *v;
        }
        let v = crate::math::floor_f32(self.s.surface_level.sample_value(ctx, qx, 0, qz));
        self.surface_level_cache.insert(key, v);
        v
    }

    fn max_surface_level(
        &mut self,
        ctx: &mut SamplerContext,
        min_x: i32,
        min_z: i32,
        max_x: i32,
        max_z: i32,
    ) -> i32 {
        let (min_qx, max_qx, min_qz, max_qz) = (min_x >> 2, max_x >> 2, min_z >> 2, max_z >> 2);
        let vol = DensityVolume::new(
            max_qx - min_qx + 1,
            1,
            max_qz - min_qz + 1,
            min_qx << 2,
            0,
            min_qz << 2,
            4,
            1,
            4,
        );
        let mut buf = ctx.acquire(vol.size());
        self.s.surface_level.sample_volume(ctx, &mut buf, &vol);
        let mut max_y = i32::MIN;
        for z in 0..vol.size_z {
            for x in 0..vol.size_x {
                let level = crate::math::floor_f32(buf[vol.index(x, 0, z)]);
                self.surface_level_cache
                    .insert(chunk_pos_pack(vol.block_x(x), vol.block_z(z)), level);
                if level > max_y {
                    max_y = level;
                }
            }
        }
        ctx.release(buf);
        max_y
    }

    #[inline(always)]
    fn index(&self, gx: i32, gy: i32, gz: i32) -> usize {
        let x = gx - self.min_grid_x;
        let y = gy - self.min_grid_y;
        let z = gz - self.min_grid_z;
        ((y * self.grid_size_z + z) * self.grid_size_x + x) as usize
    }

    fn is_lava(&self, s: BlockState) -> bool {
        self.reg.block_of(s) == self.reg.block_of(self.blocks.lava_default)
    }

    pub fn compute_substance(
        &mut self,
        ctx: &mut SamplerContext,
        x: i32,
        y: i32,
        z: i32,
        density: f64,
    ) -> Option<BlockState> {
        let air = self.blocks.air;
        if density > 0.0 {
            self.should_schedule_fluid_update = false;
            return None;
        }
        let global = self.picker.compute(x, y, z);
        if y > self.skip_sampling_above_y {
            self.should_schedule_fluid_update = false;
            return Some(global.at(y, air));
        }
        if self.is_lava(global.at(y, air)) {
            self.should_schedule_fluid_update = false;
            return Some(self.blocks.lava_default);
        }
        let xa = grid_x(x - 5);
        let ya = grid_y(y + 1);
        let za = grid_x(z - 5);
        let (mut d1, mut d2, mut d3, mut d4) = (i32::MAX, i32::MAX, i32::MAX, i32::MAX);
        let (mut c1, mut c2, mut c3, mut c4) = (0usize, 0usize, 0usize, 0usize);
        for x1 in 0..=1 {
            for y1 in -1..=1 {
                for z1 in 0..=1 {
                    let gx = xa + x1;
                    let gy = ya + y1;
                    let gz = za + z1;
                    let index = self.index(gx, gy, gz);
                    let existing = self.location_cache[index];
                    let location = if existing != i64::MAX {
                        existing
                    } else {
                        let mut r = self.random.at(gx, gy, gz);
                        let lx = from_grid_x(gx, r.next_int_bounded(10));
                        let ly = from_grid_y(gy, r.next_int_bounded(9));
                        let lz = from_grid_x(gz, r.next_int_bounded(10));
                        let l = pack(lx, ly, lz);
                        self.location_cache[index] = l;
                        l
                    };
                    let dx = unpack_x(location) - x;
                    let dy = unpack_y(location) - y;
                    let dz = unpack_z(location) - z;
                    let nd = dx * dx + dy * dy + dz * dz;
                    if d1 >= nd {
                        c4 = c3;
                        c3 = c2;
                        c2 = c1;
                        c1 = index;
                        d4 = d3;
                        d3 = d2;
                        d2 = d1;
                        d1 = nd;
                    } else if d2 >= nd {
                        c4 = c3;
                        c3 = c2;
                        c2 = index;
                        d4 = d3;
                        d3 = d2;
                        d2 = nd;
                    } else if d3 >= nd {
                        c4 = c3;
                        c3 = index;
                        d4 = d3;
                        d3 = nd;
                    } else if d4 >= nd {
                        c4 = index;
                        d4 = nd;
                    }
                }
            }
        }
        let flowing = similarity(100, 144);
        let s1 = self.status(ctx, c1);
        let sim12 = similarity(d1, d2);
        let fluid_state = s1.at(y, air);
        if sim12 <= 0.0 {
            if sim12 >= flowing {
                let s2 = self.status(ctx, c2);
                self.should_schedule_fluid_update = s1 != s2;
            } else {
                self.should_schedule_fluid_update = false;
            }
            return Some(fluid_state);
        }
        let is_water = self.reg.block_of(fluid_state) == self.reg.block_of(self.blocks.water_default);
        if is_water && self.is_lava(self.picker.compute(x, y - 1, z).at(y - 1, air)) {
            self.should_schedule_fluid_update = true;
            return Some(fluid_state);
        }
        let mut barrier_value = f64::NAN;
        let s2 = self.status(ctx, c2);
        let barrier12 = sim12 * self.pressure(ctx, x, y, z, &mut barrier_value, s1, s2);
        if density + barrier12 > 0.0 {
            self.should_schedule_fluid_update = false;
            return None;
        }
        let s3 = self.status(ctx, c3);
        let sim13 = similarity(d1, d3);
        if sim13 > 0.0 {
            let b = sim12 * sim13 * self.pressure(ctx, x, y, z, &mut barrier_value, s1, s3);
            if density + b > 0.0 {
                self.should_schedule_fluid_update = false;
                return None;
            }
        }
        let sim23 = similarity(d2, d3);
        if sim23 > 0.0 {
            let b = sim12 * sim23 * self.pressure(ctx, x, y, z, &mut barrier_value, s2, s3);
            if density + b > 0.0 {
                self.should_schedule_fluid_update = false;
                return None;
            }
        }
        let may12 = s1 != s2;
        let may23 = sim23 >= flowing && s2 != s3;
        let may13 = sim13 >= flowing && s1 != s3;
        if !may12 && !may23 && !may13 {
            self.should_schedule_fluid_update =
                sim13 >= flowing && similarity(d1, d4) >= flowing && s1 != self.status(ctx, c4);
        } else {
            self.should_schedule_fluid_update = true;
        }
        Some(fluid_state)
    }

    fn pressure(
        &mut self,
        ctx: &mut SamplerContext,
        x: i32,
        y: i32,
        z: i32,
        barrier: &mut f64,
        s1: FluidStatus,
        s2: FluidStatus,
    ) -> f64 {
        let air = self.blocks.air;
        let t1 = s1.at(y, air);
        let t2 = s2.at(y, air);
        let water = self.reg.block_of(self.blocks.water_default);
        let is_water = |s: BlockState| self.reg.block_of(s) == water;
        if (self.is_lava(t1) && is_water(t2)) || (is_water(t1) && self.is_lava(t2)) {
            return 2.0;
        }
        let diff = (s1.fluid_level - s2.fluid_level).abs();
        if diff == 0 {
            return 0.0;
        }
        let average = 0.5 * (s1.fluid_level + s2.fluid_level) as f64;
        let above = y as f64 + 0.5 - average;
        let base = diff as f64 / 2.0;
        let toward_middle = base - above.abs();
        let gradient = if above > 0.0 {
            let center = 0.0 + toward_middle;
            if center > 0.0 { center / 1.5 } else { center / 2.5 }
        } else {
            let center = 3.0 + toward_middle;
            if center > 0.0 { center / 3.0 } else { center / 10.0 }
        };
        let noise = if !(gradient < -2.0) && !(gradient > 2.0) {
            if barrier.is_nan() {
                let v = self.s.barrier.sample_value(ctx, x, y, z) as f64;
                *barrier = v;
                v
            } else {
                *barrier
            }
        } else {
            0.0
        };
        2.0 * (noise + gradient)
    }

    fn status(&mut self, ctx: &mut SamplerContext, index: usize) -> FluidStatus {
        if let Some(s) = self.cache[index] {
            return s;
        }
        let l = self.location_cache[index];
        let s = self.compute_fluid(ctx, unpack_x(l), unpack_y(l), unpack_z(l));
        self.cache[index] = Some(s);
        s
    }

    fn compute_fluid(&mut self, ctx: &mut SamplerContext, x: i32, y: i32, z: i32) -> FluidStatus {
        let air = self.blocks.air;
        let global = self.picker.compute(x, y, z);
        let mut lowest = i32::MAX;
        let top = y + 12;
        let bottom = y - 12;
        let mut center_under = false;
        for off in SURFACE_SAMPLING_OFFSETS {
            let sx = x + (off[0] << 4);
            let sz = z + (off[1] << 4);
            let level = self.surface_level(ctx, sx, sz);
            let adjusted = level + 8;
            let start = off[0] == 0 && off[1] == 0;
            if start && bottom > adjusted {
                return global;
            }
            let pokes = top > adjusted;
            if pokes || start {
                let at_surface = self.picker.compute(sx, adjusted, sz);
                if !self.reg.flags(at_surface.at(adjusted, air)).is_air() {
                    if start {
                        center_under = true;
                    }
                    if pokes {
                        return at_surface;
                    }
                }
            }
            lowest = lowest.min(level);
        }
        let surface = self.compute_surface_level(ctx, x, y, z, global, lowest, center_under);
        FluidStatus {
            fluid_level: surface,
            fluid_type: self.compute_fluid_type(ctx, x, y, z, global, surface),
        }
    }

    fn compute_surface_level(
        &mut self,
        ctx: &mut SamplerContext,
        x: i32,
        y: i32,
        z: i32,
        global: FluidStatus,
        lowest: i32,
        center_under: bool,
    ) -> i32 {
        let (partially, fully);
        if self.s.exclusion.sample_value(ctx, x, y, z) as f64 > 0.0 {
            partially = -1.0;
            fully = -1.0;
        } else {
            let below = lowest + 8 - y;
            let factor = if center_under {
                clamped_map_f64(below as f64, 0.0, 64.0, 1.0, 0.0)
            } else {
                0.0
            };
            let noise = clamp_f64(self.s.floodedness.sample_value(ctx, x, y, z) as f64, -1.0, 1.0);
            let fully_threshold = map_f64(factor, 1.0, 0.0, -0.3, 0.8);
            let partially_threshold = map_f64(factor, 1.0, 0.0, -0.8, 0.4);
            partially = noise - partially_threshold;
            fully = noise - fully_threshold;
        }
        if fully > 0.0 {
            global.fluid_level
        } else if partially > 0.0 {
            self.randomized_surface_level(ctx, x, y, z, lowest)
        } else {
            WAY_BELOW_MIN_Y
        }
    }

    fn randomized_surface_level(
        &mut self,
        ctx: &mut SamplerContext,
        x: i32,
        y: i32,
        z: i32,
        lowest: i32,
    ) -> i32 {
        let cx = floor_div(x, 16);
        let cy = floor_div(y, 40);
        let cz = floor_div(z, 16);
        let middle = cy * 40 + 20;
        let spread = (self.s.spread.sample_value(ctx, cx, cy, cz) * 10.0) as f64;
        let quantized = quantize(spread, 3);
        lowest.min(middle + quantized)
    }

    fn compute_fluid_type(
        &mut self,
        ctx: &mut SamplerContext,
        x: i32,
        y: i32,
        z: i32,
        global: FluidStatus,
        level: i32,
    ) -> BlockState {
        let mut fluid = global.fluid_type;
        if level <= -10 && level != WAY_BELOW_MIN_Y && global.fluid_type != self.blocks.lava_default {
            let cx = floor_div(x, 64);
            let cy = floor_div(y, 40);
            let cz = floor_div(z, 64);
            let v = self.s.lava.sample_value(ctx, cx, cy, cz) as f64;
            if v.abs() > 0.3 {
                fluid = self.blocks.lava_default;
            }
        }
        fluid
    }
}
