//! Blocks and block tags that vanilla feature code refers to directly (`Blocks.X`, `BlockTags.X`).

use std::sync::Arc;

use anyhow::{Context, Result};
use chunker_data::Identifier;
use chunker_data::blocks::{BlockId, BlockRegistry, BlockState};

use crate::block_set::StateSet;
use crate::registry::WorldgenRegistries;

macro_rules! known_blocks {
    ($($field:ident),* $(,)?) => {
        /// `Blocks.*` ids used by hardcoded feature logic.
        pub struct KnownBlocks {
            $(pub $field: BlockId,)*
            pub void_air_state: BlockState,
            pub cave_air_state: BlockState,
        }

        impl KnownBlocks {
            pub fn load(reg: &BlockRegistry) -> Result<Self> {
                let get = |name: &str| {
                    reg.block_by_name(&Identifier::vanilla(name)).with_context(|| format!("unknown block minecraft:{name}"))
                };
                Ok(Self {
                    $($field: get(stringify!($field))?,)*
                    void_air_state: reg.vanilla_default("void_air"),
                    cave_air_state: reg.vanilla_default("cave_air"),
                })
            }
        }
    };
}

known_blocks! {
    air, cave_air, void_air, stone, water, lava, ice, packed_ice, blue_ice, snow, snow_block, powder_snow,
    grass_block, dirt, coarse_dirt, podzol, mycelium, rooted_dirt, moss_block, mud, farmland, dirt_path,
    bedrock, obsidian, cobblestone, mossy_cobblestone, chest, spawner, barrier, sand, red_sand, gravel,
    sandstone, clay, magma_block, soul_sand, soul_soil, netherrack, basalt, blackstone, end_stone,
    vine, oak_log, oak_leaves, beehive, bee_nest, cocoa, tall_grass, large_fern, fern, short_grass,
    sugar_cane, cactus, kelp, kelp_plant, seagrass, tall_seagrass, pumpkin, melon, lily_pad,
    brown_mushroom, red_mushroom, brown_mushroom_block, red_mushroom_block, mushroom_stem,
    glow_lichen, sculk_vein, amethyst_block, budding_amethyst, calcite, smooth_basalt, amethyst_cluster,
    small_amethyst_bud, medium_amethyst_bud, large_amethyst_bud, pointed_dripstone, dripstone_block,
    cave_vines, cave_vines_plant, hanging_roots, moss_carpet, pale_moss_carpet, pale_hanging_moss, shelf_mushroom,
    big_dripleaf, big_dripleaf_stem, small_dripleaf, spore_blossom, azalea, flowering_azalea,
    bamboo, bamboo_sapling, sea_pickle, sweet_berry_bush, chorus_plant, chorus_flower, end_rod,
    crying_obsidian, iron_bars, end_portal, end_gateway, bubble_column, fire, soul_fire,
    creaking_heart, resin_clump, leaf_litter, firefly_bush, bush, dead_bush, mangrove_roots,
    muddy_mangrove_roots, mangrove_propagule, nether_wart_block, warped_wart_block, shroomlight,
    weeping_vines, weeping_vines_plant, twisting_vines, twisting_vines_plant, crimson_fungus, warped_fungus,
    crimson_nylium, warped_nylium, crimson_stem, warped_stem, glowstone, nether_bricks, nether_brick_fence,
    nether_brick_stairs, nether_wart,
    infested_stone, infested_deepslate, deepslate, tuff, sculk, sculk_catalyst, sculk_shrieker, sculk_sensor,
    prismarine, prismarine_bricks, dark_prismarine, sea_lantern, wet_sponge, gold_block,
    jigsaw, structure_void, structure_block, stone_bricks, chiseled_stone_bricks, cracked_stone_bricks, mossy_stone_bricks,
    stone_brick_stairs, mossy_stone_brick_stairs, stone_slab, stone_brick_slab, mossy_stone_brick_slab,
    mossy_stone_brick_wall, torch, wall_torch, oak_fence, spruce_fence, dark_oak_fence, pale_oak_fence, acacia_fence,
    birch_fence, jungle_fence, ladder, dispenser, andesite, granite, diorite, jungle_leaves, carved_pumpkin, jack_o_lantern, iron_chain,
}

macro_rules! known_tags {
    ($($field:ident => $tag:literal),* $(,)?) => {
        /// `BlockTags.*` used by hardcoded feature logic.
        pub struct KnownTags {
            $(pub $field: Arc<StateSet>,)*
        }

        impl KnownTags {
            pub fn load(reg: &WorldgenRegistries) -> Result<Self> {
                Ok(Self {
                    $($field: reg.block_tag(&Identifier::vanilla($tag))?,)*
                })
            }
        }
    };
}

known_tags! {
    air => "air",
    leaves => "leaves",
    logs => "logs",
    dirt => "dirt",
    sand => "sand",
    snow => "snow",
    ice => "ice",
    replaceable_by_trees => "replaceable_by_trees",
    features_cannot_replace => "features_cannot_replace",
    supports_vegetation => "supports_vegetation",
    supports_sugar_cane => "supports_sugar_cane",
    supports_sugar_cane_adjacently => "supports_sugar_cane_adjacently",
    overrides_mushroom_light_requirement => "overrides_mushroom_light_requirement",
    cannot_support_snow_layer => "cannot_support_snow_layer",
    support_override_snow_layer => "support_override_snow_layer",
    prevents_nearby_leaf_decay => "prevents_nearby_leaf_decay",
    base_stone_overworld => "base_stone_overworld",
    geode_invalid_blocks => "geode_invalid_blocks",
    lava_pool_stone_cannot_replace => "lava_pool_stone_cannot_replace",
    replaceable => "replaceable",
    wool => "wool",
    enables_bubble_column_drag_down => "enables_bubble_column_drag_down",
    enables_bubble_column_push_up => "enables_bubble_column_push_up",
    beneath_bamboo_podzol_replaceable => "beneath_bamboo_podzol_replaceable",
    replaceable_by_mushrooms => "replaceable_by_mushrooms",
    supports_bamboo => "supports_bamboo",
    fire => "fire",
    stairs => "stairs",
    slabs => "slabs",
    walls => "walls",
    fences => "fences",
    wooden_fences => "wooden_fences",
    wall_post_override => "wall_post_override",
    shulker_boxes => "shulker_boxes",
    supports_chorus_plant => "supports_chorus_plant",
    sculk_growth_inhibitors => "sculk_growth_inhibitors",
    sculk_replaceable => "sculk_replaceable",
    sculk_replaceable_world_gen => "sculk_replaceable_world_gen",
}
