//! Material rule / condition definitions (`worldgen/material_rule`, `worldgen/material_condition`).

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use chunker_data::blocks::BlockState;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::biome::{BiomeId, BiomeRegistry};
use crate::codec::{self, field};
use crate::density::Df;
use crate::registry::WorldgenRegistries;

/// `VerticalAnchor`.
#[derive(Clone, Copy, Debug)]
pub enum VerticalAnchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
    RelativeToSeaLevel(i32),
}

/// `WorldGenerationContext`.
#[derive(Clone, Copy, Debug)]
pub struct GenerationContext {
    pub min_y: i32,
    pub height: i32,
    pub sea_level: i32,
}

impl VerticalAnchor {
    pub fn parse(v: &Value) -> Result<Self> {
        if let Some(y) = v.get("absolute") {
            Ok(Self::Absolute(codec::as_i32(y)?))
        } else if let Some(y) = v.get("above_bottom") {
            Ok(Self::AboveBottom(codec::as_i32(y)?))
        } else if let Some(y) = v.get("below_top") {
            Ok(Self::BelowTop(codec::as_i32(y)?))
        } else if let Some(y) = v.get("relative_to_sea_level") {
            Ok(Self::RelativeToSeaLevel(codec::as_i32(y)?))
        } else {
            bail!("invalid vertical anchor {v}")
        }
    }

    pub fn resolve(self, ctx: &GenerationContext) -> i32 {
        match self {
            Self::Absolute(y) => y,
            Self::AboveBottom(o) => ctx.min_y + o,
            Self::BelowTop(o) => ctx.height - 1 + ctx.min_y - o,
            Self::RelativeToSeaLevel(o) => ctx.sea_level + o,
        }
    }
}

#[derive(Debug)]
pub enum MaterialCondition {
    Biome(Vec<BiomeId>),
    NoiseThreshold {
        noise: Identifier,
        min: f64,
        max: f64,
        is_3d: bool,
    },
    VerticalGradient {
        random_name: String,
        true_at_and_below: VerticalAnchor,
        false_at_and_above: VerticalAnchor,
    },
    YAbove {
        anchor: VerticalAnchor,
        surface_depth_multiplier: i32,
        add_stone_depth: bool,
    },
    Water {
        offset: i32,
        surface_depth_multiplier: i32,
        add_stone_depth: bool,
    },
    Temperature,
    Steep,
    Not(Arc<MaterialCondition>),
    Hole,
    AbovePreliminarySurface,
    StoneDepth {
        offset: i32,
        add_surface_depth: bool,
        secondary_depth_range: i32,
        ceiling: bool,
    },
}

#[derive(Debug)]
pub enum MaterialRule {
    Block(BlockState),
    Bandlands,
    Sequence(Vec<Arc<MaterialRule>>),
    Condition(Arc<MaterialCondition>, Arc<MaterialRule>),
    OreVein {
        ore: BlockState,
        raw_ore: BlockState,
        filler: BlockState,
        raw_ore_chance: f32,
        density: Df,
        richness: Df,
        filler_gap: Df,
    },
}

/// Parser with memoised registry references.
pub struct MaterialParser<'a> {
    pub reg: &'a WorldgenRegistries,
    pub biomes: &'a BiomeRegistry,
    rules: Mutex<FxHashMap<Identifier, Arc<MaterialRule>>>,
    conditions: Mutex<FxHashMap<Identifier, Arc<MaterialCondition>>>,
}

impl<'a> MaterialParser<'a> {
    pub fn new(reg: &'a WorldgenRegistries, biomes: &'a BiomeRegistry) -> Self {
        Self {
            reg,
            biomes,
            rules: Mutex::default(),
            conditions: Mutex::default(),
        }
    }

    pub fn rule(&self, json: &Value) -> Result<Arc<MaterialRule>> {
        if let Value::String(id) = json {
            let id = Identifier::parse(id).context("bad material rule id")?;
            if let Some(r) = self.rules.lock().get(&id) {
                return Ok(r.clone());
            }
            let body = self.reg.packs.require_json("worldgen/material_rule", &id)?;
            let r = self.rule(&body).with_context(|| format!("material rule {id}"))?;
            self.rules.lock().insert(id, r.clone());
            return Ok(r);
        }
        let ty = codec::type_name(json)?;
        let blocks = &self.reg.blocks;
        Ok(Arc::new(match ty.as_str() {
            "block" => MaterialRule::Block(self.reg.parse_block_state(field(json, "result_state")?)?),
            "bandlands" => MaterialRule::Bandlands,
            "sequence" => {
                let list = field(json, "sequence")?.as_array().context("sequence")?;
                MaterialRule::Sequence(list.iter().map(|r| self.rule(r)).collect::<Result<_>>()?)
            }
            "condition" => MaterialRule::Condition(
                self.condition(field(json, "if_true")?)?,
                self.rule(field(json, "then_run")?)?,
            ),
            "ore_vein" => {
                let _ = blocks;
                MaterialRule::OreVein {
                    ore: self.reg.parse_block_state(field(json, "ore_block")?)?,
                    raw_ore: self.reg.parse_block_state(field(json, "raw_ore_block")?)?,
                    filler: self.reg.parse_block_state(field(json, "filler_block")?)?,
                    raw_ore_chance: codec::f32_field(json, "raw_ore_chance")?,
                    density: self.reg.parse_density_function(field(json, "density")?)?,
                    richness: self.reg.parse_density_function(field(json, "richness")?)?,
                    filler_gap: self.reg.parse_density_function(field(json, "filler_gap")?)?,
                }
            }
            other => bail!("unknown material rule type {other}"),
        }))
    }

    pub fn condition(&self, json: &Value) -> Result<Arc<MaterialCondition>> {
        if let Value::String(id) = json {
            let id = Identifier::parse(id).context("bad material condition id")?;
            if let Some(c) = self.conditions.lock().get(&id) {
                return Ok(c.clone());
            }
            let body = self.reg.packs.require_json("worldgen/material_condition", &id)?;
            let c = self
                .condition(&body)
                .with_context(|| format!("material condition {id}"))?;
            self.conditions.lock().insert(id, c.clone());
            return Ok(c);
        }
        let ty = codec::type_name(json)?;
        Ok(Arc::new(match ty.as_str() {
            "biome" => MaterialCondition::Biome(crate::biome::parse_biome_holder_set(
                self.reg,
                self.biomes,
                field(json, "biome_is")?,
            )?),
            "noise_threshold" => MaterialCondition::NoiseThreshold {
                noise: Identifier::parse(codec::str_field(json, "noise")?).context("noise id")?,
                min: codec::f64_field(json, "min_threshold")?,
                max: codec::f64_field(json, "max_threshold")?,
                is_3d: codec::opt_bool(json, "is_3d", false)?,
            },
            "vertical_gradient" => MaterialCondition::VerticalGradient {
                random_name: Identifier::parse(codec::str_field(json, "random_name")?)
                    .context("random_name")?
                    .to_string(),
                true_at_and_below: VerticalAnchor::parse(field(json, "true_at_and_below")?)?,
                false_at_and_above: VerticalAnchor::parse(field(json, "false_at_and_above")?)?,
            },
            "y_above" => MaterialCondition::YAbove {
                anchor: VerticalAnchor::parse(field(json, "anchor")?)?,
                surface_depth_multiplier: codec::i32_field(json, "surface_depth_multiplier")?,
                add_stone_depth: codec::as_bool(field(json, "add_stone_depth")?)?,
            },
            "water" => MaterialCondition::Water {
                offset: codec::i32_field(json, "offset")?,
                surface_depth_multiplier: codec::i32_field(json, "surface_depth_multiplier")?,
                add_stone_depth: codec::as_bool(field(json, "add_stone_depth")?)?,
            },
            "temperature" => MaterialCondition::Temperature,
            "steep" => MaterialCondition::Steep,
            "not" => MaterialCondition::Not(self.condition(field(json, "invert")?)?),
            "hole" => MaterialCondition::Hole,
            "above_preliminary_surface" => MaterialCondition::AbovePreliminarySurface,
            "stone_depth" => MaterialCondition::StoneDepth {
                offset: codec::i32_field(json, "offset")?,
                add_surface_depth: codec::as_bool(field(json, "add_surface_depth")?)?,
                secondary_depth_range: codec::i32_field(json, "secondary_depth_range")?,
                ceiling: match codec::str_field(json, "surface_type")? {
                    "ceiling" => true,
                    "floor" => false,
                    s => bail!("unknown surface_type {s}"),
                },
            },
            other => bail!("unknown material condition type {other}"),
        }))
    }
}
