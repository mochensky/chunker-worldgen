//! Feature types (`FeatureTypes`).

mod basic;
mod cave;
mod dungeon;
mod end;
mod geode;
mod iceberg;
mod misc;
mod multiface;
mod nether;
mod ore;
mod patch;
mod sculk;
mod speleothem;
mod template;

use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;

pub use basic::*;
pub use cave::*;
pub use dungeon::*;
pub use end::*;
pub use geode::*;
pub use iceberg::*;
pub use misc::*;
pub use multiface::*;
pub use nether::*;
pub use ore::*;
pub use patch::*;
pub use sculk::*;
pub use speleothem::*;
pub use template::*;

use super::Region;
use super::registry::{Parser, PlacedFeature};
use crate::codec::type_name;
use crate::random::RandomSource;
use crate::structure::geom::BlockPos;

pub enum Feature {
    NoOp,
    SimpleBlock(SimpleBlock),
    RandomSelector(RandomSelector),
    SimpleRandomSelector(Vec<Arc<PlacedFeature>>),
    RandomBooleanSelector(Arc<PlacedFeature>, Arc<PlacedFeature>),
    WeightedRandomSelector(crate::providers::WeightedList<Arc<PlacedFeature>>),
    Sequence(Vec<Arc<PlacedFeature>>),
    BlockColumn(BlockColumn),
    Ore(Ore),
    ScatteredOre(Ore),
    ReplaceSingleBlock(Vec<Replacement>),
    Disk(Disk),
    Spring(Spring),
    FreezeTopLayer,
    FillLayer(i32, chunker_data::blocks::BlockState),
    Lake(Lake),
    Tree(Box<super::tree::Tree>),
    FallenTree(Box<super::tree::FallenTree>),
    Overlay(Vec<Arc<PlacedFeature>>),
    BlueIce,
    BlockBlob(BlockBlob),
    Spike(Spike),
    Vines,
    Bamboo(f32),
    HugeMushroom(HugeMushroom),
    MonsterRoom,
    Geode(Box<Geode>),
    Iceberg(Iceberg),
    SculkPatch(SculkPatch),
    CoralClaw(Arc<PlacedFeature>),
    CoralTree(Arc<PlacedFeature>),
    Speleothem(Speleothem),
    SpeleothemCluster(Box<SpeleothemCluster>),
    LargeDripstone(Box<LargeDripstone>),
    VegetationPatch(Box<VegetationPatch>),
    RootSystem(Box<RootSystem>),
    UnderwaterMagma(UnderwaterMagma),
    MultifaceGrowth(MultifaceGrowth),
    Delta(Delta),
    BlockPile(BlockPile),
    Template(Box<TemplateFeature>),
    Fossil(Box<Fossil>),
    EndSpike(Box<EndSpike>),
    EndIsland,
    EndPlatform,
    EndGateway(EndGateway),
    ChorusPlant,
    ReplaceBlobs(ReplaceBlobs),
    RandomNeighborSpread(Box<RandomNeighborSpread>),
    SteppedColumnCluster(Box<SteppedColumnCluster>),
    HugeFungus(Box<HugeFungus>),
    /// Not ported yet: places nothing.
    Unimplemented(String),
}

impl Feature {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let t = type_name(json)?;
        let list = |name: &str| -> Result<Vec<Arc<PlacedFeature>>> {
            let v = crate::codec::field(json, name)?;
            match v {
                Value::Array(a) => a.iter().map(|f| p.placed(f)).collect(),
                other => Ok(vec![p.placed(other)?]),
            }
        };
        Ok::<Self, anyhow::Error>(match t.as_str() {
            "no_op" => Feature::NoOp,
            "simple_block" => Feature::SimpleBlock(SimpleBlock::parse(p, json)?),
            "random_selector" => Feature::RandomSelector(RandomSelector::parse(p, json)?),
            "simple_random_selector" => Feature::SimpleRandomSelector(list("features")?),
            "random_boolean_selector" => Feature::RandomBooleanSelector(
                p.placed(crate::codec::field(json, "feature_true")?)?,
                p.placed(crate::codec::field(json, "feature_false")?)?,
            ),
            "weighted_random_selector" => Feature::WeightedRandomSelector(
                crate::providers::WeightedList::parse(crate::codec::field(json, "features")?, |v| {
                    p.placed(v)
                })?,
            ),
            "sequence" => Feature::Sequence(list("features")?),
            "block_column" => Feature::BlockColumn(BlockColumn::parse(p, json)?),
            "ore" => Feature::Ore(Ore::parse(p, json)?),
            "scattered_ore" => Feature::ScatteredOre(Ore::parse(p, json)?),
            "replace_single_block" => Feature::ReplaceSingleBlock(Replacement::parse_list(
                p,
                crate::codec::field(json, "targets")?,
            )?),
            "disk" => Feature::Disk(Disk::parse(p, json)?),
            "spring_feature" => Feature::Spring(Spring::parse(p, json)?),
            "freeze_top_layer" => Feature::FreezeTopLayer,
            "fill_layer" => Feature::FillLayer(
                crate::codec::i32_field(json, "height")?,
                p.state(crate::codec::field(json, "state")?)?,
            ),
            "lake" => Feature::Lake(Lake::parse(p, json)?),
            "tree" => Feature::Tree(Box::new(super::tree::Tree::parse(p, json)?)),
            "overlay" => Feature::Overlay(list("features")?),
            "blue_ice" => Feature::BlueIce,
            "block_blob" => Feature::BlockBlob(BlockBlob::parse(p, json)?),
            "spike" => Feature::Spike(Spike::parse(p, json)?),
            "vines" => Feature::Vines,
            "monster_room" => Feature::MonsterRoom,
            "iceberg" => Feature::Iceberg(Iceberg {
                state: p.state(crate::codec::field(json, "state")?)?,
            }),
            "vegetation_patch" => Feature::VegetationPatch(Box::new(VegetationPatch::parse(p, json, false)?)),
            "waterlogged_vegetation_patch" => {
                Feature::VegetationPatch(Box::new(VegetationPatch::parse(p, json, true)?))
            }
            "root_system" => Feature::RootSystem(Box::new(RootSystem::parse(p, json)?)),
            "speleothem" => Feature::Speleothem(Speleothem::parse(p, json)?),
            "speleothem_cluster" => Feature::SpeleothemCluster(Box::new(SpeleothemCluster::parse(p, json)?)),
            "large_dripstone" => Feature::LargeDripstone(Box::new(LargeDripstone::parse(p, json)?)),
            "coral_claw" => Feature::CoralClaw(p.placed(crate::codec::field(json, "feature")?)?),
            "coral_tree" => Feature::CoralTree(p.placed(crate::codec::field(json, "feature")?)?),
            "sculk_patch" => Feature::SculkPatch(SculkPatch::parse(json)?),
            "geode" => Feature::Geode(Box::new(Geode::parse(p, json)?)),
            "underwater_magma" => Feature::UnderwaterMagma(UnderwaterMagma::parse(json)?),
            "multiface_growth" => Feature::MultifaceGrowth(MultifaceGrowth::parse(p, json)?),
            "end_spike" => Feature::EndSpike(Box::new(EndSpike::parse(json)?)),
            "end_island" => Feature::EndIsland,
            "end_platform" => Feature::EndPlatform,
            "end_gateway" => Feature::EndGateway(EndGateway::parse(json)?),
            "chorus_plant" => Feature::ChorusPlant,
            "template" => Feature::Template(Box::new(TemplateFeature::parse(p, json)?)),
            "fossil" => Feature::Fossil(Box::new(Fossil::parse(p, json)?)),
            "block_pile" => Feature::BlockPile(BlockPile::parse(p, json)?),
            "delta_feature" => Feature::Delta(Delta::parse(p, json)?),
            "netherrack_replace_blobs" => Feature::ReplaceBlobs(ReplaceBlobs::parse(p, json)?),
            "random_neighbor_spread" => {
                Feature::RandomNeighborSpread(Box::new(RandomNeighborSpread::parse(p, json)?))
            }
            "stepped_column_cluster" => {
                Feature::SteppedColumnCluster(Box::new(SteppedColumnCluster::parse(p, json)?))
            }
            "huge_fungus" => Feature::HugeFungus(Box::new(HugeFungus::parse(p, json)?)),
            "bamboo" => Feature::Bamboo(bamboo_probability(json)?),
            "huge_brown_mushroom" => Feature::HugeMushroom(HugeMushroom::parse(p, json, false)?),
            "huge_red_mushroom" => Feature::HugeMushroom(HugeMushroom::parse(p, json, true)?),
            "fallen_tree" => Feature::FallenTree(Box::new(super::tree::FallenTree::parse(p, json)?)),
            other => Feature::Unimplemented(other.to_owned()),
        })
        .with_context(|| format!("feature type {t}"))
    }

    /// `Feature.place(level, generator, random, origin)`.
    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        match self {
            Feature::NoOp => true,
            Feature::SimpleBlock(f) => f.place(region, random, origin),
            Feature::RandomSelector(f) => f.place(region, random, origin),
            Feature::SimpleRandomSelector(list) => {
                let i = random.next_int_bounded(list.len() as i32) as usize;
                super::placement::place(region, &list[i], None, random, origin)
            }
            Feature::RandomBooleanSelector(a, b) => {
                let f = if random.next_bool() { a } else { b };
                super::placement::place(region, f, None, random, origin)
            }
            Feature::WeightedRandomSelector(list) => match list.get_random(random) {
                Some(f) => super::placement::place(region, f, None, random, origin),
                None => false,
            },
            Feature::Sequence(list) => {
                for f in list {
                    if !super::placement::place(region, f, None, random, origin) {
                        return false;
                    }
                }
                true
            }
            Feature::BlockColumn(f) => f.place(region, random, origin),
            Feature::Ore(f) => f.place(region, random, origin),
            Feature::ScatteredOre(f) => f.place_scattered(region, random, origin),
            Feature::ReplaceSingleBlock(list) => {
                let state = region.get(origin);
                for r in list {
                    if r.target.test(&region.env.blocks, state, origin, random) {
                        region.set(origin, r.state, 2);
                        break;
                    }
                }
                true
            }
            Feature::Disk(f) => f.place(region, random, origin),
            Feature::Spring(f) => f.place(region, origin),
            Feature::FreezeTopLayer => basic::freeze_top_layer(region, origin),
            Feature::FillLayer(height, state) => {
                for dx in 0..16 {
                    for dz in 0..16 {
                        let p = BlockPos::new(origin.x + dx, region.min_y() + height, origin.z + dz);
                        if region.flags(p).is_air() {
                            region.set(p, *state, 2);
                        }
                    }
                }
                true
            }
            Feature::Lake(f) => f.place(region, random, origin),
            Feature::Tree(t) => t.place(region, random, origin),
            Feature::FallenTree(t) => t.place(region, random, origin),
            Feature::Overlay(list) => {
                let mut any = false;
                for f in list {
                    any |= super::placement::place(region, f, None, random, origin);
                }
                any
            }
            Feature::BlueIce => blue_ice(region, random, origin),
            Feature::BlockBlob(f) => f.place(region, random, origin),
            Feature::Spike(f) => f.place(region, random, origin),
            Feature::Vines => vines(region, origin),
            Feature::MonsterRoom => monster_room(region, random, origin),
            Feature::Geode(f) => f.place(region, random, origin),
            Feature::Iceberg(f) => f.place(region, random, origin),
            Feature::SculkPatch(f) => f.place(region, random, origin),
            Feature::CoralClaw(f) => coral_claw(region, random, origin, f),
            Feature::CoralTree(f) => coral_tree(region, random, origin, f),
            Feature::Speleothem(f) => f.place(region, random, origin),
            Feature::SpeleothemCluster(f) => f.place(region, random, origin),
            Feature::LargeDripstone(f) => f.place(region, random, origin),
            Feature::VegetationPatch(f) => f.place(region, random, origin),
            Feature::RootSystem(f) => f.place(region, random, origin),
            Feature::UnderwaterMagma(f) => f.place(region, random, origin),
            Feature::MultifaceGrowth(f) => f.place(region, random, origin),
            Feature::Delta(f) => f.place(region, random, origin),
            Feature::BlockPile(f) => f.place(region, random, origin),
            Feature::Template(f) => f.place(region, random, origin),
            Feature::Fossil(f) => f.place(region, random, origin),
            Feature::EndSpike(f) => f.place(region, random, origin),
            Feature::EndIsland => end_island(region, random, origin),
            Feature::EndPlatform => end_platform(region, origin),
            Feature::EndGateway(f) => f.place(region, origin),
            Feature::ChorusPlant => chorus_plant(region, random, origin),
            Feature::ReplaceBlobs(f) => f.place(region, random, origin),
            Feature::RandomNeighborSpread(f) => f.place(region, random, origin),
            Feature::SteppedColumnCluster(f) => f.place(region, random, origin),
            Feature::HugeFungus(f) => f.place(region, random, origin),
            Feature::Bamboo(p) => bamboo(region, random, origin, *p),
            Feature::HugeMushroom(f) => f.place(region, random, origin),
            Feature::Unimplemented(_) => false,
        }
    }

    pub fn is_implemented(&self) -> bool {
        !matches!(self, Feature::Unimplemented(_))
    }

    /// Whether this feature and everything it can place are ported.
    pub fn deep_implemented(&self) -> bool {
        let all = |list: &[Arc<PlacedFeature>]| list.iter().all(|f| f.feature.deep_implemented());
        match self {
            Feature::Unimplemented(_) => false,
            Feature::RandomSelector(f) => {
                f.default.feature.deep_implemented()
                    && f.features.iter().all(|(p, _)| p.feature.deep_implemented())
            }
            Feature::SimpleRandomSelector(list) | Feature::Sequence(list) | Feature::Overlay(list) => {
                all(list)
            }
            Feature::RandomBooleanSelector(a, b) => {
                a.feature.deep_implemented() && b.feature.deep_implemented()
            }
            Feature::WeightedRandomSelector(list) => {
                list.entries.iter().all(|(p, _)| p.feature.deep_implemented())
            }
            Feature::Tree(t) => t.decorators.iter().all(|d| d.is_implemented()),
            Feature::FallenTree(t) => t.is_implemented(),
            Feature::CoralClaw(f) | Feature::CoralTree(f) => f.feature.deep_implemented(),
            _ => true,
        }
    }
}
