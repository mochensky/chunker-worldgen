//! `Mob.finalizeSpawn` for the mobs structures place (`EntitySpawnReason.STRUCTURE`) and the jockeys
//! they bring along. Entities are NBT compounds, changed the way the vanilla setters change the
//! saved data; the server fills in every default when it loads the chunk.
//!
//! `finalizeSpawn` draws from the region random (`level.getRandom()`), so the calls have to happen in
//! vanilla's order for everything that uses that random later in the chunk. A few values come from
//! the entity's own random instead (`Zombie.handleAttributes`): vanilla seeds that one from the clock,
//! so here it is seeded from the entity position.

mod enchant;
mod variant;

use std::sync::Arc;

use anyhow::Result;
use chunker_data::Identifier;
use chunker_data::entities::EntityTables;
use chunker_nbt::{Compound, List, Tag};
use serde_json::Value;

use crate::biome::{BiomeId, BiomeRegistry};
use crate::feature::Region;
use crate::feature::registry::{BiomeSet, Parser};
use crate::math;
use crate::random::{AnyRandom, LegacyRandom, RandomSource};
use crate::registry::WorldgenRegistries;
use crate::structure::geom::BlockPos;

pub use variant::SpawnContext;

/// `Difficulty`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    pub fn id(self) -> i32 {
        self as i32
    }

    /// `Difficulty.byName` (`peaceful`, `easy`, `normal`, `hard`).
    pub fn by_name(name: &str) -> Option<Self> {
        Some(match name {
            "peaceful" => Self::Peaceful,
            "easy" => Self::Easy,
            "normal" => Self::Normal,
            "hard" => Self::Hard,
            _ => return None,
        })
    }

    pub fn by_id(id: i32) -> Option<Self> {
        [Self::Peaceful, Self::Easy, Self::Normal, Self::Hard]
            .get(usize::try_from(id).ok()?)
            .copied()
    }
}

/// The world state `finalizeSpawn` depends on.
#[derive(Clone, Copy, Debug)]
pub struct SpawnSettings {
    /// The world difficulty (`level.dat`).
    pub difficulty: Difficulty,
    /// Total ticks of the `minecraft:overworld` world clock (the day time).
    pub clock_time: i64,
    /// `level.getGameTime()`.
    pub game_time: i64,
    /// `SpecialDates.isHalloween()`: October 31 on the machine that generates.
    pub halloween: bool,
}

impl Default for SpawnSettings {
    /// A new world: normal difficulty, time 0.
    fn default() -> Self {
        Self {
            difficulty: Difficulty::Normal,
            clock_time: 0,
            game_time: 0,
            halloween: false,
        }
    }
}

/// `DifficultyInstance`.
#[derive(Clone, Copy, Debug)]
pub struct DifficultyInstance {
    effective: f32,
}

impl DifficultyInstance {
    pub fn new(base: Difficulty, total_game_time: i64, local_game_time: i64, moon_brightness: f32) -> Self {
        let effective = if base == Difficulty::Peaceful {
            0.0
        } else {
            let hard = base == Difficulty::Hard;
            let mut scale = 0.75f32;
            let global = math::clamp_f32((total_game_time as f32 + -72000.0) / 1440000.0, 0.0, 1.0) * 0.25;
            scale += global;
            let mut local = 0.0f32;
            local +=
                math::clamp_f32(local_game_time as f32 / 3600000.0, 0.0, 1.0) * if hard { 1.0 } else { 0.75 };
            local += math::clamp_f32(moon_brightness * 0.25, 0.0, global);
            if base == Difficulty::Easy {
                local *= 0.5;
            }
            scale += local;
            base.id() as f32 * scale
        };
        Self { effective }
    }

    pub fn special_multiplier(self) -> f32 {
        if self.effective < 2.0 {
            0.0
        } else if self.effective > 4.0 {
            1.0
        } else {
            (self.effective - 2.0) / 2.0
        }
    }
}

/// `DimensionType.MOON_BRIGHTNESS_PER_PHASE`.
const MOON_BRIGHTNESS: [f32; 8] = [1.0, 0.75, 0.5, 0.25, 0.0, 0.25, 0.5, 0.75];

/// The `visual/moon_phase` track of a dimension's timelines.
#[derive(Clone, Debug)]
pub struct MoonTimeline {
    period: i64,
    /// (ticks, phase index), sorted by ticks.
    keyframes: Vec<(i64, usize)>,
}

impl MoonTimeline {
    /// The first timeline of a dimension type (its JSON) with a moon phase track.
    pub fn load(reg: &WorldgenRegistries, dim: &Value) -> Result<Option<Self>> {
        const PHASES: [&str; 8] = [
            "full_moon",
            "waning_gibbous",
            "third_quarter",
            "waning_crescent",
            "new_moon",
            "waxing_crescent",
            "first_quarter",
            "waxing_gibbous",
        ];
        let timelines = match dim.get("timelines") {
            Some(Value::String(s)) => match s.strip_prefix('#') {
                Some(tag) => reg.packs.resolve_tag(
                    "timeline",
                    &Identifier::parse(tag).unwrap_or_else(|| Identifier::vanilla(tag)),
                    &|_| true,
                )?,
                None => Identifier::parse(s).into_iter().collect(),
            },
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().and_then(Identifier::parse))
                .collect(),
            _ => Vec::new(),
        };
        for id in timelines {
            let Some(t) = reg.packs.read_json("timeline", &id)? else {
                continue;
            };
            let Some(track) = t
                .get("tracks")
                .and_then(|tr| tr.get("minecraft:visual/moon_phase"))
            else {
                continue;
            };
            let period = t.get("period_ticks").and_then(Value::as_i64).unwrap_or(i64::MAX);
            let mut keyframes = Vec::new();
            for k in track
                .get("keyframes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let ticks = k.get("ticks").and_then(Value::as_i64).unwrap_or(0);
                let phase = k.get("value").and_then(Value::as_str).unwrap_or("full_moon");
                let phase = PHASES.iter().position(|p| *p == phase).unwrap_or(0);
                keyframes.push((ticks, phase));
            }
            keyframes.sort_by_key(|k| k.0);
            return Ok(Some(Self { period, keyframes }));
        }
        Ok(None)
    }

    /// Moon brightness at a clock time (constant keyframes; before the first one, the last applies).
    pub fn brightness(&self, clock_time: i64) -> f32 {
        let t = clock_time.rem_euclid(self.period);
        let phase = self
            .keyframes
            .iter()
            .rev()
            .find(|k| k.0 <= t)
            .or(self.keyframes.last())
            .map_or(0, |k| k.1);
        MOON_BRIGHTNESS[phase]
    }
}

/// Registries and tables `finalizeSpawn` reads, loaded once per dimension.
pub struct MobData {
    tables: Arc<EntityTables>,
    enchantments: enchant::Enchantments,
    cow: variant::Variants,
    pig: variant::Variants,
    chicken: variant::Variants,
    cat: variant::Variants,
    zombie_nautilus: variant::Variants,
    cow_sound: variant::RandomRegistry,
    pig_sound: variant::RandomRegistry,
    chicken_sound: variant::RandomRegistry,
    cat_sound: variant::RandomRegistry,
    warm_farm_animals: BiomeSet,
    cold_farm_animals: BiomeSet,
    more_frequent_drowned: BiomeSet,
}

impl MobData {
    /// Whether an entity type is a `Mob` (only mobs have `follow_range`).
    pub fn is_mob(&self, id: &str) -> bool {
        self.tables.attribute_base(id, FOLLOW_RANGE).is_some()
    }

    pub fn load(reg: &WorldgenRegistries, biomes: &BiomeRegistry) -> Result<Self> {
        let parser = Parser::new(reg, biomes);
        let tag = |t: &str| parser.biome_set(&Value::String(format!("#minecraft:{t}")));
        Ok(Self {
            tables: EntityTables::for_version(reg.blocks.version),
            enchantments: enchant::Enchantments::load(reg)?,
            cow: variant::Variants::load(reg, biomes, "cow_variant")?,
            pig: variant::Variants::load(reg, biomes, "pig_variant")?,
            chicken: variant::Variants::load(reg, biomes, "chicken_variant")?,
            cat: variant::Variants::load(reg, biomes, "cat_variant")?,
            zombie_nautilus: variant::Variants::load(reg, biomes, "zombie_nautilus_variant")?,
            cow_sound: variant::RandomRegistry::load(reg, "cow_sound_variant"),
            pig_sound: variant::RandomRegistry::load(reg, "pig_sound_variant"),
            chicken_sound: variant::RandomRegistry::load(reg, "chicken_sound_variant"),
            cat_sound: variant::RandomRegistry::load(reg, "cat_sound_variant"),
            warm_farm_animals: tag("spawns_warm_variant_farm_animals")?,
            cold_farm_animals: tag("spawns_cold_variant_farm_animals")?,
            more_frequent_drowned: tag("more_frequent_drowned_spawns")?,
        })
    }
}

/// `EntitySpawnReason`, as far as structures use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnReason {
    Structure,
    Jockey,
}

const RANDOM_SPAWN_BONUS: &str = "minecraft:random_spawn_bonus";
const FOLLOW_RANGE: &str = "minecraft:follow_range";

/// `AttributeModifier.Operation`.
#[derive(Clone, Copy)]
enum Operation {
    Value,
    MultipliedBase,
    MultipliedTotal,
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Operation::Value => "add_value",
            Operation::MultipliedBase => "add_multiplied_base",
            Operation::MultipliedTotal => "add_multiplied_total",
        }
    }
}

/// A mob being finalized: its saved data.
struct Mob {
    tag: Compound,
    id: String,
}

fn item(id: &str) -> Compound {
    let mut c = Compound::new();
    c.insert("id", id);
    c.insert("count", 1);
    c
}

impl Mob {
    fn new(tag: Compound) -> Self {
        let id = match tag.get("id") {
            Some(Tag::String(s)) => s.clone(),
            _ => String::new(),
        };
        Self { tag, id }
    }

    fn is(&self, id: &str) -> bool {
        self.id == id
    }

    fn pos(&self) -> (f64, f64, f64) {
        entity_pos(&self.tag)
    }

    /// `blockPosition()`.
    fn block_pos(&self) -> BlockPos {
        let (x, y, z) = self.pos();
        BlockPos::new(math::floor(x), math::floor(y), math::floor(z))
    }

    fn y_rot(&self) -> f32 {
        match self.tag.get_list("Rotation").and_then(|l| l.get(0)) {
            Some(Tag::Float(v)) => *v,
            _ => 0.0,
        }
    }

    fn flag(&self, key: &str) -> bool {
        matches!(self.tag.get(key), Some(Tag::Byte(v)) if *v != 0)
    }

    fn set(&mut self, key: &str, value: impl Into<Tag>) {
        self.tag.insert(key, value);
    }

    /// The saved attribute instance, created with the type's default base if missing (vanilla creates
    /// instances on first access); `None` if the type doesn't have the attribute.
    fn attribute(&mut self, tables: &EntityTables, attribute: &str) -> Option<&mut Compound> {
        let mut list = match self.tag.remove("attributes") {
            Some(Tag::List(l)) => l,
            _ => List::new(),
        };
        let found = list.compounds().any(|c| c.get_str("id") == Some(attribute));
        if !found {
            let base = tables.attribute_base(&self.id, attribute)?;
            let mut c = Compound::new();
            c.insert("id", attribute);
            c.insert("base", base);
            list.push(c);
        }
        self.tag.insert("attributes", list);
        let Some(Tag::List(list)) = self.tag.get_mut("attributes") else {
            unreachable!()
        };
        list.compounds_mut().find(|c| c.get_str("id") == Some(attribute))
    }

    fn has_modifier(&self, attribute: &str, modifier: &str) -> bool {
        self.tag.get_list("attributes").is_some_and(|l| {
            l.compounds().any(|a| {
                a.get_str("id") == Some(attribute)
                    && a.get_list("modifiers")
                        .is_some_and(|m| m.compounds().any(|m| m.get_str("id") == Some(modifier)))
            })
        })
    }

    /// `addOrReplacePermanentModifier` (`addPermanentModifier` after a `hasModifier` check).
    fn put_modifier(
        &mut self,
        tables: &EntityTables,
        attribute: &str,
        modifier: &str,
        amount: f64,
        op: Operation,
    ) {
        let Some(a) = self.attribute(tables, attribute) else {
            return;
        };
        let mut modifiers = match a.remove("modifiers") {
            Some(Tag::List(l)) => l,
            _ => List::new(),
        };
        let mut kept = List::new();
        for m in modifiers.compounds_mut() {
            if m.get_str("id") != Some(modifier) {
                kept.push(m.clone());
            }
        }
        modifiers = kept;
        let mut m = Compound::new();
        m.insert("id", modifier);
        m.insert("amount", amount);
        m.insert("operation", op.name());
        modifiers.push(m);
        a.insert("modifiers", modifiers);
    }

    fn set_base(&mut self, tables: &EntityTables, attribute: &str, value: f64) {
        if let Some(a) = self.attribute(tables, attribute) {
            a.insert("base", value);
        }
    }

    /// `AttributeInstance.getValue` of a saved attribute.
    fn attribute_value(&mut self, tables: &EntityTables, attribute: &str) -> f64 {
        let Some(a) = self.attribute(tables, attribute) else {
            return 0.0;
        };
        let base = match a.get("base") {
            Some(Tag::Double(v)) => *v,
            _ => 0.0,
        };
        let mut amounts: [Vec<f64>; 3] = Default::default();
        for m in a.get_list("modifiers").into_iter().flat_map(|l| l.compounds()) {
            let amount = match m.get("amount") {
                Some(Tag::Double(v)) => *v,
                _ => 0.0,
            };
            let i = match m.get_str("operation") {
                Some("add_value") => 0,
                Some("add_multiplied_base") => 1,
                _ => 2,
            };
            amounts[i].push(amount);
        }
        let mut value = base;
        for v in &amounts[0] {
            value += v;
        }
        let mut result = value;
        for v in &amounts[1] {
            result += value * v;
        }
        for v in &amounts[2] {
            result *= 1.0 + v;
        }
        result
    }

    fn slot(&self, slot: &str) -> Option<&Compound> {
        self.tag.get_compound("equipment")?.get_compound(slot)
    }

    fn slot_is(&self, slot: &str, id: &str) -> bool {
        self.slot(slot).and_then(|i| i.get_str("id")) == Some(id)
    }

    /// `setItemSlot`.
    fn set_slot(&mut self, slot: &str, stack: Compound) {
        if self.tag.get_compound("equipment").is_none() {
            self.tag.insert("equipment", Compound::new());
        }
        if let Some(Tag::Compound(e)) = self.tag.get_mut("equipment") {
            e.insert(slot, stack);
        }
    }

    fn set_drop_chance(&mut self, slot: &str, chance: f32) {
        if self.tag.get_compound("drop_chances").is_none() {
            self.tag.insert("drop_chances", Compound::new());
        }
        if let Some(Tag::Compound(d)) = self.tag.get_mut("drop_chances") {
            d.insert(slot, chance);
        }
    }

    /// `brain.setMemory` / `setMemoryWithExpiry`.
    fn set_memory(&mut self, memory: &str, value: impl Into<Tag>, ttl: Option<i64>) {
        let mut brain = match self.tag.remove("Brain") {
            Some(Tag::Compound(c)) => c,
            _ => Compound::new(),
        };
        let mut memories = match brain.remove("memories") {
            Some(Tag::Compound(c)) => c,
            _ => Compound::new(),
        };
        let mut m = Compound::new();
        m.insert("value", value);
        if let Some(ttl) = ttl {
            m.insert("ttl", ttl);
        }
        memories.insert(memory, m);
        brain.insert("memories", memories);
        self.tag.insert("Brain", brain);
    }

    /// `AgeableMob.setBaby` / `setAge`.
    fn set_age(&mut self, age: i32) {
        self.set("Age", age);
    }

    fn is_baby(&self) -> bool {
        match self.id.as_str() {
            "minecraft:zombie"
            | "minecraft:husk"
            | "minecraft:drowned"
            | "minecraft:zombie_villager"
            | "minecraft:piglin" => self.flag("IsBaby"),
            _ => matches!(self.tag.get("Age"), Some(Tag::Int(a)) if *a < 0),
        }
    }
}

/// The level `finalizeSpawn` runs against: the region, the mob tables and the difficulty at the mob.
struct Spawn<'r, 'a> {
    region: &'r mut Region<'a>,
    data: &'r MobData,
    difficulty: DifficultyInstance,
    reason: SpawnReason,
    /// The jockey vehicle created during this call (`level.addFreshEntity` inside `finalizeSpawn`).
    vehicle: Option<Compound>,
}

/// The `Pos` of saved entity data.
fn entity_pos(tag: &Compound) -> (f64, f64, f64) {
    let p = tag.get_list("Pos");
    let get = |i| match p.and_then(|l| l.get(i)) {
        Some(Tag::Double(v)) => *v,
        _ => 0.0,
    };
    (get(0), get(1), get(2))
}

/// `finalizeSpawn` for `tag` (`reason` is `STRUCTURE`), then `addFreshEntityWithPassengers`.
pub fn spawn_mob(region: &mut Region, tag: Compound) {
    let finalized = finalize_spawn(region, tag, SpawnReason::Structure);
    let (x, _, z) = entity_pos(&finalized);
    region.add_entity(math::floor(x), math::floor(z), finalized);
}

/// Runs `finalizeSpawn` and returns the entity to add: the mob, or the vehicle it rides.
pub fn finalize_spawn(region: &mut Region, tag: Compound, reason: SpawnReason) -> Compound {
    let env = region.env;
    let data = env.mobs();
    let mut mob = Mob::new(tag);
    let moon = env
        .moon
        .as_ref()
        .map_or(1.0, |m| m.brightness(env.spawn.clock_time));
    let mut spawn = Spawn {
        difficulty: DifficultyInstance::new(env.spawn.difficulty, env.spawn.clock_time, 0, moon),
        region,
        data,
        reason,
        vehicle: None,
    };
    spawn.finalize(&mut mob);
    match spawn.vehicle {
        Some(v) => v,
        None => mob.tag,
    }
}

impl Spawn<'_, '_> {
    fn random(&mut self) -> &mut AnyRandom {
        &mut self.region.random
    }

    fn biome(&self, pos: BlockPos) -> BiomeId {
        self.region.biome(pos)
    }

    fn finalize(&mut self, mob: &mut Mob) {
        match mob.id.as_str() {
            "minecraft:villager" => {
                self.villager_type(mob);
                self.mob_base(mob);
            }
            "minecraft:zombie_villager" => {
                self.villager_type(mob);
                self.zombie(mob);
            }
            "minecraft:zombie" | "minecraft:husk" => self.zombie(mob),
            "minecraft:drowned" => self.drowned(mob),
            "minecraft:piglin" => self.piglin(mob),
            "minecraft:piglin_brute" => {
                let pos = mob.block_pos();
                let mut home = Compound::new();
                home.insert("dimension", self.region.env.dimension.as_str());
                home.insert("pos", vec![pos.x, pos.y, pos.z]);
                mob.set_memory("minecraft:home", home, None);
                mob.set_slot("mainhand", item("minecraft:golden_axe"));
                self.mob_base(mob);
            }
            "minecraft:hoglin" => {
                if self.random().next_float() < 0.2 {
                    mob.set_age(-24000);
                }
                self.mob_base(mob);
            }
            "minecraft:cow" => self.farm_animal(mob, |d| (&d.cow, &d.cow_sound)),
            "minecraft:pig" => self.farm_animal(mob, |d| (&d.pig, &d.pig_sound)),
            "minecraft:chicken" => self.farm_animal(mob, |d| (&d.chicken, &d.chicken_sound)),
            "minecraft:cat" => {
                self.mob_base(mob);
                let data = self.data;
                self.variant(mob, &data.cat);
                if let Some(s) = data.cat_sound.pick(self.random()) {
                    mob.set("sound_variant", s.as_str());
                }
            }
            "minecraft:sheep" => {
                let biome = self.biome(mob.block_pos());
                let color = self.sheep_color(biome);
                mob.set("Color", color);
                self.mob_base(mob);
            }
            "minecraft:horse" => {
                let variant = self.random().next_int_bounded(7);
                let markings = self.random().next_int_bounded(5);
                mob.set("Variant", variant & 0xFF | markings << 8 & 0xFF00);
                self.horse_attributes(mob);
                self.mob_base(mob);
            }
            "minecraft:camel" => {
                mob.set("LastPoseTick", 0i64.max(self.region.env.spawn.game_time - 52 - 1));
                self.mob_base(mob);
            }
            "minecraft:zombie_nautilus" => {
                let data = self.data;
                self.variant(mob, &data.zombie_nautilus);
                // `NautilusAi.TIME_BETWEEN_NON_PLAYER_ATTACKS`.
                let cooldown = self.random().next_int_between_inclusive(2400, 3600);
                mob.set_memory("minecraft:attack_target_cooldown", cooldown, None);
                self.mob_base(mob);
            }
            "minecraft:witch" | "minecraft:evoker" => {
                mob.set("CanJoinRaid", true);
                self.mob_base(mob);
            }
            "minecraft:vindicator" => {
                mob.set("CanJoinRaid", true);
                self.mob_base(mob);
                mob.set_slot("mainhand", item("minecraft:iron_axe"));
                self.enchant_equipment(mob);
            }
            _ => self.mob_base(mob),
        }
    }

    /// `Mob.finalizeSpawn`.
    fn mob_base(&mut self, mob: &mut Mob) {
        let tables = self.data.tables.clone();
        if !mob.has_modifier(FOLLOW_RANGE, RANDOM_SPAWN_BONUS)
            && tables.attribute_base(&mob.id, FOLLOW_RANGE).is_some()
        {
            let amount = self.random().triangle_f64(0.0, 0.11485000000000001);
            mob.put_modifier(
                &tables,
                FOLLOW_RANGE,
                RANDOM_SPAWN_BONUS,
                amount,
                Operation::MultipliedBase,
            );
        }
        let left = self.random().next_float() < 0.05;
        mob.set("LeftHanded", left);
    }

    /// `VillagerDataHolder.finalizeVillagerType`.
    fn villager_type(&mut self, mob: &mut Mob) {
        if mob.flag("VillagerDataFinalized") {
            return;
        }
        let biome = self.biome(mob.block_pos());
        let ty = villager_type(self.region.env.biomes.name(biome));
        let mut data = mob.tag.get_compound("VillagerData").cloned().unwrap_or_default();
        data.insert("type", ty);
        mob.set("VillagerData", data);
        mob.set("VillagerDataFinalized", true);
    }

    /// `Cow`/`Pig`/`Chicken.finalizeSpawn`: variant, sound variant, then the `Mob` part.
    fn farm_animal(
        &mut self,
        mob: &mut Mob,
        registries: impl Fn(&MobData) -> (&variant::Variants, &variant::RandomRegistry),
    ) {
        let data = self.data;
        let (variants, sounds) = registries(data);
        self.variant(mob, variants);
        if let Some(s) = sounds.pick(self.random()) {
            mob.set("sound_variant", s.as_str());
        }
        self.mob_base(mob);
    }

    /// `VariantUtils.selectVariantToSpawn(SpawnContext.create(level, blockPosition()), ...)`.
    fn variant(&mut self, mob: &mut Mob, variants: &variant::Variants) {
        let pos = mob.block_pos();
        let biome = self.biome(pos);
        let pieces = &self.region.structure_pieces;
        let has_piece = |p: BlockPos, ids: &[Identifier]| {
            pieces
                .iter()
                .any(|(id, boxes)| ids.contains(id) && boxes.iter().any(|b| b.is_inside(p.x, p.y, p.z)))
        };
        let ctx = variant::SpawnContext {
            pos,
            biome,
            has_structure_piece: &has_piece,
            moon_brightness: 1.0,
        };
        let random = &mut self.region.random;
        if let Some(v) = variants.pick(&ctx, random) {
            mob.set("variant", v.as_str());
        }
    }

    /// `SheepColorSpawnRules.getSheepColor`, as a `DyeColor` id.
    fn sheep_color(&mut self, biome: BiomeId) -> i8 {
        const WHITE: i8 = 0;
        const PINK: i8 = 6;
        const GRAY: i8 = 7;
        const LIGHT_GRAY: i8 = 8;
        const BROWN: i8 = 12;
        const BLACK: i8 = 15;
        let data = self.data;
        let (list, common) = if data.warm_farm_animals.contains(biome) {
            ([(GRAY, 5), (LIGHT_GRAY, 5), (WHITE, 5), (BLACK, 3)], BROWN)
        } else if data.cold_farm_animals.contains(biome) {
            ([(LIGHT_GRAY, 5), (GRAY, 5), (WHITE, 5), (BROWN, 3)], BLACK)
        } else {
            ([(BLACK, 5), (GRAY, 5), (LIGHT_GRAY, 5), (BROWN, 3)], WHITE)
        };
        let mut selection = self.random().next_int_bounded(100);
        for (color, weight) in list {
            selection -= weight;
            if selection < 0 {
                return color;
            }
        }
        // `commonColors(default)`: the default 499 times in 500, pink otherwise.
        if self.random().next_int_bounded(500) < 499 {
            common
        } else {
            PINK
        }
    }

    /// `Horse.randomizeAttributes` (`AbstractHorse.generate*`).
    fn horse_attributes(&mut self, mob: &mut Mob) {
        let tables = self.data.tables.clone();
        let r = self.random();
        let health = 15.0f32 + r.next_int_bounded(8) as f32 + r.next_int_bounded(9) as f32;
        let speed =
            (0.45f32 as f64 + r.next_double() * 0.3 + r.next_double() * 0.3 + r.next_double() * 0.3) * 0.25;
        let jump = 0.4f32 as f64 + r.next_double() * 0.2 + r.next_double() * 0.2 + r.next_double() * 0.2;
        mob.set_base(&tables, "minecraft:max_health", health as f64);
        mob.set_base(&tables, "minecraft:movement_speed", speed);
        mob.set_base(&tables, "minecraft:jump_strength", jump);
    }

    /// `Piglin.finalizeSpawn` with `STRUCTURE` (no baby or weapon roll).
    fn piglin(&mut self, mob: &mut Mob) {
        // `PiglinAi.TIME_BETWEEN_HUNTS`: 30..120 seconds.
        let ttl = self.random().next_int_between_inclusive(600, 2400);
        mob.set_memory("minecraft:hunted_recently", true, Some(ttl as i64));
        if !mob.is_baby() {
            for (slot, id) in [
                ("head", "minecraft:golden_helmet"),
                ("chest", "minecraft:golden_chestplate"),
                ("legs", "minecraft:golden_leggings"),
                ("feet", "minecraft:golden_boots"),
            ] {
                if self.random().next_float() < 0.1 {
                    mob.set_slot(slot, item(id));
                }
            }
        }
        self.enchant_equipment(mob);
        self.mob_base(mob);
    }

    /// `Zombie.finalizeSpawn` (with `populateDefaultEquipmentSlots` of the zombie type).
    fn zombie(&mut self, mob: &mut Mob) {
        self.mob_base(mob);
        let m = self.difficulty.special_multiplier();
        let loot = self.random().next_float() < 0.55 * m;
        mob.set("CanPickUpLoot", loot);
        // `ZombieGroupData(getSpawnAsBabyOdds(random), true)`.
        let baby = self.random().next_float() < 0.05;
        if baby {
            mob.set("IsBaby", true);
            if (self.random().next_float() as f64) < 0.05 {
                // `level.getEntitiesOfClass(Chicken, ...)`: a WorldGenRegion has no entities to find.
            } else if (self.random().next_float() as f64) < 0.05 {
                self.chicken_jockey(mob);
            }
        }
        let doors = self.random().next_float() < m * 0.1;
        mob.set("CanBreakDoors", doors);
        if mob.is("minecraft:drowned") {
            self.drowned_equipment(mob);
        } else {
            self.armor(mob);
            let chance = if self.region.env.spawn.difficulty == Difficulty::Hard {
                0.05
            } else {
                0.01
            };
            if self.random().next_float() < chance {
                let id = match self.random().next_int_bounded(6) {
                    0 => "minecraft:iron_sword",
                    1 => "minecraft:iron_spear",
                    _ => "minecraft:iron_shovel",
                };
                mob.set_slot("mainhand", item(id));
            }
        }
        self.enchant_equipment(mob);
        if mob.slot("head").is_none() && self.region.env.spawn.halloween && self.random().next_float() < 0.25
        {
            let id = if self.random().next_float() < 0.1 {
                "minecraft:jack_o_lantern"
            } else {
                "minecraft:carved_pumpkin"
            };
            mob.set_slot("head", item(id));
            mob.set_drop_chance("head", 0.0);
        }
        self.zombie_attributes(mob, m);
    }

    /// `Zombie.handleAttributes`, from the entity's own random.
    fn zombie_attributes(&mut self, mob: &mut Mob, m: f32) {
        let tables = self.data.tables.clone();
        let mut r = entity_random(self.region.env.seed, mob.block_pos());
        // `randomizeReinforcementsChance` (drowned keep the zombie version).
        mob.set_base(
            &tables,
            "minecraft:spawn_reinforcements",
            r.next_double() * 0.1f32 as f64,
        );
        mob.put_modifier(
            &tables,
            "minecraft:knockback_resistance",
            RANDOM_SPAWN_BONUS,
            r.next_double() * 0.05f32 as f64,
            Operation::Value,
        );
        let follow = r.next_double() * 1.5 * m as f64;
        if follow > 1.0 {
            mob.put_modifier(
                &tables,
                FOLLOW_RANGE,
                "minecraft:zombie_random_spawn_bonus",
                follow,
                Operation::MultipliedTotal,
            );
        }
        if r.next_float() < m * 0.05 {
            mob.put_modifier(
                &tables,
                "minecraft:spawn_reinforcements",
                "minecraft:leader_zombie_bonus",
                r.next_double() * 0.25 + 0.5,
                Operation::Value,
            );
            mob.put_modifier(
                &tables,
                "minecraft:max_health",
                "minecraft:leader_zombie_bonus",
                r.next_double() * 3.0 + 1.0,
                Operation::MultipliedTotal,
            );
            let max = mob.attribute_value(&tables, "minecraft:max_health");
            mob.set("Health", max as f32);
            mob.set("CanBreakDoors", true);
        }
    }

    /// `Drowned.finalizeSpawn`: the zombie part, a nautilus shell, and a zombie nautilus to ride.
    fn drowned(&mut self, mob: &mut Mob) {
        self.zombie(mob);
        if mob.slot("offhand").is_none() && self.random().next_float() < 0.03 {
            mob.set_slot("offhand", item("minecraft:nautilus_shell"));
            mob.set_drop_chance("offhand", 2.0);
        }
        if mob.slot_is("mainhand", "minecraft:trident") && self.random().next_float() < 0.5 && !mob.is_baby()
        {
            let biome = self.biome(mob.block_pos());
            if !self.data.more_frequent_drowned.contains(biome) {
                let mut nautilus = vehicle("minecraft:zombie_nautilus", mob);
                if self.reason == SpawnReason::Structure {
                    nautilus.set("PersistenceRequired", true);
                }
                self.finalize(&mut nautilus);
                self.ride(nautilus, mob);
            }
        }
    }

    /// `Drowned.populateDefaultEquipmentSlots`.
    fn drowned_equipment(&mut self, mob: &mut Mob) {
        if self.random().next_float() as f64 > 0.9 {
            let id = if self.random().next_int_bounded(16) < 10 {
                "minecraft:trident"
            } else {
                "minecraft:fishing_rod"
            };
            mob.set_slot("mainhand", item(id));
        }
    }

    /// The chicken of a baby zombie (`EntitySpawnReason.JOCKEY`).
    fn chicken_jockey(&mut self, mob: &mut Mob) {
        let mut chicken = vehicle("minecraft:chicken", mob);
        let reason = std::mem::replace(&mut self.reason, SpawnReason::Jockey);
        self.finalize(&mut chicken);
        self.reason = reason;
        chicken.set("IsChickenJockey", true);
        self.ride(chicken, mob);
    }

    /// `startRiding` + `addFreshEntity(vehicle)`: the vehicle is saved now, with the rider as it is
    /// at this point; the rider itself is not added again (`ProtoChunk.addEntity` skips passengers).
    fn ride(&mut self, mut vehicle: Mob, rider: &Mob) {
        let mut passengers = List::new();
        passengers.push(rider.tag.clone());
        vehicle.set("Passengers", passengers);
        self.vehicle = Some(vehicle.tag);
    }

    /// `Mob.populateDefaultEquipmentSlots` (random armor).
    fn armor(&mut self, mob: &mut Mob) {
        let m = self.difficulty.special_multiplier();
        if self.random().next_float() >= 0.15 * m {
            return;
        }
        let mut armor_type = self.random().next_int_bounded(3);
        for _ in 1..=3 {
            if self.random().next_float() < 0.1087 {
                armor_type += 1;
            }
        }
        let partial = if self.region.env.spawn.difficulty == Difficulty::Hard {
            0.1
        } else {
            0.25
        };
        let mut first = true;
        for slot in ["head", "chest", "legs", "feet"] {
            let empty = mob.slot(slot).is_none();
            if !first && self.random().next_float() < partial {
                break;
            }
            first = false;
            if empty && let Some(id) = equipment_for_slot(slot, armor_type) {
                mob.set_slot(slot, item(id));
            }
        }
    }

    /// `Mob.populateDefaultEquipmentEnchantments`: the main hand, then the armor slots.
    fn enchant_equipment(&mut self, mob: &mut Mob) {
        self.enchant_slot(mob, "mainhand", 0.25);
        for slot in ["feet", "legs", "chest", "head"] {
            self.enchant_slot(mob, slot, 0.5);
        }
    }

    /// `Mob.enchantSpawnedEquipment`.
    fn enchant_slot(&mut self, mob: &mut Mob, slot: &str, chance: f32) {
        let Some(stack) = mob.slot(slot).cloned() else {
            return;
        };
        let m = self.difficulty.special_multiplier();
        if self.random().next_float() >= chance * m {
            return;
        }
        let Some(id) = stack.get_str("id").and_then(Identifier::parse) else {
            return;
        };
        let data = self.data;
        let enchantable = data.tables.enchantable(id.as_str());
        let added = data
            .enchantments
            .mob_spawn_equipment(&mut self.region.random, &id, enchantable, m);
        let mut stack = stack;
        let mut components = stack.get_compound("components").cloned().unwrap_or_default();
        let mut enchantments = components
            .get_compound("minecraft:enchantments")
            .cloned()
            .unwrap_or_default();
        for (e, level) in added {
            // `ItemEnchantments.Mutable.upgrade`.
            let old = enchantments.get_i32(e.as_str()).unwrap_or(0);
            if level > old {
                enchantments.insert(e.as_str(), level);
            }
        }
        if !enchantments.is_empty() {
            components.insert("minecraft:enchantments", enchantments);
            stack.insert("components", components);
        }
        mob.set_slot(slot, stack);
    }
}

/// The rotation of a new living entity that is never `snapTo`'d: the `LivingEntity` constructor
/// turns it by `random.nextFloat() * 2π` (degrees) from its own unseeded random.
pub fn initial_rotation(seed: i64, pos: BlockPos) -> List {
    let mut r = entity_random(seed, pos);
    let mut rotation = List::new();
    rotation.push(r.next_float() * std::f64::consts::TAU as f32);
    rotation.push(0.0f32);
    rotation
}

/// Stands in for an entity's own random (vanilla seeds it from the clock).
fn entity_random(seed: i64, p: BlockPos) -> LegacyRandom {
    let hash = (p.x as i64).wrapping_mul(3129871) ^ (p.z as i64).wrapping_mul(116129781) ^ p.y as i64;
    LegacyRandom::new(hash ^ seed)
}

/// A fresh entity at the rider's position (`snapTo(x, y, z, yRot, 0)`).
fn vehicle(id: &str, rider: &Mob) -> Mob {
    let (x, y, z) = rider.pos();
    let mut tag = Compound::new();
    tag.insert("id", id);
    let mut pos = List::new();
    pos.push(x);
    pos.push(y);
    pos.push(z);
    tag.insert("Pos", pos);
    let mut rotation = List::new();
    rotation.push(rider.y_rot());
    rotation.push(0.0f32);
    tag.insert("Rotation", rotation);
    Mob::new(tag)
}

/// `Mob.getEquipmentForSlot` (armor of material `armor_type`, 0 leather .. 5 diamond).
fn equipment_for_slot(slot: &str, armor_type: i32) -> Option<&'static str> {
    const MATERIALS: [&str; 6] = ["leather", "copper", "golden", "chainmail", "iron", "diamond"];
    let piece = match slot {
        "head" => "helmet",
        "chest" => "chestplate",
        "legs" => "leggings",
        "feet" => "boots",
        _ => return None,
    };
    let material = MATERIALS.get(usize::try_from(armor_type).ok()?)?;
    Some(match (*material, piece) {
        ("leather", "helmet") => "minecraft:leather_helmet",
        ("leather", "chestplate") => "minecraft:leather_chestplate",
        ("leather", "leggings") => "minecraft:leather_leggings",
        ("leather", "boots") => "minecraft:leather_boots",
        ("copper", "helmet") => "minecraft:copper_helmet",
        ("copper", "chestplate") => "minecraft:copper_chestplate",
        ("copper", "leggings") => "minecraft:copper_leggings",
        ("copper", "boots") => "minecraft:copper_boots",
        ("golden", "helmet") => "minecraft:golden_helmet",
        ("golden", "chestplate") => "minecraft:golden_chestplate",
        ("golden", "leggings") => "minecraft:golden_leggings",
        ("golden", "boots") => "minecraft:golden_boots",
        ("chainmail", "helmet") => "minecraft:chainmail_helmet",
        ("chainmail", "chestplate") => "minecraft:chainmail_chestplate",
        ("chainmail", "leggings") => "minecraft:chainmail_leggings",
        ("chainmail", "boots") => "minecraft:chainmail_boots",
        ("iron", "helmet") => "minecraft:iron_helmet",
        ("iron", "chestplate") => "minecraft:iron_chestplate",
        ("iron", "leggings") => "minecraft:iron_leggings",
        ("iron", "boots") => "minecraft:iron_boots",
        ("diamond", "helmet") => "minecraft:diamond_helmet",
        ("diamond", "chestplate") => "minecraft:diamond_chestplate",
        ("diamond", "leggings") => "minecraft:diamond_leggings",
        _ => "minecraft:diamond_boots",
    })
}

/// `VillagerType.byBiome`.
fn villager_type(biome: &Identifier) -> &'static str {
    match biome.as_str() {
        "minecraft:badlands"
        | "minecraft:desert"
        | "minecraft:eroded_badlands"
        | "minecraft:wooded_badlands" => "minecraft:desert",
        "minecraft:bamboo_jungle" | "minecraft:jungle" | "minecraft:sparse_jungle" => "minecraft:jungle",
        "minecraft:savanna_plateau" | "minecraft:savanna" | "minecraft:windswept_savanna" => {
            "minecraft:savanna"
        }
        "minecraft:deep_frozen_ocean"
        | "minecraft:frozen_ocean"
        | "minecraft:frozen_river"
        | "minecraft:ice_spikes"
        | "minecraft:snowy_beach"
        | "minecraft:snowy_taiga"
        | "minecraft:snowy_plains"
        | "minecraft:grove"
        | "minecraft:snowy_slopes"
        | "minecraft:frozen_peaks"
        | "minecraft:jagged_peaks" => "minecraft:snow",
        "minecraft:swamp" | "minecraft:mangrove_swamp" => "minecraft:swamp",
        "minecraft:old_growth_spruce_taiga"
        | "minecraft:old_growth_pine_taiga"
        | "minecraft:windswept_gravelly_hills"
        | "minecraft:windswept_hills"
        | "minecraft:taiga"
        | "minecraft:windswept_forest" => "minecraft:taiga",
        _ => "minecraft:plains",
    }
}
