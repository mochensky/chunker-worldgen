//! `BlendedNoise` ("old_blended_noise" density function).

use std::sync::Arc;

use super::perlin::SmearedPerlinNoise;
use super::stack::{NoiseLayer, NoiseStack, OctaveNoise};
use crate::density::BlendedNoiseParams;
use crate::density::sampler::{Sampler, SamplerRef};
use crate::math::Interval;
use crate::random::RandomSource;

const BASE_SCALE: f64 = 684.412;
/// `LIMIT_FACTOR = 0.99998474F` (float literal promoted to double).
const LIMIT_FACTOR: f64 = 0.999_984_74_f32 as f64;
const MAIN_FACTOR: f64 = 12.75;

fn create_fbm(
    random: &mut dyn RandomSource,
    first_octave: i32,
    smear_scale_y: f64,
    mut value_factor: f64,
) -> NoiseStack {
    assert!(first_octave <= 0, "firstOctave>0");
    let octaves = -first_octave + 1;
    let mut factor = 1.0f64;
    value_factor /= 2.0f64.powi(octaves) - 1.0;
    let mut layers = Vec::with_capacity(octaves as usize);
    for _ in (0..octaves).rev() {
        layers.push(NoiseLayer {
            noise: OctaveNoise::Smeared(SmearedPerlinNoise::new(random, smear_scale_y * factor)),
            frequency: factor,
            amplitude: value_factor as f32,
        });
        factor /= 2.0;
        value_factor *= 2.0;
    }
    NoiseStack::new(layers)
}

/// `BlendedNoise.compileSampler(random)`.
pub fn compile_blended_noise(p: &BlendedNoiseParams, random: &mut dyn RandomSource) -> SamplerRef {
    let y_mul = BASE_SCALE * p.y_scale;
    let xz_mul = BASE_SCALE * p.xz_scale;
    let limit_smear = y_mul * p.smear_scale_multiplier;
    let main_smear = limit_smear / p.y_factor;
    let min_limit = Arc::new(create_fbm(random, -15, limit_smear, LIMIT_FACTOR));
    let max_limit = Arc::new(create_fbm(random, -15, limit_smear, LIMIT_FACTOR));
    let main = Arc::new(create_fbm(random, -7, main_smear, MAIN_FACTOR));
    let min_s = Arc::new(Sampler::Noise {
        noise: min_limit,
        xz_scale: xz_mul,
        y_scale: y_mul,
    });
    let max_s = Arc::new(Sampler::Noise {
        noise: max_limit,
        xz_scale: xz_mul,
        y_scale: y_mul,
    });
    let main_s = Arc::new(Sampler::Noise {
        noise: main,
        xz_scale: xz_mul / p.xz_factor,
        y_scale: y_mul / p.y_factor,
    });
    let choice = Arc::new(Sampler::Clamp(Arc::new(Sampler::ConstAdd(main_s, 0.5)), 0.0, 1.0));
    Arc::new(Sampler::Lerp(choice, min_s, max_s))
}

/// `BlendedNoise.range()` / `computeFbmRange(-15, yMultiplier * smear, LIMIT_FACTOR)`.
pub fn blended_noise_range(p: &BlendedNoiseParams) -> Interval {
    let smear = BASE_SCALE * p.y_scale * p.smear_scale_multiplier;
    let octaves = 16;
    let mut factor = 1.0f64;
    let mut value_factor = LIMIT_FACTOR / (2.0f64.powi(octaves) - 1.0);
    let mut range = Interval::of_exact(0.0);
    for _ in (0..octaves).rev() {
        let layer = Interval::mul(
            SmearedPerlinNoise::range_for(smear * factor),
            Interval::of_exact(value_factor as f32),
        );
        range = Interval::add(range, layer);
        factor /= 2.0;
        value_factor *= 2.0;
    }
    range
}
