//! Spawn variants (`VariantUtils.selectVariantToSpawn` over `PriorityProvider` selectors) and the
//! sound variant registries.

use anyhow::{Context, Result};
use chunker_data::Identifier;
use serde_json::Value;

use crate::biome::{BiomeId, BiomeRegistry};
use crate::codec::{as_array, as_f64, as_i32, as_str};
use crate::feature::registry::{BiomeSet, Parser};
use crate::random::RandomSource;
use crate::registry::WorldgenRegistries;
use crate::structure::geom::BlockPos;

/// `SpawnCondition`.
enum Condition {
    Biome(BiomeSet),
    Structure(Vec<Identifier>),
    /// `MinMaxBounds.Doubles`.
    MoonBrightness(Option<f64>, Option<f64>),
}

/// What a condition sees (`SpawnContext`).
pub struct SpawnContext<'a> {
    pub pos: BlockPos,
    pub biome: BiomeId,
    /// `structureManager().getStructureWithPieceAt(pos, structures).isValid()`.
    pub has_structure_piece: &'a dyn Fn(BlockPos, &[Identifier]) -> bool,
    /// The moon brightness of the region's static environment attributes (no timelines, so full moon).
    pub moon_brightness: f32,
}

impl Condition {
    fn parse(parser: &Parser, reg: &WorldgenRegistries, json: &Value) -> Result<Self> {
        Ok(match as_str(&json["type"])? {
            "minecraft:biome" => Condition::Biome(parser.biome_set(&json["biomes"])?),
            "minecraft:structure" => {
                let ids = match &json["structures"] {
                    Value::String(s) => match s.strip_prefix('#') {
                        Some(tag) => reg.packs.resolve_tag(
                            "worldgen/structure",
                            &Identifier::parse(tag).context("structure tag")?,
                            &|_| true,
                        )?,
                        None => vec![Identifier::parse(s).context("structure id")?],
                    },
                    v => as_array(v)?
                        .iter()
                        .map(|s| Identifier::parse(as_str(s)?).context("structure id"))
                        .collect::<Result<_>>()?,
                };
                Condition::Structure(ids)
            }
            "minecraft:moon_brightness" => match &json["range"] {
                Value::Number(_) => {
                    let v = as_f64(&json["range"])?;
                    Condition::MoonBrightness(Some(v), Some(v))
                }
                range => Condition::MoonBrightness(
                    range.get("min").map(as_f64).transpose()?,
                    range.get("max").map(as_f64).transpose()?,
                ),
            },
            other => anyhow::bail!("unknown spawn condition {other}"),
        })
    }

    fn test(&self, ctx: &SpawnContext) -> bool {
        match self {
            Condition::Biome(set) => set.contains(ctx.biome),
            Condition::Structure(ids) => (ctx.has_structure_piece)(ctx.pos, ids),
            Condition::MoonBrightness(min, max) => {
                let v = ctx.moon_brightness as f64;
                !min.is_some_and(|m| m > v) && !max.is_some_and(|m| m < v)
            }
        }
    }
}

/// `PriorityProvider.Selector`: an optional condition and a priority.
type Selector = (Option<Condition>, i32);

/// A variant registry (`cow_variant`, `cat_variant`, ...) in registry order.
pub struct Variants {
    entries: Vec<(Identifier, Vec<Selector>)>,
}

impl Variants {
    pub fn load(reg: &WorldgenRegistries, biomes: &BiomeRegistry, kind: &str) -> Result<Self> {
        let parser = Parser::new(reg, biomes);
        let mut entries = Vec::new();
        for id in reg.packs.list(kind) {
            let json = reg.packs.require_json(kind, &id)?;
            let mut selectors = Vec::new();
            if let Some(list) = json.get("spawn_conditions") {
                for s in as_array(list)? {
                    let condition = s
                        .get("condition")
                        .map(|c| Condition::parse(&parser, reg, c))
                        .transpose()
                        .with_context(|| format!("{kind} {id}"))?;
                    selectors.push((condition, as_i32(&s["priority"])?));
                }
            }
            entries.push((id, selectors));
        }
        Ok(Self { entries })
    }

    /// `PriorityProvider.pick`: the matching selectors of the highest matching priority, then a
    /// uniform pick (`Util.getRandomSafe`, no random draw when nothing matches).
    pub fn pick(&self, ctx: &SpawnContext, random: &mut dyn RandomSource) -> Option<&Identifier> {
        let mut unpacked: Vec<(&Identifier, i32, Option<&Condition>)> = Vec::new();
        for (id, selectors) in &self.entries {
            for (condition, priority) in selectors {
                unpacked.push((id, *priority, condition.as_ref()));
            }
        }
        // Stable, like `List.sort` with `HIGHEST_PRIORITY_FIRST`.
        unpacked.sort_by_key(|e| std::cmp::Reverse(e.1));
        let mut highest = i32::MIN;
        let mut selected = Vec::new();
        for (id, priority, condition) in unpacked {
            if priority < highest {
                continue;
            }
            if condition.is_none_or(|c| c.test(ctx)) {
                highest = priority;
                selected.push(id);
            }
        }
        if selected.is_empty() {
            return None;
        }
        Some(selected[random.next_int_bounded(selected.len() as i32) as usize])
    }
}

/// A plain registry picked from with `Registry.getRandom` (sound variants).
pub struct RandomRegistry(Vec<Identifier>);

impl RandomRegistry {
    pub fn load(reg: &WorldgenRegistries, kind: &str) -> Self {
        Self(reg.packs.list(kind))
    }

    pub fn pick(&self, random: &mut dyn RandomSource) -> Option<&Identifier> {
        if self.0.is_empty() {
            return None;
        }
        Some(&self.0[random.next_int_bounded(self.0.len() as i32) as usize])
    }
}
