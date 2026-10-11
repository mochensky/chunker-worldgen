//! `PlacementModifier`s and `FeaturePlacer`.

use anyhow::{Context, Result, bail};
use chunker_data::blocks::BlockId;
use serde_json::Value;

use super::Region;
use super::predicate::BlockPredicate;
use super::registry::{Parser, PlacedFeature};
use crate::chunk::HeightmapKind;
use crate::codec::{as_array, f32_field, f64_field, field, i32_field, str_field, type_name};
use crate::providers::{HeightProvider, IntProvider};
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

#[derive(Debug)]
pub enum Placement {
    Count(IntProvider),
    InSquare,
    Heightmap(HeightmapKind),
    Biome,
    BlockPredicateFilter(BlockPredicate),
    Rarity(i32),
    RandomChance(f32),
    SurfaceWaterDepth(i32),
    SurfaceRelativeThreshold {
        kind: HeightmapKind,
        min: i32,
        max: i32,
    },
    HeightRange(HeightProvider),
    Offset(IntProvider, IntProvider, IntProvider),
    EnvironmentScan {
        direction: Direction,
        target: BlockPredicate,
        allowed: BlockPredicate,
        max_steps: i32,
    },
    Fixed(Vec<BlockPos>),
    NoiseBasedCount {
        ratio: i32,
        factor: f64,
        offset: f64,
    },
    NoiseThresholdCount {
        level: f64,
        below: i32,
        above: i32,
    },
    CountOnEveryLayer(IntProvider),
    RandomlySelected(Vec<Placement>),
    Cuboid {
        xz: IntProvider,
        y: IntProvider,
        edges: bool,
        interior: bool,
    },
}

impl Placement {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let int = |name: &str| IntProvider::parse(field(json, name)?);
        Ok::<Self, anyhow::Error>(match type_name(json)?.as_str() {
            "count" => Placement::Count(int("count")?),
            "in_square" => Placement::InSquare,
            "heightmap" => Placement::Heightmap(HeightmapKind::from_key(str_field(json, "heightmap")?)?),
            "biome" => Placement::Biome,
            "block_predicate_filter" => {
                Placement::BlockPredicateFilter(BlockPredicate::parse(p, field(json, "predicate")?)?)
            }
            "rarity_filter" => Placement::Rarity(i32_field(json, "chance")?),
            "random_chance" => Placement::RandomChance(f32_field(json, "chance")?),
            "surface_water_depth_filter" => Placement::SurfaceWaterDepth(i32_field(json, "max_water_depth")?),
            "surface_relative_threshold_filter" => Placement::SurfaceRelativeThreshold {
                kind: HeightmapKind::from_key(str_field(json, "heightmap")?)?,
                min: json
                    .get("min_inclusive")
                    .map(crate::codec::as_i32)
                    .transpose()?
                    .unwrap_or(i32::MIN),
                max: json
                    .get("max_inclusive")
                    .map(crate::codec::as_i32)
                    .transpose()?
                    .unwrap_or(i32::MAX),
            },
            "height_range" => Placement::HeightRange(HeightProvider::parse(field(json, "height")?)?),
            "offset" => {
                let zero = || IntProvider::Constant(0);
                let get = |n: &str| {
                    json.get(n)
                        .map(IntProvider::parse)
                        .transpose()
                        .map(|v| v.unwrap_or_else(zero))
                };
                Placement::Offset(get("x")?, get("y")?, get("z")?)
            }
            "environment_scan" => Placement::EnvironmentScan {
                direction: Direction::parse(str_field(json, "direction_of_search")?)?,
                target: BlockPredicate::parse(p, field(json, "target_condition")?)?,
                allowed: match json.get("allowed_search_condition") {
                    Some(v) => BlockPredicate::parse(p, v)?,
                    None => BlockPredicate::True,
                },
                max_steps: i32_field(json, "max_steps")?,
            },
            "fixed_placement" => {
                let mut list = Vec::new();
                for v in as_array(field(json, "positions")?)? {
                    let a = as_array(v)?;
                    list.push(BlockPos::new(
                        crate::codec::as_i32(&a[0])?,
                        crate::codec::as_i32(&a[1])?,
                        crate::codec::as_i32(&a[2])?,
                    ));
                }
                Placement::Fixed(list)
            }
            "noise_based_count" => Placement::NoiseBasedCount {
                ratio: i32_field(json, "noise_to_count_ratio")?,
                factor: f64_field(json, "noise_factor")?,
                offset: json
                    .get("noise_offset")
                    .map(crate::codec::as_f64)
                    .transpose()?
                    .unwrap_or(0.0),
            },
            "noise_threshold_count" => Placement::NoiseThresholdCount {
                level: f64_field(json, "noise_level")?,
                below: i32_field(json, "below_noise")?,
                above: i32_field(json, "above_noise")?,
            },
            "cuboid" => Placement::Cuboid {
                xz: int("xz_size")?,
                y: int("y_size")?,
                edges: crate::codec::opt_bool(json, "include_edges", true)?,
                interior: crate::codec::opt_bool(json, "include_interior", true)?,
            },
            "count_on_every_layer" => Placement::CountOnEveryLayer(int("count")?),
            "randomly_selected" => Placement::RandomlySelected(
                as_array(field(json, "placements")?)?
                    .iter()
                    .map(|v| Placement::parse(p, v))
                    .collect::<Result<_>>()?,
            ),
            other => bail!("unknown placement modifier {other}"),
        })
        .with_context(|| format!("placement {json}"))
    }

    /// `modify(context, random, origin, output)`.
    fn modify(
        &self,
        region: &mut Region,
        top: Option<&PlacedFeature>,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        out: &mut Vec<BlockPos>,
    ) {
        match self {
            Placement::Count(c) => {
                let n = c.sample(random);
                for _ in 0..n {
                    out.push(origin);
                }
            }
            Placement::InSquare => {
                let x = random.next_int_bounded(16) + origin.x;
                let z = random.next_int_bounded(16) + origin.z;
                out.push(BlockPos::new(x, origin.y, z));
            }
            Placement::Heightmap(kind) => {
                let h = region.height(*kind, origin.x, origin.z);
                if h > region.min_y() {
                    out.push(BlockPos::new(origin.x, h, origin.z));
                }
            }
            Placement::Biome => {
                let top = top.expect("biome filter on an unregistered feature");
                let biome = region.biome(origin);
                if region.env.features.biome_has_feature(biome, top) {
                    out.push(origin);
                }
            }
            Placement::BlockPredicateFilter(pred) => {
                if pred.test(region, origin) {
                    out.push(origin);
                }
            }
            Placement::Rarity(chance) => {
                if random.next_float() < 1.0 / *chance as f32 {
                    out.push(origin);
                }
            }
            Placement::RandomChance(chance) => {
                if random.next_float() < *chance {
                    out.push(origin);
                }
            }
            Placement::SurfaceWaterDepth(max) => {
                let floor = region.height(HeightmapKind::OceanFloor, origin.x, origin.z);
                let surface = region.height(HeightmapKind::WorldSurface, origin.x, origin.z);
                if surface - floor <= *max {
                    out.push(origin);
                }
            }
            Placement::SurfaceRelativeThreshold { kind, min, max } => {
                let surface = region.height(*kind, origin.x, origin.z) as i64;
                let lo = surface + *min as i64;
                let hi = surface + *max as i64;
                if lo <= origin.y as i64 && origin.y as i64 <= hi {
                    out.push(origin);
                }
            }
            Placement::HeightRange(h) => {
                let y = h.sample(random, &region.env.gen_ctx);
                out.push(BlockPos::new(origin.x, y, origin.z));
            }
            Placement::Offset(x, y, z) => {
                let dx = x.sample(random);
                let dy = y.sample(random);
                let dz = z.sample(random);
                out.push(origin.offset(dx, dy, dz));
            }
            Placement::EnvironmentScan {
                direction,
                target,
                allowed,
                max_steps,
            } => {
                let mut pos = origin;
                if !allowed.test(region, pos) {
                    return;
                }
                for _ in 0..*max_steps {
                    if target.test(region, pos) {
                        out.push(pos);
                        return;
                    }
                    pos = pos.relative(*direction, 1);
                    if region.is_outside_build_height(pos.y) {
                        return;
                    }
                    if !allowed.test(region, pos) {
                        break;
                    }
                }
                if target.test(region, pos) {
                    out.push(pos);
                }
            }
            Placement::Fixed(list) => {
                let (cx, cz) = (origin.x >> 4, origin.z >> 4);
                for p in list {
                    if p.x >> 4 == cx && p.z >> 4 == cz {
                        out.push(*p);
                    }
                }
            }
            Placement::NoiseBasedCount {
                ratio,
                factor,
                offset,
            } => {
                let noise = crate::biome::data::noises()
                    .biome_info
                    .get2(origin.x as f64 / factor, origin.z as f64 / factor)
                    as f64;
                let n = ((noise + offset) * *ratio as f64).ceil() as i32;
                for _ in 0..n {
                    out.push(origin);
                }
            }
            Placement::NoiseThresholdCount { level, below, above } => {
                let noise = crate::biome::data::noises()
                    .biome_info
                    .get2(origin.x as f64 / 200.0, origin.z as f64 / 200.0)
                    as f64;
                let n = if noise < *level { *below } else { *above };
                for _ in 0..n {
                    out.push(origin);
                }
            }
            Placement::CountOnEveryLayer(count) => {
                let mut layer = 0;
                loop {
                    let mut found = false;
                    let mut i = 0;
                    while i < count.sample(random) {
                        let x = random.next_int_bounded(16) + origin.x;
                        let z = random.next_int_bounded(16) + origin.z;
                        let start = region.height(HeightmapKind::MotionBlocking, x, z);
                        if let Some(y) = find_on_ground_y(region, x, start, z, layer) {
                            out.push(BlockPos::new(x, y, z));
                            found = true;
                        }
                        i += 1;
                    }
                    layer += 1;
                    if !found {
                        break;
                    }
                }
            }
            Placement::Cuboid {
                xz,
                y,
                edges,
                interior,
            } => {
                let height = y.sample(random);
                let width = xz.sample(random);
                let length = xz.sample(random);
                let (e, i) = (*edges, *interior);
                for x in 0..=width {
                    for y in 0..=height {
                        for z in 0..=length {
                            if (e || (x != 0 && x != width) || (y != 0 && y != height))
                                && (e || (z != 0 && z != length) || (y != 0 && y != height))
                                && (e || (x != 0 && x != width) || (z != 0 && z != length))
                                && (i
                                    || x == 0
                                    || x == width
                                    || y == 0
                                    || y == height
                                    || z == 0
                                    || z == length)
                            {
                                out.push(origin.offset(x, y, z));
                            }
                        }
                    }
                }
            }
            Placement::RandomlySelected(list) => {
                let i = random.next_int_bounded(list.len() as i32) as usize;
                list[i].modify(region, top, random, origin, out);
            }
        }
    }
}

/// `CountOnEveryLayerPlacement.findOnGroundYPosition`.
fn find_on_ground_y(region: &Region, x: i32, y_start: i32, z: i32, layer: i32) -> Option<i32> {
    let b = &region.env.b;
    let empty = |id: BlockId, air: bool| air || id == b.water || id == b.lava;
    let mut current_layer = 0;
    let mut current = region.get(BlockPos::new(x, y_start, z));
    let mut y = y_start;
    while y >= region.min_y() + 1 {
        let below_pos = BlockPos::new(x, y - 1, z);
        let below = region.get(below_pos);
        let blocks = &region.env.blocks;
        let below_empty = empty(blocks.block_of(below), blocks.flags(below).is_air());
        let current_empty = empty(blocks.block_of(current), blocks.flags(current).is_air());
        if !below_empty && current_empty && blocks.block_of(below) != b.bedrock {
            if current_layer == layer {
                return Some(below_pos.y + 1);
            }
            current_layer += 1;
        }
        current = below;
        y -= 1;
    }
    None
}

/// `FeaturePlacer.place`: applies the modifiers depth first and places the feature at every
/// resulting position.
pub fn place(
    region: &mut Region,
    placed: &PlacedFeature,
    top: Option<&PlacedFeature>,
    random: &mut dyn RandomSource,
    origin: BlockPos,
) -> bool {
    let modifiers = &placed.placement;
    if modifiers.is_empty() {
        return placed.feature.place(region, random, origin);
    }
    let mut placed_any = false;
    let mut positions = vec![origin];
    let mut indices = vec![0usize];
    let mut out = Vec::new();
    while let Some(pos) = positions.pop() {
        let index = indices.pop().unwrap();
        out.clear();
        modifiers[index].modify(region, top, random, pos, &mut out);
        let next = index + 1;
        if next < modifiers.len() {
            for &p in out.iter().rev() {
                positions.push(p);
                indices.push(next);
            }
        } else {
            for &p in out.iter() {
                placed_any |= placed.feature.place(region, random, p);
            }
        }
    }
    placed_any
}

/// `FeaturePlacer.placeWithBiomeCheck`.
pub fn place_with_biome_check(
    region: &mut Region,
    placed: &PlacedFeature,
    random: &mut dyn RandomSource,
    origin: BlockPos,
) -> bool {
    place(region, placed, Some(placed), random, origin)
}
