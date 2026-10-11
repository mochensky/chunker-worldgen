//! `IntProvider`, `FloatProvider`, `HeightProvider` and `WeightedList`.

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::codec::{self, field};
use crate::math::{clamp_f32, clamp_i32};
use crate::random::RandomSource;
use crate::terrain::material::{GenerationContext, VerticalAnchor};

/// `Mth.randomBetweenInclusive`.
#[inline]
pub fn random_between_inclusive(random: &mut dyn RandomSource, min: i32, max_inclusive: i32) -> i32 {
    random.next_int_bounded(max_inclusive - min + 1) + min
}

/// `Mth.nextInt(random, min, maxInclusive)`.
#[inline]
pub fn mth_next_int(random: &mut dyn RandomSource, min: i32, max_inclusive: i32) -> i32 {
    if min >= max_inclusive {
        min
    } else {
        random.next_int_bounded(max_inclusive - min + 1) + min
    }
}

/// `Mth.randomBetween(random, min, maxExclusive)` (float).
#[inline]
pub fn random_between_f32(random: &mut dyn RandomSource, min: f32, max_exclusive: f32) -> f32 {
    random.next_float() * (max_exclusive - min) + min
}

/// `WeightedList<E>`: entries with integer weights, selected by `nextInt(totalWeight)`.
#[derive(Clone, Debug)]
pub struct WeightedList<T> {
    pub entries: Vec<(T, i32)>,
    total: i32,
}

impl<T> WeightedList<T> {
    pub fn new(entries: Vec<(T, i32)>) -> Self {
        let total = entries.iter().map(|(_, w)| *w).sum();
        Self { entries, total }
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn total_weight(&self) -> i32 {
        self.total
    }

    /// `getRandom`: `None` for an empty list (no random call).
    pub fn get_random(&self, random: &mut dyn RandomSource) -> Option<&T> {
        if self.total == 0 {
            return None;
        }
        let mut selection = random.next_int_bounded(self.total);
        for (e, w) in &self.entries {
            if selection < *w {
                return Some(e);
            }
            selection -= w;
        }
        unreachable!()
    }

    /// Parses `[{"data": ..., "weight": n}, ...]`.
    pub fn parse(json: &Value, mut element: impl FnMut(&Value) -> Result<T>) -> Result<Self> {
        let list = json.as_array().context("weighted list must be an array")?;
        let mut entries = Vec::with_capacity(list.len());
        for e in list {
            let w = codec::i32_field(e, "weight")?;
            if w < 0 {
                bail!("negative weight");
            }
            entries.push((element(field(e, "data")?)?, w));
        }
        Ok(Self::new(entries))
    }
}

#[derive(Clone, Debug)]
pub enum IntProvider {
    Constant(i32),
    Uniform {
        min: i32,
        max: i32,
    },
    BiasedToBottom {
        min: i32,
        max: i32,
    },
    VeryBiasedToBottom {
        min: i32,
        max: i32,
    },
    Clamped {
        source: Box<IntProvider>,
        min: i32,
        max: i32,
    },
    WeightedList(WeightedList<IntProvider>),
    ClampedNormal {
        mean: f32,
        deviation: f32,
        min: i32,
        max: i32,
    },
    Trapezoid {
        min: i32,
        max: i32,
        plateau: i32,
    },
}

impl IntProvider {
    pub fn sample(&self, random: &mut dyn RandomSource) -> i32 {
        match self {
            IntProvider::Constant(v) => *v,
            IntProvider::Uniform { min, max } => random_between_inclusive(random, *min, *max),
            IntProvider::BiasedToBottom { min, max } => {
                let inner = random.next_int_bounded(max - min + 1) + 1;
                min + random.next_int_bounded(inner)
            }
            IntProvider::VeryBiasedToBottom { min, max } => {
                let a = random.next_int_bounded(max - min + 1) + 1;
                let b = random.next_int_bounded(a) + 1;
                min + random.next_int_bounded(b)
            }
            IntProvider::Clamped { source, min, max } => clamp_i32(source.sample(random), *min, *max),
            IntProvider::WeightedList(list) => list
                .get_random(random)
                .expect("Weighted list has no elements")
                .sample(random),
            IntProvider::ClampedNormal {
                mean,
                deviation,
                min,
                max,
            } => {
                let v = mean + random.next_gaussian() as f32 * deviation;
                clamp_f32(v, *min as f32, *max as f32) as i32
            }
            IntProvider::Trapezoid { min, max, plateau } => {
                if *plateau == 0 && *max == -*min {
                    random.next_int_bounded(max + 1) - random.next_int_bounded(max + 1)
                } else {
                    let range = max - min;
                    if *plateau == range {
                        random_between_inclusive(random, *min, *max)
                    } else {
                        let start = (range - plateau) / 2;
                        let end = range - start;
                        min + random_between_inclusive(random, 0, end)
                            + random_between_inclusive(random, 0, start)
                    }
                }
            }
        }
    }

    pub fn min_inclusive(&self) -> i32 {
        match self {
            IntProvider::Constant(v) => *v,
            IntProvider::Uniform { min, .. }
            | IntProvider::BiasedToBottom { min, .. }
            | IntProvider::VeryBiasedToBottom { min, .. }
            | IntProvider::ClampedNormal { min, .. }
            | IntProvider::Trapezoid { min, .. } => *min,
            IntProvider::Clamped { source, min, .. } => (*min).max(source.min_inclusive()),
            IntProvider::WeightedList(l) => l
                .entries
                .iter()
                .map(|(e, _)| e.min_inclusive())
                .min()
                .unwrap_or(i32::MAX),
        }
    }

    pub fn max_inclusive(&self) -> i32 {
        match self {
            IntProvider::Constant(v) => *v,
            IntProvider::Uniform { max, .. }
            | IntProvider::BiasedToBottom { max, .. }
            | IntProvider::VeryBiasedToBottom { max, .. }
            | IntProvider::ClampedNormal { max, .. }
            | IntProvider::Trapezoid { max, .. } => *max,
            IntProvider::Clamped { source, max, .. } => (*max).min(source.max_inclusive()),
            IntProvider::WeightedList(l) => l
                .entries
                .iter()
                .map(|(e, _)| e.max_inclusive())
                .max()
                .unwrap_or(i32::MIN),
        }
    }

    pub fn parse(json: &Value) -> Result<IntProvider> {
        if json.is_number() {
            return Ok(IntProvider::Constant(codec::as_i32(json)?));
        }
        let ty = codec::type_name(json)?;
        let i = |n: &str| codec::i32_field(json, n);
        Ok(match ty.as_str() {
            "constant" => IntProvider::Constant(i("value")?),
            "uniform" => IntProvider::Uniform {
                min: i("min_inclusive")?,
                max: i("max_inclusive")?,
            },
            "biased_to_bottom" => IntProvider::BiasedToBottom {
                min: i("min_inclusive")?,
                max: i("max_inclusive")?,
            },
            "very_biased_to_bottom" => IntProvider::VeryBiasedToBottom {
                min: i("min_inclusive")?,
                max: i("max_inclusive")?,
            },
            "clamped" => IntProvider::Clamped {
                source: Box::new(IntProvider::parse(field(json, "source")?)?),
                min: i("min_inclusive")?,
                max: i("max_inclusive")?,
            },
            "weighted_list" => IntProvider::WeightedList(WeightedList::parse(
                field(json, "distribution")?,
                IntProvider::parse,
            )?),
            "clamped_normal" => IntProvider::ClampedNormal {
                mean: codec::f32_field(json, "mean")?,
                deviation: codec::f32_field(json, "deviation")?,
                min: i("min_inclusive")?,
                max: i("max_inclusive")?,
            },
            "trapezoid" => IntProvider::Trapezoid {
                min: i("min")?,
                max: i("max")?,
                plateau: i("plateau")?,
            },
            other => bail!("unknown int provider {other}"),
        })
    }
}

#[derive(Clone, Debug)]
pub enum FloatProvider {
    Constant(f32),
    Uniform {
        min: f32,
        max: f32,
    },
    ClampedNormal {
        mean: f32,
        deviation: f32,
        min: f32,
        max: f32,
    },
    Trapezoid {
        min: f32,
        max: f32,
        plateau: f32,
    },
}

impl FloatProvider {
    pub fn sample(&self, random: &mut dyn RandomSource) -> f32 {
        match self {
            FloatProvider::Constant(v) => *v,
            FloatProvider::Uniform { min, max } => random_between_f32(random, *min, *max),
            FloatProvider::ClampedNormal {
                mean,
                deviation,
                min,
                max,
            } => clamp_f32(mean + random.next_gaussian() as f32 * deviation, *min, *max),
            FloatProvider::Trapezoid { min, max, plateau } => {
                let range = max - min;
                let start = (range - plateau) / 2.0;
                let end = range - start;
                min + random.next_float() * end + random.next_float() * start
            }
        }
    }

    pub fn parse(json: &Value) -> Result<FloatProvider> {
        if json.is_number() {
            return Ok(FloatProvider::Constant(codec::as_f32(json)?));
        }
        let ty = codec::type_name(json)?;
        let f = |n: &str| codec::f32_field(json, n);
        Ok(match ty.as_str() {
            "constant" => FloatProvider::Constant(f("value")?),
            "uniform" => FloatProvider::Uniform {
                min: f("min_inclusive")?,
                max: f("max_exclusive")?,
            },
            "clamped_normal" => FloatProvider::ClampedNormal {
                mean: f("mean")?,
                deviation: f("deviation")?,
                min: f("min")?,
                max: f("max")?,
            },
            "trapezoid" => FloatProvider::Trapezoid {
                min: f("min")?,
                max: f("max")?,
                plateau: f("plateau")?,
            },
            other => bail!("unknown float provider {other}"),
        })
    }
}

#[derive(Clone, Debug)]
pub enum HeightProvider {
    Constant(VerticalAnchor),
    Uniform {
        min: VerticalAnchor,
        max: VerticalAnchor,
    },
    BiasedToBottom {
        min: VerticalAnchor,
        max: VerticalAnchor,
        inner: i32,
    },
    VeryBiasedToBottom {
        min: VerticalAnchor,
        max: VerticalAnchor,
        inner: i32,
    },
    Trapezoid {
        min: VerticalAnchor,
        max: VerticalAnchor,
        plateau: i32,
    },
    WeightedList(WeightedList<HeightProvider>),
}

impl HeightProvider {
    pub fn sample(&self, random: &mut dyn RandomSource, ctx: &GenerationContext) -> i32 {
        match self {
            HeightProvider::Constant(a) => a.resolve(ctx),
            HeightProvider::Uniform { min, max } => {
                let (min, max) = (min.resolve(ctx), max.resolve(ctx));
                if min > max {
                    min
                } else {
                    random_between_inclusive(random, min, max)
                }
            }
            HeightProvider::BiasedToBottom { min, max, inner } => {
                let (min, max) = (min.resolve(ctx), max.resolve(ctx));
                if max - min - inner + 1 <= 0 {
                    return min;
                }
                let limit = random.next_int_bounded(max - min - inner + 1);
                random.next_int_bounded(limit + inner) + min
            }
            HeightProvider::VeryBiasedToBottom { min, max, inner } => {
                let (min, max) = (min.resolve(ctx), max.resolve(ctx));
                if max - min - inner + 1 <= 0 {
                    return min;
                }
                let upper = mth_next_int(random, min + inner, max);
                let biased = mth_next_int(random, min, upper - 1);
                mth_next_int(random, min, biased - 1 + inner)
            }
            HeightProvider::Trapezoid { min, max, plateau } => {
                let (min, max) = (min.resolve(ctx), max.resolve(ctx));
                if min > max {
                    return min;
                }
                let range = max - min;
                if *plateau >= range {
                    random_between_inclusive(random, min, max)
                } else {
                    let start = (range - plateau) / 2;
                    let end = range - start;
                    min + random_between_inclusive(random, 0, end)
                        + random_between_inclusive(random, 0, start)
                }
            }
            HeightProvider::WeightedList(l) => l
                .get_random(random)
                .expect("Weighted list has no elements")
                .sample(random, ctx),
        }
    }

    pub fn parse(json: &Value) -> Result<HeightProvider> {
        // A bare anchor object is accepted as a constant height (`Codec.either(VerticalAnchor, typed)`).
        if json.get("type").is_none() {
            return Ok(HeightProvider::Constant(VerticalAnchor::parse(json)?));
        }
        let ty = codec::type_name(json)?;
        let a = |n: &str| VerticalAnchor::parse(field(json, n)?);
        Ok(match ty.as_str() {
            "constant" => HeightProvider::Constant(a("value")?),
            "uniform" => HeightProvider::Uniform {
                min: a("min_inclusive")?,
                max: a("max_inclusive")?,
            },
            "biased_to_bottom" => HeightProvider::BiasedToBottom {
                min: a("min_inclusive")?,
                max: a("max_inclusive")?,
                inner: codec::opt_i32(json, "inner", 1)?,
            },
            "very_biased_to_bottom" => HeightProvider::VeryBiasedToBottom {
                min: a("min_inclusive")?,
                max: a("max_inclusive")?,
                inner: codec::opt_i32(json, "inner", 1)?,
            },
            "trapezoid" => HeightProvider::Trapezoid {
                min: a("min_inclusive")?,
                max: a("max_inclusive")?,
                plateau: codec::opt_i32(json, "plateau", 0)?,
            },
            "weighted_list" => HeightProvider::WeightedList(WeightedList::parse(
                field(json, "distribution")?,
                HeightProvider::parse,
            )?),
            other => bail!("unknown height provider {other}"),
        })
    }
}
