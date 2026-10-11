//! `worldgen/structure` and `worldgen/structure_set` registries.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;

use super::placement::Placement;
use crate::biome::{BiomeId, BiomeRegistry, parse_biome_holder_set};
use crate::codec::{self, field};
use crate::providers::HeightProvider;
use crate::registry::WorldgenRegistries;

/// `GenerationStep.Decoration`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Decoration {
    RawGeneration,
    Lakes,
    LocalModifications,
    UndergroundStructures,
    SurfaceStructures,
    Strongholds,
    UndergroundOres,
    UndergroundDecoration,
    FluidSprings,
    VegetalDecoration,
    TopLayerModification,
}

impl Decoration {
    pub const ALL: [Decoration; 11] = [
        Decoration::RawGeneration,
        Decoration::Lakes,
        Decoration::LocalModifications,
        Decoration::UndergroundStructures,
        Decoration::SurfaceStructures,
        Decoration::Strongholds,
        Decoration::UndergroundOres,
        Decoration::UndergroundDecoration,
        Decoration::FluidSprings,
        Decoration::VegetalDecoration,
        Decoration::TopLayerModification,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Decoration::RawGeneration => "raw_generation",
            Decoration::Lakes => "lakes",
            Decoration::LocalModifications => "local_modifications",
            Decoration::UndergroundStructures => "underground_structures",
            Decoration::SurfaceStructures => "surface_structures",
            Decoration::Strongholds => "strongholds",
            Decoration::UndergroundOres => "underground_ores",
            Decoration::UndergroundDecoration => "underground_decoration",
            Decoration::FluidSprings => "fluid_springs",
            Decoration::VegetalDecoration => "vegetal_decoration",
            Decoration::TopLayerModification => "top_layer_modification",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|d| d.name() == s)
            .with_context(|| format!("unknown generation step {s}"))
    }
}

/// `TerrainAdjustment`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TerrainAdjustment {
    None,
    Bury,
    BeardThin,
    BeardBox,
    Encapsulate,
}

impl TerrainAdjustment {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "none" => Self::None,
            "bury" => Self::Bury,
            "beard_thin" => Self::BeardThin,
            "beard_box" => Self::BeardBox,
            "encapsulate" => Self::Encapsulate,
            other => bail!("unknown terrain adaptation {other}"),
        })
    }
}

/// `RuinedPortalStructure.Setup`.
#[derive(Clone, Debug)]
pub struct RuinedPortalSetup {
    pub placement: String,
    pub air_pocket_probability: f32,
    pub mossiness: f32,
    pub overgrown: bool,
    pub vines: bool,
    pub can_be_cold: bool,
    pub replace_with_blackstone: bool,
    pub weight: f32,
}

/// Type-specific configuration of a structure.
pub enum StructureKind {
    Jigsaw(Arc<super::jigsaw::JigsawConfig>),
    BuriedTreasure,
    DesertPyramid,
    EndCity,
    Fortress,
    Igloo,
    JungleTemple,
    Mansion,
    Mineshaft {
        mesa: bool,
    },
    OceanMonument {
        surrounding: FxHashSet<BiomeId>,
    },
    NetherFossil {
        height: HeightProvider,
    },
    OceanRuin {
        warm: bool,
        large_probability: f32,
        cluster_probability: f32,
    },
    RuinedPortal {
        setups: Vec<RuinedPortalSetup>,
    },
    Shipwreck {
        beached: bool,
    },
    Stronghold,
    SwampHut,
}

pub struct Structure {
    pub id: Identifier,
    pub biomes: FxHashSet<BiomeId>,
    pub step: Decoration,
    pub terrain_adaptation: TerrainAdjustment,
    pub kind: StructureKind,
    /// Raw `spawn_overrides` (used by the spawn step).
    pub spawn_overrides: Value,
}

pub struct StructureSet {
    pub id: Identifier,
    pub placement: Placement,
    /// `(structure index, weight)`.
    pub structures: Vec<(usize, i32)>,
}

pub struct StructureRegistry {
    pub structures: Vec<Structure>,
    pub by_id: FxHashMap<Identifier, usize>,
    pub sets: Vec<StructureSet>,
    pub pools: super::jigsaw::PoolRegistry,
}

impl StructureRegistry {
    pub fn load(reg: &WorldgenRegistries, biomes: &BiomeRegistry) -> Result<Self> {
        let mut structures = Vec::new();
        let mut by_id = FxHashMap::default();
        for id in reg.packs.list("worldgen/structure") {
            let json = reg.packs.require_json("worldgen/structure", &id)?;
            let s = parse_structure(reg, biomes, &id, &json).with_context(|| format!("structure {id}"))?;
            by_id.insert(id, structures.len());
            structures.push(s);
        }
        let set_ids = reg.packs.list("worldgen/structure_set");
        let set_index: FxHashMap<Identifier, usize> = set_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        let mut sets = Vec::new();
        for id in &set_ids {
            let json = reg.packs.require_json("worldgen/structure_set", id)?;
            let placement = Placement::parse(reg, biomes, field(&json, "placement")?, &set_index)
                .with_context(|| format!("structure set {id}"))?;
            let mut list = Vec::new();
            for entry in codec::as_array(field(&json, "structures")?)? {
                let sid = Identifier::parse(codec::as_str(field(entry, "structure")?)?)
                    .context("bad structure id")?;
                let index = *by_id
                    .get(&sid)
                    .with_context(|| format!("structure set {id}: unknown structure {sid}"))?;
                let weight = codec::as_i32(field(entry, "weight")?)?;
                list.push((index, weight));
            }
            sets.push(StructureSet {
                id: id.clone(),
                placement,
                structures: list,
            });
        }
        let pools = super::jigsaw::PoolRegistry::load(reg)?;
        Ok(Self {
            structures,
            by_id,
            sets,
            pools,
        })
    }
}

fn opt_f32(json: &Value, key: &str, default: f32) -> Result<f32> {
    json.get(key).map_or(Ok(default), codec::as_f32)
}

fn opt_bool(json: &Value, key: &str, default: bool) -> Result<bool> {
    json.get(key).map_or(Ok(default), codec::as_bool)
}

fn parse_structure(
    reg: &WorldgenRegistries,
    biomes: &BiomeRegistry,
    id: &Identifier,
    json: &Value,
) -> Result<Structure> {
    let ty = codec::type_name(json)?;
    let biome_list = parse_biome_holder_set(reg, biomes, field(json, "biomes")?)?;
    let step = Decoration::parse(codec::as_str(field(json, "step")?)?)?;
    let terrain_adaptation = match json.get("terrain_adaptation") {
        Some(v) => TerrainAdjustment::parse(codec::as_str(v)?)?,
        None => TerrainAdjustment::None,
    };
    let kind = match ty.as_str() {
        "jigsaw" => StructureKind::Jigsaw(Arc::new(super::jigsaw::JigsawConfig::parse(reg, json)?)),
        "buried_treasure" => StructureKind::BuriedTreasure,
        "desert_pyramid" => StructureKind::DesertPyramid,
        "end_city" => StructureKind::EndCity,
        "fortress" => StructureKind::Fortress,
        "igloo" => StructureKind::Igloo,
        "jungle_temple" => StructureKind::JungleTemple,
        "woodland_mansion" => StructureKind::Mansion,
        "mineshaft" => StructureKind::Mineshaft {
            mesa: codec::as_str(field(json, "mineshaft_type")?)? == "mesa",
        },
        "ocean_monument" => StructureKind::OceanMonument {
            surrounding: parse_biome_holder_set(
                reg,
                biomes,
                &Value::String("#minecraft:required_ocean_monument_surrounding".into()),
            )?
            .into_iter()
            .collect(),
        },
        "nether_fossil" => StructureKind::NetherFossil {
            height: HeightProvider::parse(field(json, "height")?)?,
        },
        "ocean_ruin" => StructureKind::OceanRuin {
            warm: codec::as_str(field(json, "biome_temp")?)? == "warm",
            large_probability: codec::as_f32(field(json, "large_probability")?)?,
            cluster_probability: codec::as_f32(field(json, "cluster_probability")?)?,
        },
        "ruined_portal" => {
            let mut setups = Vec::new();
            for s in codec::as_array(field(json, "setups")?)? {
                setups.push(RuinedPortalSetup {
                    placement: codec::as_str(field(s, "placement")?)?.to_owned(),
                    air_pocket_probability: codec::as_f32(field(s, "air_pocket_probability")?)?,
                    mossiness: codec::as_f32(field(s, "mossiness")?)?,
                    overgrown: opt_bool(s, "overgrown", false)?,
                    vines: opt_bool(s, "vines", false)?,
                    can_be_cold: opt_bool(s, "can_be_cold", false)?,
                    replace_with_blackstone: opt_bool(s, "replace_with_blackstone", false)?,
                    weight: opt_f32(s, "weight", 1.0)?,
                });
            }
            StructureKind::RuinedPortal { setups }
        }
        "shipwreck" => StructureKind::Shipwreck {
            beached: opt_bool(json, "is_beached", false)?,
        },
        "stronghold" => StructureKind::Stronghold,
        "swamp_hut" => StructureKind::SwampHut,
        other => bail!("unsupported structure type {other}"),
    };
    Ok(Structure {
        id: id.clone(),
        biomes: biome_list.into_iter().collect(),
        step,
        terrain_adaptation,
        kind,
        spawn_overrides: json.get("spawn_overrides").cloned().unwrap_or(Value::Null),
    })
}
