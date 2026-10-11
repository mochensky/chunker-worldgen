//! `BlockStateProvider`.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::blocks::{BlockId, BlockState};
use serde_json::Value;

use super::Region;
use super::predicate::BlockPredicate;
use super::registry::Parser;
use crate::codec::{as_array, f32_field, field, str_field, type_name};
use crate::math;
use crate::noise::NoiseStack;
use crate::providers::{IntProvider, WeightedList};
use crate::random::{AnyRandom, LegacyRandom, RandomSource, WorldgenRandom};
use crate::structure::geom::{BlockPos, Direction};

pub enum StateProvider {
    Simple(BlockState),
    Weighted(WeightedList<BlockState>),
    RandomBlock(Vec<BlockId>),
    RandomizedInt {
        source: Arc<StateProvider>,
        property: String,
        values: IntProvider,
    },
    Rotated {
        state: Arc<StateProvider>,
        direction: Option<Direction>,
    },
    RuleBased {
        fallback: Option<Arc<StateProvider>>,
        rules: Vec<(BlockPredicate, Arc<StateProvider>)>,
    },
    CopyProperties(Arc<StateProvider>),
    Noise {
        noise: NoiseStack,
        scale: f32,
        states: Vec<BlockState>,
    },
    DualNoise {
        noise: NoiseStack,
        scale: f32,
        states: Vec<BlockState>,
        variety: (i32, i32),
        slow: NoiseStack,
        slow_scale: f32,
    },
    NoiseThreshold {
        noise: NoiseStack,
        scale: f32,
        threshold: f32,
        high_chance: f32,
        default: BlockState,
        low: Vec<BlockState>,
        high: Vec<BlockState>,
    },
}

impl std::fmt::Debug for StateProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StateProvider")
    }
}

/// `NoiseBasedStateProvider` noise: `parameters.create(new WorldgenRandom(new LegacyRandomSource(seed)))`.
fn provider_noise(json: &Value, name: &str, seed: i64) -> Result<NoiseStack> {
    let params = crate::noise::NormalNoiseParams::from_json(field(json, name)?)?;
    let mut random = WorldgenRandom::new(AnyRandom::Legacy(LegacyRandom::new(seed)));
    Ok(crate::noise::NormalNoise::new(params).create(&mut random))
}

fn states(p: &Parser, json: &Value, name: &str) -> Result<Vec<BlockState>> {
    as_array(field(json, name)?)?.iter().map(|v| p.state(v)).collect()
}

impl StateProvider {
    /// A typed provider object (`{"type": ...}`) or a block state (`{"id": ...}`).
    pub fn parse_direct(p: &Parser, json: &Value) -> Result<Self> {
        if json.get("type").is_none() {
            return Ok(StateProvider::Simple(p.state(json)?));
        }
        Ok(match type_name(json)?.as_str() {
            "simple" => StateProvider::Simple(p.state(field(json, "state")?)?),
            "weighted" => {
                StateProvider::Weighted(WeightedList::parse(field(json, "entries")?, |v| p.state(v))?)
            }
            "random_block" => {
                let set = p.block_list(field(json, "blocks")?)?;
                StateProvider::RandomBlock(set)
            }
            "randomized_int" => StateProvider::RandomizedInt {
                source: p.provider(field(json, "source")?)?,
                property: str_field(json, "property")?.to_owned(),
                values: IntProvider::parse(field(json, "values")?)?,
            },
            "rotated" => StateProvider::Rotated {
                state: p.provider(field(json, "state")?)?,
                direction: json
                    .get("direction")
                    .map(|d| Direction::parse(d.as_str().context("direction")?))
                    .transpose()?,
            },
            "rule_based" => {
                let fallback = json.get("fallback").map(|f| p.provider(f)).transpose()?;
                let mut rules = Vec::new();
                for r in as_array(field(json, "rules")?)? {
                    rules.push((
                        BlockPredicate::parse(p, field(r, "if_true")?)?,
                        p.provider(field(r, "then")?)?,
                    ));
                }
                StateProvider::RuleBased { fallback, rules }
            }
            "copy_properties" => StateProvider::CopyProperties(p.provider(field(json, "source")?)?),
            "noise" => {
                let seed = crate::codec::as_i64(field(json, "seed")?)?;
                StateProvider::Noise {
                    noise: provider_noise(json, "noise", seed)?,
                    scale: f32_field(json, "scale")?,
                    states: states(p, json, "states")?,
                }
            }
            "dual_noise" => {
                let seed = crate::codec::as_i64(field(json, "seed")?)?;
                let variety = field(json, "variety")?;
                let variety = match variety {
                    Value::Array(a) => (crate::codec::as_i32(&a[0])?, crate::codec::as_i32(&a[1])?),
                    v => (
                        crate::codec::i32_field(v, "min_inclusive")?,
                        crate::codec::i32_field(v, "max_inclusive")?,
                    ),
                };
                StateProvider::DualNoise {
                    noise: provider_noise(json, "noise", seed)?,
                    scale: f32_field(json, "scale")?,
                    states: states(p, json, "states")?,
                    variety,
                    slow: provider_noise(json, "slow_noise", seed)?,
                    slow_scale: f32_field(json, "slow_scale")?,
                }
            }
            "noise_threshold" => {
                let seed = crate::codec::as_i64(field(json, "seed")?)?;
                StateProvider::NoiseThreshold {
                    noise: provider_noise(json, "noise", seed)?,
                    scale: f32_field(json, "scale")?,
                    threshold: f32_field(json, "threshold")?,
                    high_chance: f32_field(json, "high_chance")?,
                    default: p.state(field(json, "default_state")?)?,
                    low: states(p, json, "low_states")?,
                    high: states(p, json, "high_states")?,
                }
            }
            other => bail!("unknown block state provider {other}"),
        })
    }

    /// `getState(level, random, pos)`.
    pub fn get(&self, region: &Region, random: &mut dyn RandomSource, pos: BlockPos) -> BlockState {
        match self {
            StateProvider::RandomBlock(_) | StateProvider::RuleBased { .. } => self
                .get_optional(region, random, pos)
                .unwrap_or_else(|| region.get(pos)),
            _ => self
                .get_optional(region, random, pos)
                .expect("provider always yields a state"),
        }
    }

    /// `getOptionalState(level, random, pos)`.
    pub fn get_optional(
        &self,
        region: &Region,
        random: &mut dyn RandomSource,
        pos: BlockPos,
    ) -> Option<BlockState> {
        let blocks = &region.env.blocks;
        Some(match self {
            StateProvider::Simple(s) => *s,
            StateProvider::Weighted(list) => *list.get_random(random).expect("non-empty weighted provider"),
            StateProvider::RandomBlock(list) => {
                if list.is_empty() {
                    return None;
                }
                let b = list[random.next_int_bounded(list.len() as i32) as usize];
                blocks.block(b).default_state
            }
            StateProvider::RandomizedInt {
                source,
                property,
                values,
            } => {
                let state = source.get(region, random, pos);
                if !blocks.has_property(state, property) {
                    return Some(state);
                }
                let v = values.sample(random);
                blocks
                    .with_property(state, property, &v.to_string())
                    .unwrap_or(state)
            }
            StateProvider::Rotated { state, direction } => {
                let d = match direction {
                    Some(d) => *d,
                    None => Direction::ALL[random.next_int_bounded(6) as usize],
                };
                let mut s = state.get(region, random, pos);
                s = blocks.with_property(s, "axis", d.axis().name()).unwrap_or(s);
                s = blocks.with_property(s, "facing", d.name()).unwrap_or(s);
                if d.axis() != crate::structure::geom::Axis::Y {
                    // Directional blocks with only horizontal facings (`HORIZONTAL_FACING`).
                    s = blocks.with_property(s, "facing", d.name()).unwrap_or(s);
                }
                s
            }
            StateProvider::RuleBased { fallback, rules } => {
                for (cond, then) in rules {
                    if cond.test(region, pos)
                        && let Some(s) = then.get_optional(region, random, pos)
                    {
                        return Some(s);
                    }
                }
                return fallback
                    .as_ref()
                    .and_then(|f| f.get_optional(region, random, pos));
            }
            StateProvider::CopyProperties(source) => {
                let s = source.get(region, random, pos);
                copy_properties(blocks, s, region.get(pos))
            }
            StateProvider::Noise { noise, scale, states } => {
                let v = noise_value(noise, pos, *scale as f64);
                pick(states, v)
            }
            StateProvider::DualNoise {
                noise,
                scale,
                states,
                variety,
                slow,
                slow_scale,
            } => {
                let variety_noise = slow_value(slow, pos, *slow_scale) as f64;
                let local =
                    math::clamped_map_f64(variety_noise, -1.0, 1.0, variety.0 as f64, (variety.1 + 1) as f64)
                        as i32;
                let mut possible = Vec::with_capacity(local.max(0) as usize);
                for i in 0..local {
                    possible.push(pick(
                        states,
                        slow_value(slow, pos.offset(i * 54545, 0, i * 34234), *slow_scale),
                    ));
                }
                pick(&possible, noise_value(noise, pos, *scale as f64))
            }
            StateProvider::NoiseThreshold {
                noise,
                scale,
                threshold,
                high_chance,
                default,
                low,
                high,
            } => {
                let local = noise_value(noise, pos, *scale as f64) as f64;
                if local < *threshold as f64 {
                    low[random.next_int_bounded(low.len() as i32) as usize]
                } else if random.next_float() < *high_chance {
                    high[random.next_int_bounded(high.len() as i32) as usize]
                } else {
                    *default
                }
            }
        })
    }
}

/// `getNoiseValue(pos, scale)`.
fn noise_value(noise: &NoiseStack, pos: BlockPos, scale: f64) -> f32 {
    noise.get(pos.x as f64 * scale, pos.y as f64 * scale, pos.z as f64 * scale)
}

/// `DualNoiseProvider.getSlowNoiseValue`: float scale multiplied in float before widening.
fn slow_value(noise: &NoiseStack, pos: BlockPos, scale: f32) -> f32 {
    noise.get(
        (pos.x as f32 * scale) as f64,
        (pos.y as f32 * scale) as f64,
        (pos.z as f32 * scale) as f64,
    )
}

/// `NoiseProvider.getRandomState(states, noiseValue)`.
fn pick(states: &[BlockState], noise_value: f32) -> BlockState {
    let placement = math::clamp_f32((1.0 + noise_value) / 2.0, 0.0, 0.9999);
    states[(placement * states.len() as f32) as usize]
}

/// `BlockState.withPropertiesOf(source)`: copies every property both states share.
pub fn copy_properties(
    blocks: &chunker_data::blocks::BlockRegistry,
    target: BlockState,
    source: BlockState,
) -> BlockState {
    let mut s = target;
    for (k, v) in blocks.properties(source) {
        if blocks.has_property(s, k) {
            s = blocks.with_property(s, k, v).unwrap_or(s);
        }
    }
    s
}
