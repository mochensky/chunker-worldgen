//! `BiomeSource` implementations and climate sampling.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use serde_json::Value;

use super::climate::{Parameter, ParameterList, ParameterPoint, TargetPoint, quantize};
use super::{BiomeId, BiomeRegistry, nether_parameters, overworld_parameters};
use crate::codec::{self, field, str_field};
use crate::density::sampler::SamplerRef;
use crate::density::{DensityVolume, SamplerContext};
use crate::random_state::RandomState;
use crate::registry::WorldgenRegistries;
use crate::settings::NoiseRouter;

/// `Climate.Sampler` (six compiled router functions).
pub struct ClimateSampler {
    pub temperature: SamplerRef,
    pub humidity: SamplerRef,
    pub continentalness: SamplerRef,
    pub erosion: SamplerRef,
    pub depth: SamplerRef,
    pub weirdness: SamplerRef,
}

impl ClimateSampler {
    pub fn new(state: &RandomState, router: &NoiseRouter) -> Self {
        Self {
            temperature: state.sampler(&router.temperature),
            humidity: state.sampler(&router.vegetation),
            continentalness: state.sampler(&router.continents),
            erosion: state.sampler(&router.erosion),
            depth: state.sampler(&router.depth),
            weirdness: state.sampler(&router.ridges),
        }
    }

    /// `Climate.Sampler.sample(quartX, quartY, quartZ)`.
    pub fn sample(&self, ctx: &mut SamplerContext, qx: i32, qy: i32, qz: i32) -> TargetPoint {
        let (x, y, z) = (qx << 2, qy << 2, qz << 2);
        TargetPoint::new(
            self.temperature.sample_value(ctx, x, y, z),
            self.humidity.sample_value(ctx, x, y, z),
            self.continentalness.sample_value(ctx, x, y, z),
            self.erosion.sample_value(ctx, x, y, z),
            self.depth.sample_value(ctx, x, y, z),
            self.weirdness.sample_value(ctx, x, y, z),
        )
    }
}

pub enum BiomeSource {
    MultiNoise(Arc<ParameterList<BiomeId>>),
    TheEnd {
        end: BiomeId,
        highlands: BiomeId,
        midlands: BiomeId,
        islands: BiomeId,
        barrens: BiomeId,
    },
    Fixed(BiomeId),
    Checkerboard {
        biomes: Vec<BiomeId>,
        bit_shift: i32,
    },
}

/// Resolver over a quart box from [`BiomeSource::box_resolver`].
pub struct BoxResolver {
    min: (i32, i32, i32),
    climate: Option<(DensityVolume, [Vec<f32>; 6])>,
}

impl BoxResolver {
    pub fn get(
        &self,
        source: &BiomeSource,
        climate: &ClimateSampler,
        ctx: &mut SamplerContext,
        qx: i32,
        qy: i32,
        qz: i32,
    ) -> BiomeId {
        match (&self.climate, source) {
            (Some((vol, c)), BiomeSource::MultiNoise(list)) => {
                let i = vol.index(qx - self.min.0, qy - self.min.1, qz - self.min.2);
                *list.find_value(&TargetPoint::new(
                    c[0][i], c[1][i], c[2][i], c[3][i], c[4][i], c[5][i],
                ))
            }
            _ => source.noise_biome(climate, ctx, qx, qy, qz),
        }
    }
}

/// Biomes of one chunk at quart resolution, as produced by `fillBiomesFromNoise`.
pub struct BiomeResolver {
    pub min_qx: i32,
    pub min_qy: i32,
    pub min_qz: i32,
    pub size_y: i32,
    /// Indexed `[(qy - min_qy) * 16 + (qz - min_qz) * 4 + (qx - min_qx)]`.
    pub biomes: Vec<BiomeId>,
}

impl BiomeResolver {
    #[inline]
    pub fn get(&self, qx: i32, qy: i32, qz: i32) -> BiomeId {
        let y = (qy - self.min_qy).clamp(0, self.size_y - 1);
        self.biomes[((y * 16) + (qz - self.min_qz) * 4 + (qx - self.min_qx)) as usize]
    }
}

impl BiomeSource {
    /// Single point lookup (`createResolver(sampler).getNoiseBiome`).
    pub fn noise_biome(
        &self,
        climate: &ClimateSampler,
        ctx: &mut SamplerContext,
        qx: i32,
        qy: i32,
        qz: i32,
    ) -> BiomeId {
        match self {
            BiomeSource::MultiNoise(list) => *list.find_value(&climate.sample(ctx, qx, qy, qz)),
            BiomeSource::TheEnd {
                end,
                highlands,
                midlands,
                islands,
                barrens,
            } => {
                let (bx, by, bz) = (qx << 2, qy << 2, qz << 2);
                let (cx, cz) = (bx >> 4, bz >> 4);
                if (cx as i64) * (cx as i64) + (cz as i64) * (cz as i64) <= 4096 {
                    return *end;
                }
                let wx = ((bx >> 4) * 2 + 1) * 8;
                let wz = ((bz >> 4) * 2 + 1) * 8;
                let h = climate.erosion.sample_value(ctx, wx, by, wz) as f64;
                if h > 0.25 {
                    *highlands
                } else if h >= -0.0625 {
                    *midlands
                } else if h < -0.21875 {
                    *islands
                } else {
                    *barrens
                }
            }
            BiomeSource::Fixed(b) => *b,
            BiomeSource::Checkerboard { biomes, bit_shift } => {
                let i = crate::math::floor_mod((qx >> bit_shift) + (qz >> bit_shift), biomes.len() as i32);
                biomes[i as usize]
            }
        }
    }

    /// `ChunkGenerator.doCreateBiomes` for one chunk: `createResolverForChunk` + `fillBiomesFromNoise`,
    /// evaluated in vanilla order (sections bottom-up, then x, y, z).
    pub fn resolve_chunk(
        &self,
        climate: &ClimateSampler,
        ctx: &mut SamplerContext,
        chunk_x: i32,
        chunk_z: i32,
        min_y: i32,
        height: i32,
    ) -> BiomeResolver {
        let min_qx = chunk_x << 2;
        let min_qz = chunk_z << 2;
        let min_qy = min_y >> 2;
        let size_y = height >> 2;
        let mut biomes = vec![BiomeId(0); (size_y * 16) as usize];
        let put = |biomes: &mut Vec<BiomeId>, x: i32, y: i32, z: i32, b: BiomeId| {
            biomes[(y * 16 + z * 4 + x) as usize] = b;
        };
        match self {
            BiomeSource::MultiNoise(list) => {
                let vol = DensityVolume::new(4, size_y, 4, min_qx << 2, min_qy << 2, min_qz << 2, 4, 4, 4);
                let n = vol.size();
                let sample = |s: &SamplerRef, ctx: &mut SamplerContext| {
                    let mut buf = vec![0.0f32; n];
                    s.sample_volume(ctx, &mut buf, &vol);
                    buf
                };
                let t = sample(&climate.temperature, ctx);
                let h = sample(&climate.humidity, ctx);
                let c = sample(&climate.continentalness, ctx);
                let e = sample(&climate.erosion, ctx);
                let d = sample(&climate.depth, ctx);
                let w = sample(&climate.weirdness, ctx);
                for section in 0..size_y / 4 {
                    for x in 0..4 {
                        for yi in 0..4 {
                            let y = section * 4 + yi;
                            for z in 0..4 {
                                let i = vol.index(x, y, z);
                                let target = TargetPoint::new(t[i], h[i], c[i], e[i], d[i], w[i]);
                                put(&mut biomes, x, y, z, *list.find_value(&target));
                            }
                        }
                    }
                }
            }
            _ => {
                for section in 0..size_y / 4 {
                    for x in 0..4 {
                        for yi in 0..4 {
                            let y = section * 4 + yi;
                            for z in 0..4 {
                                let b = self.noise_biome(climate, ctx, min_qx + x, min_qy + y, min_qz + z);
                                put(&mut biomes, x, y, z, b);
                            }
                        }
                    }
                }
            }
        }
        BiomeResolver {
            min_qx,
            min_qy,
            min_qz,
            size_y,
            biomes,
        }
    }

    /// `BiomeSource.possibleBiomes()`.
    pub fn possible_biomes(&self) -> Vec<BiomeId> {
        let mut out: Vec<BiomeId> = match self {
            BiomeSource::MultiNoise(list) => list.values.iter().map(|(_, b)| *b).collect(),
            BiomeSource::TheEnd {
                end,
                highlands,
                midlands,
                islands,
                barrens,
            } => vec![*end, *highlands, *midlands, *islands, *barrens],
            BiomeSource::Fixed(b) => vec![*b],
            BiomeSource::Checkerboard { biomes, .. } => biomes.clone(),
        };
        let mut seen = rustc_hash::FxHashSet::default();
        out.retain(|b| seen.insert(*b));
        out
    }

    /// `createResolverForChunk` for an arbitrary quart box: multi-noise sources sample the climate
    /// over the box up front (volume path); lookups then resolve lazily like the vanilla lambda.
    pub fn box_resolver(
        &self,
        climate: &ClimateSampler,
        ctx: &mut SamplerContext,
        min_qx: i32,
        min_qy: i32,
        min_qz: i32,
        size_x: i32,
        size_y: i32,
        size_z: i32,
    ) -> BoxResolver {
        let climate_values = match self {
            BiomeSource::MultiNoise(_) => {
                let vol = DensityVolume::new(
                    size_x,
                    size_y,
                    size_z,
                    min_qx << 2,
                    min_qy << 2,
                    min_qz << 2,
                    4,
                    4,
                    4,
                );
                let n = vol.size();
                let mut sample = |s: &SamplerRef| {
                    let mut buf = vec![0.0f32; n];
                    s.sample_volume(ctx, &mut buf, &vol);
                    buf
                };
                Some((
                    vol,
                    [
                        sample(&climate.temperature),
                        sample(&climate.humidity),
                        sample(&climate.continentalness),
                        sample(&climate.erosion),
                        sample(&climate.depth),
                        sample(&climate.weirdness),
                    ],
                ))
            }
            _ => None,
        };
        BoxResolver {
            min: (min_qx, min_qy, min_qz),
            climate: climate_values,
        }
    }

    /// Parses a `biome_source` object of a chunk generator.
    pub fn parse(reg: &WorldgenRegistries, biomes: &BiomeRegistry, json: &Value) -> Result<BiomeSource> {
        let ty = codec::type_name(json)?;
        let biome = |v: &Value| -> Result<BiomeId> {
            biomes.require(&Identifier::parse(codec::as_str(v)?).context("bad biome id")?)
        };
        Ok(match ty.as_str() {
            "multi_noise" => {
                let values: Vec<(ParameterPoint, BiomeId)> = if let Some(preset) = json.get("preset") {
                    let preset_id = match preset {
                        Value::String(s) => {
                            let id = Identifier::parse(s).context("bad preset id")?;
                            let file = reg
                                .packs
                                .require_json("worldgen/multi_noise_biome_source_parameter_list", &id)?;
                            str_field(&file, "preset")?.to_owned()
                        }
                        Value::Object(_) => str_field(preset, "preset")?.to_owned(),
                        _ => bail!("invalid preset"),
                    };
                    let raw = match Identifier::parse(&preset_id).context("bad preset")?.as_str() {
                        "minecraft:overworld" => overworld_parameters(),
                        "minecraft:nether" => nether_parameters(),
                        other => bail!("Unknown preset: {other}"),
                    };
                    raw.into_iter()
                        .map(|(p, b)| Ok((p, biomes.require(&Identifier::vanilla(b))?)))
                        .collect::<Result<_>>()?
                } else {
                    field(json, "biomes")?
                        .as_array()
                        .context("biomes must be a list")?
                        .iter()
                        .map(|e| {
                            Ok((
                                parse_parameter_point(field(e, "parameters")?)?,
                                biome(field(e, "biome")?)?,
                            ))
                        })
                        .collect::<Result<_>>()?
                };
                BiomeSource::MultiNoise(Arc::new(ParameterList::new(values)))
            }
            "the_end" => {
                let b = |p: &str| biomes.require(&Identifier::vanilla(p));
                BiomeSource::TheEnd {
                    end: b("the_end")?,
                    highlands: b("end_highlands")?,
                    midlands: b("end_midlands")?,
                    islands: b("small_end_islands")?,
                    barrens: b("end_barrens")?,
                }
            }
            "fixed" => BiomeSource::Fixed(biome(field(json, "biome")?)?),
            "checkerboard" => {
                let list = parse_biome_holder_set(reg, biomes, field(json, "biomes")?)?;
                let scale = codec::opt_i32(json, "scale", 2)?;
                BiomeSource::Checkerboard {
                    biomes: list,
                    bit_shift: scale + 2,
                }
            }
            other => bail!("unsupported biome source {other}"),
        })
    }
}

/// `HolderSet<Biome>`: id, list of ids, or `#tag`.
pub fn parse_biome_holder_set(
    reg: &WorldgenRegistries,
    biomes: &BiomeRegistry,
    json: &Value,
) -> Result<Vec<BiomeId>> {
    match json {
        Value::String(s) if s.starts_with('#') => {
            let tag = Identifier::parse(&s[1..]).context("bad tag")?;
            let ids = reg
                .packs
                .resolve_tag("worldgen/biome", &tag, &|id| biomes.get(id).is_some())?;
            ids.iter().map(|id| biomes.require(id)).collect()
        }
        Value::String(s) => Ok(vec![biomes.require(&Identifier::parse(s).context("bad biome")?)?]),
        Value::Array(a) => a
            .iter()
            .map(|v| biomes.require(&Identifier::parse(codec::as_str(v)?).context("bad biome")?))
            .collect(),
        other => bail!("invalid biome holder set {other}"),
    }
}

fn parse_parameter(v: &Value) -> Result<Parameter> {
    let (min, max) = match v {
        Value::Array(a) if a.len() == 2 => (codec::as_f32(&a[0])?, codec::as_f32(&a[1])?),
        Value::Array(a) if a.len() == 1 => {
            let x = codec::as_f32(&a[0])?;
            (x, x)
        }
        other => {
            let x = codec::as_f32(other)?;
            (x, x)
        }
    };
    if !(-2.0..=2.0).contains(&min) || !(-2.0..=2.0).contains(&max) || min > max {
        bail!("invalid climate parameter [{min}, {max}]");
    }
    Ok(Parameter {
        min: quantize(min),
        max: quantize(max),
    })
}

fn parse_parameter_point(v: &Value) -> Result<ParameterPoint> {
    let offset = codec::f32_field(v, "offset")?;
    if !(0.0..=1.0).contains(&offset) {
        bail!("offset out of range");
    }
    Ok(ParameterPoint {
        temperature: parse_parameter(field(v, "temperature")?)?,
        humidity: parse_parameter(field(v, "humidity")?)?,
        continentalness: parse_parameter(field(v, "continentalness")?)?,
        erosion: parse_parameter(field(v, "erosion")?)?,
        depth: parse_parameter(field(v, "depth")?)?,
        weirdness: parse_parameter(field(v, "weirdness")?)?,
        offset: quantize(offset),
    })
}
