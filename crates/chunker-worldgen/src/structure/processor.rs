//! Structure processors (`levelgen.structure.templatesystem.*Processor`, `ProcessorRule`,
//! `PosRuleTest`, `RuleBlockEntityModifier`).

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::blocks::{BlockId, BlockState};
use chunker_nbt::Compound;
use serde_json::Value;

use super::geom::{Axis, BlockPos, Direction};
use super::template::BlockInfo;
use crate::block_set::StateSet;
use crate::chunk::HeightmapKind;
use crate::codec::{f32_field, field, i32_field, str_field, type_name};
use crate::feature::Region;
use crate::feature::registry::Parser;
use crate::feature::rule::RuleTest;
use crate::math;
use crate::providers::IntProvider;
use crate::random::{LegacyRandom, RandomSource};

/// `PosRuleTest`.
pub enum PosRule {
    AlwaysTrue,
    Linear {
        min_chance: f32,
        max_chance: f32,
        min_dist: i32,
        max_dist: i32,
    },
    AxisAligned {
        min_chance: f32,
        max_chance: f32,
        min_dist: i32,
        max_dist: i32,
        axis: Axis,
    },
}

impl PosRule {
    fn parse(json: &Value) -> Result<Self> {
        let t = str_field(json, "predicate_type")?;
        let chance = |k: &str| {
            json.get(k)
                .map(|_| f32_field(json, k))
                .transpose()
                .map(|v| v.unwrap_or(0.0))
        };
        let dist = |k: &str| {
            json.get(k)
                .map(|_| i32_field(json, k))
                .transpose()
                .map(|v| v.unwrap_or(0))
        };
        Ok(match t.strip_prefix("minecraft:").unwrap_or(t) {
            "always_true" => PosRule::AlwaysTrue,
            "linear_pos" => PosRule::Linear {
                min_chance: chance("min_chance")?,
                max_chance: chance("max_chance")?,
                min_dist: dist("min_dist")?,
                max_dist: dist("max_dist")?,
            },
            "axis_aligned_linear_pos" => PosRule::AxisAligned {
                min_chance: chance("min_chance")?,
                max_chance: chance("max_chance")?,
                min_dist: dist("min_dist")?,
                max_dist: dist("max_dist")?,
                axis: match json.get("axis").and_then(Value::as_str).unwrap_or("y") {
                    "x" => Axis::X,
                    "z" => Axis::Z,
                    _ => Axis::Y,
                },
            },
            other => bail!("unknown position rule test {other}"),
        })
    }

    fn test(&self, world: BlockPos, reference: BlockPos, random: &mut dyn RandomSource) -> bool {
        let chance = |dist: i32,
                      min_chance: f32,
                      max_chance: f32,
                      min_dist: i32,
                      max_dist: i32,
                      random: &mut dyn RandomSource| {
            let rnd = random.next_float();
            rnd <= math::clamped_lerp_f32(
                math::inverse_lerp_f32(dist as f32, min_dist as f32, max_dist as f32),
                min_chance,
                max_chance,
            )
        };
        match *self {
            PosRule::AlwaysTrue => true,
            PosRule::Linear {
                min_chance,
                max_chance,
                min_dist,
                max_dist,
            } => {
                let dist = (world.x - reference.x).abs()
                    + (world.y - reference.y).abs()
                    + (world.z - reference.z).abs();
                chance(dist, min_chance, max_chance, min_dist, max_dist, random)
            }
            PosRule::AxisAligned {
                min_chance,
                max_chance,
                min_dist,
                max_dist,
                axis,
            } => {
                let d = Direction::from_axis(axis, true);
                let xd = ((world.x - reference.x) * d.step_x()).abs() as f32;
                let yd = ((world.y - reference.y) * d.step_y()).abs() as f32;
                let zd = ((world.z - reference.z) * d.step_z()).abs() as f32;
                chance(
                    (xd + yd + zd) as i32,
                    min_chance,
                    max_chance,
                    min_dist,
                    max_dist,
                    random,
                )
            }
        }
    }
}

/// `RuleBlockEntityModifier`.
pub enum BlockEntityModifier {
    Passthrough,
    Clear,
    AppendStatic(Compound),
    AppendLoot(String),
}

impl BlockEntityModifier {
    fn parse(json: &Value) -> Result<Self> {
        Ok(match type_name(json)?.as_str() {
            "passthrough" => BlockEntityModifier::Passthrough,
            "clear" => BlockEntityModifier::Clear,
            "append_loot" => BlockEntityModifier::AppendLoot(str_field(json, "loot_table")?.to_owned()),
            "append_static" => BlockEntityModifier::AppendStatic(match field(json, "data")? {
                Value::String(_) => bail!("SNBT block entity data is not supported"),
                other => match crate::structure::jigsaw::json_to_nbt(other) {
                    chunker_nbt::Tag::Compound(c) => c,
                    _ => bail!("append_static data is not a compound"),
                },
            }),
            other => bail!("unknown block entity modifier {other}"),
        })
    }

    fn apply(&self, random: &mut dyn RandomSource, existing: Option<Compound>) -> Option<Compound> {
        match self {
            BlockEntityModifier::Passthrough => existing,
            BlockEntityModifier::Clear => Some(Compound::new()),
            BlockEntityModifier::AppendStatic(tag) => Some(match existing {
                None => tag.clone(),
                Some(mut e) => {
                    e.merge(tag);
                    e
                }
            }),
            BlockEntityModifier::AppendLoot(table) => {
                let mut result = existing.unwrap_or_default();
                result.insert("LootTable", table.as_str());
                result.insert("LootTableSeed", random.next_long());
                Some(result)
            }
        }
    }
}

/// `ProcessorRule`.
pub struct ProcessorRule {
    input: RuleTest,
    location: RuleTest,
    position: PosRule,
    output: BlockState,
    modifier: BlockEntityModifier,
}

impl ProcessorRule {
    /// `new ProcessorRule(input, location, position, output, modifier)`.
    pub fn new(
        input: RuleTest,
        location: RuleTest,
        position: PosRule,
        output: BlockState,
        modifier: BlockEntityModifier,
    ) -> Self {
        Self {
            input,
            location,
            position,
            output,
            modifier,
        }
    }

    fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            input: RuleTest::parse(p, field(json, "input_predicate")?)?,
            location: RuleTest::parse(p, field(json, "location_predicate")?)?,
            position: match json.get("position_predicate") {
                Some(v) => PosRule::parse(v)?,
                None => PosRule::AlwaysTrue,
            },
            output: p.state(field(json, "output_state")?)?,
            modifier: match json.get("block_entity_modifier") {
                Some(v) => BlockEntityModifier::parse(v).unwrap_or(BlockEntityModifier::Passthrough),
                None => BlockEntityModifier::Passthrough,
            },
        })
    }
}

/// `StructureProcessor` implementations.
pub enum Processor {
    Nop,
    BlockIgnore(Vec<BlockId>),
    BlockRot {
        blocks: Option<Arc<StateSet>>,
        integrity: f32,
    },
    Gravity {
        heightmap: HeightmapKind,
        offset: i32,
    },
    Rule(Vec<ProcessorRule>),
    JigsawReplacement,
    ProtectedBlocks(Arc<StateSet>),
    Capped {
        delegate: Box<Processor>,
        limit: IntProvider,
    },
    LavaSubmerged,
    BlockAge(f32),
    BlackstoneReplace,
}

/// `StructureProcessorType.LIST_CODEC`: an id, `{"processors": [...]}` or a bare list.
pub fn parse_list(p: &Parser, json: &Value) -> Result<Vec<Arc<Processor>>> {
    match json {
        Value::String(s) => {
            let id = crate::feature::registry::id(s)?;
            let v = p.reg.packs.require_json("worldgen/processor_list", &id)?;
            parse_list(p, &v).with_context(|| format!("processor list {id}"))
        }
        Value::Object(o) if o.contains_key("processors") => parse_list(p, &o["processors"]),
        Value::Array(a) => a.iter().map(|v| Processor::parse(p, v).map(Arc::new)).collect(),
        other => bail!("invalid processor list {other}"),
    }
}

impl Processor {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let t = str_field(json, "processor_type")?;
        Ok(match t.strip_prefix("minecraft:").unwrap_or(t) {
            "nop" => Processor::Nop,
            "block_ignore" => Processor::BlockIgnore(
                crate::codec::as_array(field(json, "blocks")?)?
                    .iter()
                    .map(|b| p.state(b).map(|s| p.reg.blocks.block_of(s)))
                    .collect::<Result<_>>()?,
            ),
            "block_rot" => Processor::BlockRot {
                blocks: json.get("rottable_blocks").map(|v| p.block_set(v)).transpose()?,
                integrity: f32_field(json, "integrity")?,
            },
            "gravity" => Processor::Gravity {
                heightmap: HeightmapKind::from_key(
                    json.get("heightmap")
                        .and_then(Value::as_str)
                        .unwrap_or("WORLD_SURFACE_WG"),
                )
                .context("gravity heightmap")?,
                offset: json.get("offset").and_then(Value::as_i64).unwrap_or(0) as i32,
            },
            "rule" => Processor::Rule(
                crate::codec::as_array(field(json, "rules")?)?
                    .iter()
                    .map(|r| ProcessorRule::parse(p, r))
                    .collect::<Result<_>>()?,
            ),
            "jigsaw_replacement" => Processor::JigsawReplacement,
            "protected_blocks" => Processor::ProtectedBlocks(p.block_set(field(json, "value")?)?),
            "capped" => Processor::Capped {
                delegate: Box::new(Processor::parse(p, field(json, "delegate")?)?),
                limit: IntProvider::parse(field(json, "limit")?)?,
            },
            "lava_submerged_block" => Processor::LavaSubmerged,
            "block_age" => Processor::BlockAge(f32_field(json, "mossiness")?),
            "blackstone_replace" => Processor::BlackstoneReplace,
            other => bail!("unknown structure processor {other}"),
        })
    }

    /// `evaluatesEntirePieceState`.
    pub fn evaluates_entire_piece(&self) -> bool {
        matches!(self, Processor::Capped { .. })
    }

    /// `processBlock(level, targetPosition, referencePos, templateRelativePos, info, settings)`.
    /// `random` is the settings random (`None`: a positional random per block).
    pub fn process_block(
        &self,
        region: &mut Region,
        _target: BlockPos,
        reference: BlockPos,
        template_pos: BlockPos,
        info: BlockInfo,
        random: &mut Option<&mut dyn RandomSource>,
    ) -> Option<BlockInfo> {
        let env = region.env;
        let blocks = &env.blocks;
        let mut positional = None;
        match self {
            Processor::Nop | Processor::Capped { .. } => Some(info),
            Processor::BlockIgnore(list) => (!list.contains(&blocks.block_of(info.state))).then_some(info),
            Processor::BlockRot {
                blocks: set,
                integrity,
            } => {
                let r = settings_random(random, &mut positional, info.pos);
                let rottable = set.as_ref().is_none_or(|s| s.contains(info.state));
                if rottable && !(r.next_float() <= *integrity) {
                    None
                } else {
                    Some(info)
                }
            }
            Processor::Gravity { heightmap, offset } => {
                let height = region.height(*heightmap, info.pos.x, info.pos.z) + offset;
                Some(BlockInfo {
                    pos: BlockPos::new(info.pos.x, height + template_pos.y, info.pos.z),
                    ..info
                })
            }
            Processor::Rule(rules) => {
                let mut r = LegacyRandom::new(math::get_seed(info.pos.x, info.pos.y, info.pos.z));
                for rule in rules {
                    let world_state = region.get(info.pos);
                    if rule.input.test(blocks, info.state, info.pos, &mut r)
                        && rule.location.test(blocks, world_state, info.pos, &mut r)
                        && rule.position.test(info.pos, reference, &mut r)
                    {
                        let nbt = rule.modifier.apply(&mut r, info.nbt);
                        return Some(BlockInfo {
                            pos: info.pos,
                            state: rule.output,
                            nbt,
                        });
                    }
                }
                Some(info)
            }
            Processor::JigsawReplacement => {
                if blocks.block_of(info.state) != env.b.jigsaw {
                    return Some(info);
                }
                let Some(nbt) = info.nbt.as_ref() else {
                    return Some(info);
                };
                let text = nbt.get_str("final_state").unwrap_or("minecraft:air");
                // `BlockStateParser.parseForBlock(..., allowNbt = true)`: the NBT part is dropped.
                let text = text.split_once('{').map_or(text, |(s, _)| s);
                let state = blocks.parse_state_str(text).ok()?;
                (blocks.block_of(state) != env.b.structure_void).then_some(BlockInfo {
                    pos: info.pos,
                    state,
                    nbt: None,
                })
            }
            Processor::ProtectedBlocks(set) => (!set.contains(region.get(info.pos))).then_some(info),
            Processor::LavaSubmerged => {
                let was_lava = region.is(info.pos, env.b.lava);
                if was_lava && !blocks.flags(info.state).is_shape_full_block() {
                    Some(BlockInfo {
                        state: blocks.block(env.b.lava).default_state,
                        ..info
                    })
                } else {
                    Some(info)
                }
            }
            Processor::BlockAge(mossiness) => {
                let r = settings_random(random, &mut positional, info.pos);
                block_age(region, *mossiness, info, r)
            }
            Processor::BlackstoneReplace => Some(blackstone_replace(region, info)),
        }
    }

    /// `finalizeProcessing` (only `CappedProcessor` overrides it).
    pub fn finalize(
        &self,
        region: &mut Region,
        position: BlockPos,
        reference: BlockPos,
        original: &[BlockInfo],
        mut processed: Vec<BlockInfo>,
        random: &mut Option<&mut dyn RandomSource>,
    ) -> Vec<BlockInfo> {
        let Processor::Capped { delegate, limit } = self else {
            return processed;
        };
        if limit.max_inclusive() == 0 || processed.is_empty() || original.len() != processed.len() {
            return processed;
        }
        let mut r = LegacyRandom::new(region.env.seed)
            .fork_positional()
            .at(position.x, position.y, position.z);
        let max = limit.sample(&mut r).min(processed.len() as i32);
        if max < 1 {
            return processed;
        }
        let mut indices: Vec<usize> = (0..processed.len()).collect();
        for i in (2..=indices.len()).rev() {
            let j = r.next_int_bounded(i as i32) as usize;
            indices.swap(i - 1, j);
        }
        let mut replaced = 0;
        for index in indices {
            if replaced >= max {
                break;
            }
            let before = processed[index].clone();
            let altered = delegate.process_block(
                region,
                position,
                reference,
                original[index].pos,
                before.clone(),
                random,
            );
            if let Some(a) = altered
                && !block_info_eq(&before, &a)
            {
                replaced += 1;
                processed[index] = a;
            }
        }
        processed
    }
}

/// `StructurePlaceSettings.getRandom(pos)`: the settings random, else a positional legacy random.
fn settings_random<'a>(
    random: &'a mut Option<&mut dyn RandomSource>,
    positional: &'a mut Option<LegacyRandom>,
    pos: BlockPos,
) -> &'a mut dyn RandomSource {
    match random {
        Some(r) => &mut **r,
        None => positional.insert(LegacyRandom::new(math::get_seed(pos.x, pos.y, pos.z))),
    }
}

/// `StructureBlockInfo.equals` (record equality, NBT compared by value).
fn block_info_eq(a: &BlockInfo, b: &BlockInfo) -> bool {
    a.pos == b.pos && a.state == b.state && a.nbt == b.nbt
}

/// `BlockAgeProcessor.processBlock`.
fn block_age(
    region: &Region,
    mossiness: f32,
    info: BlockInfo,
    random: &mut dyn RandomSource,
) -> Option<BlockInfo> {
    let env = region.env;
    let blocks = &env.blocks;
    let b = &env.b;
    let block = blocks.block_of(info.state);
    let default = |id: BlockId| blocks.block(id).default_state;
    let random_stairs = |random: &mut dyn RandomSource, id: BlockId| {
        let facing = Direction::random_horizontal(random);
        let half = ["top", "bottom"][random.next_int_bounded(2) as usize];
        let s = blocks
            .with_property(default(id), "facing", facing.name())
            .unwrap();
        blocks.with_property(s, "half", half).unwrap()
    };
    let pick = |random: &mut dyn RandomSource, non_mossy: &[BlockState], mossy: &[BlockState]| {
        let list = if random.next_float() < mossiness {
            mossy
        } else {
            non_mossy
        };
        list[random.next_int_bounded(list.len() as i32) as usize]
    };
    let new_state = if block == b.stone_bricks || block == b.stone || block == b.chiseled_stone_bricks {
        if random.next_float() >= 0.5 {
            None
        } else {
            let non_mossy = [
                default(b.cracked_stone_bricks),
                random_stairs(random, b.stone_brick_stairs),
            ];
            let mossy = [
                default(b.mossy_stone_bricks),
                random_stairs(random, b.mossy_stone_brick_stairs),
            ];
            Some(pick(random, &non_mossy, &mossy))
        }
    } else if env.t.stairs.contains(info.state) {
        if random.next_float() >= 0.5 {
            None
        } else {
            let non_mossy = [default(b.stone_slab), default(b.stone_brick_slab)];
            let mossy = [
                crate::feature::provider::copy_properties(
                    blocks,
                    default(b.mossy_stone_brick_stairs),
                    info.state,
                ),
                default(b.mossy_stone_brick_slab),
            ];
            Some(pick(random, &non_mossy, &mossy))
        }
    } else if env.t.slabs.contains(info.state) {
        (random.next_float() < mossiness).then(|| {
            crate::feature::provider::copy_properties(blocks, default(b.mossy_stone_brick_slab), info.state)
        })
    } else if env.t.walls.contains(info.state) {
        (random.next_float() < mossiness).then(|| {
            crate::feature::provider::copy_properties(blocks, default(b.mossy_stone_brick_wall), info.state)
        })
    } else if block == b.obsidian {
        (random.next_float() < 0.15).then(|| default(b.crying_obsidian))
    } else {
        None
    };
    Some(match new_state {
        Some(state) => BlockInfo { state, ..info },
        None => info,
    })
}

/// `BlackstoneReplaceProcessor.processBlock`.
fn blackstone_replace(region: &Region, info: BlockInfo) -> BlockInfo {
    let blocks = &region.env.blocks;
    let name = blocks.name_of(info.state).path();
    let target = match name {
        "cobblestone" | "mossy_cobblestone" => "blackstone",
        "stone" => "polished_blackstone",
        "stone_bricks" | "mossy_stone_bricks" => "polished_blackstone_bricks",
        "cobblestone_stairs" | "mossy_cobblestone_stairs" => "blackstone_stairs",
        "stone_stairs" => "polished_blackstone_stairs",
        "stone_brick_stairs" | "mossy_stone_brick_stairs" => "polished_blackstone_brick_stairs",
        "cobblestone_slab" | "mossy_cobblestone_slab" => "blackstone_slab",
        "smooth_stone_slab" | "stone_slab" => "polished_blackstone_slab",
        "stone_brick_slab" | "mossy_stone_brick_slab" => "polished_blackstone_brick_slab",
        "stone_brick_wall" | "mossy_stone_brick_wall" => "polished_blackstone_brick_wall",
        "cobblestone_wall" | "mossy_cobblestone_wall" => "blackstone_wall",
        "chiseled_stone_bricks" => "chiseled_polished_blackstone",
        "cracked_stone_bricks" => "cracked_polished_blackstone_bricks",
        "iron_bars" => "iron_chain",
        _ => return info,
    };
    let mut state = blocks.vanilla_default(target);
    for prop in ["facing", "half", "type"] {
        if let Some(v) = blocks.get_property(info.state, prop)
            && let Some(s) = blocks.with_property(state, prop, v)
        {
            state = s;
        }
    }
    BlockInfo { state, ..info }
}
