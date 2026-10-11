//! `MaterialSystem` (26.3 surface rules + ore veins) and per-chunk rule evaluation.

pub mod rules;

use std::sync::Arc;

use anyhow::Result;
use chunker_data::Identifier;
use chunker_data::blocks::BlockState;
use rustc_hash::{FxHashMap, FxHashSet};

pub use rules::{GenerationContext, MaterialCondition, MaterialParser, MaterialRule, VerticalAnchor};

use crate::biome::{BiomeId, BiomeRegistry};
use crate::chunk::{HeightmapKind, ProtoChunk};
use crate::density::sampler::SamplerRef;
use crate::density::{DensityVolume, SamplerContext};
use crate::math::{floor_f32, map_f64, min_f64, round_f32};
use crate::noise::NoiseStack;
use crate::random::{PositionalRandomFactory, RandomSource};
use crate::random_state::RandomState;

/// Per-world material system state (`MaterialSystem`).
pub struct MaterialSystem {
    pub default_block: BlockState,
    pub sea_level: i32,
    pub preliminary_surface: SamplerRef,
    clay_bands: [BlockState; 192],
    clay_bands_offset: Arc<NoiseStack>,
    surface: Arc<NoiseStack>,
    surface_secondary: Arc<NoiseStack>,
    badlands_pillar: Arc<NoiseStack>,
    badlands_pillar_roof: Arc<NoiseStack>,
    badlands_surface: Arc<NoiseStack>,
    iceberg_pillar: Arc<NoiseStack>,
    iceberg_pillar_roof: Arc<NoiseStack>,
    iceberg_surface: Arc<NoiseStack>,
    noise_random: PositionalRandomFactory,
    pub rule: Arc<MaterialRule>,
    /// `rule` with every registry lookup resolved once, so per-chunk compilation takes no locks.
    template: Template,
    biomes: Arc<BiomeRegistry>,
    eroded_badlands: Option<BiomeId>,
    frozen_oceans: [Option<BiomeId>; 2],
    packed_ice: BlockState,
    snow_block: BlockState,
    water_block: chunker_data::blocks::BlockId,
    default_block_id: chunker_data::blocks::BlockId,
}

fn generate_bands(
    random: &mut dyn RandomSource,
    reg: &chunker_data::blocks::BlockRegistry,
) -> [BlockState; 192] {
    let b = |n: &str| reg.vanilla_default(n);
    let (terracotta, white, orange, yellow, brown, red, light_gray) = (
        b("terracotta"),
        b("white_terracotta"),
        b("orange_terracotta"),
        b("yellow_terracotta"),
        b("brown_terracotta"),
        b("red_terracotta"),
        b("light_gray_terracotta"),
    );
    let mut bands = [terracotta; 192];
    let mut i = 0usize;
    while i < bands.len() {
        i += random.next_int_bounded(5) as usize + 1;
        if i < bands.len() {
            bands[i] = orange;
        }
        i += 1;
    }
    let mut make = |random: &mut dyn RandomSource, base_width: i32, state: BlockState| {
        let count = random.next_int_between_inclusive(6, 15);
        for _ in 0..count {
            let width = base_width + random.next_int_bounded(3);
            let start = random.next_int_bounded(bands.len() as i32);
            let mut p = 0;
            while start + p < bands.len() as i32 && p < width {
                bands[(start + p) as usize] = state;
                p += 1;
            }
        }
    };
    make(random, 1, yellow);
    make(random, 2, brown);
    make(random, 1, red);
    let white_count = random.next_int_between_inclusive(9, 15);
    let mut ix = 0;
    let mut start = 0usize;
    while ix < white_count && start < bands.len() {
        bands[start] = white;
        if start > 1 && random.next_bool() {
            bands[start - 1] = light_gray;
        }
        if start + 1 < bands.len() && random.next_bool() {
            bands[start + 1] = light_gray;
        }
        ix += 1;
        start += random.next_int_bounded(16) as usize + 4;
    }
    bands
}

impl MaterialSystem {
    pub fn new(
        state: Arc<RandomState>,
        biomes: Arc<BiomeRegistry>,
        settings: &crate::settings::NoiseGeneratorSettings,
        parser: &MaterialParser,
    ) -> Result<Self> {
        let reg = &state.registries.blocks;
        let n = |p: &str| state.noise(&Identifier::vanilla(p));
        let noise_random = state.random;
        let clay_bands = generate_bands(&mut noise_random.from_hash_of("minecraft:clay_bands"), reg);
        let rule = parser.rule(&settings.material_rule)?;
        let template = Template::build(&state, &rule)?;
        Ok(Self {
            default_block: settings.default_block,
            sea_level: settings.sea_level,
            preliminary_surface: state.sampler(&settings.router.chunk_surface_level),
            clay_bands_offset: n("clay_bands_offset")?,
            clay_bands,
            surface: n("surface")?,
            surface_secondary: n("surface_secondary")?,
            badlands_pillar: n("badlands_pillar")?,
            badlands_pillar_roof: n("badlands_pillar_roof")?,
            badlands_surface: n("badlands_surface")?,
            iceberg_pillar: n("iceberg_pillar")?,
            iceberg_pillar_roof: n("iceberg_pillar_roof")?,
            iceberg_surface: n("iceberg_surface")?,
            noise_random,
            rule,
            template,
            eroded_badlands: biomes.get(&Identifier::vanilla("eroded_badlands")),
            frozen_oceans: [
                biomes.get(&Identifier::vanilla("frozen_ocean")),
                biomes.get(&Identifier::vanilla("deep_frozen_ocean")),
            ],
            packed_ice: reg.vanilla_default("packed_ice"),
            snow_block: reg.vanilla_default("snow_block"),
            water_block: reg.block_of(reg.vanilla_default("water")),
            default_block_id: reg.block_of(settings.default_block),
            biomes,
        })
    }

    fn surface_depth(&self, x: i32, z: i32) -> i32 {
        let noise = self.surface.get(x as f64, 0.0, z as f64) as f64;
        (noise * 2.75 + 3.0 + self.noise_random.at(x, 0, z).next_double() * 0.25) as i32
    }

    fn surface_secondary(&self, x: i32, z: i32) -> f64 {
        self.surface_secondary.get(x as f64, 0.0, z as f64) as f64
    }

    fn band(&self, x: i32, y: i32, z: i32) -> BlockState {
        let offset = round_f32(self.clay_bands_offset.get(x as f64, 0.0, z as f64) * 4.0);
        let len = self.clay_bands.len() as i32;
        self.clay_bands[((y + offset + len) % len) as usize]
    }
}

// ---------------------------------------------------------------------------------------------
// Per-chunk compiled rules.

// ---------------------------------------------------------------------------------------------
// World-level template: the rule tree with noises, samplers and random factories resolved.

enum TCond {
    Biome(FxHashSet<BiomeId>, Vec<BiomeId>),
    Noise {
        supplier: usize,
        min: f64,
        max: f64,
    },
    VerticalGradient {
        true_at: VerticalAnchor,
        false_at: VerticalAnchor,
        random: PositionalRandomFactory,
    },
    YAbove {
        anchor: VerticalAnchor,
        mult: i32,
        add_stone_depth: bool,
    },
    Water {
        offset: i32,
        mult: i32,
        add_stone_depth: bool,
    },
    Temperature,
    Steep,
    Not(Box<TCond>),
    Hole,
    AbovePreliminarySurface,
    StoneDepth {
        offset: i32,
        add_surface_depth: bool,
        secondary_range: i32,
        ceiling: bool,
    },
}

enum TRule {
    Block(BlockState),
    Bandlands,
    Sequence(Vec<TRule>),
    Condition(TCond, Box<TRule>),
    OreVein {
        density: SamplerRef,
        richness: SamplerRef,
        filler_gap: SamplerRef,
        random: PositionalRandomFactory,
        ore: BlockState,
        raw_ore: BlockState,
        filler: BlockState,
        chance: f32,
    },
}

struct Template {
    rule: TRule,
    /// Noise suppliers deduplicated by `(noise, is_3d)` like `MaterialRuleContext`'s supplier map.
    suppliers: Vec<(Arc<NoiseStack>, bool)>,
}

type SupplierIndex = FxHashMap<(Identifier, bool), usize>;

impl Template {
    fn build(state: &RandomState, rule: &MaterialRule) -> Result<Self> {
        let mut index = SupplierIndex::default();
        let mut suppliers = Vec::new();
        let rule = Self::rule(state, rule, &mut index, &mut suppliers)?;
        Ok(Self { rule, suppliers })
    }

    fn rule(
        state: &RandomState,
        rule: &MaterialRule,
        index: &mut SupplierIndex,
        suppliers: &mut Vec<(Arc<NoiseStack>, bool)>,
    ) -> Result<TRule> {
        Ok(match rule {
            MaterialRule::Block(s) => TRule::Block(*s),
            MaterialRule::Bandlands => TRule::Bandlands,
            MaterialRule::Sequence(list) => {
                if list.len() == 1 {
                    Self::rule(state, &list[0], index, suppliers)?
                } else {
                    TRule::Sequence(
                        list.iter()
                            .map(|r| Self::rule(state, r, index, suppliers))
                            .collect::<Result<_>>()?,
                    )
                }
            }
            MaterialRule::Condition(c, r) => {
                let c = Self::cond(state, c, index, suppliers)?;
                TRule::Condition(c, Box::new(Self::rule(state, r, index, suppliers)?))
            }
            MaterialRule::OreVein {
                ore,
                raw_ore,
                filler,
                raw_ore_chance,
                density,
                richness,
                filler_gap,
            } => TRule::OreVein {
                density: state.sampler(density),
                richness: state.sampler(richness),
                filler_gap: state.sampler(filler_gap),
                random: state.random_factory("minecraft:ore"),
                ore: *ore,
                raw_ore: *raw_ore,
                filler: *filler,
                chance: *raw_ore_chance,
            },
        })
    }

    fn cond(
        state: &RandomState,
        cond: &MaterialCondition,
        index: &mut SupplierIndex,
        suppliers: &mut Vec<(Arc<NoiseStack>, bool)>,
    ) -> Result<TCond> {
        Ok(match cond {
            MaterialCondition::Biome(list) => TCond::Biome(list.iter().copied().collect(), list.clone()),
            MaterialCondition::NoiseThreshold {
                noise,
                min,
                max,
                is_3d,
            } => {
                let key = (noise.clone(), *is_3d);
                let supplier = match index.get(&key) {
                    Some(i) => *i,
                    None => {
                        suppliers.push((state.noise(noise)?, *is_3d));
                        index.insert(key, suppliers.len() - 1);
                        suppliers.len() - 1
                    }
                };
                TCond::Noise {
                    supplier,
                    min: *min,
                    max: *max,
                }
            }
            MaterialCondition::VerticalGradient {
                random_name,
                true_at_and_below,
                false_at_and_above,
            } => TCond::VerticalGradient {
                true_at: *true_at_and_below,
                false_at: *false_at_and_above,
                random: state.random_factory(random_name),
            },
            MaterialCondition::YAbove {
                anchor,
                surface_depth_multiplier,
                add_stone_depth,
            } => TCond::YAbove {
                anchor: *anchor,
                mult: *surface_depth_multiplier,
                add_stone_depth: *add_stone_depth,
            },
            MaterialCondition::Water {
                offset,
                surface_depth_multiplier,
                add_stone_depth,
            } => TCond::Water {
                offset: *offset,
                mult: *surface_depth_multiplier,
                add_stone_depth: *add_stone_depth,
            },
            MaterialCondition::Temperature => TCond::Temperature,
            MaterialCondition::Steep => TCond::Steep,
            MaterialCondition::Not(target) => {
                TCond::Not(Box::new(Self::cond(state, target, index, suppliers)?))
            }
            MaterialCondition::Hole => TCond::Hole,
            MaterialCondition::AbovePreliminarySurface => TCond::AbovePreliminarySurface,
            MaterialCondition::StoneDepth {
                offset,
                add_surface_depth,
                secondary_depth_range,
                ceiling,
            } => TCond::StoneDepth {
                offset: *offset,
                add_surface_depth: *add_surface_depth,
                secondary_range: *secondary_depth_range,
                ceiling: *ceiling,
            },
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Per-chunk compiled rules.

enum Laziness {
    None,
    Xz,
    Y,
}

enum CondKind<'a> {
    Const(bool),
    Biome(&'a FxHashSet<BiomeId>),
    Noise {
        supplier: usize,
        min: f64,
        max: f64,
    },
    VerticalGradient {
        true_at: i32,
        false_at: i32,
        random: PositionalRandomFactory,
    },
    YAbove {
        anchor: VerticalAnchor,
        mult: i32,
        add_stone_depth: bool,
    },
    Water {
        offset: i32,
        mult: i32,
        add_stone_depth: bool,
    },
    Temperature,
    Steep,
    Not(usize),
    Hole,
    AbovePreliminarySurface,
    StoneDepth {
        offset: i32,
        add_surface_depth: bool,
        secondary_range: i32,
        ceiling: bool,
    },
}

struct Cond<'a> {
    kind: CondKind<'a>,
    laziness: Laziness,
    last: i64,
    result: bool,
}

struct NoiseSupplier<'a> {
    noise: &'a NoiseStack,
    is_3d: bool,
    last: i64,
    value: f64,
}

enum Getter<'a> {
    Prefilled(&'a SamplerRef, Vec<f32>),
    Direct(&'a SamplerRef),
}

enum Rule<'a> {
    Block(BlockState),
    Bandlands,
    Sequence(Vec<Rule<'a>>),
    Condition(usize, Box<Rule<'a>>),
    OreVein {
        density: Getter<'a>,
        richness: Getter<'a>,
        filler_gap: Getter<'a>,
        random: PositionalRandomFactory,
        ore: BlockState,
        raw_ore: BlockState,
        filler: BlockState,
        chance: f32,
    },
}

/// `MaterialRuleContext` + compiled evaluators for one chunk.
pub struct RuleContext<'a> {
    system: &'a MaterialSystem,
    pub samplers: &'a mut SamplerContext,
    biome_getter: &'a mut dyn FnMut(i32, i32, i32) -> BiomeId,
    gen_ctx: GenerationContext,
    possible_biomes: Option<&'a FxHashSet<BiomeId>>,
    expected_volume: DensityVolume,
    prelim_volume: DensityVolume,
    prelim_buffer: Option<Vec<f32>>,
    conds: Vec<Cond<'a>>,
    suppliers: Vec<NoiseSupplier<'a>>,
    last_xz: i64,
    last_y: i64,
    block_x: i32,
    block_y: i32,
    block_z: i32,
    gradient_x: i32,
    gradient_z: i32,
    surface_depth: i32,
    last_secondary: i64,
    secondary: f64,
    last_min_surface: i64,
    min_surface: i32,
    biome: Option<BiomeId>,
    water_height: i32,
    stone_below: i32,
    stone_above: i32,
}

impl<'a> RuleContext<'a> {
    pub fn new(
        system: &'a MaterialSystem,
        samplers: &'a mut SamplerContext,
        expected_volume: DensityVolume,
        biome_getter: &'a mut dyn FnMut(i32, i32, i32) -> BiomeId,
        gen_ctx: GenerationContext,
        possible_biomes: Option<&'a FxHashSet<BiomeId>>,
    ) -> Self {
        let start = i64::MIN + 1;
        Self {
            system,
            samplers,
            biome_getter,
            gen_ctx,
            possible_biomes,
            prelim_volume: DensityVolume::blocks(
                expected_volume.size_x,
                1,
                expected_volume.size_z,
                expected_volume.min_x,
                0,
                expected_volume.min_z,
            ),
            expected_volume,
            prelim_buffer: None,
            conds: Vec::new(),
            suppliers: Vec::new(),
            last_xz: start,
            last_y: start,
            block_x: 0,
            block_y: 0,
            block_z: 0,
            gradient_x: 0,
            gradient_z: 0,
            surface_depth: 0,
            last_secondary: start - 1,
            secondary: 0.0,
            last_min_surface: start - 1,
            min_surface: 0,
            biome: None,
            water_height: 0,
            stone_below: 0,
            stone_above: 0,
        }
    }

    fn update_xz(&mut self, x: i32, z: i32, gx: i32, gz: i32) {
        self.last_xz = self.last_xz.wrapping_add(1);
        self.last_y = self.last_y.wrapping_add(1);
        self.block_x = x;
        self.block_z = z;
        self.gradient_x = gx;
        self.gradient_z = gz;
        self.surface_depth = self.system.surface_depth(x, z);
    }

    fn update_y(&mut self, stone_above: i32, stone_below: i32, water_height: i32, y: i32) {
        self.last_y = self.last_y.wrapping_add(1);
        self.biome = None;
        self.block_y = y;
        self.water_height = water_height;
        self.stone_below = stone_below;
        self.stone_above = stone_above;
    }

    fn get_surface_secondary(&mut self) -> f64 {
        if self.last_secondary != self.last_xz {
            self.last_secondary = self.last_xz;
            self.secondary = self.system.surface_secondary(self.block_x, self.block_z);
        }
        self.secondary
    }

    fn get_biome(&mut self) -> BiomeId {
        if let Some(b) = self.biome {
            return b;
        }
        let b = (self.biome_getter)(self.block_x, self.block_y, self.block_z);
        self.biome = Some(b);
        b
    }

    pub fn get_min_surface_level(&mut self) -> i32 {
        if self.last_min_surface != self.last_xz {
            self.last_min_surface = self.last_xz;
            let level = match self.prelim_volume.index_of_block(self.block_x, 0, self.block_z) {
                Some(i) => {
                    if self.prelim_buffer.is_none() {
                        let mut buf = vec![0.0f32; self.prelim_volume.size()];
                        let vol = self.prelim_volume;
                        self.system
                            .preliminary_surface
                            .sample_volume(self.samplers, &mut buf, &vol);
                        self.prelim_buffer = Some(buf);
                    }
                    self.prelim_buffer.as_ref().unwrap()[i]
                }
                None => {
                    self.system
                        .preliminary_surface
                        .sample_value(self.samplers, self.block_x, 0, self.block_z)
                }
            };
            self.min_surface = floor_f32(level) + self.surface_depth - 8;
        }
        self.min_surface
    }

    fn getter(&mut self, sampler: &'a SamplerRef, prefill: bool) -> Getter<'a> {
        if prefill {
            let mut buf = vec![0.0f32; self.expected_volume.size()];
            let vol = self.expected_volume;
            sampler.sample_volume(self.samplers, &mut buf, &vol);
            Getter::Prefilled(sampler, buf)
        } else {
            Getter::Direct(sampler)
        }
    }

    fn get(&mut self, g: &Getter<'a>) -> f32 {
        let (x, y, z) = (self.block_x, self.block_y, self.block_z);
        match g {
            Getter::Prefilled(s, buf) => match self.expected_volume.index_of_block(x, y, z) {
                Some(i) => buf[i],
                None => s.sample_value(self.samplers, x, y, z),
            },
            Getter::Direct(s) => s.sample_value(self.samplers, x, y, z),
        }
    }

    // --- compilation (order matters: prefilled getters sample through the shared caches) ---

    /// Compiles the world's rule template for this chunk.
    fn compile(&mut self) -> Rule<'a> {
        let template = &self.system.template;
        let (xz, y) = (self.last_xz.wrapping_sub(1), self.last_y.wrapping_sub(1));
        self.suppliers = template
            .suppliers
            .iter()
            .map(|(noise, is_3d)| NoiseSupplier {
                noise,
                is_3d: *is_3d,
                last: if *is_3d { y } else { xz },
                value: 0.0,
            })
            .collect();
        self.compile_rule(&template.rule)
    }

    fn compile_rule(&mut self, rule: &'a TRule) -> Rule<'a> {
        match rule {
            TRule::Block(s) => Rule::Block(*s),
            TRule::Bandlands => Rule::Bandlands,
            TRule::Sequence(list) => Rule::Sequence(list.iter().map(|r| self.compile_rule(r)).collect()),
            TRule::Condition(c, r) => {
                let c = self.compile_cond(c);
                let r = self.compile_rule(r);
                Rule::Condition(c, Box::new(r))
            }
            TRule::OreVein {
                density,
                richness,
                filler_gap,
                random,
                ore,
                raw_ore,
                filler,
                chance,
            } => {
                let density = self.getter(density, true);
                let richness = self.getter(richness, true);
                let filler_gap = self.getter(filler_gap, false);
                Rule::OreVein {
                    density,
                    richness,
                    filler_gap,
                    random: *random,
                    ore: *ore,
                    raw_ore: *raw_ore,
                    filler: *filler,
                    chance: *chance,
                }
            }
        }
    }

    fn push_cond(&mut self, kind: CondKind<'a>, laziness: Laziness) -> usize {
        let last = match laziness {
            Laziness::Xz => self.last_xz.wrapping_sub(1),
            Laziness::Y => self.last_y.wrapping_sub(1),
            Laziness::None => 0,
        };
        self.conds.push(Cond {
            kind,
            laziness,
            last,
            result: false,
        });
        self.conds.len() - 1
    }

    fn compile_cond(&mut self, cond: &'a TCond) -> usize {
        match cond {
            TCond::Biome(set, list) => {
                if let Some(possible) = self.possible_biomes {
                    if !list.iter().any(|b| possible.contains(b)) {
                        return self.push_cond(CondKind::Const(false), Laziness::None);
                    }
                    if possible.iter().all(|b| set.contains(b)) {
                        return self.push_cond(CondKind::Const(true), Laziness::None);
                    }
                }
                self.push_cond(CondKind::Biome(set), Laziness::Y)
            }
            &TCond::Noise { supplier, min, max } => {
                self.push_cond(CondKind::Noise { supplier, min, max }, Laziness::None)
            }
            TCond::VerticalGradient {
                true_at,
                false_at,
                random,
            } => {
                let true_at = true_at.resolve(&self.gen_ctx);
                let false_at = false_at.resolve(&self.gen_ctx);
                self.push_cond(
                    CondKind::VerticalGradient {
                        true_at,
                        false_at,
                        random: *random,
                    },
                    Laziness::Y,
                )
            }
            &TCond::YAbove {
                anchor,
                mult,
                add_stone_depth,
            } => self.push_cond(
                CondKind::YAbove {
                    anchor,
                    mult,
                    add_stone_depth,
                },
                Laziness::Y,
            ),
            &TCond::Water {
                offset,
                mult,
                add_stone_depth,
            } => self.push_cond(
                CondKind::Water {
                    offset,
                    mult,
                    add_stone_depth,
                },
                Laziness::Y,
            ),
            TCond::Temperature => self.push_cond(CondKind::Temperature, Laziness::None),
            TCond::Steep => self.push_cond(CondKind::Steep, Laziness::Xz),
            TCond::Not(target) => {
                let t = self.compile_cond(target);
                self.push_cond(CondKind::Not(t), Laziness::None)
            }
            TCond::Hole => self.push_cond(CondKind::Hole, Laziness::Xz),
            TCond::AbovePreliminarySurface => {
                self.push_cond(CondKind::AbovePreliminarySurface, Laziness::None)
            }
            &TCond::StoneDepth {
                offset,
                add_surface_depth,
                secondary_range,
                ceiling,
            } => self.push_cond(
                CondKind::StoneDepth {
                    offset,
                    add_surface_depth,
                    secondary_range,
                    ceiling,
                },
                Laziness::Y,
            ),
        }
    }

    // --- evaluation ---

    fn test(&mut self, i: usize) -> bool {
        let stamp = match self.conds[i].laziness {
            Laziness::None => None,
            Laziness::Xz => Some(self.last_xz),
            Laziness::Y => Some(self.last_y),
        };
        if let Some(stamp) = stamp {
            if self.conds[i].last == stamp {
                return self.conds[i].result;
            }
            let r = self.compute(i);
            self.conds[i].last = stamp;
            self.conds[i].result = r;
            r
        } else {
            self.compute(i)
        }
    }

    fn compute(&mut self, i: usize) -> bool {
        // Copy out the small descriptors to keep the borrow checker happy.
        match &self.conds[i].kind {
            CondKind::Const(v) => *v,
            &CondKind::Biome(set) => {
                let b = self.get_biome();
                set.contains(&b)
            }
            &CondKind::Noise { supplier, min, max } => {
                let stamp = if self.suppliers[supplier].is_3d {
                    self.last_y
                } else {
                    self.last_xz
                };
                let s = &mut self.suppliers[supplier];
                if s.last != stamp {
                    s.value = if s.is_3d {
                        s.noise
                            .get(self.block_x as f64, self.block_y as f64, self.block_z as f64)
                            as f64
                    } else {
                        s.noise.get(self.block_x as f64, 0.0, self.block_z as f64) as f64
                    };
                    s.last = stamp;
                }
                let v = s.value;
                v >= min && v <= max
            }
            &CondKind::VerticalGradient {
                true_at,
                false_at,
                random,
            } => {
                let y = self.block_y;
                if y <= true_at {
                    true
                } else if y >= false_at {
                    false
                } else {
                    let p = map_f64(y as f64, true_at as f64, false_at as f64, 1.0, 0.0);
                    (random.at(self.block_x, y, self.block_z).next_float() as f64) < p
                }
            }
            &CondKind::YAbove {
                anchor,
                mult,
                add_stone_depth,
            } => {
                self.block_y + if add_stone_depth { self.stone_above } else { 0 }
                    >= anchor.resolve(&self.gen_ctx) + self.surface_depth * mult
            }
            &CondKind::Water {
                offset,
                mult,
                add_stone_depth,
            } => {
                self.water_height == i32::MIN
                    || self.block_y + if add_stone_depth { self.stone_above } else { 0 }
                        >= self.water_height + offset + self.surface_depth * mult
            }
            CondKind::Temperature => {
                let b = self.get_biome();
                self.system.biomes.data(b).cold_enough_to_snow(
                    self.block_x,
                    self.block_y,
                    self.block_z,
                    self.system.sea_level,
                )
            }
            CondKind::Steep => self.gradient_x <= -4 || self.gradient_z >= 4,
            &CondKind::Not(t) => !self.test(t),
            CondKind::Hole => self.surface_depth <= 0,
            CondKind::AbovePreliminarySurface => self.block_y >= self.get_min_surface_level(),
            &CondKind::StoneDepth {
                offset,
                add_surface_depth,
                secondary_range,
                ceiling,
            } => {
                let stone = if ceiling {
                    self.stone_below
                } else {
                    self.stone_above
                };
                let surface = if add_surface_depth { self.surface_depth } else { 0 };
                let secondary = if secondary_range == 0 {
                    0
                } else {
                    map_f64(
                        self.get_surface_secondary(),
                        -1.0,
                        1.0,
                        0.0,
                        secondary_range as f64,
                    ) as i32
                };
                stone <= 1 + offset + surface + secondary
            }
        }
    }

    fn try_apply(&mut self, rule: &Rule<'a>, x: i32, y: i32, z: i32) -> Option<BlockState> {
        match rule {
            Rule::Block(s) => Some(*s),
            Rule::Bandlands => Some(self.system.band(x, y, z)),
            Rule::Sequence(list) => {
                for r in list {
                    if let Some(s) = self.try_apply(r, x, y, z) {
                        return Some(s);
                    }
                }
                None
            }
            Rule::Condition(c, r) => {
                if !self.test(*c) {
                    None
                } else {
                    self.try_apply(r, x, y, z)
                }
            }
            Rule::OreVein {
                density,
                richness,
                filler_gap,
                random,
                ore,
                raw_ore,
                filler,
                chance,
            } => {
                let d = self.get(density);
                if d <= 0.0 {
                    return None;
                }
                let mut r = random.at(x, y, z);
                if r.next_float() > d {
                    return None;
                }
                let rich = self.get(richness);
                if r.next_float() < rich && self.get(filler_gap) < 0.0 {
                    Some(if r.next_float() < *chance { *raw_ore } else { *ore })
                } else {
                    Some(*filler)
                }
            }
        }
    }
}

/// Inputs of `MaterialSystem.buildSurface` that come from the generation region.
pub struct SurfaceInputs<'a> {
    pub biome_getter: &'a mut dyn FnMut(i32, i32, i32) -> BiomeId,
    pub possible_biomes: &'a FxHashSet<BiomeId>,
    pub gen_ctx: GenerationContext,
}

impl MaterialSystem {
    /// `MaterialSystem.buildSurface`.
    pub fn build_surface(
        &self,
        chunk: &mut ProtoChunk,
        samplers: &mut SamplerContext,
        noise_volume: &DensityVolume,
        inputs: SurfaceInputs,
    ) {
        let min_x = chunk.min_block_x();
        let min_z = chunk.min_block_z();
        let min_y = chunk.min_y;
        let max_y = chunk.max_y();
        let highest = chunk.highest_filled_section_index();
        let max_block_y = (chunk.min_section_y() + highest) * 16 + 15;
        let narrowed = DensityVolume::blocks(
            noise_volume.size_x,
            (max_block_y - noise_volume.min_y + 1).max(1),
            noise_volume.size_z,
            noise_volume.min_x,
            noise_volume.min_y,
            noise_volume.min_z,
        );
        let SurfaceInputs {
            biome_getter,
            possible_biomes,
            gen_ctx,
        } = inputs;
        let mut ctx = RuleContext::new(
            self,
            samplers,
            narrowed,
            biome_getter,
            gen_ctx,
            Some(possible_biomes),
        );
        let t = crate::terrain::profile::Timer::start();
        let rule = ctx.compile();
        t.lap(crate::terrain::profile::Phase::SurfaceCompile);
        let reg = chunk.blocks.clone();
        for x in 0..16 {
            for z in 0..16 {
                let bx = min_x + x;
                let bz = min_z + z;
                let starting_height = chunk.height_at(HeightmapKind::WorldSurfaceWg, x, z) + 1;
                let surface_biome = (ctx.biome_getter)(bx, starting_height, bz);
                if Some(surface_biome) == self.eroded_badlands {
                    self.eroded_badlands_extension(chunk, bx, bz, starting_height);
                }
                let height = chunk.height_at(HeightmapKind::WorldSurfaceWg, x, z) + 1;
                let gx = chunk.height_at(HeightmapKind::WorldSurfaceWg, (x + 1).min(15), z)
                    - chunk.height_at(HeightmapKind::WorldSurfaceWg, (x - 1).max(0), z);
                let gz = chunk.height_at(HeightmapKind::WorldSurfaceWg, x, (z + 1).min(15))
                    - chunk.height_at(HeightmapKind::WorldSurfaceWg, x, (z - 1).max(0));
                ctx.update_xz(bx, bz, gx, gz);
                let mut stone_above = 0;
                let mut water_height = i32::MIN;
                let mut next_ceiling = i32::MAX;
                let end_y = chunk.min_y;
                let mut y = height;
                while y >= end_y {
                    let old = chunk.get_block(bx, y, bz);
                    let f = reg.flags(old);
                    if f.is_air() {
                        stone_above = 0;
                        water_height = i32::MIN;
                    } else if f.has_fluid() {
                        if water_height == i32::MIN {
                            water_height = y + 1;
                        }
                    } else {
                        if next_ceiling >= y {
                            next_ceiling = crate::terrain::aquifer::WAY_BELOW_MIN_Y;
                            let mut ly = y - 1;
                            while ly >= end_y - 1 {
                                let s = chunk.get_block(bx, ly, bz);
                                let sf = reg.flags(s);
                                if sf.is_air() || sf.has_fluid() {
                                    next_ceiling = ly + 1;
                                    break;
                                }
                                ly -= 1;
                            }
                        }
                        stone_above += 1;
                        let stone_below = y - next_ceiling + 1;
                        ctx.update_y(stone_above, stone_below, water_height, y);
                        if y >= min_y
                            && y <= max_y
                            && let Some(state) = ctx.try_apply(&rule, bx, y, bz)
                        {
                            set_column_block(chunk, bx, y, bz, state, min_y, max_y);
                        }
                    }
                    y -= 1;
                }
                if self.frozen_oceans.contains(&Some(surface_biome)) {
                    let min_surface = ctx.get_min_surface_level();
                    self.frozen_ocean_extension(chunk, min_surface, surface_biome, bx, bz, starting_height);
                }
            }
        }
    }

    /// `MaterialSystem.topMaterial` (used when carving exposes dirt below grass).
    pub fn top_material(
        &self,
        chunk: &mut ProtoChunk,
        samplers: &mut SamplerContext,
        biome_getter: &mut dyn FnMut(i32, i32, i32) -> BiomeId,
        gen_ctx: GenerationContext,
        x: i32,
        y: i32,
        z: i32,
        under_fluid: bool,
    ) -> Option<BlockState> {
        let volume = DensityVolume::blocks(1, 1, 1, x, y, z);
        let mut ctx = RuleContext::new(self, samplers, volume, biome_getter, gen_ctx, None);
        let rule = ctx.compile();
        let lx = x & 15;
        let lz = z & 15;
        let gx = chunk.height_at(HeightmapKind::WorldSurfaceWg, (lx + 1).min(15), lz)
            - chunk.height_at(HeightmapKind::WorldSurfaceWg, (lx - 1).max(0), lz);
        let gz = chunk.height_at(HeightmapKind::WorldSurfaceWg, lx, (lz + 1).min(15))
            - chunk.height_at(HeightmapKind::WorldSurfaceWg, lx, (lz - 1).max(0));
        ctx.update_xz(x, z, gx, gz);
        ctx.update_y(1, 1, if under_fluid { y + 1 } else { i32::MIN }, y);
        ctx.try_apply(&rule, x, y, z)
    }

    fn eroded_badlands_extension(&self, chunk: &mut ProtoChunk, x: i32, z: i32, height: i32) {
        let buffer = min_f64(
            (self.badlands_surface.get(x as f64, 0.0, z as f64) as f64 * 8.25).abs(),
            (self.badlands_pillar.get(x as f64 * 0.2, 0.0, z as f64 * 0.2) * 15.0) as f64,
        );
        if buffer <= 0.0 {
            return;
        }
        let floor = (self
            .badlands_pillar_roof
            .get(x as f64 * 0.75, 0.0, z as f64 * 0.75) as f64
            * 1.5)
            .abs();
        let top = 64.0 + min_f64(buffer * buffer * 2.5, (floor * 50.0).ceil() + 24.0);
        let start_y = crate::math::floor(top);
        if height > start_y {
            return;
        }
        let reg = chunk.blocks.clone();
        let mut y = start_y;
        while y >= chunk.min_y {
            let old = chunk.get_block(x, y, z);
            if reg.block_of(old) == self.default_block_id {
                break;
            }
            if reg.block_of(old) == self.water_block {
                return;
            }
            y -= 1;
        }
        let (min_y, max_y) = (chunk.min_y, chunk.max_y());
        let mut y = start_y;
        while y >= chunk.min_y && reg.flags(chunk.get_block(x, y, z)).is_air() {
            set_column_block(chunk, x, y, z, self.default_block, min_y, max_y);
            y -= 1;
        }
    }

    fn frozen_ocean_extension(
        &self,
        chunk: &mut ProtoChunk,
        min_surface: i32,
        biome: BiomeId,
        x: i32,
        z: i32,
        height: i32,
    ) {
        let iceberg = min_f64(
            (self.iceberg_surface.get(x as f64, 0.0, z as f64) as f64 * 8.25).abs(),
            (self.iceberg_pillar.get(x as f64 * 1.28, 0.0, z as f64 * 1.28) * 15.0) as f64,
        );
        if iceberg <= 1.8 {
            return;
        }
        let roof = (self
            .iceberg_pillar_roof
            .get(x as f64 * 1.17, 0.0, z as f64 * 1.17) as f64
            * 1.5)
            .abs();
        let mut top = min_f64(iceberg * iceberg * 1.2, (roof * 40.0).ceil() + 14.0);
        if self.biomes.data(biome).should_melt_frozen_ocean_iceberg_slightly(
            x,
            self.sea_level,
            z,
            self.sea_level,
        ) {
            top -= 2.0;
        }
        if top <= 2.0 {
            return;
        }
        let bottom = self.sea_level as f64 - top - 7.0;
        top += self.sea_level as f64;
        let extension_top = top;
        let mut random = self.noise_random.at(x, 0, z);
        let max_snow_depth = 2 + random.next_int_bounded(4);
        let min_snow_height = self.sea_level + 18 + random.next_int_bounded(10);
        let mut snow_depth = 0;
        let reg = chunk.blocks.clone();
        let (min_y, max_y) = (chunk.min_y, chunk.max_y());
        let mut y = height.max(top as i32 + 1);
        while y >= min_surface {
            let block = chunk.get_block(x, y, z);
            let cond_air =
                reg.flags(block).is_air() && y < extension_top as i32 && random.next_double() > 0.01;
            let place = cond_air || {
                let b = chunk.get_block(x, y, z);
                reg.block_of(b) == self.water_block
                    && y > bottom as i32
                    && y < self.sea_level
                    && random.next_double() > 0.15
            };
            if place {
                if snow_depth <= max_snow_depth && y > min_snow_height {
                    set_column_block(chunk, x, y, z, self.snow_block, min_y, max_y);
                    snow_depth += 1;
                } else {
                    set_column_block(chunk, x, y, z, self.packed_ice, min_y, max_y);
                }
            }
            y -= 1;
        }
    }
}

/// The `BlockColumn.setBlock` of `buildSurface`.
fn set_column_block(
    chunk: &mut ProtoChunk,
    x: i32,
    y: i32,
    z: i32,
    state: BlockState,
    min_y: i32,
    max_y: i32,
) {
    if y >= min_y && y <= max_y {
        chunk.set_block(x, y, z, state);
        if chunk.blocks.flags(state).has_fluid() {
            chunk.mark_post_processing(x, y, z);
        }
    }
}
