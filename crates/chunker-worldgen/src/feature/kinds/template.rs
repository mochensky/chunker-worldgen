//! Features that place structure templates: `TemplateFeature` (desert wells, sulfur springs) and
//! `FossilFeature`.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use serde_json::Value;

use crate::chunk::HeightmapKind;
use crate::codec::{as_array, field, i32_field};
use crate::feature::Region;
use crate::feature::registry::{Parser, id};
use crate::providers::WeightedList;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, BoundingBox, Direction, Mirror, Rotation};
use crate::structure::place::{PlaceSettings, place_in_world};
use crate::structure::processor::{self, Processor};
use crate::structure::template::zero_position_with_transform;

fn parse_rotation(s: &str) -> Result<Rotation> {
    Ok(match s {
        "none" => Rotation::None,
        "clockwise_90" => Rotation::Clockwise90,
        "180" => Rotation::Clockwise180,
        "counterclockwise_90" => Rotation::CounterClockwise90,
        other => bail!("unknown rotation {other}"),
    })
}

/// `TemplateFeature.TemplateEntry`.
pub struct TemplateEntry {
    template: Identifier,
    rotations: Vec<Rotation>,
}

/// `TemplateFeature`.
pub struct TemplateFeature {
    templates: WeightedList<TemplateEntry>,
    processors: Vec<Arc<Processor>>,
}

impl TemplateFeature {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let templates = WeightedList::parse(field(json, "templates")?, |v| {
            let rotations = match v.get("rotations") {
                Some(r) => as_array(r)?
                    .iter()
                    .map(|x| parse_rotation(x.as_str().unwrap_or("")))
                    .collect::<Result<_>>()?,
                None => Rotation::ALL.to_vec(),
            };
            Ok(TemplateEntry {
                template: id(crate::codec::str_field(v, "id")?)?,
                rotations,
            })
        })?;
        let processors = match json.get("processors") {
            Some(v) => processor::parse_list(p, v)?,
            None => Vec::new(),
        };
        Ok(Self {
            templates,
            processors,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let Some(entry) = self.templates.get_random(random) else {
            return false;
        };
        let rotation = entry.rotations[random.next_int_bounded(entry.rotations.len() as i32) as usize];
        let template = region.env.templates.get(&entry.template);
        // `getRotatedOffset`: the rotated negative axis direction times half the size.
        let offset = |axis_dir: Direction, half: i32| {
            let d = rotation.rotate(axis_dir);
            (d.step_x() * half, d.step_z() * half)
        };
        let (ax, az) = offset(Direction::West, template.size.0 / 2);
        let (bx, bz) = offset(Direction::North, template.size.2 / 2);
        let pos = origin.offset(ax + bx, 0, az + bz);
        let settings = PlaceSettings {
            rotation,
            use_random: true,
            processors: self.processors.clone(),
            ..Default::default()
        };
        place_in_world(region, template, pos, pos, &settings, random, 3)
    }
}

/// `FossilFeature`.
pub struct Fossil {
    fossils: Vec<Identifier>,
    overlays: Vec<Identifier>,
    fossil_processors: Vec<Arc<Processor>>,
    overlay_processors: Vec<Arc<Processor>>,
    max_empty_corners: i32,
}

impl Fossil {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let ids = |name: &str| -> Result<Vec<Identifier>> {
            as_array(field(json, name)?)?
                .iter()
                .map(|v| id(v.as_str().context("template id")?))
                .collect()
        };
        let fossils = ids("fossil_structures")?;
        let overlays = ids("overlay_structures")?;
        if fossils.len() != overlays.len() {
            bail!("Fossil structure lists must be equal lengths");
        }
        Ok(Self {
            fossils,
            overlays,
            fossil_processors: processor::parse_list(p, field(json, "fossil_processors")?)?,
            overlay_processors: processor::parse_list(p, field(json, "overlay_processors")?)?,
            max_empty_corners: i32_field(json, "max_empty_corners_allowed")?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let rotation = Rotation::random(random);
        let index = random.next_int_bounded(self.fossils.len() as i32) as usize;
        let env = region.env;
        let base = env.templates.get(&self.fossils[index]);
        let overlay = env.templates.get(&self.overlays[index]);
        let (cx, cz) = (origin.x >> 4 << 4, origin.z >> 4 << 4);
        let bb = BoundingBox::new(
            cx - 16,
            region.min_y(),
            cz - 16,
            cx + 15 + 16,
            region.max_y(),
            cz + 15 + 16,
        );
        let mut settings = PlaceSettings {
            rotation,
            bounding_box: Some(bb),
            use_random: true,
            ..Default::default()
        };
        let size = base.size_rotated(rotation);
        let low = origin.offset(-size.0 / 2, 0, -size.2 / 2);
        let mut lowest = origin.y;
        for x in 0..size.0 {
            for z in 0..size.2 {
                lowest = lowest.min(region.height(HeightmapKind::OceanFloorWg, low.x + x, low.z + z));
            }
        }
        let target_y = (lowest - 15 - random.next_int_bounded(10)).max(region.min_y() + 10);
        let target = zero_position_with_transform(
            BlockPos::new(low.x, target_y, low.z),
            Mirror::None,
            rotation,
            base.size.0,
            base.size.2,
        );
        let bounds = base.bounding_box(target, rotation, settings.pivot, settings.mirror);
        if count_empty_corners(region, &bounds) > self.max_empty_corners {
            return false;
        }
        settings.processors = self.fossil_processors.clone();
        place_in_world(region, base, target, target, &settings, random, 260);
        settings.processors = self.overlay_processors.clone();
        place_in_world(region, overlay, target, target, &settings, random, 260);
        true
    }
}

/// `FossilFeature.countEmptyCorners`.
fn count_empty_corners(region: &Region, b: &BoundingBox) -> i32 {
    let env = region.env;
    let mut count = 0;
    for x in [b.min_x, b.max_x] {
        for y in [b.min_y, b.max_y] {
            for z in [b.min_z, b.max_z] {
                let p = BlockPos::new(x, y, z);
                let block = region.block_of(p);
                if region.flags(p).is_air() || block == env.b.lava || block == env.b.water {
                    count += 1;
                }
            }
        }
    }
    count
}
