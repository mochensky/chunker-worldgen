//! JSON -> [`DensityFunction`](super::DensityFunction) (`DensityFunction.CODEC` / `DensityFunctions.DIRECT_CODEC`).

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use serde_json::Value;

use super::spline::Spline;
use super::{
    Axis, BinaryKind, BlendedNoiseParams, DensityFunction as D, Df, DistanceMetric, NoiseHolder, RoundKind,
    SimpleKind, TilingMode, UnaryKind,
};
use crate::codec::{self, f32_field, f64_field, field, i32_field, str_field};
use crate::registry::WorldgenRegistries;

const MAX_REASONABLE: f32 = 1_000_000.0;

fn noise_value(v: &Value) -> Result<f32> {
    let f = codec::as_f32(v)?;
    if !(-MAX_REASONABLE..=MAX_REASONABLE).contains(&f) {
        bail!("value {f} outside of [-1000000; 1000000]");
    }
    Ok(f)
}

fn noise_value_field(obj: &Value, name: &str) -> Result<f32> {
    noise_value(field(obj, name)?).with_context(|| format!("field `{name}`"))
}

impl WorldgenRegistries {
    /// Parses a density function value (registry id string, number, or typed object).
    pub fn parse_density_function(&self, json: &Value) -> Result<Df> {
        match json {
            Value::String(id) => {
                let id = Identifier::parse(id).with_context(|| format!("bad density function id {id}"))?;
                self.density_function(&id)
            }
            Value::Number(_) => Ok(Arc::new(D::Constant(noise_value(json)?))),
            Value::Object(_) => self.parse_direct(json),
            other => bail!("invalid density function {other}"),
        }
    }

    fn df_field(&self, obj: &Value, name: &str) -> Result<Df> {
        self.parse_density_function(field(obj, name)?)
            .with_context(|| format!("in `{name}`"))
    }

    fn df_field_or_zero(&self, obj: &Value, name: &str) -> Result<Df> {
        match obj.get(name) {
            Some(v) => self
                .parse_density_function(v)
                .with_context(|| format!("in `{name}`")),
            None => Ok(D::zero()),
        }
    }

    fn noise_holder(&self, json: &Value) -> Result<NoiseHolder> {
        match json {
            Value::String(id) => {
                let id = Identifier::parse(id).with_context(|| format!("bad noise id {id}"))?;
                Ok(NoiseHolder {
                    noise: self.noise(&id)?,
                    id,
                })
            }
            // Vanilla `RandomState.createNoiseSampler` requires a registry key (`unwrapKey().orElseThrow()`).
            _ => bail!("inline noise definitions are not supported by vanilla density functions"),
        }
    }

    fn parse_direct(&self, obj: &Value) -> Result<Df> {
        let ty = codec::type_name(obj)?;
        let node = match ty.as_str() {
            "constant" => D::Constant(noise_value_field(obj, "value")?),
            "blend_alpha" => D::Simple(SimpleKind::BlendAlpha),
            "blend_offset" => D::Simple(SimpleKind::BlendOffset),
            "beardifier" => D::Simple(SimpleKind::Beardifier),
            "noise" => D::Noise {
                noise: self.noise_holder(field(obj, "noise")?)?,
                xz_scale: f64_field(obj, "xz_scale")?,
                y_scale: f64_field(obj, "y_scale")?,
                shift_x: self.df_field_or_zero(obj, "shift_x")?,
                shift_y: self.df_field_or_zero(obj, "shift_y")?,
                shift_z: self.df_field_or_zero(obj, "shift_z")?,
            },
            "end_outer_islands" => D::EndIslands,
            "distance_to_point" => {
                let p = field(obj, "point")?
                    .as_array()
                    .context("point must be [x, y, z]")?;
                if p.len() != 3 {
                    bail!("point must have 3 elements");
                }
                let metric = match str_field(obj, "metric")? {
                    "euclidean" => DistanceMetric::Euclidean,
                    "euclidean_squared" => DistanceMetric::EuclideanSquared,
                    "manhattan" => DistanceMetric::Manhattan,
                    "chebyshev" => DistanceMetric::Chebyshev,
                    m => bail!("unknown distance metric {m}"),
                };
                D::DistanceToPoint {
                    point: [
                        codec::as_i32(&p[0])?,
                        codec::as_i32(&p[1])?,
                        codec::as_i32(&p[2])?,
                    ],
                    metric,
                }
            }
            "gradient" => {
                let axis = Axis::parse(str_field(obj, "axis")?).context("bad axis")?;
                let tiling = match obj.get("tiling").map(codec::as_str).transpose()? {
                    None | Some("clamp_to_edge") => TilingMode::ClampToEdge,
                    Some("repeat") => TilingMode::Repeat,
                    Some("mirrored_repeat") => TilingMode::MirroredRepeat,
                    Some(t) => bail!("unknown tiling {t}"),
                };
                let from_coordinate = i32_field(obj, "from_coordinate")?;
                let to_coordinate = i32_field(obj, "to_coordinate")?;
                if from_coordinate == to_coordinate {
                    bail!("from_coordinate cannot be equal to to_coordinate");
                }
                D::Gradient {
                    axis,
                    tiling,
                    from_coordinate,
                    to_coordinate,
                    from_value: noise_value_field(obj, "from_value")?,
                    to_value: noise_value_field(obj, "to_value")?,
                }
            }
            "shift_a" => D::ShiftA(self.noise_holder(field(obj, "noise")?)?),
            "shift_b" => D::ShiftB(self.noise_holder(field(obj, "noise")?)?),
            "shift" => D::Shift(self.noise_holder(field(obj, "noise")?)?),
            "pow" => D::Pow {
                base: self.df_field(obj, "base")?,
                exponent: self.df_field(obj, "exponent")?,
            },
            "spline" => D::Spline(Arc::new(self.parse_spline(field(obj, "spline")?)?)),
            "lerp" => D::Lerp {
                alpha: self.df_field(obj, "alpha")?,
                first: self.df_field(obj, "first")?,
                second: self.df_field(obj, "second")?,
            },
            "clamp" => {
                let min = noise_value_field(obj, "min")?;
                let max = noise_value_field(obj, "max")?;
                if max < min {
                    bail!("min ({min}) must be less than or equal to max ({max})");
                }
                D::Clamp {
                    input: self.df_field(obj, "input")?,
                    min,
                    max,
                }
            }
            "range_choice" => D::RangeChoice {
                input: self.df_field(obj, "input")?,
                min_inclusive: noise_value_field(obj, "min_inclusive")?,
                max_exclusive: noise_value_field(obj, "max_exclusive")?,
                when_in_range: self.df_field(obj, "when_in_range")?,
                when_out_of_range: self.df_field(obj, "when_out_of_range")?,
            },
            "interval_select" => {
                let thresholds: Vec<f32> = field(obj, "thresholds")?
                    .as_array()
                    .context("thresholds")?
                    .iter()
                    .map(noise_value)
                    .collect::<Result<_>>()?;
                let functions: Vec<Df> = field(obj, "functions")?
                    .as_array()
                    .context("functions")?
                    .iter()
                    .map(|f| self.parse_density_function(f))
                    .collect::<Result<_>>()?;
                if functions.len() < 2 {
                    bail!("interval_select needs at least 2 functions");
                }
                if thresholds.len() != functions.len() - 1 {
                    bail!(
                        "Expected {} thresholds for {} functions, but got {}",
                        functions.len() - 1,
                        functions.len(),
                        thresholds.len()
                    );
                }
                if thresholds.windows(2).any(|w| w[0] > w[1]) {
                    bail!("Threshold values must be ordered from smallest to largest");
                }
                D::IntervalSelect {
                    input: self.df_field(obj, "input")?,
                    thresholds,
                    functions,
                }
            }
            "cache" => D::Cache(self.df_field(obj, "input")?),
            "blend_density" => D::BlendDensity(self.df_field(obj, "input")?),
            "interpolated" => {
                let cell_size_xz = i32_field(obj, "cell_size_xz")?;
                let cell_size_y = i32_field(obj, "cell_size_y")?;
                if cell_size_xz <= 0 || cell_size_y <= 0 {
                    bail!("interpolated cell sizes must be positive");
                }
                D::Interpolated {
                    input: self.df_field(obj, "input")?,
                    cell_size_xz,
                    cell_size_y,
                }
            }
            "slice" => D::Slice {
                axis: Axis::parse(str_field(obj, "axis")?).context("bad axis")?,
                coordinate: i32_field(obj, "coordinate")?,
                input: self.df_field(obj, "input")?,
            },
            "find_top_surface" => {
                let cell_height = i32_field(obj, "cell_height")?;
                if cell_height <= 0 {
                    bail!("cell_height must be positive");
                }
                D::FindTopSurface {
                    density: self.df_field(obj, "density")?,
                    upper_bound: self.df_field(obj, "upper_bound")?,
                    lower_bound: i32_field(obj, "lower_bound")?,
                    cell_height,
                }
            }
            "old_blended_noise" => {
                let scale = |name: &str| -> Result<f64> {
                    let v = f64_field(obj, name)?;
                    if !(0.001..=1000.0).contains(&v) {
                        bail!("{name} out of range");
                    }
                    Ok(v)
                };
                let smear = f64_field(obj, "smear_scale_multiplier")?;
                if !(1.0..=8.0).contains(&smear) {
                    bail!("smear_scale_multiplier out of range");
                }
                D::OldBlendedNoise(BlendedNoiseParams {
                    xz_scale: scale("xz_scale")?,
                    y_scale: scale("y_scale")?,
                    xz_factor: scale("xz_factor")?,
                    y_factor: scale("y_factor")?,
                    smear_scale_multiplier: smear,
                })
            }
            other => {
                if let Some(kind) = UnaryKind::from_id(other) {
                    D::Unary(kind, self.df_field(obj, "input")?)
                } else if let Some(kind) = BinaryKind::from_id(other) {
                    D::Binary(kind, self.df_field(obj, "left")?, self.df_field(obj, "right")?)
                } else if let Some(kind) = RoundKind::from_id(other) {
                    let multiple = match obj.get("multiple") {
                        Some(v) => self.parse_density_function(v)?,
                        None => Arc::new(D::Constant(1.0)),
                    };
                    D::Round {
                        kind,
                        input: self.df_field(obj, "input")?,
                        multiple,
                    }
                } else {
                    bail!("unknown density function type {other}");
                }
            }
        };
        Ok(Arc::new(node))
    }

    fn parse_spline(&self, json: &Value) -> Result<Spline> {
        match json {
            Value::Number(_) => Ok(Spline::Constant(codec::as_f32(json)?)),
            Value::Object(_) => {
                let coordinate = self.df_field(json, "coordinate")?;
                let points = field(json, "points")?.as_array().context("points")?;
                if points.is_empty() {
                    bail!("spline needs at least one point");
                }
                let mut locations = Vec::with_capacity(points.len());
                let mut values = Vec::with_capacity(points.len());
                let mut derivatives = Vec::with_capacity(points.len());
                for p in points {
                    locations.push(f32_field(p, "location")?);
                    values.push(self.parse_spline(field(p, "value")?)?);
                    derivatives.push(f32_field(p, "derivative")?);
                }
                Ok(Spline::Multipoint {
                    coordinate,
                    locations,
                    values,
                    derivatives,
                })
            }
            other => bail!("invalid spline {other}"),
        }
    }
}
