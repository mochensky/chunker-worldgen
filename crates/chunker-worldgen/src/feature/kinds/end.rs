//! End feature types: spikes, islands, gateways, the spawn platform, chorus plants.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use chunker_data::blocks::BlockState;
use chunker_nbt::{Compound, List};
use serde_json::Value;

use crate::codec::field;
use crate::feature::Region;
use crate::math::{self, trig};
use crate::random::LegacyRandom;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

/// `EndSpikeFeature.EndSpike`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndSpikeDef {
    pub center_x: i32,
    pub center_z: i32,
    pub radius: i32,
    pub height: i32,
    pub guarded: bool,
}

impl EndSpikeDef {
    fn parse(json: &Value) -> Result<Self> {
        let int = |name: &str| json.get(name).and_then(Value::as_i64).unwrap_or(0) as i32;
        Ok(Self {
            center_x: int("centerX"),
            center_z: int("centerZ"),
            radius: int("radius"),
            height: int("height"),
            guarded: json.get("guarded").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// `EndSpikeFeature.getSpikesForLevel` (+ `SpikeCacheLoader.load`).
pub fn spikes_for_seed(seed: i64) -> Vec<EndSpikeDef> {
    let key = LegacyRandom::new(seed).next_long() & 65535;
    let mut random = LegacyRandom::new(key);
    // `Util.toShuffledList(IntStream.range(0, 10), random)`.
    let mut sizes: Vec<i32> = (0..10).collect();
    for i in (2..=10).rev() {
        let swap_to = random.next_int_bounded(i as i32) as usize;
        sizes.swap(i - 1, swap_to);
    }
    (0..10)
        .map(|i| {
            let angle = 2.0 * (-std::f64::consts::PI + (std::f64::consts::PI / 10.0) * i as f64);
            let size = sizes[i];
            EndSpikeDef {
                center_x: math::floor(42.0 * trig::cos(angle)),
                center_z: math::floor(42.0 * trig::sin(angle)),
                radius: 2 + size / 3,
                height: 76 + size * 3,
                guarded: size == 1 || size == 2,
            }
        })
        .collect()
}

/// `EndSpikeFeature`.
pub struct EndSpike {
    spikes: Vec<EndSpikeDef>,
    crystal_invulnerable: bool,
    beam_target: Option<BlockPos>,
    /// Spikes of the level seed (empty `spikes`), computed once.
    level_spikes: Mutex<Option<(i64, Arc<Vec<EndSpikeDef>>)>>,
}

fn parse_pos(v: &Value) -> Option<BlockPos> {
    let a = v.as_array()?;
    Some(BlockPos::new(
        a.first()?.as_i64()? as i32,
        a.get(1)?.as_i64()? as i32,
        a.get(2)?.as_i64()? as i32,
    ))
}

impl EndSpike {
    pub fn parse(json: &Value) -> Result<Self> {
        let spikes = match json.get("spikes") {
            Some(Value::Array(a)) => a.iter().map(EndSpikeDef::parse).collect::<Result<_>>()?,
            _ => Vec::new(),
        };
        Ok(Self {
            spikes,
            crystal_invulnerable: json
                .get("crystal_invulnerable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            beam_target: json.get("crystal_beam_target").and_then(parse_pos),
            level_spikes: Mutex::new(None),
        })
    }

    fn spikes(&self, seed: i64) -> Arc<Vec<EndSpikeDef>> {
        if !self.spikes.is_empty() {
            return Arc::new(self.spikes.clone());
        }
        let mut cache = self.level_spikes.lock().unwrap();
        if let Some((s, v)) = cache.as_ref()
            && *s == seed
        {
            return v.clone();
        }
        let v = Arc::new(spikes_for_seed(seed));
        *cache = Some((seed, v.clone()));
        v
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        for spike in self.spikes(region.env.seed).iter() {
            if origin.x >> 4 == spike.center_x >> 4 && origin.z >> 4 == spike.center_z >> 4 {
                self.place_spike(region, random, spike);
            }
        }
        true
    }

    fn place_spike(&self, region: &mut Region, random: &mut dyn RandomSource, spike: &EndSpikeDef) {
        let env = region.env;
        let b = &env.b;
        let obsidian = env.blocks.block(b.obsidian).default_state;
        let r = spike.radius;
        // `BlockPos.betweenClosed`: x fastest, then y, then z.
        for z in spike.center_z - r..=spike.center_z + r {
            for y in region.min_y()..=spike.height + 10 {
                for x in spike.center_x - r..=spike.center_x + r {
                    let p = BlockPos::new(x, y, z);
                    let (dx, dz) = ((x - spike.center_x) as f64, (z - spike.center_z) as f64);
                    if dx * dx + dz * dz <= (r * r + 1) as f64 && y < spike.height {
                        region.set_and_update(p, obsidian);
                    } else if y > 65 {
                        region.set_and_update(p, BlockState::AIR);
                    }
                }
            }
        }
        if spike.guarded {
            let bars = env.blocks.block(b.iron_bars).default_state;
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    for dy in 0..=3 {
                        let x_side = dx.abs() == 2;
                        let z_side = dz.abs() == 2;
                        let top = dy == 3;
                        if x_side || z_side || top {
                            let x_edge = dx == -2 || dx == 2 || top;
                            let z_edge = dz == -2 || dz == 2 || top;
                            let set = |s: BlockState, n: &str, v: bool| {
                                env.blocks
                                    .with_property(s, n, if v { "true" } else { "false" })
                                    .unwrap()
                            };
                            let mut s = set(bars, "north", x_edge && dz != -2);
                            s = set(s, "south", x_edge && dz != 2);
                            s = set(s, "west", z_edge && dx != -2);
                            s = set(s, "east", z_edge && dx != 2);
                            region.set_and_update(
                                BlockPos::new(spike.center_x + dx, spike.height + dy, spike.center_z + dz),
                                s,
                            );
                        }
                    }
                }
            }
        }
        let y_rot = random.next_float() * 360.0;
        let crystal = BlockPos::new(spike.center_x, spike.height + 1, spike.center_z);
        region.add_entity(
            crystal.x,
            crystal.z,
            self.crystal_nbt(region.env.seed, spike, y_rot),
        );
        region.set_and_update(crystal.below(), env.blocks.block(b.bedrock).default_state);
        // `FireBlock.getState`: bedrock below is sturdy, so plain fire.
        region.set_and_update(crystal, env.blocks.block(b.fire).default_state);
    }

    /// `EndCrystal.save` of a freshly created crystal.
    fn crystal_nbt(&self, seed: i64, spike: &EndSpikeDef, y_rot: f32) -> Compound {
        let mut tag = Compound::new();
        tag.insert("id", "minecraft:end_crystal");
        let mut pos = List::new();
        pos.push(spike.center_x as f64 + 0.5);
        pos.push((spike.height + 1) as f64);
        pos.push(spike.center_z as f64 + 0.5);
        tag.insert("Pos", pos);
        let mut motion = List::new();
        for _ in 0..3 {
            motion.push(0.0f64);
        }
        tag.insert("Motion", motion);
        let mut rotation = List::new();
        rotation.push(y_rot);
        rotation.push(0.0f32);
        tag.insert("Rotation", rotation);
        tag.insert("fall_distance", 0.0f64);
        tag.insert("Fire", 0i16);
        tag.insert("Air", 300i16);
        tag.insert("OnGround", false);
        tag.insert("Invulnerable", self.crystal_invulnerable);
        tag.insert("PortalCooldown", 0i32);
        // Vanilla draws the UUID from the level random; any unique value works, keep it reproducible.
        tag.insert(
            "UUID",
            entity_uuid(seed, spike.center_x, spike.height + 1, spike.center_z),
        );
        if let Some(t) = self.beam_target {
            tag.insert("beam_target", vec![t.x, t.y, t.z]);
        }
        tag.insert("ShowBottom", true);
        tag
    }
}

/// A version 4 UUID (as `UUIDUtil.CODEC` int array) derived from the seed and a block position.
pub fn entity_uuid(seed: i64, x: i32, y: i32, z: i32) -> Vec<i32> {
    let mut r = LegacyRandom::new(
        seed ^ (x as i64).wrapping_mul(3129871) ^ (z as i64).wrapping_mul(116129781) ^ (y as i64) << 40,
    );
    let most = (r.next_long() & !0xF000) | 0x4000;
    let least = (r.next_long() & !(0b11 << 62)) | (0b10 << 62);
    vec![
        (most >> 32) as i32,
        most as i32,
        (least >> 32) as i32,
        least as i32,
    ]
}

/// `EndIslandFeature`.
pub fn end_island(region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
    let end_stone = region.env.blocks.block(region.env.b.end_stone).default_state;
    let mut size = random.next_int_bounded(3) as f32 + 4.0;
    let mut y = 0;
    while size > 0.5 {
        for x in math::floor_f32(-size)..=math::ceil(size as f64) {
            for z in math::floor_f32(-size)..=math::ceil(size as f64) {
                if ((x * x + z * z) as f32) <= (size + 1.0) * (size + 1.0) {
                    region.set_and_update(origin.offset(x, y, z), end_stone);
                }
            }
        }
        size -= random.next_int_bounded(2) as f32 + 0.5;
        y -= 1;
    }
    true
}

/// `EndPlatformFeature.createEndPlatform(level, origin, false)`.
pub fn end_platform(region: &mut Region, origin: BlockPos) -> bool {
    let env = region.env;
    let (obsidian, air) = (env.b.obsidian, env.b.air);
    for dz in -2..=2 {
        for dx in -2..=2 {
            for dy in -1..3 {
                let p = origin.offset(dx, dy, dz);
                let block = if dy == -1 { obsidian } else { air };
                if !region.is(p, block) {
                    region.set_and_update(p, env.blocks.block(block).default_state);
                }
            }
        }
    }
    true
}

/// `EndGatewayFeature`.
pub struct EndGateway {
    exit: Option<BlockPos>,
    exact: bool,
}

impl EndGateway {
    pub fn parse(json: &Value) -> Result<Self> {
        Ok(Self {
            exit: json.get("exit").and_then(parse_pos),
            exact: field(json, "exact")?.as_bool().unwrap_or(false),
        })
    }

    pub fn place(&self, region: &mut Region, origin: BlockPos) -> bool {
        let env = region.env;
        let b = &env.b;
        let state = |id| env.blocks.block(id).default_state;
        for z in origin.z - 1..=origin.z + 1 {
            for y in origin.y - 2..=origin.y + 2 {
                for x in origin.x - 1..=origin.x + 1 {
                    let p = BlockPos::new(x, y, z);
                    let same_x = x == origin.x;
                    let same_y = y == origin.y;
                    let same_z = z == origin.z;
                    let end = (y - origin.y).abs() == 2;
                    if same_x && same_y && same_z {
                        region.set_and_update(p, state(b.end_gateway));
                        if let Some(exit) = self.exit
                            && let Some(tag) = region.block_entity(p)
                            && tag.get_str("id") == Some("minecraft:end_gateway")
                        {
                            tag.insert("exit_portal", vec![exit.x, exit.y, exit.z]);
                            if self.exact {
                                tag.insert("ExactTeleport", true);
                            }
                        }
                    } else if same_y {
                        region.set_and_update(p, BlockState::AIR);
                    } else if (end && same_x && same_z) || ((same_x || same_z) && !end) {
                        region.set_and_update(p, state(b.bedrock));
                    } else {
                        region.set_and_update(p, BlockState::AIR);
                    }
                }
            }
        }
        true
    }
}

/// `ChorusPlantFeature`.
pub fn chorus_plant(region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
    if region.is_empty_block(origin)
        && region
            .env
            .t
            .supports_chorus_plant
            .contains(region.get(origin.below()))
    {
        generate_chorus(region, origin, random, 8);
        true
    } else {
        false
    }
}

/// `ChorusPlantBlock.getStateWithConnections` for a chorus plant.
fn chorus_with_connections(region: &Region, p: BlockPos) -> BlockState {
    let env = region.env;
    let (plant, flower) = (env.b.chorus_plant, env.b.chorus_flower);
    let connects = |q: BlockPos| {
        let id = region.block_of(q);
        id == plant || id == flower
    };
    let blocks = &env.blocks;
    let set = |s: BlockState, n: &str, v: bool| {
        blocks
            .with_property(s, n, if v { "true" } else { "false" })
            .unwrap()
    };
    let mut s = blocks.block(plant).default_state;
    s = set(
        s,
        "down",
        connects(p.below()) || env.t.supports_chorus_plant.contains(region.get(p.below())),
    );
    s = set(s, "up", connects(p.above()));
    s = set(s, "north", connects(p.relative(Direction::North, 1)));
    s = set(s, "east", connects(p.relative(Direction::East, 1)));
    s = set(s, "south", connects(p.relative(Direction::South, 1)));
    set(s, "west", connects(p.relative(Direction::West, 1)))
}

/// `ChorusFlowerBlock.generatePlant`.
fn generate_chorus(region: &mut Region, target: BlockPos, random: &mut dyn RandomSource, spread: i32) {
    let s = chorus_with_connections(region, target);
    region.set(target, s, 2);
    grow_chorus(region, target, random, target, spread, 0);
}

/// `ChorusFlowerBlock.allNeighborsEmpty`.
fn all_neighbors_empty(region: &Region, p: BlockPos, ignore: Option<Direction>) -> bool {
    Direction::HORIZONTAL
        .iter()
        .all(|d| Some(*d) == ignore || region.is_empty_block(p.relative(*d, 1)))
}

/// `ChorusFlowerBlock.growTreeRecursive`.
fn grow_chorus(
    region: &mut Region,
    current: BlockPos,
    random: &mut dyn RandomSource,
    start: BlockPos,
    spread: i32,
    depth: i32,
) {
    let mut height = random.next_int_bounded(4) + 1;
    if depth == 0 {
        height += 1;
    }
    for i in 0..height {
        let target = current.offset(0, i + 1, 0);
        if !all_neighbors_empty(region, target, None) {
            return;
        }
        let s = chorus_with_connections(region, target);
        region.set(target, s, 2);
        let s = chorus_with_connections(region, target.below());
        region.set(target.below(), s, 2);
    }
    let mut placed_stem = false;
    if depth < 4 {
        let mut stems = random.next_int_bounded(4);
        if depth == 0 {
            stems += 1;
        }
        for _ in 0..stems {
            let direction = Direction::random_horizontal(random);
            let target = current.offset(0, height, 0).relative(direction, 1);
            if (target.x - start.x).abs() < spread
                && (target.z - start.z).abs() < spread
                && region.is_empty_block(target)
                && region.is_empty_block(target.below())
                && all_neighbors_empty(region, target, Some(direction.opposite()))
            {
                placed_stem = true;
                let s = chorus_with_connections(region, target);
                region.set(target, s, 2);
                let back = target.relative(direction.opposite(), 1);
                let s = chorus_with_connections(region, back);
                region.set(back, s, 2);
                grow_chorus(region, target, random, start, spread, depth + 1);
            }
        }
    }
    if !placed_stem {
        let env = region.env;
        let flower = env
            .blocks
            .with_property(env.blocks.block(env.b.chorus_flower).default_state, "age", "5")
            .unwrap();
        region.set(current.offset(0, height, 0), flower, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spike_layout_is_a_ring() {
        let spikes = spikes_for_seed(12345);
        assert_eq!(spikes.len(), 10);
        assert_eq!((spikes[0].center_x, spikes[0].center_z), (42, 0));
        let mut heights: Vec<_> = spikes.iter().map(|s| s.height).collect();
        heights.sort();
        assert_eq!(heights, (0..10).map(|s| 76 + s * 3).collect::<Vec<_>>());
    }
}
