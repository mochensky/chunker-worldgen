//! `StructureTemplate.placeInWorld` with `StructurePlaceSettings`, on a feature-step region.

use std::sync::Arc;

use chunker_data::blocks::{BlockState, Fluid};
use chunker_nbt::{Compound, List, Tag};

use super::geom::{BlockPos, BoundingBox, Direction, Mirror, Rotation};
use super::processor::Processor;
use super::template::{BlockInfo, Template, transform};
use crate::feature::Region;
use crate::feature::behaviour;
use crate::feature::tree::{Shape, update_shape_at_edge_with};
use crate::math;
use crate::random::{LegacyRandom, RandomSource};

/// `StructurePlaceSettings`.
#[derive(Clone)]
pub struct PlaceSettings {
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub pivot: BlockPos,
    pub ignore_entities: bool,
    pub bounding_box: Option<BoundingBox>,
    /// `LiquidSettings.APPLY_WATERLOGGING` (default) vs `IGNORE_WATERLOGGING`.
    pub waterlogging: bool,
    /// `setRandom(random)` with the random later passed to `placeInWorld` (features); otherwise
    /// processors and the palette use positional randoms.
    pub use_random: bool,
    pub processors: Vec<Arc<Processor>>,
    pub known_shape: bool,
    pub finalize_entities: bool,
}

impl Default for PlaceSettings {
    fn default() -> Self {
        Self {
            mirror: Mirror::None,
            rotation: Rotation::None,
            pivot: BlockPos::ZERO,
            ignore_entities: false,
            bounding_box: None,
            waterlogging: true,
            use_random: false,
            processors: Vec::new(),
            known_shape: false,
            finalize_entities: false,
        }
    }
}

/// `Block.UPDATE_KNOWN_SHAPE`.
const KNOWN_SHAPE: u32 = 16;

/// `StructureTemplate.processBlockInfos`.
pub fn process_block_infos(
    region: &mut Region,
    position: BlockPos,
    reference: BlockPos,
    settings: &PlaceSettings,
    infos: &[BlockInfo],
    mut random: Option<&mut dyn RandomSource>,
) -> Vec<BlockInfo> {
    let only_in_chunk = !settings.processors.iter().any(|p| p.evaluates_entire_piece());
    let mut original = Vec::new();
    let mut processed = Vec::new();
    for info in infos {
        let pos = transform(info.pos, settings.mirror, settings.rotation, settings.pivot).add(position);
        if only_in_chunk
            && settings
                .bounding_box
                .is_some_and(|b| !b.is_inside(pos.x, pos.y, pos.z))
        {
            continue;
        }
        let mut current = Some(BlockInfo {
            pos,
            state: info.state,
            nbt: info.nbt.clone(),
        });
        for p in &settings.processors {
            let Some(c) = current else { break };
            current = p.process_block(region, position, reference, info.pos, c, &mut random);
        }
        if let Some(c) = current {
            processed.push(c);
            original.push(info.clone());
        }
    }
    for p in &settings.processors {
        processed = p.finalize(region, position, reference, &original, processed, &mut random);
    }
    processed
}

/// `StructureTemplate.placeInWorld(level, position, referencePos, settings, random, updateMode)`.
pub fn place_in_world(
    region: &mut Region,
    template: &Template,
    position: BlockPos,
    reference: BlockPos,
    settings: &PlaceSettings,
    random: &mut dyn RandomSource,
    update_mode: u32,
) -> bool {
    if template.palettes.is_empty() {
        return false;
    }
    // `settings.getRandomPalette(palettes, position)`.
    let n = template.palettes.len() as i32;
    let palette = if settings.use_random {
        random.next_int_bounded(n)
    } else {
        LegacyRandom::new(math::get_seed(position.x, position.y, position.z)).next_int_bounded(n)
    };
    let infos = &template.palettes[palette as usize].blocks;
    let (sx, sy, sz) = template.size;
    if (infos.is_empty() && (settings.ignore_entities || template.entities.is_empty()))
        || sx < 1
        || sy < 1
        || sz < 1
    {
        return false;
    }
    let env = region.env;
    let blocks = &env.blocks;
    let bb = settings.bounding_box;
    let mut to_fill: Vec<BlockPos> = Vec::new();
    let mut locked: Vec<BlockPos> = Vec::new();
    let mut placed: Vec<(BlockPos, bool)> = Vec::new();
    let (mut min, mut max) = ((i32::MAX, i32::MAX, i32::MAX), (i32::MIN, i32::MIN, i32::MIN));
    let processed = {
        let r: Option<&mut dyn RandomSource> = if settings.use_random {
            Some(&mut *random)
        } else {
            None
        };
        process_block_infos(region, position, reference, settings, infos, r)
    };
    let barrier = blocks.block(env.b.barrier).default_state;
    for info in processed {
        let pos = info.pos;
        if bb.is_some_and(|b| !b.is_inside(pos.x, pos.y, pos.z)) {
            continue;
        }
        let previous_fluid = settings.waterlogging.then(|| region.fluid(pos));
        let state = blocks.rotate(
            blocks.mirror(info.state, settings.mirror as u8),
            settings.rotation as u8,
        );
        if info.nbt.is_some() {
            region.set(pos, barrier, 820);
        }
        if !region.set(pos, state, update_mode) {
            continue;
        }
        min = (min.0.min(pos.x), min.1.min(pos.y), min.2.min(pos.z));
        max = (max.0.max(pos.x), max.1.max(pos.y), max.2.max(pos.z));
        placed.push((pos, info.nbt.is_some()));
        if let Some(mut nbt) = info.nbt
            && let Some(be) = region.block_entity(pos)
        {
            if behaviour::is_randomizable_container(be) {
                nbt.insert("LootTableSeed", random.next_long());
            }
            behaviour::load_block_entity(env, state, be, &nbt);
        }
        if let Some(prev) = previous_fluid {
            let flags = blocks.flags(state);
            if flags.is_fluid_source() {
                locked.push(pos);
            } else if flags.is_liquid_container() {
                place_liquid(region, pos, state, prev);
                if !matches!(prev, Fluid::Water | Fluid::Lava) {
                    to_fill.push(pos);
                }
            }
        }
    }
    let dirs = [
        Direction::Up,
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
    ];
    let mut filled = true;
    while filled && !to_fill.is_empty() {
        filled = false;
        let mut i = 0;
        while i < to_fill.len() {
            let pos = to_fill[i];
            let mut fluid = region.fluid(pos);
            for d in dirs {
                if is_source(fluid) {
                    break;
                }
                let n = pos.relative(d, 1);
                let nf = region.fluid(n);
                if is_source(nf) && !locked.contains(&n) {
                    fluid = nf;
                }
            }
            if is_source(fluid) {
                let state = region.get(pos);
                if blocks.flags(state).is_liquid_container() {
                    place_liquid(region, pos, state, fluid);
                    filled = true;
                    to_fill.remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }
    if min.0 <= max.0 && !settings.known_shape {
        let mut shape = Shape::new(max.0 - min.0 + 1, max.1 - min.1 + 1, max.2 - min.2 + 1);
        for (p, _) in &placed {
            shape.fill(p.x - min.0, p.y - min.1, p.z - min.2);
        }
        update_shape_at_edge_with(region, &shape, min.0, min.1, min.2, update_mode & !1);
        for (p, _) in &placed {
            let state = region.get(*p);
            let new_state = update_from_neighbour_shapes(region, state, *p);
            if state != new_state {
                region.set(*p, new_state, (update_mode & !1) | KNOWN_SHAPE);
            }
        }
    }
    if !settings.ignore_entities {
        place_entities(region, template, position, settings, bb);
    }
    true
}

/// `StructureTemplate.filterBlocks(pos, settings, block)` (absolute positions inside the settings box,
/// positional palette).
pub fn filter_blocks(
    template: &Template,
    position: BlockPos,
    settings: &PlaceSettings,
    block: chunker_data::blocks::BlockId,
    blocks: &chunker_data::blocks::BlockRegistry,
) -> Vec<BlockInfo> {
    let Some(palette) = template.palette_at(position) else {
        return Vec::new();
    };
    palette
        .blocks
        .iter()
        .filter(|b| blocks.block_of(b.state) == block)
        .map(|b| BlockInfo {
            pos: transform(b.pos, settings.mirror, settings.rotation, settings.pivot).add(position),
            state: b.state,
            nbt: b.nbt.clone(),
        })
        .filter(|b| {
            settings
                .bounding_box
                .is_none_or(|bb| bb.is_inside(b.pos.x, b.pos.y, b.pos.z))
        })
        .collect()
}

fn is_source(f: Fluid) -> bool {
    matches!(f, Fluid::Water | Fluid::Lava)
}

/// `LiquidBlockContainer.placeLiquid` (only `SimpleWaterloggedBlock` accepts liquid in worldgen).
fn place_liquid(region: &mut Region, pos: BlockPos, state: BlockState, fluid: Fluid) -> bool {
    let blocks = &region.env.blocks;
    if !blocks.flags(state).is_simple_waterlogged() || fluid != Fluid::Water {
        return false;
    }
    // `SlabBlock.placeLiquid`: double slabs stay dry.
    if blocks.get_property(state, "waterlogged") != Some("false")
        || blocks.get_property(state, "type") == Some("double")
    {
        return false;
    }
    let s = blocks.with_property(state, "waterlogged", "true").unwrap();
    region.set(pos, s, 3);
    region.schedule_fluid_tick(pos, Fluid::Water);
    true
}

/// `Block.updateFromNeighbourShapes`.
pub fn update_from_neighbour_shapes(region: &mut Region, state: BlockState, pos: BlockPos) -> BlockState {
    let mut s = state;
    for d in [
        Direction::West,
        Direction::East,
        Direction::North,
        Direction::South,
        Direction::Down,
        Direction::Up,
    ] {
        let n = pos.relative(d, 1);
        let ns = region.get(n);
        s = behaviour::update_shape(region, s, pos, d, n, ns);
    }
    s
}

/// `StructureTemplate.transform(Vec3, ...)`.
fn transform_vec(p: (f64, f64, f64), mirror: Mirror, rotation: Rotation, pivot: BlockPos) -> (f64, f64, f64) {
    let (mut x, y, mut z) = p;
    match mirror {
        Mirror::LeftRight => z = 1.0 - z,
        Mirror::FrontBack => x = 1.0 - x,
        Mirror::None => {}
    }
    let (px, pz) = (pivot.x as f64, pivot.z as f64);
    match rotation {
        Rotation::CounterClockwise90 => (px - pz + z, y, px + pz + 1.0 - x),
        Rotation::Clockwise90 => (px + pz + 1.0 - z, y, pz - px + x),
        Rotation::Clockwise180 => (px + px + 1.0 - x, y, pz + pz + 1.0 - z),
        Rotation::None => (x, y, z),
    }
}

/// `StructureTemplate.placeEntities`. Entities are saved as the template NBT with the new position
/// and rotation (no UUID: the server assigns one on load); mobs are finalized if the settings say so.
fn place_entities(
    region: &mut Region,
    template: &Template,
    position: BlockPos,
    settings: &PlaceSettings,
    bb: Option<BoundingBox>,
) {
    for e in &template.entities {
        let block_pos =
            transform(e.block_pos, settings.mirror, settings.rotation, settings.pivot).add(position);
        if bb.is_some_and(|b| !b.is_inside(block_pos.x, block_pos.y, block_pos.z)) {
            continue;
        }
        let mut tag = e.nbt.clone();
        let rel = transform_vec(e.pos, settings.mirror, settings.rotation, settings.pivot);
        let pos = (
            rel.0 + position.x as f64,
            rel.1 + position.y as f64,
            rel.2 + position.z as f64,
        );
        let mut list = List::new();
        list.push(pos.0);
        list.push(pos.1);
        list.push(pos.2);
        tag.insert("Pos", list);
        tag.remove("UUID");
        if tag.contains_key("block_pos") {
            tag.insert("block_pos", vec![block_pos.x, block_pos.y, block_pos.z]);
        }
        rotate_entity(&mut tag, settings.rotation, settings.mirror);
        let id = tag.get_str("id").unwrap_or("").to_owned();
        if settings.finalize_entities && region.env.mobs().is_mob(&id) {
            crate::entity::spawn_mob(region, tag);
        } else {
            region.add_entity(math::floor(pos.0), math::floor(pos.2), tag);
        }
    }
}

/// `Entity.rotate` + `Entity.mirror` (hanging entities turn their facing instead).
fn rotate_entity(tag: &mut Compound, rotation: Rotation, mirror: Mirror) {
    let yaw = match tag.get_list("Rotation").and_then(|l| l.get(0)) {
        Some(Tag::Float(f)) => *f,
        _ => 0.0,
    };
    let pitch = match tag.get_list("Rotation").and_then(|l| l.get(1)) {
        Some(Tag::Float(f)) => *f,
        _ => 0.0,
    };
    let facing_key = if tag.contains_key("Facing") {
        Some("Facing")
    } else if tag.contains_key("facing") {
        Some("facing")
    } else {
        None
    };
    let new_yaw = if let Some(key) = facing_key {
        let two_d = key == "facing";
        let value = match tag.get(key) {
            Some(Tag::Byte(b)) => *b as i32,
            Some(Tag::Int(i)) => *i,
            _ => 0,
        };
        let dir = if two_d {
            Direction::from_2d(value)
        } else {
            Direction::from_3d(value)
        };
        let mut d = dir;
        let turn = |d: Direction, rotation: Rotation| if d.is_horizontal() { rotation.rotate(d) } else { d };
        d = turn(d, rotation);
        let m = match mirror {
            Mirror::LeftRight if d.axis() == super::geom::Axis::Z => Rotation::Clockwise180,
            Mirror::FrontBack if d.axis() == super::geom::Axis::X => Rotation::Clockwise180,
            _ => Rotation::None,
        };
        d = turn(d, m);
        let v = if two_d { d.data_2d() } else { d.data_3d() };
        tag.insert(key, v as i8);
        if d.is_horizontal() {
            (d.data_2d() * 90) as f32
        } else {
            yaw
        }
    } else {
        let angle = math::wrap_degrees_f32(yaw);
        let rotated = match rotation {
            Rotation::Clockwise180 => angle + 180.0,
            Rotation::CounterClockwise90 => angle + 270.0,
            Rotation::Clockwise90 => angle + 90.0,
            Rotation::None => angle,
        };
        let mirrored = match mirror {
            Mirror::FrontBack => -angle,
            Mirror::LeftRight => 180.0 - angle,
            Mirror::None => angle,
        };
        rotated + mirrored - yaw
    };
    let mut list = List::new();
    list.push(new_yaw);
    list.push(pitch);
    tag.insert("Rotation", list);
}
