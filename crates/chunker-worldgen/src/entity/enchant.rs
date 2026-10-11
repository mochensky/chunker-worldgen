//! Enchanting of spawn equipment: the data-driven `Enchantment` registry and
//! `EnchantmentHelper.selectEnchantment` behind `VanillaEnchantmentProviders.MOB_SPAWN_EQUIPMENT`.

use anyhow::{Context, Result};
use chunker_data::Identifier;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;

use crate::codec::{as_i32, as_str};
use crate::math;
use crate::random::RandomSource;
use crate::registry::WorldgenRegistries;

/// `Enchantment.Cost`.
#[derive(Clone, Copy, Debug)]
struct Cost {
    base: i32,
    per_level_above_first: i32,
}

impl Cost {
    fn parse(json: &Value) -> Result<Self> {
        Ok(Self {
            base: as_i32(&json["base"])?,
            per_level_above_first: as_i32(&json["per_level_above_first"])?,
        })
    }

    fn calculate(self, level: i32) -> i32 {
        self.base + self.per_level_above_first * (level - 1)
    }
}

struct Enchantment {
    id: Identifier,
    max_level: i32,
    weight: i32,
    min_cost: Cost,
    max_cost: Cost,
    supported_items: FxHashSet<Identifier>,
    primary_items: Option<FxHashSet<Identifier>>,
    exclusive_set: FxHashSet<usize>,
}

/// `EnchantmentsByCostWithDifficulty`.
struct ByCostWithDifficulty {
    enchantments: Vec<usize>,
    min_cost: i32,
    max_cost_span: i32,
}

pub struct Enchantments {
    list: Vec<Enchantment>,
    mob_spawn_equipment: Option<ByCostWithDifficulty>,
}

/// A `HolderSet` of a registry (`#tag`, an id or a list) resolved to ids.
fn holder_set(reg: &WorldgenRegistries, kind: &str, json: &Value) -> Result<Vec<Identifier>> {
    let parse = |s: &str| Identifier::parse(s).with_context(|| format!("invalid identifier {s}"));
    match json {
        Value::String(s) => match s.strip_prefix('#') {
            Some(tag) => reg.packs.resolve_tag(kind, &parse(tag)?, &|_| true),
            None => Ok(vec![parse(s)?]),
        },
        Value::Array(a) => a.iter().map(|v| parse(as_str(v)?)).collect(),
        _ => anyhow::bail!("expected a holder set, got {json}"),
    }
}

impl Enchantments {
    pub fn load(reg: &WorldgenRegistries) -> Result<Self> {
        let ids = reg.packs.list("enchantment");
        let index: FxHashMap<Identifier, usize> =
            ids.iter().cloned().enumerate().map(|(i, id)| (id, i)).collect();
        let mut list = Vec::with_capacity(ids.len());
        for id in &ids {
            let json = reg.packs.require_json("enchantment", id)?;
            let e = (|| -> Result<Enchantment> {
                let items = |v: &Value| -> Result<FxHashSet<Identifier>> {
                    Ok(holder_set(reg, "item", v)?.into_iter().collect())
                };
                let exclusive_set = match json.get("exclusive_set") {
                    Some(v) => holder_set(reg, "enchantment", v)?
                        .iter()
                        .filter_map(|e| index.get(e).copied())
                        .collect(),
                    None => FxHashSet::default(),
                };
                Ok(Enchantment {
                    id: id.clone(),
                    max_level: as_i32(&json["max_level"])?,
                    weight: as_i32(&json["weight"])?,
                    min_cost: Cost::parse(&json["min_cost"])?,
                    max_cost: Cost::parse(&json["max_cost"])?,
                    supported_items: items(&json["supported_items"])?,
                    primary_items: json.get("primary_items").map(items).transpose()?,
                    exclusive_set,
                })
            })()
            .with_context(|| format!("enchantment {id}"))?;
            list.push(e);
        }
        let provider_id = Identifier::vanilla("mob_spawn_equipment");
        let mob_spawn_equipment = match reg.packs.read_json("enchantment_provider", &provider_id)? {
            Some(json) if as_str(&json["type"])? == "minecraft:by_cost_with_difficulty" => {
                Some(ByCostWithDifficulty {
                    enchantments: holder_set(reg, "enchantment", &json["enchantments"])?
                        .iter()
                        .filter_map(|e| index.get(e).copied())
                        .collect(),
                    min_cost: as_i32(&json["min_cost"])?,
                    max_cost_span: as_i32(&json["max_cost_span"])?,
                })
            }
            _ => None,
        };
        Ok(Self {
            list,
            mob_spawn_equipment,
        })
    }

    /// `Enchantment.areCompatible`.
    fn compatible(&self, a: usize, b: usize) -> bool {
        a != b && !self.list[a].exclusive_set.contains(&b) && !self.list[b].exclusive_set.contains(&a)
    }

    /// `EnchantmentHelper.getAvailableEnchantmentResults` (never a book here).
    fn available(&self, value: i32, item: &Identifier, source: &[usize]) -> Vec<(usize, i32)> {
        let mut results = Vec::new();
        for &e in source {
            let ench = &self.list[e];
            let primary = ench.supported_items.contains(item)
                && ench.primary_items.as_ref().is_none_or(|p| p.contains(item));
            if !primary {
                continue;
            }
            for level in (1..=ench.max_level).rev() {
                if value >= ench.min_cost.calculate(level) && value <= ench.max_cost.calculate(level) {
                    results.push((e, level));
                    break;
                }
            }
        }
        results
    }

    /// `WeightedRandom.getRandomItem` by enchantment weight.
    fn pick(&self, random: &mut dyn RandomSource, candidates: &[(usize, i32)]) -> Option<(usize, i32)> {
        let total: i64 = candidates.iter().map(|&(e, _)| self.list[e].weight as i64).sum();
        if total == 0 {
            return None;
        }
        let mut selection = random.next_int_bounded(total as i32);
        for &c in candidates {
            selection -= self.list[c.0].weight;
            if selection < 0 {
                return Some(c);
            }
        }
        None
    }

    /// `EnchantmentHelper.selectEnchantment`; `enchantable` is the item's `enchantable` value.
    fn select(
        &self,
        random: &mut dyn RandomSource,
        item: &Identifier,
        enchantable: Option<i32>,
        mut cost: i32,
        source: &[usize],
    ) -> Vec<(usize, i32)> {
        let mut results = Vec::new();
        let Some(value) = enchantable else {
            return results;
        };
        cost += 1 + random.next_int_bounded(value / 4 + 1) + random.next_int_bounded(value / 4 + 1);
        let span = (random.next_float() + random.next_float() - 1.0) * 0.15;
        cost = math::clamp_i32(math::round_f32(cost as f32 + cost as f32 * span), 1, i32::MAX);
        let mut candidates = self.available(cost, item, source);
        if !candidates.is_empty() {
            results.extend(self.pick(random, &candidates));
            while random.next_int_bounded(50) <= cost {
                if let Some(&(last, _)) = results.last() {
                    candidates.retain(|&(e, _)| self.compatible(last, e));
                }
                if candidates.is_empty() {
                    break;
                }
                results.extend(self.pick(random, &candidates));
                cost /= 2;
            }
        }
        results
    }

    /// `EnchantmentHelper.enchantItemFromProvider(..., MOB_SPAWN_EQUIPMENT, ...)`: the enchantments
    /// to add to an item with `ItemEnchantments.Mutable.upgrade`, as (id, level).
    pub fn mob_spawn_equipment(
        &self,
        random: &mut dyn RandomSource,
        item: &Identifier,
        enchantable: Option<i32>,
        special_multiplier: f32,
    ) -> Vec<(&Identifier, i32)> {
        let Some(p) = &self.mob_spawn_equipment else {
            return Vec::new();
        };
        let max = p.min_cost + (special_multiplier * p.max_cost_span as f32) as i32;
        let cost = random.next_int_between_inclusive(p.min_cost, max);
        self.select(random, item, enchantable, cost, &p.enchantments)
            .into_iter()
            .map(|(e, level)| (&self.list[e].id, level))
            .collect()
    }
}
