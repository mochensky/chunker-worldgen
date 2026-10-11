//! `RuleTest` (ore targets, structure processors).

use std::sync::Arc;

use anyhow::{Result, bail};
use chunker_data::blocks::{BlockId, BlockRegistry, BlockState};
use serde_json::Value;

use super::registry::Parser;
use crate::block_set::StateSet;
use crate::codec::{as_array, f32_field, field, i32_field, str_field};
use crate::random::RandomSource;
use crate::structure::geom::BlockPos;

#[derive(Debug)]
pub enum RuleTest {
    AlwaysTrue,
    Tag(Arc<StateSet>),
    Block(BlockId),
    BlockState(BlockState),
    RandomBlock(BlockId, f32),
    RandomBlockState(BlockState, f32),
    Height { min: i32, max: i32 },
    AllOf(Vec<RuleTest>),
    AnyOf(Vec<RuleTest>),
    Not(Box<RuleTest>),
}

impl RuleTest {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let t = str_field(json, "predicate_type")?;
        let t = t.strip_prefix("minecraft:").unwrap_or(t);
        Ok(match t {
            "always_true" => RuleTest::AlwaysTrue,
            "tag_match" => RuleTest::Tag(p.tag(str_field(json, "tag")?)?),
            "block_match" => RuleTest::Block(p.block(str_field(json, "block")?)?),
            "blockstate_match" => RuleTest::BlockState(p.state(field(json, "block_state")?)?),
            "random_block_match" => RuleTest::RandomBlock(
                p.block(str_field(json, "block")?)?,
                f32_field(json, "probability")?,
            ),
            "random_blockstate_match" => RuleTest::RandomBlockState(
                p.state(field(json, "block_state")?)?,
                f32_field(json, "probability")?,
            ),
            "height_match" => RuleTest::Height {
                min: i32_field(json, "min_inclusive")?,
                max: i32_field(json, "max_inclusive")?,
            },
            "all_of" => RuleTest::AllOf(
                as_array(field(json, "rules")?)?
                    .iter()
                    .map(|r| RuleTest::parse(p, r))
                    .collect::<Result<_>>()?,
            ),
            "any_of" => RuleTest::AnyOf(
                as_array(field(json, "rules")?)?
                    .iter()
                    .map(|r| RuleTest::parse(p, r))
                    .collect::<Result<_>>()?,
            ),
            "not" => RuleTest::Not(Box::new(RuleTest::parse(p, field(json, "rule")?)?)),
            other => bail!("unknown rule test {other}"),
        })
    }

    /// `test(state, pos, random)`.
    pub fn test(
        &self,
        blocks: &BlockRegistry,
        state: BlockState,
        pos: BlockPos,
        random: &mut dyn RandomSource,
    ) -> bool {
        match self {
            RuleTest::AlwaysTrue => true,
            RuleTest::Tag(set) => set.contains(state),
            RuleTest::Block(b) => blocks.block_of(state) == *b,
            RuleTest::BlockState(s) => state == *s,
            RuleTest::RandomBlock(b, prob) => blocks.block_of(state) == *b && random.next_float() < *prob,
            RuleTest::RandomBlockState(s, prob) => state == *s && random.next_float() < *prob,
            RuleTest::Height { min, max } => *min <= pos.y && pos.y <= *max,
            RuleTest::AllOf(rules) => rules.iter().all(|r| r.test(blocks, state, pos, random)),
            RuleTest::AnyOf(rules) => rules.iter().any(|r| r.test(blocks, state, pos, random)),
            RuleTest::Not(r) => !r.test(blocks, state, pos, random),
        }
    }
}
