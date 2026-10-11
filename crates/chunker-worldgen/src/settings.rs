//! `NoiseGeneratorSettings` (`worldgen/noise_settings/*.json`).

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use chunker_data::blocks::BlockState;
use serde_json::Value;

use crate::codec::{self, field, i32_field};
use crate::density::Df;
use crate::registry::WorldgenRegistries;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoiseSettings {
    pub min_y: i32,
    pub height: i32,
}

#[derive(Clone)]
pub struct NoiseRouter {
    pub temperature: Df,
    pub vegetation: Df,
    pub continents: Df,
    pub erosion: Df,
    pub depth: Df,
    pub ridges: Df,
    pub chunk_surface_level: Df,
    pub final_density: Df,
}

/// `Aquifer.Config`.
#[derive(Clone)]
pub struct AquiferConfig {
    pub barrier: Df,
    pub fluid_level_floodedness: Df,
    pub fluid_level_spread: Df,
    pub lava: Df,
    pub exclusion: Df,
    pub surface_level: Df,
}

pub struct NoiseGeneratorSettings {
    pub id: Identifier,
    pub noise: NoiseSettings,
    pub default_block: BlockState,
    pub default_fluid: BlockState,
    pub router: NoiseRouter,
    /// Material rule as raw JSON (or registry id string); compiled by the material system.
    pub material_rule: Value,
    pub spawn_target: Value,
    pub sea_level: i32,
    pub disable_mob_generation: bool,
    pub aquifers: Option<AquiferConfig>,
    pub legacy_random_source: bool,
}

impl WorldgenRegistries {
    /// Parses a block state in either `"ns:name[props]"` or `{"Name":..., "Properties":...}` form.
    pub fn parse_block_state(&self, json: &Value) -> Result<BlockState> {
        match json {
            Value::String(_) | Value::Object(_) => self.blocks.parse_state_json(json),
            other => bail!("invalid block state {other}"),
        }
    }

    pub fn noise_settings(&self, id: &Identifier) -> Result<Arc<NoiseGeneratorSettings>> {
        let json = self.packs.require_json("worldgen/noise_settings", id)?;
        self.parse_noise_settings(id.clone(), &json)
            .with_context(|| format!("noise settings {id}"))
            .map(Arc::new)
    }

    pub fn parse_noise_settings(&self, id: Identifier, json: &Value) -> Result<NoiseGeneratorSettings> {
        let noise = field(json, "noise")?;
        let noise = NoiseSettings {
            min_y: i32_field(noise, "min_y")?,
            height: i32_field(noise, "height")?,
        };
        if noise.height % 16 != 0 || noise.min_y % 16 != 0 {
            bail!("noise min_y/height must be multiples of 16");
        }
        let r = field(json, "noise_router")?;
        let df = |name: &str| -> Result<Df> {
            self.parse_density_function(field(r, name)?)
                .with_context(|| format!("noise_router.{name}"))
        };
        let router = NoiseRouter {
            temperature: df("temperature")?,
            vegetation: df("vegetation")?,
            continents: df("continents")?,
            erosion: df("erosion")?,
            depth: df("depth")?,
            ridges: df("ridges")?,
            chunk_surface_level: df("chunk_surface_level")?,
            final_density: df("final_density")?,
        };
        let aquifers = match json.get("aquifers") {
            None => None,
            Some(a) => {
                let df = |name: &str| -> Result<Df> {
                    self.parse_density_function(field(a, name)?)
                        .with_context(|| format!("aquifers.{name}"))
                };
                Some(AquiferConfig {
                    barrier: df("barrier")?,
                    fluid_level_floodedness: df("fluid_level_floodedness")?,
                    fluid_level_spread: df("fluid_level_spread")?,
                    lava: df("lava")?,
                    exclusion: df("exclusion")?,
                    surface_level: df("surface_level")?,
                })
            }
        };
        Ok(NoiseGeneratorSettings {
            id,
            noise,
            default_block: self.parse_block_state(field(json, "default_block")?)?,
            default_fluid: self.parse_block_state(field(json, "default_fluid")?)?,
            router,
            material_rule: field(json, "material_rule")?.clone(),
            spawn_target: field(json, "spawn_target")?.clone(),
            sea_level: i32_field(json, "sea_level")?,
            disable_mob_generation: codec::as_bool(field(json, "disable_mob_generation")?)?,
            aquifers,
            legacy_random_source: codec::as_bool(field(json, "legacy_random_source")?)?,
        })
    }
}
