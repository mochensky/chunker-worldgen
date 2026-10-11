//! `BlockPredicate`.

use std::sync::Arc;

use anyhow::{Result, bail};
use chunker_data::blocks::{BlockState, Fluid};
use serde_json::Value;

use super::Region;
use super::registry::{BiomeSet, Parser};
use crate::block_set::StateSet;
use crate::codec::{as_array, field, type_name};
use crate::structure::geom::{BlockPos, Direction};
use crate::terrain::material::VerticalAnchor;

#[derive(Debug)]
pub enum BlockPredicate {
    MatchingBlocks(BlockPos, Arc<StateSet>),
    MatchingBlockTag(BlockPos, Arc<StateSet>),
    MatchingFluids(BlockPos, Vec<Fluid>),
    MatchingBiomes(BiomeSet),
    HasSturdyFace(BlockPos, Direction),
    Solid(BlockPos),
    Replaceable(BlockPos),
    WouldSurvive(BlockPos, BlockState),
    InsideWorldBounds(BlockPos),
    AnyOf(Vec<BlockPredicate>),
    AllOf(Vec<BlockPredicate>),
    Not(Box<BlockPredicate>),
    True,
    Unobstructed,
    HeightRange(VerticalAnchor, VerticalAnchor),
    VolumeMatch(BlockPos, BlockPos, Box<BlockPredicate>),
}

pub fn parse_offset(json: &Value, name: &str) -> Result<BlockPos> {
    match json.get(name) {
        None => Ok(BlockPos::ZERO),
        Some(v) => {
            let a = as_array(v)?;
            if a.len() != 3 {
                bail!("expected 3 coordinates in {name}");
            }
            Ok(BlockPos::new(
                crate::codec::as_i32(&a[0])?,
                crate::codec::as_i32(&a[1])?,
                crate::codec::as_i32(&a[2])?,
            ))
        }
    }
}

impl BlockPredicate {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let offset = || parse_offset(json, "offset");
        Ok(match type_name(json)?.as_str() {
            "matching_blocks" => {
                BlockPredicate::MatchingBlocks(offset()?, p.block_set(field(json, "blocks")?)?)
            }
            "matching_block_tag" => {
                BlockPredicate::MatchingBlockTag(offset()?, p.tag(crate::codec::str_field(json, "tag")?)?)
            }
            "matching_fluids" => {
                BlockPredicate::MatchingFluids(offset()?, p.fluid_set(field(json, "fluids")?)?)
            }
            "matching_biomes" => BlockPredicate::MatchingBiomes(p.biome_set(field(json, "biomes")?)?),
            "has_sturdy_face" => BlockPredicate::HasSturdyFace(
                offset()?,
                Direction::parse(crate::codec::str_field(json, "direction")?)?,
            ),
            "solid" => BlockPredicate::Solid(offset()?),
            "replaceable" => BlockPredicate::Replaceable(offset()?),
            "would_survive" => BlockPredicate::WouldSurvive(offset()?, p.state(field(json, "state")?)?),
            "inside_world_bounds" => BlockPredicate::InsideWorldBounds(offset()?),
            "any_of" => BlockPredicate::AnyOf(Self::list(p, json)?),
            "all_of" => BlockPredicate::AllOf(Self::list(p, json)?),
            "not" => BlockPredicate::Not(Box::new(Self::parse(p, field(json, "predicate")?)?)),
            "true" => BlockPredicate::True,
            "unobstructed" => BlockPredicate::Unobstructed,
            "height_range" => BlockPredicate::HeightRange(
                VerticalAnchor::parse(field(json, "min_inclusive")?)?,
                VerticalAnchor::parse(field(json, "max_inclusive")?)?,
            ),
            "volume_match" => BlockPredicate::VolumeMatch(
                parse_offset(json, "min")?,
                parse_offset(json, "max")?,
                Box::new(Self::parse(p, field(json, "match")?)?),
            ),
            other => bail!("unknown block predicate {other}"),
        })
    }

    fn list(p: &Parser, json: &Value) -> Result<Vec<BlockPredicate>> {
        as_array(field(json, "predicates")?)?
            .iter()
            .map(|v| Self::parse(p, v))
            .collect()
    }

    pub fn test(&self, region: &Region, pos: BlockPos) -> bool {
        let env = region.env;
        match self {
            BlockPredicate::MatchingBlocks(o, set) | BlockPredicate::MatchingBlockTag(o, set) => {
                set.contains(region.get(pos.add(*o)))
            }
            BlockPredicate::MatchingFluids(o, fluids) => fluids.contains(&region.fluid(pos.add(*o))),
            BlockPredicate::MatchingBiomes(set) => set.contains(region.biome(pos)),
            BlockPredicate::HasSturdyFace(o, d) => {
                let at = pos.add(*o);
                super::behaviour::is_face_sturdy(env, region.get(at), *d)
            }
            BlockPredicate::Solid(o) => region.flags(pos.add(*o)).is_solid(),
            BlockPredicate::Replaceable(o) => region.flags(pos.add(*o)).can_be_replaced(),
            BlockPredicate::WouldSurvive(o, state) => {
                super::behaviour::can_survive(region, *state, pos.add(*o))
            }
            BlockPredicate::InsideWorldBounds(o) => !region.is_outside_build_height(pos.add(*o).y),
            BlockPredicate::AnyOf(list) => list.iter().any(|p| p.test(region, pos)),
            BlockPredicate::AllOf(list) => list.iter().all(|p| p.test(region, pos)),
            BlockPredicate::Not(p) => !p.test(region, pos),
            BlockPredicate::True => true,
            // No entities exist in a `WorldGenRegion`.
            BlockPredicate::Unobstructed => true,
            BlockPredicate::HeightRange(min, max) => {
                let ctx = &env.gen_ctx;
                pos.y >= min.resolve(ctx) && pos.y <= max.resolve(ctx)
            }
            BlockPredicate::VolumeMatch(min, max, m) => {
                for ox in min.x..=max.x {
                    for oz in min.z..=max.z {
                        for oy in min.y..=max.y {
                            if !m.test(region, pos.offset(ox, oy, oz)) {
                                return false;
                            }
                        }
                    }
                }
                true
            }
        }
    }
}
