//! Biome definitions (`worldgen/biome/*.json`) and the climate helpers used during generation.

use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::codec::{self, field};
use crate::noise::SimplexNoise;
use crate::random::{AnyRandom, LegacyRandom, WorldgenRandom};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemperatureModifier {
    None,
    Frozen,
}

#[derive(Clone, Debug)]
pub struct BiomeClimate {
    pub has_precipitation: bool,
    pub temperature: f32,
    pub temperature_modifier: TemperatureModifier,
    pub downfall: f32,
}

/// Parsed biome. Generation settings (carvers/features) are added by later stages.
#[derive(Clone, Debug)]
pub struct BiomeData {
    pub climate: BiomeClimate,
    pub json: Value,
}

impl BiomeData {
    pub fn parse(json: Value) -> Result<Self> {
        let modifier = match json.get("temperature_modifier").and_then(Value::as_str) {
            None | Some("none") => TemperatureModifier::None,
            Some("frozen") => TemperatureModifier::Frozen,
            Some(m) => bail!("unknown temperature modifier {m}"),
        };
        let climate = BiomeClimate {
            has_precipitation: codec::as_bool(field(&json, "has_precipitation")?)
                .context("has_precipitation")?,
            temperature: codec::f32_field(&json, "temperature")?,
            temperature_modifier: modifier,
            downfall: codec::f32_field(&json, "downfall")?,
        };
        Ok(Self { climate, json })
    }

    /// `TemperatureModifier.modifyTemperature`.
    fn modify_temperature(&self, x: i32, z: i32, base: f32) -> f32 {
        match self.climate.temperature_modifier {
            TemperatureModifier::None => base,
            TemperatureModifier::Frozen => {
                let n = noises();
                let large = (n.frozen_get(x as f64 * 0.05, z as f64 * 0.05) * 7.0) as f64;
                let edge = n.biome_info.get2(x as f64 * 0.2, z as f64 * 0.2) as f64;
                if large + edge < 0.3 {
                    let small = n.biome_info.get2(x as f64 * 0.09, z as f64 * 0.09) as f64;
                    if small < 0.8 {
                        return 0.2;
                    }
                }
                base
            }
        }
    }

    /// `Biome.getHeightAdjustedTemperature` (the vanilla cache does not change results).
    pub fn temperature(&self, x: i32, y: i32, z: i32, sea_level: i32) -> f32 {
        let adjusted = self.modify_temperature(x, z, self.climate.temperature);
        let snow_level = sea_level + 17;
        if y > snow_level {
            let v = noises()
                .temperature
                .get2((x as f32 / 8.0) as f64, (z as f32 / 8.0) as f64)
                * 8.0;
            adjusted - (v + y as f32 - snow_level as f32) * 0.05 / 40.0
        } else {
            adjusted
        }
    }

    pub fn warm_enough_to_rain(&self, x: i32, y: i32, z: i32, sea_level: i32) -> bool {
        self.temperature(x, y, z, sea_level) >= 0.15
    }

    pub fn cold_enough_to_snow(&self, x: i32, y: i32, z: i32, sea_level: i32) -> bool {
        !self.warm_enough_to_rain(x, y, z, sea_level)
    }

    pub fn should_melt_frozen_ocean_iceberg_slightly(&self, x: i32, y: i32, z: i32, sea_level: i32) -> bool {
        self.temperature(x, y, z, sea_level) > 0.1
    }
}

/// `Biome.TEMPERATURE_NOISE`, `FROZEN_TEMPERATURE_NOISE`, `BIOME_INFO_NOISE`.
pub struct BiomeNoises {
    pub temperature: SimplexNoise,
    frozen: [(SimplexNoise, f64, f32); 3],
    pub biome_info: SimplexNoise,
}

impl BiomeNoises {
    /// `NoiseStack.get(x, y)` over the three simplex layers.
    fn frozen_get(&self, x: f64, z: f64) -> f32 {
        let mut v = 0.0f32;
        for (n, freq, amp) in &self.frozen {
            v += amp * n.get2(x * freq, z * freq);
        }
        v
    }
}

pub fn noises() -> &'static BiomeNoises {
    static N: OnceLock<BiomeNoises> = OnceLock::new();
    N.get_or_init(|| {
        let wg = |seed: i64| WorldgenRandom::new(AnyRandom::Legacy(LegacyRandom::new(seed)));
        let temperature = SimplexNoise::new_discard_offset(&mut wg(1234), true);
        let mut r = wg(3456);
        let a = SimplexNoise::new_discard_offset(&mut r, true);
        let b = SimplexNoise::new_discard_offset(&mut r, true);
        let c = SimplexNoise::new_discard_offset(&mut r, true);
        let biome_info = SimplexNoise::new_discard_offset(&mut wg(2345), true);
        BiomeNoises {
            temperature,
            frozen: [(a, 1.0, 0.14285715), (b, 0.5, 0.2857143), (c, 0.25, 0.5714286)],
            biome_info,
        }
    })
}
