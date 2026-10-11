//! Block and block-state registry.
//!
//! State ids are the vanilla global ids of the given version (`Block.BLOCK_STATE_REGISTRY`), so
//! they are only meaningful together with a [`BlockRegistry`]. Inside a block, states are ordered as
//! the mixed-radix product of its properties, last property varying fastest (vanilla order).

use std::fmt;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::{Identifier, McVersion};

/// Global block-state id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct BlockState(pub u16);

impl BlockState {
    /// `minecraft:air` is always state 0.
    pub const AIR: BlockState = BlockState(0);

    #[inline]
    pub fn id(self) -> u16 {
        self.0
    }
}

impl fmt::Debug for BlockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlockState({})", self.0)
    }
}

/// Code-defined per-state properties extracted from vanilla (`cargo xtask extract`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockFlags(pub u64);

impl BlockFlags {
    /// `BlockState.isAir()`.
    #[inline(always)]
    pub fn is_air(self) -> bool {
        self.0 & 1 != 0
    }
    /// `!getFluidState().isEmpty()`.
    #[inline(always)]
    pub fn has_fluid(self) -> bool {
        self.0 & (1 << 1) != 0
    }
    /// Fluid state is in `#minecraft:water`.
    #[inline(always)]
    pub fn is_water_fluid(self) -> bool {
        self.0 & (1 << 2) != 0
    }
    /// Fluid state is in `#minecraft:lava`.
    #[inline(always)]
    pub fn is_lava_fluid(self) -> bool {
        self.0 & (1 << 3) != 0
    }
    #[inline(always)]
    pub fn is_fluid_source(self) -> bool {
        self.0 & (1 << 4) != 0
    }
    /// `BlockState.liquid()`.
    #[inline(always)]
    pub fn liquid(self) -> bool {
        self.0 & (1 << 5) != 0
    }
    /// `BlockState.isSolid()` (legacy solid).
    #[inline(always)]
    pub fn is_solid(self) -> bool {
        self.0 & (1 << 6) != 0
    }
    #[inline(always)]
    pub fn can_occlude(self) -> bool {
        self.0 & (1 << 7) != 0
    }
    /// `BlockState.canBeReplaced()`.
    #[inline(always)]
    pub fn can_be_replaced(self) -> bool {
        self.0 & (1 << 8) != 0
    }
    #[inline(always)]
    pub fn is_randomly_ticking(self) -> bool {
        self.0 & (1 << 9) != 0
    }
    #[inline(always)]
    pub fn has_block_entity(self) -> bool {
        self.0 & (1 << 10) != 0
    }
    #[inline(always)]
    pub fn is_solid_render(self) -> bool {
        self.0 & (1 << 11) != 0
    }
    #[inline(always)]
    pub fn fluid_amount(self) -> u8 {
        ((self.0 >> 12) & 15) as u8
    }
    #[inline(always)]
    pub fn light_emission(self) -> u8 {
        ((self.0 >> 16) & 15) as u8
    }
    #[inline(always)]
    pub fn light_dampening(self) -> u8 {
        ((self.0 >> 20) & 15) as u8
    }
    #[inline(always)]
    pub fn propagates_skylight_down(self) -> bool {
        self.0 & (1 << 24) != 0
    }

    /// `!hasDynamicShape() && isCollisionShapeFullBlock()` (template block ordering).
    pub fn is_full_collision(self) -> bool {
        self.0 & (1 << 25) != 0
    }

    /// `isFaceSturdy(EmptyBlockGetter, ZERO, UP)`.
    pub fn is_sturdy_up(self) -> bool {
        self.0 & (1 << 26) != 0
    }

    /// `isFaceSturdy(EmptyBlockGetter, ZERO, DOWN)`.
    pub fn is_sturdy_down(self) -> bool {
        self.0 & (1 << 27) != 0
    }

    /// `isFaceSturdy(.., direction, SupportType.FULL)`, direction in `Direction.values()` order
    /// (down, up, north, south, west, east).
    #[inline(always)]
    pub fn is_face_sturdy(self, direction_index: usize) -> bool {
        self.0 & (1 << (28 + direction_index)) != 0
    }

    /// `isFaceSturdy(.., UP, SupportType.CENTER)` (`canSupportCenter`).
    pub fn is_sturdy_center_up(self) -> bool {
        self.0 & (1 << 34) != 0
    }

    /// `isFaceSturdy(.., UP, SupportType.RIGID)` (`canSupportRigidBlock`).
    pub fn is_sturdy_rigid_up(self) -> bool {
        self.0 & (1 << 35) != 0
    }

    /// `isCollisionShapeFullBlock`.
    pub fn is_collision_full_block(self) -> bool {
        self.0 & (1 << 36) != 0
    }

    /// `isRedstoneConductor`.
    pub fn is_redstone_conductor(self) -> bool {
        self.0 & (1 << 37) != 0
    }

    /// `isSuffocating`.
    pub fn is_suffocating(self) -> bool {
        self.0 & (1 << 38) != 0
    }

    /// Fluid type of the fluid state.
    #[inline(always)]
    pub fn fluid(self) -> Fluid {
        match (self.0 >> 39) & 7 {
            1 => Fluid::Water,
            2 => Fluid::FlowingWater,
            3 => Fluid::Lava,
            4 => Fluid::FlowingLava,
            _ => Fluid::Empty,
        }
    }

    /// `FlowingFluid.FALLING` of the fluid state.
    pub fn fluid_falling(self) -> bool {
        self.0 & (1 << 42) != 0
    }

    /// `getCollisionShape().isEmpty()`.
    pub fn collision_empty(self) -> bool {
        self.0 & (1 << 43) != 0
    }

    /// `Block.isFaceFull(getCollisionShape(), UP)`.
    pub fn collision_face_full_up(self) -> bool {
        self.0 & (1 << 44) != 0
    }

    /// `!getCollisionShape().getFaceShape(UP).isEmpty()`.
    pub fn collision_face_up_nonempty(self) -> bool {
        self.0 & (1 << 57) != 0
    }

    /// `block instanceof LiquidBlockContainer`.
    pub fn is_liquid_container(self) -> bool {
        self.0 & (1 << 58) != 0
    }

    /// `block instanceof SimpleWaterloggedBlock`.
    pub fn is_simple_waterlogged(self) -> bool {
        self.0 & (1 << 59) != 0
    }

    /// `Block.canSupportCenter(level, pos, DOWN)`.
    pub fn supports_center_down(self) -> bool {
        self.0 & (1 << 61) != 0
    }

    /// `Block.isShapeFullBlock(state.getShape(level, pos))` (outline shape).
    pub fn is_shape_full_block(self) -> bool {
        self.0 & (1 << 60) != 0
    }

    /// `Block.isFaceFull(getCollisionShape(), direction)`, `Direction.values()` order.
    #[inline(always)]
    pub fn collision_face_full(self, direction_index: usize) -> bool {
        self.0 & (1 << (51 + direction_index)) != 0
    }

    /// `Block.isShapeFullBlock(getFaceOcclusionShape(direction))`, `Direction.values()` order.
    #[inline(always)]
    pub fn face_occlusion_full(self, direction_index: usize) -> bool {
        self.0 & (1 << (45 + direction_index)) != 0
    }
}

/// Fluid types (`Fluids`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fluid {
    Empty,
    Water,
    FlowingWater,
    Lava,
    FlowingLava,
}

impl Fluid {
    pub fn id(self) -> &'static str {
        match self {
            Fluid::Empty => "minecraft:empty",
            Fluid::Water => "minecraft:water",
            Fluid::FlowingWater => "minecraft:flowing_water",
            Fluid::Lava => "minecraft:lava",
            Fluid::FlowingLava => "minecraft:flowing_lava",
        }
    }

    pub fn from_id(id: &str) -> Option<Fluid> {
        [
            Fluid::Empty,
            Fluid::Water,
            Fluid::FlowingWater,
            Fluid::Lava,
            Fluid::FlowingLava,
        ]
        .into_iter()
        .find(|f| f.id() == id || &f.id()["minecraft:".len()..] == id)
    }

    /// `#minecraft:water`.
    pub fn is_water(self) -> bool {
        matches!(self, Fluid::Water | Fluid::FlowingWater)
    }

    /// `#minecraft:lava`.
    pub fn is_lava(self) -> bool {
        matches!(self, Fluid::Lava | Fluid::FlowingLava)
    }
}

/// Block index in registry order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct BlockId(pub u16);

#[derive(Debug)]
pub struct Property {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug)]
pub struct BlockInfo {
    pub name: Identifier,
    pub properties: Vec<Property>,
    pub first_state: u16,
    pub state_count: u16,
    pub default_state: BlockState,
}

impl BlockInfo {
    fn stride(&self, prop_index: usize) -> usize {
        self.properties[prop_index + 1..]
            .iter()
            .map(|p| p.values.len())
            .product()
    }
}

pub struct BlockRegistry {
    pub version: McVersion,
    blocks: Vec<BlockInfo>,
    by_name: FxHashMap<Identifier, BlockId>,
    state_to_block: Vec<BlockId>,
    flags: Vec<u64>,
    colors: Vec<u32>,
    /// Vanilla class chain per block, e.g. `SaplingBlock>VegetationBlock>Block`.
    classes: Vec<String>,
    /// Per state: `rotate(CLOCKWISE_90, CLOCKWISE_180, COUNTERCLOCKWISE_90)`, `mirror(LEFT_RIGHT, FRONT_BACK)`.
    transforms: Vec<[BlockState; 5]>,
    /// Per block: `newBlockEntity(0,0,0, defaultState).saveWithFullMetadata()` (`block_entities.nbt`).
    block_entities: Vec<Option<chunker_nbt::Compound>>,
    /// Per block: the block entity after `loadWithComponents({})`, saved.
    loaded_block_entities: Vec<Option<chunker_nbt::Compound>>,
    /// Per state: shape tests of its collision face DOWN (`block_shapes.txt`).
    shapes: Vec<u8>,
}

static REGISTRY_26_3: OnceLock<Arc<BlockRegistry>> = OnceLock::new();

impl BlockRegistry {
    /// The embedded registry for a version.
    pub fn for_version(version: McVersion) -> Arc<BlockRegistry> {
        match version {
            McVersion::V26_3 => REGISTRY_26_3
                .get_or_init(|| {
                    Arc::new(
                        Self::parse(
                            version,
                            include_str!("../data/26.3/blocks.txt"),
                            include_str!("../data/26.3/block_flags.txt"),
                            include_str!("../data/26.3/block_colors.txt"),
                            include_str!("../data/26.3/block_classes.txt"),
                            include_str!("../data/26.3/block_transforms.txt"),
                            include_bytes!("../data/26.3/block_entities.nbt"),
                            include_str!("../data/26.3/block_shapes.txt"),
                        )
                        .expect("embedded block table"),
                    )
                })
                .clone(),
        }
    }

    fn parse(
        version: McVersion,
        text: &str,
        flags_text: &str,
        colors_text: &str,
        classes_text: &str,
        transforms_text: &str,
        block_entities_nbt: &[u8],
        shapes_text: &str,
    ) -> Result<Self> {
        let mut blocks = Vec::new();
        let mut by_name = FxHashMap::default();
        let mut state_to_block = Vec::new();
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split('|');
            let name = Identifier::vanilla(parts.next().context("name")?);
            let props_raw = parts.next().context("props")?;
            let default: usize = parts.next().context("default")?.parse()?;
            let properties: Vec<Property> = if props_raw.is_empty() {
                Vec::new()
            } else {
                props_raw
                    .split(';')
                    .map(|p| {
                        let (n, v) = p.split_once('=').expect("prop");
                        Property {
                            name: n.to_owned(),
                            values: v.split(',').map(str::to_owned).collect(),
                        }
                    })
                    .collect()
            };
            let count: usize = properties.iter().map(|p| p.values.len()).product();
            let first = state_to_block.len();
            let id = BlockId(blocks.len() as u16);
            state_to_block.extend(std::iter::repeat_n(id, count));
            by_name.insert(name.clone(), id);
            blocks.push(BlockInfo {
                name,
                properties,
                first_state: first as u16,
                state_count: count as u16,
                default_state: BlockState((first + default) as u16),
            });
        }
        if state_to_block.len() > u16::MAX as usize {
            bail!("too many block states for u16 ids");
        }
        let flags: Vec<u64> = flags_text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| u64::from_str_radix(l, 16))
            .collect::<Result<_, _>>()?;
        if flags.len() != state_to_block.len() {
            bail!(
                "block_flags.txt has {} entries, expected {}",
                flags.len(),
                state_to_block.len()
            );
        }
        let colors: Vec<u32> = colors_text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| u32::from_str_radix(l, 16))
            .collect::<Result<_, _>>()?;
        if colors.len() != state_to_block.len() {
            bail!(
                "block_colors.txt has {} entries, expected {}",
                colors.len(),
                state_to_block.len()
            );
        }
        let mut classes = vec![String::new(); blocks.len()];
        for line in classes_text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (name, chain) = line.split_once('=').context("block class line")?;
            let id = by_name
                .get(&Identifier::vanilla(name))
                .with_context(|| format!("unknown block {name} in block_classes.txt"))?;
            classes[id.0 as usize] = chain.to_owned();
        }
        let shapes: Vec<u8> = shapes_text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| u8::from_str_radix(l, 16))
            .collect::<Result<_, _>>()?;
        if shapes.len() != state_to_block.len() {
            bail!(
                "block_shapes.txt has {} entries, expected {}",
                shapes.len(),
                state_to_block.len()
            );
        }
        let be_root = chunker_nbt::read_compressed(block_entities_nbt).context("block_entities.nbt")?;
        let table = |key: &str| -> Result<Vec<Option<chunker_nbt::Compound>>> {
            let mut out = vec![None; blocks.len()];
            for (name, tag) in be_root
                .get_compound(key)
                .context("block_entities.nbt sections")?
                .iter()
            {
                let id = by_name
                    .get(&Identifier::vanilla(name))
                    .with_context(|| format!("unknown block {name} in block_entities.nbt"))?;
                if let chunker_nbt::Tag::Compound(c) = tag {
                    out[id.0 as usize] = Some(c.clone());
                }
            }
            Ok(out)
        };
        let block_entities = table("fresh")?;
        let loaded_block_entities = table("loaded")?;
        let mut transforms: Vec<[BlockState; 5]> = (0..state_to_block.len())
            .map(|i| [BlockState(i as u16); 5])
            .collect();
        for line in transforms_text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (id, rest) = line.split_once(':').context("block transform line")?;
            let id: usize = id.parse()?;
            let entry = transforms
                .get_mut(id)
                .context("block transform state id out of range")?;
            for (slot, v) in entry.iter_mut().zip(rest.split(',')) {
                *slot = BlockState(v.parse()?);
            }
        }
        Ok(Self {
            version,
            blocks,
            by_name,
            state_to_block,
            flags,
            colors,
            classes,
            transforms,
            block_entities,
            loaded_block_entities,
            shapes,
        })
    }

    /// The block's block entity as loaded from empty NBT (fields absent from loaded data take these).
    pub fn loaded_block_entity(&self, block: BlockId) -> Option<&chunker_nbt::Compound> {
        self.loaded_block_entities[block.0 as usize].as_ref()
    }

    /// `WallBlock.isCovered(aboveShape, TEST_SHAPE_POST)` with `state` as the block above a wall.
    pub fn covers_wall_post(&self, state: BlockState) -> bool {
        self.shapes[state.0 as usize] & 1 != 0
    }

    /// `WallBlock.isCovered(aboveShape, TEST_SHAPES_WALL[direction])`; `side` 0..4 = north, east, south, west.
    pub fn covers_wall_side(&self, state: BlockState, side: usize) -> bool {
        self.shapes[state.0 as usize] & (2 << side) != 0
    }

    /// The block's default block entity NBT (`saveWithFullMetadata` of a fresh one at 0,0,0).
    pub fn default_block_entity(&self, block: BlockId) -> Option<&chunker_nbt::Compound> {
        self.block_entities[block.0 as usize].as_ref()
    }

    /// `BlockState.rotate(rotation)`; `quarter_turns` clockwise (0..4).
    pub fn rotate(&self, state: BlockState, quarter_turns: u8) -> BlockState {
        match quarter_turns & 3 {
            0 => state,
            n => self.transforms[state.0 as usize][n as usize - 1],
        }
    }

    /// `BlockState.mirror(mirror)`: 0 none, 1 `LEFT_RIGHT`, 2 `FRONT_BACK`.
    pub fn mirror(&self, state: BlockState, mirror: u8) -> BlockState {
        match mirror {
            1 => self.transforms[state.0 as usize][3],
            2 => self.transforms[state.0 as usize][4],
            _ => state,
        }
    }

    pub fn state_count(&self) -> usize {
        self.state_to_block.len()
    }

    pub fn blocks(&self) -> &[BlockInfo] {
        &self.blocks
    }

    pub fn block_by_name(&self, name: &Identifier) -> Option<BlockId> {
        self.by_name.get(name).copied()
    }

    pub fn block(&self, id: BlockId) -> &BlockInfo {
        &self.blocks[id.0 as usize]
    }

    #[inline]
    pub fn block_of(&self, state: BlockState) -> BlockId {
        self.state_to_block[state.0 as usize]
    }

    /// Vanilla class chain of a block (`SaplingBlock>VegetationBlock>Block`).
    pub fn class_chain(&self, id: BlockId) -> &str {
        &self.classes[id.0 as usize]
    }

    /// Whether the block's class is, or extends, the vanilla class with this simple name.
    pub fn is_instance(&self, id: BlockId, class: &str) -> bool {
        self.classes[id.0 as usize].split('>').any(|c| c == class)
    }

    pub fn block_info_of(&self, state: BlockState) -> &BlockInfo {
        self.block(self.block_of(state))
    }

    pub fn name_of(&self, state: BlockState) -> &Identifier {
        &self.block_info_of(state).name
    }

    pub fn default_state(&self, name: &Identifier) -> Option<BlockState> {
        self.block_by_name(name).map(|b| self.block(b).default_state)
    }

    /// Default state by vanilla path, panicking if unknown. For blocks hardcoded in vanilla code.
    pub fn vanilla_default(&self, path: &str) -> BlockState {
        self.default_state(&Identifier::vanilla(path))
            .unwrap_or_else(|| panic!("unknown block minecraft:{path}"))
    }

    #[inline(always)]
    pub fn flags(&self, state: BlockState) -> BlockFlags {
        BlockFlags(self.flags[state.0 as usize])
    }

    /// Map color (`0xRRGGBB`, 0 for none) used by the preview renderer.
    #[inline]
    pub fn map_color(&self, state: BlockState) -> u32 {
        self.colors[state.0 as usize]
    }

    /// Property values of a state, in definition order.
    pub fn properties(&self, state: BlockState) -> Vec<(&str, &str)> {
        let info = self.block_info_of(state);
        let mut index = (state.0 - info.first_state) as usize;
        let mut out = Vec::with_capacity(info.properties.len());
        for (i, p) in info.properties.iter().enumerate() {
            let stride = info.stride(i);
            let vi = index / stride;
            index %= stride;
            out.push((p.name.as_str(), p.values[vi].as_str()));
        }
        out
    }

    pub fn get_property<'a>(&'a self, state: BlockState, name: &str) -> Option<&'a str> {
        let info = self.block_info_of(state);
        let pi = info.properties.iter().position(|p| p.name == name)?;
        let index = (state.0 - info.first_state) as usize;
        let vi = (index / info.stride(pi)) % info.properties[pi].values.len();
        Some(info.properties[pi].values[vi].as_str())
    }

    /// Returns the state with one property changed; `None` if the block lacks the property/value.
    pub fn with_property(&self, state: BlockState, name: &str, value: &str) -> Option<BlockState> {
        let info = self.block_info_of(state);
        let pi = info.properties.iter().position(|p| p.name == name)?;
        let new_vi = info.properties[pi].values.iter().position(|v| v == value)?;
        let stride = info.stride(pi);
        let index = (state.0 - info.first_state) as usize;
        let old_vi = (index / stride) % info.properties[pi].values.len();
        let new_index = index - old_vi * stride + new_vi * stride;
        Some(BlockState(info.first_state + new_index as u16))
    }

    pub fn has_property(&self, state: BlockState, name: &str) -> bool {
        self.block_info_of(state)
            .properties
            .iter()
            .any(|p| p.name == name)
    }

    /// Parses `BlockState.CODEC` objects: 26.x `{"id": ..., "properties": {...}}` or the legacy
    /// `{"Name": ..., "Properties": {...}}`. Missing properties take default values.
    pub fn parse_state_json(&self, json: &Value) -> Result<BlockState> {
        if let Value::String(s) = json {
            return self.parse_state_str(s);
        }
        let name = json
            .get("id")
            .or_else(|| json.get("Name"))
            .and_then(Value::as_str)
            .context("block state without id")?;
        let id = Identifier::parse(name).with_context(|| format!("bad block name {name}"))?;
        let mut state = self
            .default_state(&id)
            .with_context(|| format!("unknown block {id}"))?;
        if let Some(props) = json
            .get("properties")
            .or_else(|| json.get("Properties"))
            .and_then(Value::as_object)
        {
            for (k, v) in props {
                let v = v.as_str().context("property value must be string")?;
                state = self
                    .with_property(state, k, v)
                    .with_context(|| format!("{id} has no property {k}={v}"))?;
            }
        }
        Ok(state)
    }

    /// Parses `minecraft:oak_log[axis=y]` syntax.
    pub fn parse_state_str(&self, s: &str) -> Result<BlockState> {
        let (name, props) = match s.split_once('[') {
            Some((n, rest)) => (n, Some(rest.strip_suffix(']').context("missing ]")?)),
            None => (s, None),
        };
        let id = Identifier::parse(name).with_context(|| format!("bad block name {name}"))?;
        let mut state = self
            .default_state(&id)
            .with_context(|| format!("unknown block {id}"))?;
        for kv in props
            .into_iter()
            .flat_map(|p| p.split(','))
            .filter(|p| !p.is_empty())
        {
            let (k, v) = kv.split_once('=').context("property without =")?;
            state = self
                .with_property(state, k.trim(), v.trim())
                .with_context(|| format!("{id} has no property {kv}"))?;
        }
        Ok(state)
    }

    /// Human readable `name[props]`.
    pub fn describe(&self, state: BlockState) -> String {
        let props = self.properties(state);
        if props.is_empty() {
            self.name_of(state).to_string()
        } else {
            let p: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{}[{}]", self.name_of(state), p.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_lookups() {
        let reg = BlockRegistry::for_version(McVersion::V26_3);
        assert_eq!(reg.name_of(BlockState::AIR).as_str(), "minecraft:air");
        let stone = reg.vanilla_default("stone");
        assert_eq!(reg.describe(stone), "minecraft:stone");
        let grass = reg.vanilla_default("grass_block");
        assert_eq!(reg.get_property(grass, "snowy"), Some("false"));
        let snowy = reg.with_property(grass, "snowy", "true").unwrap();
        assert_eq!(reg.describe(snowy), "minecraft:grass_block[snowy=true]");
        let leaves = reg
            .parse_state_str("minecraft:oak_leaves[persistent=true,distance=1]")
            .unwrap();
        assert_eq!(reg.get_property(leaves, "distance"), Some("1"));
        assert_eq!(reg.get_property(leaves, "waterlogged"), Some("false"));
        let water = reg.vanilla_default("water");
        assert_eq!(reg.get_property(water, "level"), Some("0"));
    }
}
