//! `StructurePlacement`: `random_spread` and `concentric_rings`.

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;

use super::util::legacy_worldgen_random;
use crate::biome::{BiomeId, BiomeRegistry, parse_biome_holder_set};
use crate::codec::{self, field};
use crate::random::RandomSource;
use crate::registry::WorldgenRegistries;

/// `AbstractSpreadingStructurePlacement.FrequencyReductionMethod`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrequencyReduction {
    Default,
    LegacyType1,
    LegacyType2,
    LegacyType3,
}

impl FrequencyReduction {
    fn should_generate(self, seed: i64, salt: i32, x: i32, z: i32, probability: f32) -> bool {
        let mut random = legacy_worldgen_random();
        match self {
            // Vanilla passes (seed, salt, x, z) into setLargeFeatureWithSalt(seed, x, z, salt).
            FrequencyReduction::Default => {
                random.set_large_feature_with_salt(seed, salt, x, z);
                random.next_float() < probability
            }
            FrequencyReduction::LegacyType1 => {
                let cx = x >> 4;
                let cz = z >> 4;
                random.set_seed((cx ^ (cz << 4)) as i64 ^ seed);
                random.next_int();
                random.next_int_bounded((1.0f32 / probability) as i32) == 0
            }
            FrequencyReduction::LegacyType2 => {
                random.set_large_feature_with_salt(seed, x, z, 10387320);
                random.next_float() < probability
            }
            FrequencyReduction::LegacyType3 => {
                random.set_large_feature_seed(seed, x, z);
                random.next_double() < probability as f64
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpreadType {
    Linear,
    Triangular,
}

impl SpreadType {
    fn evaluate(self, random: &mut dyn RandomSource, limit: i32) -> i32 {
        match self {
            SpreadType::Linear => random.next_int_bounded(limit),
            SpreadType::Triangular => (random.next_int_bounded(limit) + random.next_int_bounded(limit)) / 2,
        }
    }
}

#[derive(Clone, Debug)]
pub enum PlacementKind {
    RandomSpread {
        spacing: i32,
        separation: i32,
        spread: SpreadType,
    },
    ConcentricRings {
        distance: i32,
        spread: i32,
        count: i32,
        preferred_biomes: FxHashSet<BiomeId>,
    },
}

#[derive(Clone, Debug)]
pub struct Placement {
    pub locate_offset: (i32, i32, i32),
    pub frequency_reduction: FrequencyReduction,
    pub frequency: f32,
    pub salt: i32,
    /// `(other set index, chunk_count)`.
    pub exclusion_zone: Option<(usize, i32)>,
    pub kind: PlacementKind,
}

impl Placement {
    pub fn parse(
        reg: &WorldgenRegistries,
        biomes: &BiomeRegistry,
        json: &Value,
        sets: &FxHashMap<Identifier, usize>,
    ) -> Result<Self> {
        let ty = codec::type_name(json)?;
        let locate_offset = match json.get("locate_offset") {
            Some(v) => {
                let a = codec::as_array(v)?;
                (
                    codec::as_i32(&a[0])?,
                    codec::as_i32(&a[1])?,
                    codec::as_i32(&a[2])?,
                )
            }
            None => (0, 0, 0),
        };
        let frequency_reduction = match json
            .get("frequency_reduction_method")
            .map(codec::as_str)
            .transpose()?
        {
            None | Some("default") => FrequencyReduction::Default,
            Some("legacy_type_1") => FrequencyReduction::LegacyType1,
            Some("legacy_type_2") => FrequencyReduction::LegacyType2,
            Some("legacy_type_3") => FrequencyReduction::LegacyType3,
            Some(other) => bail!("unknown frequency reduction method {other}"),
        };
        let frequency = json.get("frequency").map_or(Ok(1.0), codec::as_f32)?;
        let salt = codec::as_i32(field(json, "salt")?)?;
        let exclusion_zone = match json.get("exclusion_zone") {
            Some(z) => {
                let other = Identifier::parse(codec::as_str(field(z, "other_set")?)?)
                    .context("bad structure set id")?;
                let index = *sets
                    .get(&other)
                    .with_context(|| format!("unknown structure set {other}"))?;
                Some((index, codec::as_i32(field(z, "chunk_count")?)?))
            }
            None => None,
        };
        let kind = match ty.as_str() {
            "random_spread" => {
                let spacing = codec::as_i32(field(json, "spacing")?)?;
                let separation = codec::as_i32(field(json, "separation")?)?;
                if spacing <= separation {
                    bail!("Spacing has to be larger than separation");
                }
                let spread = match json.get("spread_type").map(codec::as_str).transpose()? {
                    None | Some("linear") => SpreadType::Linear,
                    Some("triangular") => SpreadType::Triangular,
                    Some(other) => bail!("unknown spread type {other}"),
                };
                PlacementKind::RandomSpread {
                    spacing,
                    separation,
                    spread,
                }
            }
            "concentric_rings" => PlacementKind::ConcentricRings {
                distance: codec::as_i32(field(json, "distance")?)?,
                spread: codec::as_i32(field(json, "spread")?)?,
                count: codec::as_i32(field(json, "count")?)?,
                preferred_biomes: parse_biome_holder_set(reg, biomes, field(json, "preferred_biomes")?)?
                    .into_iter()
                    .collect(),
            },
            other => bail!("unsupported structure placement {other}"),
        };
        Ok(Self {
            locate_offset,
            frequency_reduction,
            frequency,
            salt,
            exclusion_zone,
            kind,
        })
    }

    /// `RandomSpreadStructurePlacement.getPotentialStructureChunk`.
    pub fn potential_chunk(&self, seed: i64, x: i32, z: i32) -> Option<(i32, i32)> {
        let PlacementKind::RandomSpread {
            spacing,
            separation,
            spread,
        } = self.kind
        else {
            return None;
        };
        let gx = x.div_euclid(spacing);
        let gz = z.div_euclid(spacing);
        let mut random = legacy_worldgen_random();
        random.set_large_feature_with_salt(seed, gx, gz, self.salt);
        let limit = spacing - separation;
        let sx = spread.evaluate(&mut random, limit);
        let sz = spread.evaluate(&mut random, limit);
        Some((gx * spacing + sx, gz * spacing + sz))
    }

    /// `isPlacementChunk && applyAdditionalChunkRestrictions` (the exclusion zone is checked by the caller).
    pub fn is_placement_chunk(
        &self,
        seed: i64,
        rings: Option<&FxHashSet<(i32, i32)>>,
        x: i32,
        z: i32,
    ) -> bool {
        let placed = match &self.kind {
            PlacementKind::RandomSpread { .. } => self.potential_chunk(seed, x, z) == Some((x, z)),
            PlacementKind::ConcentricRings { .. } => rings.is_some_and(|r| r.contains(&(x, z))),
        };
        placed
            && (self.frequency >= 1.0
                || self
                    .frequency_reduction
                    .should_generate(seed, self.salt, x, z, self.frequency))
    }
}
