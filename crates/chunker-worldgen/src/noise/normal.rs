use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::stack::{NoiseLayer, NoiseStack, OctaveNoise};
use super::{PERLIN_STANDARD_DEVIATION, PerlinNoise};
use crate::math::Interval;
use crate::random::{LegacyRandom, PositionalRandomFactory, RandomSource};

const INPUT_FACTOR: f64 = 1.0181268882175227;
const TARGET_DEVIATION: f64 = 0.3333333333333333;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalization {
    Disabled,
    Enabled,
    Legacy,
}

/// `NormalNoise.Parameters` (the JSON in `worldgen/noise`).
#[derive(Clone, Debug, PartialEq)]
pub struct NormalNoiseParams {
    pub base_amplitude: f64,
    pub base_octave: i32,
    pub octave_count: i32,
    pub normalize: Normalization,
    pub amplitude_modifiers: Vec<f64>,
}

impl NormalNoiseParams {
    pub fn from_json(json: &Value) -> Result<Self> {
        let base_amplitude = json
            .get("base_amplitude")
            .map(|v| v.as_f64().context("base_amplitude"))
            .transpose()?
            .unwrap_or(1.0);
        let base_octave = json
            .get("base_octave")
            .and_then(Value::as_i64)
            .context("base_octave")? as i32;
        let octave_count = json
            .get("octave_count")
            .map(|v| v.as_i64().context("octave_count"))
            .transpose()?
            .unwrap_or(1) as i32;
        let normalize = match json.get("normalize") {
            None => Normalization::Enabled,
            Some(Value::Bool(true)) => Normalization::Enabled,
            Some(Value::Bool(false)) => Normalization::Disabled,
            Some(Value::String(s)) if s == "legacy" => Normalization::Legacy,
            Some(other) => bail!("invalid normalization {other}"),
        };
        let amplitude_modifiers: Vec<f64> = match json.get("amplitude_modifiers") {
            None => Vec::new(),
            Some(v) => v
                .as_array()
                .context("amplitude_modifiers")?
                .iter()
                .map(|x| x.as_f64().context("amplitude"))
                .collect::<Result<_>>()?,
        };
        if !(1.0e-5_f32 as f64..=1_000_000.0).contains(&base_amplitude) {
            bail!("base_amplitude out of range: {base_amplitude}");
        }
        if !(-32..=32).contains(&base_octave) || !(1..=32).contains(&octave_count) {
            bail!("octave parameters out of range");
        }
        if !amplitude_modifiers.is_empty() && amplitude_modifiers.len() != octave_count as usize {
            bail!(
                "amplitude_modifiers had size {}, but octave_count was {octave_count}",
                amplitude_modifiers.len()
            );
        }
        Ok(Self {
            base_amplitude,
            base_octave,
            octave_count,
            normalize,
            amplitude_modifiers,
        })
    }
}

fn amplitude_modifier(modifiers: &[f64], index: usize) -> f64 {
    if modifiers.is_empty() {
        1.0
    } else {
        modifiers[index]
    }
}

#[derive(Clone, Copy, Debug)]
struct OctaveInfo {
    octave_index: i32,
    frequency: f64,
    amplitude: f64,
}

/// `NormalNoise`: parameters plus derived octave layout and normalisation factor.
#[derive(Clone, Debug)]
pub struct NormalNoise {
    pub params: NormalNoiseParams,
    octaves: Vec<OctaveInfo>,
    normalization_factor: f64,
    range: Interval,
}

fn build_octaves(
    base_octave: i32,
    base_amplitude: f64,
    octave_count: i32,
    normalize: bool,
    modifiers: &[f64],
) -> Vec<OctaveInfo> {
    let mut frequency = 2.0f64.powi(base_octave);
    let mut amplitude = base_amplitude;
    if normalize {
        amplitude = base_amplitude * (0.5f64.powi(-(octave_count - 1)) / (0.5f64.powi(-octave_count) - 1.0));
    }
    let mut out = Vec::with_capacity(octave_count as usize);
    for i in 0..octave_count as usize {
        let m = amplitude_modifier(modifiers, i);
        if m != 0.0 {
            out.push(OctaveInfo {
                octave_index: base_octave + i as i32,
                frequency,
                amplitude: amplitude * m,
            });
        }
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    out
}

fn estimate_deviation(octaves: &[OctaveInfo]) -> f64 {
    let mut variance = 0.0;
    for o in octaves {
        let d = PERLIN_STANDARD_DEVIATION * o.amplitude.abs();
        variance += d * d;
    }
    variance.sqrt()
}

fn compute_normalization_factor(target_amplitude: f64, octaves: &[OctaveInfo]) -> f64 {
    let input_deviation = estimate_deviation(octaves);
    if input_deviation == 0.0 {
        return 0.0;
    }
    let input_sum_deviation = input_deviation * 2.0f64.sqrt();
    let target_deviation = target_amplitude * TARGET_DEVIATION;
    target_deviation / input_sum_deviation
}

fn parity_expected_deviation(span: i32) -> f64 {
    0.1 * (1.0 + 1.0 / (span + 1) as f64)
}

fn compute_parity_normalization_factor(base_amplitude: f64, octave_count: i32, modifiers: &[f64]) -> f64 {
    let mut min_octave = i32::MAX;
    let mut max_octave = i32::MIN;
    for i in 0..octave_count {
        if amplitude_modifier(modifiers, i as usize) != 0.0 {
            min_octave = min_octave.min(i);
            max_octave = max_octave.max(i);
        }
    }
    base_amplitude * 0.5 * TARGET_DEVIATION / parity_expected_deviation(max_octave.wrapping_sub(min_octave))
}

impl NormalNoise {
    pub fn new(params: NormalNoiseParams) -> Self {
        let octaves = build_octaves(
            params.base_octave,
            params.base_amplitude,
            params.octave_count,
            params.normalize != Normalization::Disabled,
            &params.amplitude_modifiers,
        );
        // `DoubleStream.sum()` uses Kahan-compensated summation.
        let mut target_amplitude = kahan_sum(octaves.iter().map(|o| o.amplitude.abs()));
        let mut normalization_factor = compute_normalization_factor(target_amplitude, &octaves);
        if params.normalize == Normalization::Legacy && normalization_factor != 0.0 {
            let parity = compute_parity_normalization_factor(
                params.base_amplitude,
                params.octave_count,
                &params.amplitude_modifiers,
            );
            target_amplitude *= parity / normalization_factor;
            normalization_factor = parity;
        }
        let range = Interval::of_symmetric((target_amplitude * TARGET_DEVIATION * 6.0) as f32);
        Self {
            params,
            octaves,
            normalization_factor,
            range,
        }
    }

    pub fn range(&self) -> Interval {
        self.range
    }

    /// `NormalNoise.createParity(firstOctave, amplitudes)`: the base amplitude is chosen so the
    /// normalized noise matches the old parity normalization.
    pub fn create_parity(first_octave: i32, amplitudes: &[f64]) -> Self {
        let count = amplitudes.len() as i32;
        let octaves = build_octaves(first_octave, 1.0, count, true, amplitudes);
        let target = kahan_sum(octaves.iter().map(|o| o.amplitude.abs()));
        let new_factor = compute_normalization_factor(target, &octaves);
        let base_amplitude = if new_factor == 0.0 {
            1.0
        } else {
            compute_parity_normalization_factor(1.0, count, amplitudes) / new_factor
        };
        let modifiers = if amplitudes.iter().all(|&a| a == 1.0) {
            Vec::new()
        } else {
            amplitudes.to_vec()
        };
        Self::new(NormalNoiseParams {
            base_amplitude,
            base_octave: first_octave,
            octave_count: count,
            normalize: Normalization::Enabled,
            amplitude_modifiers: modifiers,
        })
    }

    /// `NormalNoise.create(random)`.
    pub fn create(&self, random: &mut dyn RandomSource) -> NoiseStack {
        let first: PositionalRandomFactory = random.fork_positional();
        let second: PositionalRandomFactory = random.fork_positional();
        let mut layers = Vec::with_capacity(self.octaves.len() * 2);
        for o in &self.octaves {
            let seed = format!("octave_{}", o.octave_index);
            let first_noise = PerlinNoise::new(&mut first.from_hash_of(&seed));
            let second_noise = PerlinNoise::new(&mut second.from_hash_of(&seed));
            let value_factor = (self.normalization_factor * o.amplitude) as f32;
            layers.push(NoiseLayer {
                noise: OctaveNoise::Perlin(first_noise),
                frequency: o.frequency,
                amplitude: value_factor,
            });
            layers.push(NoiseLayer {
                noise: OctaveNoise::Perlin(second_noise),
                frequency: o.frequency * INPUT_FACTOR,
                amplitude: value_factor,
            });
        }
        NoiseStack::new(layers)
    }

    /// `NormalNoise.createForLegacyNetherBiome(random)`.
    pub fn create_for_legacy_nether_biome(&self, random: &mut LegacyRandom) -> NoiseStack {
        let amplitudes: Vec<f64> = if self.params.amplitude_modifiers.is_empty() {
            vec![1.0; self.params.octave_count as usize]
        } else {
            self.params.amplitude_modifiers.clone()
        };
        let first = legacy_fbm_for_nether_biome(random, self.params.base_octave, &amplitudes);
        let second = legacy_fbm_for_nether_biome(random, self.params.base_octave, &amplitudes);
        let value_factor = (self.normalization_factor * self.params.base_amplitude) as f32;
        let mut layers = Vec::new();
        NoiseStack::push_stack(&mut layers, &first, 1.0, value_factor);
        NoiseStack::push_stack(&mut layers, &second, INPUT_FACTOR, value_factor);
        NoiseStack::new(layers)
    }

    /// Exposed for completeness of `NormalNoise.createParity` (used by vanilla bootstrap only).
    pub fn params(&self) -> &NormalNoiseParams {
        &self.params
    }
}

/// Kahan summation as used by `java.util.stream.DoubleStream.sum()` (`Collectors.sumWithCompensation`).
pub(crate) fn kahan_sum(values: impl Iterator<Item = f64>) -> f64 {
    // DoubleStream.sum: summation[0]=high-order sum, [1]=compensation, [2]=simple sum.
    let mut sum = 0.0f64;
    let mut c = 0.0f64;
    let mut simple = 0.0f64;
    for v in values {
        simple += v;
        let y = v - c;
        let t = sum + y;
        c = (t - sum) - y;
        sum = t;
    }
    let tmp = sum - c;
    if tmp.is_nan() && simple.is_infinite() {
        simple
    } else {
        tmp
    }
}

/// `LegacyFbmInitializer.createForLegacyNetherBiome`.
fn legacy_fbm_for_nether_biome(
    random: &mut LegacyRandom,
    first_octave: i32,
    amplitudes: &[f64],
) -> NoiseStack {
    let octaves = amplitudes.len() as i32;
    let zero_octave_index = -first_octave;
    let mut levels: Vec<Option<PerlinNoise>> = vec![None; octaves as usize];
    let zero_octave = PerlinNoise::new(random);
    if zero_octave_index >= 0 && zero_octave_index < octaves && amplitudes[zero_octave_index as usize] != 0.0
    {
        levels[zero_octave_index as usize] = Some(zero_octave);
    }
    let mut i = zero_octave_index - 1;
    while i >= 0 {
        if i < octaves {
            if amplitudes[i as usize] != 0.0 {
                levels[i as usize] = Some(PerlinNoise::new(random));
            } else {
                random.consume_count(262);
            }
        } else {
            random.consume_count(262);
        }
        i -= 1;
    }
    assert!(
        zero_octave_index >= octaves - 1,
        "Positive octaves are temporarily disabled"
    );
    let mut factor = 2.0f64.powi(-zero_octave_index);
    let mut value_factor = 2.0f64.powi(octaves - 1) / (2.0f64.powi(octaves) - 1.0);
    let mut layers = Vec::new();
    for (ix, noise) in levels.into_iter().enumerate() {
        if let Some(noise) = noise {
            layers.push(NoiseLayer {
                noise: OctaveNoise::Perlin(noise),
                frequency: factor,
                amplitude: (value_factor * amplitudes[ix]) as f32,
            });
        }
        factor *= 2.0;
        value_factor /= 2.0;
    }
    NoiseStack::new(layers)
}
