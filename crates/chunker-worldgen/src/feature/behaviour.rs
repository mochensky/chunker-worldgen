//! Block behaviour used during worldgen: `canSurvive`, `updateShape`, post-processing positions,
//! fresh block entities. Keyed by the vanilla block class (see `block_classes.txt`).

use std::sync::Arc;

use anyhow::Result;
use chunker_data::Identifier;
use chunker_data::blocks::{BlockId, BlockRegistry, BlockState, Fluid};
use chunker_nbt::Compound;

use super::{FeatureEnv, Region};
use crate::block_set::StateSet;
use crate::registry::WorldgenRegistries;
use crate::structure::geom::{Axis, BlockPos, Direction};

/// Behaviour family of a block (most specific vanilla class that matters for worldgen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Default,
    /// `VegetationBlock`: `canSurvive` is `mayPlaceOn(below)`, removed when it cannot survive.
    Vegetation,
    /// `SeagrassBlock`: vegetation that schedules a water tick when it stays.
    Seagrass,
    /// `SeaPickleBlock`.
    SeaPickle,
    /// `MangrovePropaguleBlock` (hanging ones need supporting leaves above).
    Propagule,
    /// `LeafLitterBlock`: needs a sturdy top face below.
    LeafLitter,
    Mushroom,
    DoublePlant,
    /// `TallSeagrassBlock`: double plant whose lower half needs full water.
    TallSeagrass,
    SugarCane,
    SnowLayer,
    Snowy,
    Leaves,
    Liquid,
    Bamboo,
    Vine,
    /// `MossyCarpetBlock` (pale moss carpets with side growth).
    MossyCarpet,
    /// `HangingRootsBlock`: needs a sturdy bottom face above.
    HangingRoots,
    /// `MultifaceBlock` (glow lichen, sculk veins, resin): faces need something to attach to.
    Multiface,
    /// `FallingBlock`: always schedules a tick.
    Falling,
    /// Blocks whose `updateShape` only schedules a water tick when waterlogged.
    WaterloggedTick,
    /// `StairBlock`: shape from the neighbouring stairs.
    Stairs,
    /// `IronBarsBlock` (bars and glass panes).
    CrossCollision,
    /// `FenceBlock`.
    Fence,
    /// `WallBlock`.
    Wall,
    /// `DoorBlock`: halves copy each other.
    Door,
    /// `ChestBlock`: double chests connect and fall apart.
    Chest,
    /// `CopperChestBlock`: connects to every copper chest (`#copper_chests`).
    CopperChest,
}

/// `mayPlaceOn(belowState, level, belowPos)`.
#[derive(Clone)]
pub enum Support {
    None,
    Tag(Arc<StateSet>),
    /// Mushrooms: `isSolidRender`.
    SolidRender,
    /// Seagrass: sturdy top face and not in `#cannot_support_seagrass`.
    SturdyUpExcept(Arc<StateSet>),
    /// Lily pads: supporting fluid (or block) with no fluid above.
    LilyPad {
        fluids: Vec<Fluid>,
        blocks: Arc<StateSet>,
    },
    /// Sea pickles: a collision top face or a sturdy top face.
    SeaPickle,
}

/// Per-block behaviour table of a world.
pub struct Behaviours {
    kinds: Vec<Kind>,
    supports: Vec<Support>,
    /// `#supports_sugar_cane_adjacently` for fluids.
    sugar_cane_fluids: Vec<Fluid>,
    supports_hanging_propagule: Arc<StateSet>,
    /// Per block: `class_bits` flags.
    pub class_bits: Vec<u8>,
}

fn fluid_tag(reg: &WorldgenRegistries, name: &str) -> Result<Vec<Fluid>> {
    Ok(reg
        .packs
        .resolve_tag("fluid", &Identifier::vanilla(name), &|i| {
            Fluid::from_id(&i.to_string()).is_some()
        })?
        .iter()
        .filter_map(|i| Fluid::from_id(&i.to_string()))
        .collect())
}

impl Behaviours {
    pub fn load(reg: &WorldgenRegistries) -> Result<Self> {
        let blocks = &reg.blocks;
        let tag = |name: &str| reg.block_tag(&Identifier::vanilla(name));
        let vegetation = Support::Tag(tag("supports_vegetation")?);
        let mut kinds = Vec::with_capacity(blocks.blocks().len());
        let mut supports = Vec::with_capacity(blocks.blocks().len());
        let mut class_bits = Vec::with_capacity(blocks.blocks().len());
        for (i, info) in blocks.blocks().iter().enumerate() {
            let id = BlockId(i as u16);
            let has = |c: &str| blocks.is_instance(id, c);
            let name = info.name.path();
            let veg_support = || -> Result<Support> {
                Ok(if has("AzaleaBlock") {
                    Support::Tag(tag("supports_azalea")?)
                } else if has("DryVegetationBlock") {
                    Support::Tag(tag("supports_dry_vegetation")?)
                } else if has("WitherRoseBlock") {
                    Support::Tag(tag("supports_wither_rose")?)
                } else if has("NetherSproutsBlock") {
                    Support::Tag(tag("supports_nether_sprouts")?)
                } else if has("NetherWartBlock") {
                    Support::Tag(tag("supports_nether_wart")?)
                } else if has("NetherRootsBlock") {
                    Support::Tag(tag(if name == "warped_roots" {
                        "supports_warped_roots"
                    } else {
                        "supports_crimson_roots"
                    })?)
                } else if has("NetherFungusBlock") {
                    Support::Tag(tag(if name == "warped_fungus" {
                        "supports_warped_fungus"
                    } else {
                        "supports_crimson_fungus"
                    })?)
                } else if has("CropBlock") || has("PitcherCropBlock") {
                    Support::Tag(tag("supports_crops")?)
                } else if has("MangrovePropaguleBlock") {
                    Support::Tag(tag("supports_mangrove_propagule")?)
                } else if has("SeagrassBlock") || has("TallSeagrassBlock") {
                    Support::SturdyUpExcept(tag("cannot_support_seagrass")?)
                } else if has("LilyPadBlock") {
                    Support::LilyPad {
                        fluids: fluid_tag(reg, "supports_lily_pad")?,
                        blocks: tag("supports_lily_pad")?,
                    }
                } else if has("SeaPickleBlock") {
                    Support::SeaPickle
                } else if has("MushroomBlock") {
                    Support::SolidRender
                } else {
                    vegetation.clone()
                })
            };
            let mut bits = 0u8;
            for (class, bit) in [
                ("IronBarsBlock", class_bits::IRON_BARS),
                ("FenceGateBlock", class_bits::FENCE_GATE),
                ("WallBlock", class_bits::WALL),
                ("LeavesBlock", class_bits::LEAVES),
                ("StairBlock", class_bits::STAIRS),
                ("DoorBlock", class_bits::DOOR),
            ] {
                if has(class) {
                    bits |= bit;
                }
            }
            class_bits.push(bits);
            let (kind, support) = if has("MushroomBlock") {
                (Kind::Mushroom, Support::SolidRender)
            } else if has("TallSeagrassBlock") {
                (Kind::TallSeagrass, veg_support()?)
            } else if has("DoublePlantBlock") && !has("SmallDripleafBlock") && !has("PitcherCropBlock") {
                (Kind::DoublePlant, veg_support()?)
            } else if has("SeagrassBlock") {
                (Kind::Seagrass, veg_support()?)
            } else if has("SeaPickleBlock") {
                (Kind::SeaPickle, veg_support()?)
            } else if has("MangrovePropaguleBlock") {
                (Kind::Propagule, veg_support()?)
            } else if has("LeafLitterBlock") {
                (Kind::LeafLitter, Support::None)
            } else if has("SugarCaneBlock") {
                (Kind::SugarCane, Support::None)
            } else if has("SnowLayerBlock") {
                (Kind::SnowLayer, Support::None)
            } else if has("SnowyBlock") {
                (Kind::Snowy, Support::None)
            } else if has("LeavesBlock") {
                (Kind::Leaves, Support::None)
            } else if has("LiquidBlock") {
                (Kind::Liquid, Support::None)
            } else if has("VineBlock") {
                (Kind::Vine, Support::None)
            } else if has("BambooStalkBlock") {
                (Kind::Bamboo, Support::None)
            } else if has("MossyCarpetBlock") {
                (Kind::MossyCarpet, Support::None)
            } else if has("HangingRootsBlock") {
                (Kind::HangingRoots, Support::None)
            } else if has("MultifaceBlock") {
                (Kind::Multiface, Support::None)
            } else if (has("FallingBlock") && !has("ConcretePowderBlock")) || has("BrushableBlock") {
                (Kind::Falling, Support::None)
            } else if has("MangroveRootsBlock") {
                (Kind::WaterloggedTick, Support::None)
            } else if has("StairBlock") {
                (Kind::Stairs, Support::None)
            } else if has("IronBarsBlock") {
                (Kind::CrossCollision, Support::None)
            } else if has("FenceBlock") {
                (Kind::Fence, Support::None)
            } else if has("WallBlock") {
                (Kind::Wall, Support::None)
            } else if has("DoorBlock") {
                (Kind::Door, Support::None)
            } else if has("CopperChestBlock") {
                (Kind::CopperChest, Support::None)
            } else if has("ChestBlock") {
                (Kind::Chest, Support::None)
            } else if has("VegetationBlock")
                && !has("SmallDripleafBlock")
                && !has("StemBlock")
                && !has("AttachedStemBlock")
                && !has("CactusFlowerBlock")
                && !has("PitcherCropBlock")
            {
                (Kind::Vegetation, veg_support()?)
            } else {
                (Kind::Default, Support::None)
            };
            kinds.push(kind);
            supports.push(support);
        }
        Ok(Self {
            kinds,
            supports,
            sugar_cane_fluids: fluid_tag(reg, "supports_sugar_cane_adjacently")?,
            supports_hanging_propagule: tag("supports_hanging_mangrove_propagule")?,
            class_bits,
        })
    }

    #[inline]
    pub fn kind(&self, block: BlockId) -> Kind {
        self.kinds[block.0 as usize]
    }
}

/// `mayPlaceOn(below)` of a block.
fn may_place_on(region: &Region, block: BlockId, below_pos: BlockPos) -> bool {
    let env = region.env;
    let below = region.get(below_pos);
    let f = env.blocks.flags(below);
    match &env.behaviours.supports[block.0 as usize] {
        Support::None => true,
        Support::Tag(t) => t.contains(below),
        Support::SolidRender => f.is_solid_render(),
        Support::SturdyUpExcept(t) => f.is_face_sturdy(1) && !t.contains(below),
        Support::LilyPad { fluids, blocks } => {
            (fluids.contains(&f.fluid()) || blocks.contains(below))
                && region.fluid(below_pos.above()) == Fluid::Empty
        }
        Support::SeaPickle => f.collision_face_up_nonempty() || f.is_face_sturdy(1),
    }
}

fn dir_index(d: Direction) -> usize {
    match d {
        Direction::Down => 0,
        Direction::Up => 1,
        Direction::North => 2,
        Direction::South => 3,
        Direction::West => 4,
        Direction::East => 5,
    }
}

/// `isFaceSturdy(level, pos, direction)` (full support).
pub fn is_face_sturdy(env: &FeatureEnv, state: BlockState, d: Direction) -> bool {
    env.blocks.flags(state).is_face_sturdy(dir_index(d))
}

fn prop<'a>(blocks: &'a BlockRegistry, state: BlockState, name: &str) -> Option<&'a str> {
    blocks.get_property(state, name)
}

/// `BlockBehaviour.Properties.postProcess`: mushrooms mark themselves, soul sand and magma the block above.
pub fn post_process_pos(env: &FeatureEnv, state: BlockState, p: BlockPos) -> Option<BlockPos> {
    let b = &env.b;
    let block = env.blocks.block_of(state);
    if block == b.brown_mushroom || block == b.red_mushroom {
        Some(p)
    } else if block == b.soul_sand || block == b.magma_block {
        Some(p.above())
    } else {
        None
    }
}

/// `BlockState.canSurvive(level, pos)`.
pub fn can_survive(region: &Region, state: BlockState, pos: BlockPos) -> bool {
    let env = region.env;
    let blocks = &env.blocks;
    let block = blocks.block_of(state);
    let beh = &env.behaviours;
    match beh.kind(block) {
        Kind::Default
        | Kind::Snowy
        | Kind::Leaves
        | Kind::Liquid
        | Kind::Falling
        | Kind::WaterloggedTick
        | Kind::Stairs
        | Kind::CrossCollision
        | Kind::Fence
        | Kind::Wall
        | Kind::Chest
        | Kind::CopperChest => true,
        Kind::Door => {
            let below = region.get(pos.below());
            if prop(blocks, state, "half") == Some("lower") {
                is_face_sturdy(env, below, Direction::Up)
            } else {
                blocks.block_of(below) == block
            }
        }
        Kind::Vegetation | Kind::Seagrass | Kind::SeaPickle => may_place_on(region, block, pos.below()),
        Kind::Propagule => {
            if prop(blocks, state, "hanging") == Some("true") {
                beh.supports_hanging_propagule.contains(region.get(pos.above()))
            } else {
                may_place_on(region, block, pos.below())
            }
        }
        Kind::LeafLitter => is_face_sturdy(env, region.get(pos.below()), Direction::Up),
        Kind::HangingRoots => is_face_sturdy(env, region.get(pos.above()), Direction::Down),
        Kind::MossyCarpet => mossy_can_survive(region, state, pos),
        Kind::Bamboo => env.t.supports_bamboo.contains(region.get(pos.below())),
        Kind::Vine => vine_has_faces(blocks, vine_updated_state(region, state, pos)),
        Kind::Multiface => {
            let mut any = false;
            for d in Direction::ALL {
                if prop(blocks, state, d.name()) == Some("true") {
                    if !can_attach_to(region, d, pos.relative(d, 1)) {
                        return false;
                    }
                    any = true;
                }
            }
            any
        }
        Kind::DoublePlant | Kind::TallSeagrass => {
            if prop(blocks, state, "half") == Some("upper") {
                let below = region.get(pos.below());
                blocks.block_of(below) == block && prop(blocks, below, "half") == Some("lower")
            } else if beh.kind(block) == Kind::TallSeagrass {
                let here = blocks.flags(region.get(pos));
                may_place_on(region, block, pos.below())
                    && here.fluid().is_water()
                    && here.fluid_amount() == 8
            } else {
                may_place_on(region, block, pos.below())
            }
        }
        Kind::Mushroom => {
            let below = region.get(pos.below());
            if env.t.overrides_mushroom_light_requirement.contains(below) {
                true
            } else {
                region.raw_brightness(pos, 0) < 13 && blocks.flags(below).is_solid_render()
            }
        }
        Kind::SugarCane => {
            let below_pos = pos.below();
            let below = region.get(below_pos);
            if blocks.block_of(below) == block {
                return true;
            }
            if env.t.supports_sugar_cane.contains(below) {
                for d in Direction::HORIZONTAL {
                    let n = below_pos.relative(d, 1);
                    let ns = region.get(n);
                    if beh.sugar_cane_fluids.contains(&blocks.flags(ns).fluid())
                        || env.t.supports_sugar_cane_adjacently.contains(ns)
                    {
                        return true;
                    }
                }
            }
            false
        }
        Kind::SnowLayer => {
            let below = region.get(pos.below());
            if env.t.cannot_support_snow_layer.contains(below) {
                false
            } else if env.t.support_override_snow_layer.contains(below) {
                true
            } else {
                blocks.flags(below).collision_face_full_up()
                    || (blocks.block_of(below) == block && prop(blocks, below, "layers") == Some("8"))
            }
        }
    }
}

const VINE_FACES: [&str; 5] = ["up", "north", "east", "south", "west"];

fn vine_has_faces(blocks: &BlockRegistry, state: BlockState) -> bool {
    VINE_FACES.iter().any(|f| prop(blocks, state, f) == Some("true"))
}

/// `MultifaceBlock.canAttachTo`: the neighbour's support or collision face toward us is full.
pub fn can_attach_to(region: &Region, toward: Direction, neighbour: BlockPos) -> bool {
    let f = region.env.blocks.flags(region.get(neighbour));
    let i = dir_index(toward.opposite());
    f.is_face_sturdy(i) || f.collision_face_full(i)
}

/// `VineBlock.getUpdatedState`.
fn vine_updated_state(region: &Region, mut state: BlockState, pos: BlockPos) -> BlockState {
    let blocks = &region.env.blocks;
    let set = |s: BlockState, k: &str, v: bool| {
        blocks
            .with_property(s, k, if v { "true" } else { "false" })
            .unwrap()
    };
    let above = pos.above();
    if prop(blocks, state, "up") == Some("true") {
        state = set(state, "up", can_attach_to(region, Direction::Down, above));
    }
    let vine = blocks.block_of(state);
    let mut above_state = None;
    for d in Direction::HORIZONTAL {
        let face = d.name();
        if prop(blocks, state, face) == Some("true") {
            // `canSupportAtFace`
            let mut ok = can_attach_to(region, d, pos.relative(d, 1));
            if !ok {
                let a = region.get(above);
                ok = blocks.block_of(a) == vine && prop(blocks, a, face) == Some("true");
            }
            if !ok {
                let a = *above_state.get_or_insert_with(|| region.get(above));
                ok = blocks.block_of(a) == vine && prop(blocks, a, face) == Some("true");
            }
            state = set(state, face, ok);
        }
    }
    state
}

/// `LeavesBlock.getOptionalDistanceAt`.
pub fn leaf_distance(env: &FeatureEnv, state: BlockState) -> Option<i32> {
    if env.t.prevents_nearby_leaf_decay.contains(state) {
        Some(0)
    } else {
        prop(&env.blocks, state, "distance").map(|v| v.parse().unwrap())
    }
}

/// `BlockState.updateShape(level, ticks, pos, direction, neighbourPos, neighbourState, random)`.
pub fn update_shape(
    region: &mut Region,
    state: BlockState,
    pos: BlockPos,
    direction: Direction,
    _neighbour_pos: BlockPos,
    neighbour: BlockState,
) -> BlockState {
    let env = region.env;
    let blocks = &env.blocks;
    let block = blocks.block_of(state);
    let air = BlockState::AIR;
    let kind = env.behaviours.kind(block);
    if let Some(s) = connecting_update_shape(region, kind, state, pos, direction, _neighbour_pos, neighbour) {
        return s;
    }
    match kind {
        Kind::Default | Kind::Stairs | Kind::CrossCollision | Kind::Fence | Kind::Wall => state,
        Kind::Door => {
            let half = prop(blocks, state, "half").unwrap_or("lower");
            let lower = half == "lower";
            if direction.axis() != crate::structure::geom::Axis::Y || lower != (direction == Direction::Up) {
                if lower && direction == Direction::Down && !can_survive(region, state, pos) {
                    air
                } else {
                    state
                }
            } else if has_class(env, neighbour, class_bits::DOOR)
                && prop(blocks, neighbour, "half") != Some(half)
            {
                set_prop(blocks, neighbour, "half", half)
            } else {
                air
            }
        }
        Kind::Falling => {
            region.schedule_block_tick(pos, block);
            state
        }
        Kind::MossyCarpet => {
            if !mossy_can_survive(region, state, pos) {
                return air;
            }
            let s = mossy_updated_state(region, state, pos, false);
            if mossy_has_faces(blocks, s) { s } else { air }
        }
        Kind::HangingRoots => {
            if direction == Direction::Up && !can_survive(region, state, pos) {
                return air;
            }
            if prop(blocks, state, "waterlogged") == Some("true") {
                region.schedule_fluid_tick(pos, Fluid::Water);
            }
            state
        }
        Kind::Multiface => {
            if prop(blocks, state, "waterlogged") == Some("true") {
                region.schedule_fluid_tick(pos, Fluid::Water);
            }
            let faces = ["down", "up", "north", "south", "west", "east"];
            if !faces.iter().any(|f| prop(blocks, state, f) == Some("true")) {
                return air;
            }
            if prop(blocks, state, direction.name()) == Some("true")
                && !can_attach_to(region, direction, pos.relative(direction, 1))
            {
                // `removeFace`
                let s = blocks
                    .with_property(state, direction.name(), "false")
                    .unwrap_or(state);
                if faces.iter().any(|f| prop(blocks, s, f) == Some("true")) {
                    s
                } else {
                    air
                }
            } else {
                state
            }
        }
        Kind::Chest | Kind::CopperChest => chest_update_shape(region, kind, state, pos, direction, neighbour),
        Kind::WaterloggedTick => {
            if prop(blocks, state, "waterlogged") == Some("true") {
                region.schedule_fluid_tick(pos, Fluid::Water);
            }
            state
        }
        Kind::Vegetation | Kind::Mushroom | Kind::SnowLayer | Kind::LeafLitter => {
            if can_survive(region, state, pos) {
                state
            } else {
                air
            }
        }
        Kind::Seagrass => {
            if can_survive(region, state, pos) {
                region.schedule_fluid_tick(pos, Fluid::Water);
                state
            } else {
                air
            }
        }
        Kind::SeaPickle => {
            if !can_survive(region, state, pos) {
                return air;
            }
            if prop(blocks, state, "waterlogged") == Some("true") {
                region.schedule_fluid_tick(pos, Fluid::Water);
            }
            state
        }
        Kind::Propagule => {
            if prop(blocks, state, "waterlogged") == Some("true") {
                region.schedule_fluid_tick(pos, Fluid::Water);
            }
            // Both the `UP` check and `VegetationBlock.updateShape` remove it when it cannot survive.
            if can_survive(region, state, pos) {
                state
            } else {
                air
            }
        }
        Kind::DoublePlant | Kind::TallSeagrass => {
            let half = prop(blocks, state, "half");
            let lower = half == Some("lower");
            let same_other_half =
                blocks.block_of(neighbour) == block && prop(blocks, neighbour, "half") != half;
            if direction.axis() != Axis::Y || lower != (direction == Direction::Up) || same_other_half {
                // `VegetationBlock.updateShape` (the DOWN check is a special case of it).
                if can_survive(region, state, pos) {
                    state
                } else {
                    air
                }
            } else {
                air
            }
        }
        Kind::SugarCane => {
            if !can_survive(region, state, pos) {
                region.schedule_block_tick(pos, block);
            }
            state
        }
        Kind::Vine => {
            if direction == Direction::Down {
                state
            } else {
                let updated = vine_updated_state(region, state, pos);
                if vine_has_faces(blocks, updated) {
                    updated
                } else {
                    air
                }
            }
        }
        Kind::Bamboo => {
            if !can_survive(region, state, pos) {
                region.schedule_block_tick(pos, block);
            }
            let age = |s: BlockState| {
                prop(blocks, s, "age")
                    .and_then(|v| v.parse::<i32>().ok())
                    .unwrap_or(0)
            };
            if direction == Direction::Up
                && blocks.block_of(neighbour) == block
                && age(neighbour) > age(state)
            {
                let next = (age(state) + 1) % 2;
                blocks
                    .with_property(state, "age", &next.to_string())
                    .unwrap_or(state)
            } else {
                state
            }
        }
        Kind::Snowy => {
            if direction == Direction::Up {
                blocks
                    .with_property(
                        state,
                        "snowy",
                        if env.t.snow.contains(neighbour) {
                            "true"
                        } else {
                            "false"
                        },
                    )
                    .unwrap_or(state)
            } else {
                state
            }
        }
        Kind::Leaves => {
            if prop(blocks, state, "waterlogged") == Some("true") {
                region.schedule_fluid_tick(pos, Fluid::Water);
            }
            let from_neighbour = leaf_distance(env, neighbour).unwrap_or(7) + 1;
            let own: i32 = prop(blocks, state, "distance")
                .map(|v| v.parse().unwrap())
                .unwrap_or(7);
            if from_neighbour != 1 || own != from_neighbour {
                region.schedule_block_tick(pos, block);
            }
            state
        }
        Kind::Liquid => {
            let f = blocks.flags(state);
            if f.is_fluid_source() || blocks.flags(neighbour).is_fluid_source() {
                region.schedule_fluid_tick(pos, f.fluid());
            }
            // `shouldBubbleColumnOccupy` (water sources) over a bubble column base.
            if direction == Direction::Down
                && f.fluid() == Fluid::Water
                && (env.t.enables_bubble_column_drag_down.contains(neighbour)
                    || env.t.enables_bubble_column_push_up.contains(neighbour))
            {
                region.schedule_block_tick(pos, block);
            }
            state
        }
    }
}

/// `EntityBlock.newBlockEntity(pos, state)` as saved by `saveWithFullMetadata` (vanilla defaults per
/// block from `block_entities.nbt`).
pub fn new_block_entity(env: &FeatureEnv, state: BlockState, p: BlockPos) -> Option<Compound> {
    let mut tag = env
        .blocks
        .default_block_entity(env.blocks.block_of(state))?
        .clone();
    tag.insert("x", p.x);
    tag.insert("y", p.y);
    tag.insert("z", p.z);
    Some(tag)
}

/// `BeehiveBlockEntity.Occupant.create(ticksInHive)` as saved in the `bees` list.
pub fn beehive_occupant(ticks_in_hive: i32) -> Compound {
    let mut entity = Compound::new();
    entity.insert("id", "minecraft:bee");
    let mut tag = Compound::new();
    tag.insert("entity_data", entity);
    tag.insert("ticks_in_hive", ticks_in_hive);
    tag.insert("min_ticks_in_hive", 600i32);
    tag
}

/// `MossyCarpetBlock.placeAt(level, pos, random, flags)`.
pub fn place_mossy_carpet(region: &mut Region, pos: BlockPos, flags: u32) {
    let blocks = &region.env.blocks;
    let simple = blocks.block(region.env.b.pale_moss_carpet).default_state;
    let adjusted = mossy_updated_state(region, simple, pos, true);
    region.set(pos, adjusted, flags);
    // `createTopperWithSideChance(level, pos, random::nextBoolean)` with the region's random.
    let mut random = std::mem::replace(
        &mut region.random,
        crate::random::AnyRandom::Legacy(crate::random::LegacyRandom::new(0)),
    );
    let top = mossy_topper(region, pos, &mut random);
    region.random = random;
    if !region.env.blocks.flags(top).is_air() {
        region.set(pos.above(), top, flags);
        let updated = mossy_updated_state(region, adjusted, pos, true);
        region.set(pos, updated, flags);
    }
}

const SIDES: [Direction; 4] = [
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::West,
];

fn mossy_has_faces(blocks: &BlockRegistry, s: BlockState) -> bool {
    prop(blocks, s, "bottom") == Some("true")
        || SIDES.iter().any(|d| prop(blocks, s, d.name()) != Some("none"))
}

/// `MossyCarpetBlock.getUpdatedState`.
fn mossy_updated_state(
    region: &Region,
    mut state: BlockState,
    pos: BlockPos,
    mut create_sides: bool,
) -> BlockState {
    let env = region.env;
    let blocks = &env.blocks;
    let carpet = env.b.pale_moss_carpet;
    let base = prop(blocks, state, "bottom") == Some("true");
    create_sides |= base;
    let mut above = None;
    let mut below = None;
    for d in SIDES {
        let face = d.name();
        let mut side = if can_attach_to(region, d, pos.relative(d, 1)) {
            if create_sides {
                "low"
            } else {
                prop(blocks, state, face).unwrap_or("none")
            }
        } else {
            "none"
        };
        if side == "low" {
            let a = *above.get_or_insert_with(|| region.get(pos.above()));
            if blocks.block_of(a) == carpet
                && prop(blocks, a, face) != Some("none")
                && prop(blocks, a, "bottom") != Some("true")
            {
                side = "tall";
            }
            if !base {
                let b = *below.get_or_insert_with(|| region.get(pos.below()));
                if blocks.block_of(b) == carpet && prop(blocks, b, face) == Some("none") {
                    side = "none";
                }
            }
        }
        state = blocks.with_property(state, face, side).unwrap_or(state);
    }
    state
}

/// `MossyCarpetBlock.createTopperWithSideChance`.
fn mossy_topper(region: &Region, pos: BlockPos, random: &mut dyn crate::random::RandomSource) -> BlockState {
    let env = region.env;
    let blocks = &env.blocks;
    let carpet = env.b.pale_moss_carpet;
    let above = pos.above();
    let previous = region.get(above);
    let is_carpet = blocks.block_of(previous) == carpet;
    if !((!is_carpet || prop(blocks, previous, "bottom") != Some("true"))
        && (is_carpet || blocks.flags(previous).can_be_replaced()))
    {
        return BlockState::AIR;
    }
    let no_base = blocks
        .with_property(blocks.block(carpet).default_state, "bottom", "false")
        .unwrap();
    let mut s = mossy_updated_state(region, no_base, above, true);
    for d in SIDES {
        if prop(blocks, s, d.name()) != Some("none") && !random.next_bool() {
            s = blocks.with_property(s, d.name(), "none").unwrap();
        }
    }
    if mossy_has_faces(blocks, s) && s != previous {
        s
    } else {
        BlockState::AIR
    }
}

fn mossy_can_survive(region: &Region, state: BlockState, pos: BlockPos) -> bool {
    let blocks = &region.env.blocks;
    let below = region.get(pos.below());
    if prop(blocks, state, "bottom") == Some("true") {
        !blocks.flags(below).is_air()
    } else {
        blocks.block_of(below) == blocks.block_of(state) && prop(blocks, below, "bottom") == Some("true")
    }
}

/// `BlockEntity.loadWithComponents(nbt)` on a fresh block entity followed by its save: template
/// fields replace the defaults (id and position stay), and containers with a loot table drop their
/// items and keep the seed (`RandomizableContainer.trySaveLootTable`).
pub fn load_block_entity(env: &FeatureEnv, state: BlockState, tag: &mut Compound, nbt: &Compound) {
    // Start from the block entity as loaded from empty NBT (load defaults differ from a fresh one).
    if let Some(base) = env.blocks.loaded_block_entity(env.blocks.block_of(state)) {
        let (x, y, z) = (tag.get_i32("x"), tag.get_i32("y"), tag.get_i32("z"));
        *tag = base.clone();
        for (k, v) in [("x", x), ("y", y), ("z", z)] {
            if let Some(v) = v {
                tag.insert(k, v);
            }
        }
    }
    for (k, v) in nbt.iter() {
        if matches!(k, "x" | "y" | "z" | "id") {
            continue;
        }
        // Fields are read with the block entity's own types (`getIntOr` reads a short as an int).
        let v = match (tag.get(k), v) {
            (Some(default), value) => coerce_number(default, value).unwrap_or_else(|| value.clone()),
            (None, value) => value.clone(),
        };
        tag.insert(k, v);
    }
    // `ComponentSerialization`: plain text components are saved as strings.
    for side in ["front_text", "back_text"] {
        if let Some(chunker_nbt::Tag::Compound(text)) = tag.get_mut(side) {
            for list in ["messages", "filtered_messages"] {
                if let Some(chunker_nbt::Tag::List(l)) = text.get(list) {
                    let mut out = chunker_nbt::List::new();
                    for m in l.iter() {
                        match m {
                            chunker_nbt::Tag::Compound(c) if c.len() == 1 && c.get_str("text").is_some() => {
                                out.push(c.get_str("text").unwrap());
                            }
                            other => out.push(other.clone()),
                        }
                    }
                    text.insert(list, out);
                }
            }
            // `SignText.filteredMessagesForSerialization`: only saved when they differ.
            if text.get("filtered_messages").is_some()
                && text.get("filtered_messages") == text.get("messages")
            {
                text.remove("filtered_messages");
            }
        }
    }
    // `VaultConfig.CODEC` leaves out fields equal to `VaultConfig.DEFAULT`.
    if tag.get_str("id") == Some("minecraft:vault")
        && let Some(chunker_nbt::Tag::Compound(config)) = tag.get_mut("config")
    {
        if config.get_str("loot_table") == Some("minecraft:chests/trial_chambers/reward") {
            config.remove("loot_table");
        }
        if config.get_f64("activation_range") == Some(4.0) {
            config.remove("activation_range");
        }
        if config.get_f64("deactivation_range") == Some(4.5) {
            config.remove("deactivation_range");
        }
    }
    if is_randomizable_container(tag) || matches!(tag.get_str("id"), Some("minecraft:brushable_block")) {
        if tag.contains_key("LootTable") {
            if is_randomizable_container(tag) {
                tag.remove("Items");
            }
            if tag.get_i64("LootTableSeed") == Some(0) {
                tag.remove("LootTableSeed");
            }
        } else {
            tag.remove("LootTableSeed");
        }
    }
}

/// `value` converted to the numeric NBT type of `default` (`None` if either is not a number).
fn coerce_number(default: &chunker_nbt::Tag, value: &chunker_nbt::Tag) -> Option<chunker_nbt::Tag> {
    use chunker_nbt::Tag;
    let v: f64 = match value {
        Tag::Byte(x) => *x as f64,
        Tag::Short(x) => *x as f64,
        Tag::Int(x) => *x as f64,
        Tag::Long(x) => *x as f64,
        Tag::Float(x) => *x as f64,
        Tag::Double(x) => *x,
        _ => return None,
    };
    Some(match default {
        Tag::Byte(_) => Tag::Byte(v as i8),
        Tag::Short(_) => Tag::Short(v as i16),
        Tag::Int(_) => Tag::Int(v as i32),
        Tag::Long(_) => match value {
            Tag::Long(x) => Tag::Long(*x),
            _ => Tag::Long(v as i64),
        },
        Tag::Float(_) => Tag::Float(v as f32),
        Tag::Double(_) => Tag::Double(v),
        _ => return None,
    })
}

/// Block entities implementing `RandomizableContainer`.
pub fn is_randomizable_container(be: &Compound) -> bool {
    matches!(be.get("id"), Some(chunker_nbt::Tag::String(s)) if matches!(s.as_str(),
        "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" | "minecraft:dispenser" | "minecraft:dropper"
        | "minecraft:hopper" | "minecraft:shulker_box" | "minecraft:decorated_pot" | "minecraft:crafter"))
}

/// `SpawnerBlockEntity.setEntityId` on a fresh spawner (empty spawn potentials).
pub fn set_spawner_entity(be: &mut Compound, entity: &str) {
    let mut e = Compound::new();
    e.insert("id", entity);
    let mut data = Compound::new();
    data.insert("entity", e);
    be.insert("SpawnData", data);
}

// --- connecting blocks: stairs, bars and panes, fences, walls -----------------------------------

/// Class facts used by connection rules.
pub mod class_bits {
    pub const IRON_BARS: u8 = 1;
    pub const FENCE_GATE: u8 = 2;
    pub const WALL: u8 = 4;
    pub const LEAVES: u8 = 8;
    pub const STAIRS: u8 = 16;
    pub const DOOR: u8 = 32;
}

fn has_class(env: &FeatureEnv, state: BlockState, bit: u8) -> bool {
    env.behaviours.class_bits[env.blocks.block_of(state).0 as usize] & bit != 0
}

/// `ChestBlock.getConnectedDirection`.
fn chest_connected_direction(blocks: &BlockRegistry, state: BlockState) -> Option<Direction> {
    let facing = Direction::by_name(prop(blocks, state, "facing")?)?;
    Some(if prop(blocks, state, "type") == Some("left") {
        facing.clockwise()
    } else {
        facing.counter_clockwise()
    })
}

/// `ChestBlock.updateShape` (+ `CopperChestBlock.updateShape`).
fn chest_update_shape(
    region: &mut Region,
    kind: Kind,
    state: BlockState,
    pos: BlockPos,
    direction: Direction,
    neighbour: BlockState,
) -> BlockState {
    let env = region.env;
    let blocks = &env.blocks;
    if prop(blocks, state, "waterlogged") == Some("true") {
        region.schedule_fluid_tick(pos, Fluid::Water);
    }
    // `chestCanConnectTo`.
    let connects = if kind == Kind::CopperChest {
        env.behaviours.kind(blocks.block_of(neighbour)) == Kind::CopperChest
            && blocks.has_property(neighbour, "type")
    } else {
        blocks.block_of(neighbour) == blocks.block_of(state)
    };
    let mut out = state;
    if connects && direction.axis() != Axis::Y {
        let neighbour_type = prop(blocks, neighbour, "type").unwrap_or("single");
        if prop(blocks, state, "type") == Some("single")
            && neighbour_type != "single"
            && prop(blocks, state, "facing") == prop(blocks, neighbour, "facing")
            && chest_connected_direction(blocks, neighbour) == Some(direction.opposite())
        {
            let opposite = if neighbour_type == "left" { "right" } else { "left" };
            out = set_prop(blocks, state, "type", opposite);
        }
    } else if chest_connected_direction(blocks, state) == Some(direction) {
        out = set_prop(blocks, state, "type", "single");
    }
    if kind == Kind::CopperChest
        && connects
        && prop(blocks, out, "type") != Some("single")
        && chest_connected_direction(blocks, out) == Some(direction)
    {
        let target = blocks
            .default_state(&blocks.block(blocks.block_of(neighbour)).name.clone())
            .unwrap_or(neighbour);
        out = super::provider::copy_properties(blocks, target, out);
    }
    out
}

fn set_prop(blocks: &BlockRegistry, state: BlockState, name: &str, value: &str) -> BlockState {
    blocks.with_property(state, name, value).unwrap_or(state)
}

fn bool_str(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// `Block.isExceptionForConnection`.
fn is_exception_for_connection(env: &FeatureEnv, state: BlockState) -> bool {
    let b = &env.b;
    let id = env.blocks.block_of(state);
    has_class(env, state, class_bits::LEAVES)
        || id == b.barrier
        || id == b.carved_pumpkin
        || id == b.jack_o_lantern
        || id == b.melon
        || id == b.pumpkin
        || env.t.shulker_boxes.contains(state)
}

/// `FenceGateBlock.connectsToDirection`.
fn gate_connects(env: &FeatureEnv, state: BlockState, d: Direction) -> bool {
    has_class(env, state, class_bits::FENCE_GATE)
        && prop(&env.blocks, state, "facing")
            .and_then(Direction::by_name)
            .is_some_and(|f| f.axis() == d.clockwise().axis())
}

fn waterlogged_tick(region: &mut Region, state: BlockState, pos: BlockPos) {
    if prop(&region.env.blocks, state, "waterlogged") == Some("true") {
        region.schedule_fluid_tick(pos, Fluid::Water);
    }
}

/// `StairBlock.getStairsShape`.
fn stairs_shape(region: &Region, state: BlockState, pos: BlockPos) -> &'static str {
    let env = region.env;
    let blocks = &env.blocks;
    let facing = prop(blocks, state, "facing")
        .and_then(Direction::by_name)
        .unwrap_or(Direction::North);
    let half = prop(blocks, state, "half");
    let is_stairs = |s: BlockState| has_class(env, s, class_bits::STAIRS);
    let facing_of = |s: BlockState| {
        prop(blocks, s, "facing")
            .and_then(Direction::by_name)
            .unwrap_or(Direction::North)
    };
    let can_take = |d: Direction| {
        let n = region.get(pos.relative(d, 1));
        !is_stairs(n) || facing_of(n) != facing || prop(blocks, n, "half") != half
    };
    let behind = region.get(pos.relative(facing, 1));
    if is_stairs(behind) && prop(blocks, behind, "half") == half {
        let bf = facing_of(behind);
        if bf.axis() != facing.axis() && can_take(bf.opposite()) {
            return if bf == facing.counter_clockwise() {
                "outer_left"
            } else {
                "outer_right"
            };
        }
    }
    let front = region.get(pos.relative(facing.opposite(), 1));
    if is_stairs(front) && prop(blocks, front, "half") == half {
        let ff = facing_of(front);
        if ff.axis() != facing.axis() && can_take(ff) {
            return if ff == facing.counter_clockwise() {
                "inner_left"
            } else {
                "inner_right"
            };
        }
    }
    "straight"
}

/// `updateShape` of stairs, iron bars / panes, fences and walls (`None` for other kinds).
fn connecting_update_shape(
    region: &mut Region,
    kind: Kind,
    state: BlockState,
    pos: BlockPos,
    d: Direction,
    neighbour_pos: BlockPos,
    neighbour: BlockState,
) -> Option<BlockState> {
    let env = region.env;
    let blocks = &env.blocks;
    let sturdy = || is_face_sturdy(env, neighbour, d.opposite());
    let _ = neighbour_pos;
    Some(match kind {
        Kind::Stairs => {
            waterlogged_tick(region, state, pos);
            if d.is_horizontal() {
                set_prop(blocks, state, "shape", stairs_shape(region, state, pos))
            } else {
                state
            }
        }
        Kind::CrossCollision => {
            waterlogged_tick(region, state, pos);
            if d.is_horizontal() {
                // `IronBarsBlock.attachsTo`
                let attaches = (!is_exception_for_connection(env, neighbour) && sturdy())
                    || has_class(env, neighbour, class_bits::IRON_BARS)
                    || env.t.walls.contains(neighbour);
                set_prop(blocks, state, d.name(), bool_str(attaches))
            } else {
                state
            }
        }
        Kind::Fence => {
            waterlogged_tick(region, state, pos);
            if d.is_horizontal() {
                // `FenceBlock.connectsTo` / `isSameFence`
                let me_wooden = env
                    .t
                    .wooden_fences
                    .contains(blocks.block(blocks.block_of(state)).default_state);
                let same =
                    env.t.fences.contains(neighbour) && env.t.wooden_fences.contains(neighbour) == me_wooden;
                let connects = (!is_exception_for_connection(env, neighbour) && sturdy())
                    || same
                    || gate_connects(env, neighbour, d.opposite());
                set_prop(blocks, state, d.name(), bool_str(connects))
            } else {
                state
            }
        }
        Kind::Wall => {
            waterlogged_tick(region, state, pos);
            match d {
                Direction::Down => state,
                Direction::Up => {
                    let c = |side: &str| prop(blocks, state, side) != Some("none");
                    wall_shape(
                        env,
                        state,
                        neighbour,
                        [c("north"), c("east"), c("south"), c("west")],
                    )
                }
                _ => {
                    let connects = env.t.walls.contains(neighbour)
                        || (!is_exception_for_connection(env, neighbour) && sturdy())
                        || has_class(env, neighbour, class_bits::IRON_BARS)
                        || gate_connects(env, neighbour, d.opposite());
                    let sides = [
                        Direction::North,
                        Direction::East,
                        Direction::South,
                        Direction::West,
                    ]
                    .map(|s| {
                        if s == d {
                            connects
                        } else {
                            prop(blocks, state, s.name()) != Some("none")
                        }
                    });
                    let above = region.get(pos.above());
                    wall_shape(env, state, above, sides)
                }
            }
        }
        _ => return None,
    })
}

/// `WallBlock.updateShape(level, state, topPos, topNeighbour, north, east, south, west)`.
fn wall_shape(env: &FeatureEnv, state: BlockState, above: BlockState, sides: [bool; 4]) -> BlockState {
    let blocks = &env.blocks;
    let names = ["north", "east", "south", "west"];
    let mut s = state;
    for (i, name) in names.iter().enumerate() {
        let v = if !sides[i] {
            "none"
        } else if blocks.covers_wall_side(above, i) {
            "tall"
        } else {
            "low"
        };
        s = set_prop(blocks, s, name, v);
    }
    // `shouldRaisePost`
    let side = |n: &str| prop(blocks, s, n).unwrap_or("none");
    let raise = if has_class(env, above, class_bits::WALL) && prop(blocks, above, "up") == Some("true") {
        true
    } else {
        let (n, e, so, w) = (side("north"), side("east"), side("south"), side("west"));
        let (nn, en, sn, wn) = (n == "none", e == "none", so == "none", w == "none");
        let corner = (nn && sn && wn && en) || nn != sn || wn != en;
        if corner {
            true
        } else {
            let high = (n == "tall" && so == "tall") || (e == "tall" && w == "tall");
            !high && (env.t.wall_post_override.contains(above) || blocks.covers_wall_post(above))
        }
    };
    set_prop(blocks, s, "up", bool_str(raise))
}
