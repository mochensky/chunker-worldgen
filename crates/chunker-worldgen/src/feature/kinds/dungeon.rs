//! `MonsterRoomFeature` (dungeons) and the container helpers it shares with structures.

use chunker_data::blocks::BlockState;

use crate::feature::Region;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

/// `StructurePiece.reorient`: faces a chest away from its single solid neighbour.
pub fn reorient(region: &Region, pos: BlockPos, state: BlockState) -> BlockState {
    let blocks = &region.env.blocks;
    let solid = |p: BlockPos| blocks.flags(region.get(p)).is_solid_render();
    let mut neighbour = None;
    for d in Direction::HORIZONTAL {
        let p = pos.relative(d, 1);
        if region.is(p, region.env.b.chest) {
            return state;
        }
        if solid(p) {
            if neighbour.is_some() {
                neighbour = None;
                break;
            }
            neighbour = Some(d);
        }
    }
    let facing = match neighbour {
        Some(d) => d.opposite(),
        None => {
            let mut lock = blocks
                .get_property(state, "facing")
                .and_then(Direction::by_name)
                .unwrap_or(Direction::North);
            if solid(pos.relative(lock, 1)) {
                lock = lock.opposite();
            }
            if solid(pos.relative(lock, 1)) {
                lock = lock.clockwise();
            }
            if solid(pos.relative(lock, 1)) {
                lock = lock.opposite();
            }
            lock
        }
    };
    blocks
        .with_property(state, "facing", facing.name())
        .unwrap_or(state)
}

/// `RandomizableContainer.setBlockEntityLootTable`.
pub fn set_loot_table(region: &mut Region, random: &mut dyn RandomSource, pos: BlockPos, table: &str) {
    if let Some(be) = region.block_entity(pos)
        && crate::feature::behaviour::is_randomizable_container(be)
    {
        // `trySaveLootTable` replaces the item list.
        be.remove("Items");
        be.insert("LootTable", table);
        be.insert("LootTableSeed", random.next_long());
    }
}

/// `MonsterRoomFeature.place`.
pub fn monster_room(region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
    let env = region.env;
    let b = &env.b;
    let blocks = &env.blocks;
    let solid = |r: &Region, p: BlockPos| blocks.flags(r.get(p)).is_solid();
    let xr = random.next_int_bounded(2) + 2;
    let (min_x, max_x) = (-xr - 1, xr + 1);
    let zr = random.next_int_bounded(2) + 2;
    let (min_z, max_z) = (-zr - 1, zr + 1);
    let mut holes = 0;
    for dx in min_x..=max_x {
        for dy in -1..=4 {
            for dz in min_z..=max_z {
                let p = origin.offset(dx, dy, dz);
                let s = solid(region, p);
                if (dy == -1 || dy == 4) && !s {
                    return false;
                }
                if (dx == min_x || dx == max_x || dz == min_z || dz == max_z)
                    && dy == 0
                    && region.is_empty_block(p)
                    && region.is_empty_block(p.above())
                {
                    holes += 1;
                }
            }
        }
    }
    if !(1..=5).contains(&holes) {
        return false;
    }
    // `Feature.safeSetBlock` with `!#features_cannot_replace`.
    let safe_set = |r: &mut Region, p: BlockPos, s: BlockState| {
        if !env.t.features_cannot_replace.contains(r.get(p)) {
            r.set(p, s, 2);
        }
    };
    let cave_air = b.cave_air_state;
    let mossy = blocks.block(b.mossy_cobblestone).default_state;
    let cobble = blocks.block(b.cobblestone).default_state;
    for dx in min_x..=max_x {
        let mut dy = 3;
        while dy >= -1 {
            for dz in min_z..=max_z {
                let p = origin.offset(dx, dy, dz);
                let wall = region.get(p);
                let wall_block = blocks.block_of(wall);
                if dx == min_x || dy == -1 || dz == min_z || dx == max_x || dy == 4 || dz == max_z {
                    if p.y >= region.min_y() && !solid(region, p.below()) {
                        region.set(p, cave_air, 2);
                    } else if blocks.flags(wall).is_solid() && wall_block != b.chest {
                        if dy == -1 && random.next_int_bounded(4) != 0 {
                            safe_set(region, p, mossy);
                        } else {
                            safe_set(region, p, cobble);
                        }
                    }
                } else if wall_block != b.chest && wall_block != b.spawner {
                    safe_set(region, p, cave_air);
                }
            }
            dy -= 1;
        }
    }
    let chest = blocks.block(b.chest).default_state;
    for _ in 0..2 {
        for _ in 0..3 {
            let xc = origin.x + random.next_int_bounded(xr * 2 + 1) - xr;
            let zc = origin.z + random.next_int_bounded(zr * 2 + 1) - zr;
            let p = BlockPos::new(xc, origin.y, zc);
            if region.is_empty_block(p) {
                let walls = Direction::HORIZONTAL
                    .iter()
                    .filter(|d| solid(region, p.relative(**d, 1)))
                    .count();
                if walls == 1 {
                    let s = reorient(region, p, chest);
                    safe_set(region, p, s);
                    set_loot_table(region, random, p, "minecraft:chests/simple_dungeon");
                    break;
                }
            }
        }
    }
    let spawner = blocks.block(b.spawner).default_state;
    safe_set(region, origin, spawner);
    const MOBS: [&str; 4] = [
        "minecraft:skeleton",
        "minecraft:zombie",
        "minecraft:zombie",
        "minecraft:spider",
    ];
    let is_spawner = region.block_entity(origin).is_some_and(
        |be| matches!(be.get("id"), Some(chunker_nbt::Tag::String(s)) if s == "minecraft:mob_spawner"),
    );
    if is_spawner {
        let mob = MOBS[random.next_int_bounded(4) as usize];
        if let Some(be) = region.block_entity(origin) {
            crate::feature::behaviour::set_spawner_entity(be, mob);
        }
    }
    true
}
