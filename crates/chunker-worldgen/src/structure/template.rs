//! `StructureTemplate` (`data/<ns>/structure/<path>.nbt`) and `StructureTemplateManager`.

use std::sync::Arc;

use anyhow::{Context, Result};
use chunker_data::blocks::{BlockRegistry, BlockState};
use chunker_data::{DataPacks, Identifier};
use chunker_nbt::{Compound, Tag};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;

use super::geom::{BlockPos, BoundingBox, Direction, Mirror, Rotation};

/// `StructureTemplate.StructureBlockInfo`.
#[derive(Clone, Debug)]
pub struct BlockInfo {
    pub pos: BlockPos,
    pub state: BlockState,
    pub nbt: Option<Compound>,
}

/// `StructureTemplate.JigsawBlockInfo` (in template coordinates).
#[derive(Clone, Debug)]
pub struct JigsawInfo {
    pub pos: BlockPos,
    pub front: Direction,
    pub top: Direction,
    pub rollable: bool,
    /// `name` (`None` only for the synthetic jigsaw of feature pool elements).
    pub name: Option<Identifier>,
    pub pool: Identifier,
    pub target: Identifier,
    pub placement_priority: i32,
    pub selection_priority: i32,
}

impl JigsawInfo {
    /// `JigsawBlock.canAttach(this, target)`.
    pub fn can_attach(&self, target: &JigsawInfo) -> bool {
        self.front == target.front.opposite()
            && (self.rollable || self.top == target.top)
            && target.name.as_ref().is_none_or(|n| &self.target == n)
    }

    /// The jigsaw transformed like `getJigsaws(position, rotation)`.
    pub fn transformed(&self, position: BlockPos, rotation: Rotation) -> JigsawInfo {
        let mut j = self.clone();
        j.pos = transform(self.pos, Mirror::None, rotation, BlockPos::ZERO).add(position);
        j.front = rotation.rotate(self.front);
        j.top = rotation.rotate(self.top);
        j
    }
}

/// `StructureTemplate.Palette`.
pub struct Palette {
    /// Load order: full blocks, other blocks, block entities (each sorted by y, x, z).
    pub blocks: Vec<BlockInfo>,
    pub jigsaws: Vec<JigsawInfo>,
}

/// `StructureTemplate.StructureEntityInfo`.
#[derive(Clone, Debug)]
pub struct EntityInfo {
    pub pos: (f64, f64, f64),
    pub block_pos: BlockPos,
    pub nbt: Compound,
}

pub struct Template {
    pub size: (i32, i32, i32),
    pub palettes: Vec<Palette>,
    pub entities: Vec<EntityInfo>,
}

/// `StructureTemplate.transform(pos, mirror, rotation, pivot)`.
pub fn transform(pos: BlockPos, mirror: Mirror, rotation: Rotation, pivot: BlockPos) -> BlockPos {
    let (mut x, y, mut z) = (pos.x, pos.y, pos.z);
    let mirrored = match mirror {
        Mirror::LeftRight => {
            z = -z;
            true
        }
        Mirror::FrontBack => {
            x = -x;
            true
        }
        Mirror::None => false,
    };
    let (px, pz) = (pivot.x, pivot.z);
    match rotation {
        Rotation::CounterClockwise90 => BlockPos::new(px - pz + z, y, px + pz - x),
        Rotation::Clockwise90 => BlockPos::new(px + pz - z, y, pz - px + x),
        Rotation::Clockwise180 => BlockPos::new(px + px - x, y, pz + pz - z),
        Rotation::None => {
            if mirrored {
                BlockPos::new(x, y, z)
            } else {
                pos
            }
        }
    }
}

/// `StructureTemplate.getZeroPositionWithTransform(zeroPos, mirror, rotation, sizeX, sizeZ)`.
pub fn zero_position_with_transform(
    zero: BlockPos,
    mirror: Mirror,
    rotation: Rotation,
    size_x: i32,
    size_z: i32,
) -> BlockPos {
    let (sx, sz) = (size_x - 1, size_z - 1);
    let mdx = if mirror == Mirror::FrontBack { sx } else { 0 };
    let mdz = if mirror == Mirror::LeftRight { sz } else { 0 };
    match rotation {
        Rotation::CounterClockwise90 => zero.offset(mdz, 0, sx - mdx),
        Rotation::Clockwise90 => zero.offset(sz - mdz, 0, mdx),
        Rotation::Clockwise180 => zero.offset(sx - mdx, 0, sz - mdz),
        Rotation::None => zero.offset(mdx, 0, mdz),
    }
}

impl Template {
    /// `getSize(rotation)`.
    pub fn size_rotated(&self, rotation: Rotation) -> (i32, i32, i32) {
        match rotation {
            Rotation::Clockwise90 | Rotation::CounterClockwise90 => (self.size.2, self.size.1, self.size.0),
            _ => self.size,
        }
    }

    /// `getBoundingBox(position, rotation, pivot, mirror)`.
    pub fn bounding_box(
        &self,
        position: BlockPos,
        rotation: Rotation,
        pivot: BlockPos,
        mirror: Mirror,
    ) -> BoundingBox {
        let delta = BlockPos::new(self.size.0 - 1, self.size.1 - 1, self.size.2 - 1);
        let c1 = transform(BlockPos::ZERO, mirror, rotation, pivot);
        let c2 = transform(delta, mirror, rotation, pivot);
        BoundingBox::from_corners(c1, c2).moved(position.x, position.y, position.z)
    }

    /// `StructurePlaceSettings.getRandomPalette(palettes, pos)` with no explicit random: a positional
    /// legacy random picks among several palettes.
    pub fn palette_at(&self, pos: BlockPos) -> Option<&Palette> {
        match self.palettes.len() {
            0 => None,
            1 => Some(&self.palettes[0]),
            n => {
                use crate::random::RandomSource;
                let mut r = crate::random::LegacyRandom::new(crate::math::get_seed(pos.x, pos.y, pos.z));
                Some(&self.palettes[r.next_int_bounded(n as i32) as usize])
            }
        }
    }

    /// `getJigsaws(position, rotation)` (unshuffled).
    pub fn jigsaws(&self, position: BlockPos, rotation: Rotation) -> Vec<JigsawInfo> {
        match self.palette_at(position) {
            Some(p) => p
                .jigsaws
                .iter()
                .map(|j| j.transformed(position, rotation))
                .collect(),
            None => Vec::new(),
        }
    }

    fn parse(blocks: &BlockRegistry, tag: &Compound) -> Result<Template> {
        let ints = |t: Option<&chunker_nbt::List>| -> Vec<i32> {
            t.map(|l| l.iter().map(|v| v.as_i64().unwrap_or(0) as i32).collect())
                .unwrap_or_default()
        };
        let s = ints(tag.get_list("size"));
        let size = (
            s.first().copied().unwrap_or(0),
            s.get(1).copied().unwrap_or(0),
            s.get(2).copied().unwrap_or(0),
        );
        let block_list = tag.get_list("blocks");
        let mut palettes = Vec::new();
        let mut load_palette = |palette: &chunker_nbt::List| -> Result<()> {
            let states: Vec<BlockState> = palette.iter().map(|e| state_from_nbt(blocks, e)).collect();
            let (mut full, mut other, mut entities) = (Vec::new(), Vec::new(), Vec::new());
            for b in block_list.into_iter().flat_map(|l| l.compounds()) {
                let p = ints(b.get_list("pos"));
                let pos = BlockPos::new(
                    p.first().copied().unwrap_or(0),
                    p.get(1).copied().unwrap_or(0),
                    p.get(2).copied().unwrap_or(0),
                );
                let state = states
                    .get(b.get_i32("state").unwrap_or(0) as usize)
                    .copied()
                    .unwrap_or(BlockState::AIR);
                let nbt = b.get_compound("nbt").cloned();
                let info = BlockInfo { pos, state, nbt };
                if info.nbt.is_some() {
                    entities.push(info);
                } else if blocks.flags(state).is_full_collision() {
                    full.push(info);
                } else {
                    other.push(info);
                }
            }
            for list in [&mut full, &mut other, &mut entities] {
                list.sort_by_key(|b| (b.pos.y, b.pos.x, b.pos.z));
            }
            let mut all = full;
            all.append(&mut other);
            all.append(&mut entities);
            let jigsaw = blocks.block_by_name(&Identifier::vanilla("jigsaw"));
            let jigsaws = all
                .iter()
                .filter(|b| Some(blocks.block_of(b.state)) == jigsaw)
                .map(|b| jigsaw_info(blocks, b))
                .collect::<Result<Vec<_>>>()?;
            palettes.push(Palette { blocks: all, jigsaws });
            Ok(())
        };
        if let Some(list) = tag.get_list("palettes") {
            for p in list.iter() {
                if let Tag::List(l) = p {
                    load_palette(l)?;
                }
            }
        } else if let Some(p) = tag.get_list("palette") {
            load_palette(p)?;
        } else {
            load_palette(&chunker_nbt::List::new())?;
        }
        let mut entities = Vec::new();
        for e in tag.get_list("entities").into_iter().flat_map(|l| l.compounds()) {
            let pos: Vec<f64> = e
                .get_list("pos")
                .map(|l| l.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
                .unwrap_or_default();
            let bp = ints(e.get_list("blockPos"));
            if let Some(nbt) = e.get_compound("nbt") {
                entities.push(EntityInfo {
                    pos: (
                        pos.first().copied().unwrap_or(0.0),
                        pos.get(1).copied().unwrap_or(0.0),
                        pos.get(2).copied().unwrap_or(0.0),
                    ),
                    block_pos: BlockPos::new(
                        bp.first().copied().unwrap_or(0),
                        bp.get(1).copied().unwrap_or(0),
                        bp.get(2).copied().unwrap_or(0),
                    ),
                    nbt: nbt.clone(),
                });
            }
        }
        Ok(Template {
            size,
            palettes,
            entities,
        })
    }
}

/// `NbtUtils.readBlockState`: `{id/Name, properties/Properties}`; unknown blocks become air.
pub fn state_from_nbt(blocks: &BlockRegistry, tag: &Tag) -> BlockState {
    let Some(c) = tag.as_compound() else {
        return tag
            .as_str()
            .and_then(|s| blocks.parse_state_str(s).ok())
            .unwrap_or(BlockState::AIR);
    };
    let Some(name) = c.get_str("id").or_else(|| c.get_str("Name")) else {
        return BlockState::AIR;
    };
    let Some(mut state) = Identifier::parse(name).and_then(|id| blocks.default_state(&id)) else {
        return BlockState::AIR;
    };
    if let Some(props) = c
        .get_compound("properties")
        .or_else(|| c.get_compound("Properties"))
    {
        for (k, v) in props.iter() {
            if let Some(next) = v.as_str().and_then(|v| blocks.with_property(state, k, v)) {
                state = next;
            }
        }
    }
    state
}

fn jigsaw_info(blocks: &BlockRegistry, b: &BlockInfo) -> Result<JigsawInfo> {
    let nbt = b.nbt.as_ref().context("jigsaw without nbt")?;
    let orientation = blocks
        .get_property(b.state, "orientation")
        .context("jigsaw without orientation")?;
    let (front, top) = orientation.split_once('_').context("bad jigsaw orientation")?;
    let front = Direction::by_name(front).context("bad jigsaw front")?;
    let top = Direction::by_name(top).context("bad jigsaw top")?;
    let id = |key: &str| nbt.get_str(key).and_then(Identifier::parse);
    let empty = Identifier::vanilla("empty");
    let rollable = match nbt.get_str("joint") {
        Some("rollable") => true,
        Some("aligned") => false,
        _ => front.axis() == super::geom::Axis::Y,
    };
    Ok(JigsawInfo {
        pos: b.pos,
        front,
        top,
        rollable,
        name: Some(id("name").unwrap_or_else(|| empty.clone())),
        pool: id("pool").unwrap_or_else(|| empty.clone()),
        target: id("target").unwrap_or(empty),
        placement_priority: nbt.get_i32("placement_priority").unwrap_or(0),
        selection_priority: nbt.get_i32("selection_priority").unwrap_or(0),
    })
}

/// `StructureTemplateManager`: templates are loaded once and kept for the process lifetime.
pub struct TemplateManager {
    packs: Option<Arc<DataPacks>>,
    blocks: Option<Arc<BlockRegistry>>,
    cache: RwLock<FxHashMap<Identifier, &'static Template>>,
}

impl TemplateManager {
    pub fn new(packs: Arc<DataPacks>, blocks: Arc<BlockRegistry>) -> Self {
        Self {
            packs: Some(packs),
            blocks: Some(blocks),
            cache: RwLock::default(),
        }
    }

    /// A manager without data (every template is empty), for tests of non-template structures.
    pub fn empty() -> Self {
        Self {
            packs: None,
            blocks: None,
            cache: RwLock::default(),
        }
    }

    /// `getOrCreate(id)`: missing or broken templates are empty, like vanilla.
    pub fn get(&self, id: &Identifier) -> &'static Template {
        if let Some(t) = self.cache.read().get(id) {
            return t;
        }
        let template = self.load(id).unwrap_or_else(|e| {
            tracing::warn!("structure template {id}: {e:#}");
            Template {
                size: (0, 0, 0),
                palettes: Vec::new(),
                entities: Vec::new(),
            }
        });
        let leaked: &'static Template = Box::leak(Box::new(template));
        self.cache.write().entry(id.clone()).or_insert(leaked)
    }

    fn load(&self, id: &Identifier) -> Result<Template> {
        let (Some(packs), Some(blocks)) = (&self.packs, &self.blocks) else {
            anyhow::bail!("no data packs")
        };
        let bytes = packs.read_raw("structure", id, "nbt").context("not found")?;
        let tag = chunker_nbt::read_compressed(&bytes[..]).context("reading NBT")?;
        Template::parse(blocks, &tag)
    }
}
