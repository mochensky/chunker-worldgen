//! `GeodeFeature`.

use std::sync::{Arc, OnceLock};

use anyhow::Result;
use chunker_data::blocks::{BlockState, Fluid};
use serde_json::Value;

use crate::block_set::StateSet;
use crate::codec::{as_array, field, i32_field, opt_f64, opt_i32};
use crate::feature::Region;
use crate::feature::provider::StateProvider;
use crate::feature::registry::Parser;
use crate::noise::{NoiseStack, NormalNoise};
use crate::providers::IntProvider;
use crate::random::{AnyRandom, LegacyRandom, RandomSource, WorldgenRandom};
use crate::structure::geom::{BlockPos, Direction};

pub struct Geode {
    filling: Arc<StateProvider>,
    inner: Arc<StateProvider>,
    alternate_inner: Arc<StateProvider>,
    middle: Arc<StateProvider>,
    outer: Arc<StateProvider>,
    inner_placements: Vec<BlockState>,
    cannot_replace: Arc<StateSet>,
    invalid_blocks: Arc<StateSet>,
    layer_filling: f64,
    layer_inner: f64,
    layer_middle: f64,
    layer_outer: f64,
    crack_chance: f64,
    base_crack_size: f64,
    crack_point_offset: i32,
    use_potential_placements_chance: f64,
    use_alternate_layer0_chance: f64,
    placements_require_layer0_alternate: bool,
    outer_wall_distance: IntProvider,
    distribution_points: IntProvider,
    point_offset: IntProvider,
    min_gen_offset: i32,
    max_gen_offset: i32,
    noise_multiplier: f64,
    invalid_blocks_threshold: i32,
    /// `NOISE_PARAMETERS.create(new WorldgenRandom(new LegacyRandomSource(level.getSeed())))`, per world seed.
    noise: OnceLock<(i64, NoiseStack)>,
}

fn int_or(json: &Value, name: &str, min: i32, max: i32) -> Result<IntProvider> {
    match json.get(name) {
        Some(v) => IntProvider::parse(v),
        None => Ok(IntProvider::Uniform { min, max }),
    }
}

impl Geode {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let b = field(json, "blocks")?;
        let layers = field(json, "layers")?;
        let crack = field(json, "crack")?;
        Ok(Self {
            filling: p.provider(field(b, "filling_provider")?)?,
            inner: p.provider(field(b, "inner_layer_provider")?)?,
            alternate_inner: p.provider(field(b, "alternate_inner_layer_provider")?)?,
            middle: p.provider(field(b, "middle_layer_provider")?)?,
            outer: p.provider(field(b, "outer_layer_provider")?)?,
            inner_placements: as_array(field(b, "inner_placements")?)?
                .iter()
                .map(|s| p.state(s))
                .collect::<Result<_>>()?,
            cannot_replace: p.block_set(field(b, "cannot_replace")?)?,
            invalid_blocks: p.block_set(field(b, "invalid_blocks")?)?,
            layer_filling: opt_f64(layers, "filling", 1.7)?,
            layer_inner: opt_f64(layers, "inner_layer", 2.2)?,
            layer_middle: opt_f64(layers, "middle_layer", 3.2)?,
            layer_outer: opt_f64(layers, "outer_layer", 4.2)?,
            crack_chance: opt_f64(crack, "generate_crack_chance", 1.0)?,
            base_crack_size: opt_f64(crack, "base_crack_size", 2.0)?,
            crack_point_offset: opt_i32(crack, "crack_point_offset", 2)?,
            use_potential_placements_chance: opt_f64(json, "use_potential_placements_chance", 0.35)?,
            use_alternate_layer0_chance: opt_f64(json, "use_alternate_layer0_chance", 0.0)?,
            placements_require_layer0_alternate: crate::codec::opt_bool(
                json,
                "placements_require_layer0_alternate",
                true,
            )?,
            outer_wall_distance: int_or(json, "outer_wall_distance", 4, 5)?,
            distribution_points: int_or(json, "distribution_points", 3, 4)?,
            point_offset: int_or(json, "point_offset", 1, 2)?,
            min_gen_offset: opt_i32(json, "min_gen_offset", -16)?,
            max_gen_offset: opt_i32(json, "max_gen_offset", 16)?,
            noise_multiplier: opt_f64(json, "noise_multiplier", 0.05)?,
            invalid_blocks_threshold: i32_field(json, "invalid_blocks_threshold")?,
            noise: OnceLock::new(),
        })
    }

    fn noise(&self, seed: i64) -> &NoiseStack {
        let (s, n) = self.noise.get_or_init(|| {
            let mut r = WorldgenRandom::new(AnyRandom::Legacy(LegacyRandom::new(seed)));
            (seed, NormalNoise::create_parity(-4, &[1.0]).create(&mut r))
        });
        assert_eq!(*s, seed, "geode noise built for another seed");
        n
    }

    fn safe_set(&self, region: &mut Region, p: BlockPos, s: BlockState) {
        if !self.cannot_replace.contains(region.get(p)) {
            region.set(p, s, 2);
        }
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let num_points = self.distribution_points.sample(random);
        let noise = self.noise(region.env.seed);
        let crack_adjust = num_points as f64 / self.outer_wall_distance.max_inclusive() as f64;
        let inner_air = 1.0 / self.layer_filling.sqrt();
        let innermost = 1.0 / (self.layer_inner + crack_adjust).sqrt();
        let inner_crust = 1.0 / (self.layer_middle + crack_adjust).sqrt();
        let outer_crust = 1.0 / (self.layer_outer + crack_adjust).sqrt();
        let crack_size = 1.0
            / (self.base_crack_size
                + random.next_double() / 2.0
                + if num_points > 3 { crack_adjust } else { 0.0 })
            .sqrt();
        let generate_crack = (random.next_float() as f64) < self.crack_chance;
        let mut invalid = 0;
        let mut points: Vec<(BlockPos, i32)> = Vec::new();
        for _ in 0..num_points {
            let x = self.outer_wall_distance.sample(random);
            let y = self.outer_wall_distance.sample(random);
            let z = self.outer_wall_distance.sample(random);
            let pos = origin.offset(x, y, z);
            let s = region.get(pos);
            if region.env.blocks.flags(s).is_air() || self.invalid_blocks.contains(s) {
                invalid += 1;
                if invalid > self.invalid_blocks_threshold {
                    return false;
                }
            }
            points.push((pos, self.point_offset.sample(random)));
        }
        let mut crack_points = Vec::new();
        if generate_crack {
            let index = random.next_int_bounded(4);
            let off = num_points * 2 + 1;
            let (dx, dz) = match index {
                0 => (off, 0),
                1 => (0, off),
                2 => (off, off),
                _ => (0, 0),
            };
            for y in [7, 5, 1] {
                crack_points.push(origin.offset(dx, y, dz));
            }
        }
        let dist_sqr = |a: BlockPos, b: BlockPos| {
            let (x, y, z) = ((a.x - b.x) as f64, (a.y - b.y) as f64, (a.z - b.z) as f64);
            x * x + y * y + z * z
        };
        let mut crystals = Vec::new();
        let (lo, hi) = (self.min_gen_offset, self.max_gen_offset);
        for z in lo..=hi {
            for y in lo..=hi {
                for x in lo..=hi {
                    let p = origin.offset(x, y, z);
                    let noise_offset =
                        noise.get(p.x as f64, p.y as f64, p.z as f64) as f64 * self.noise_multiplier;
                    let mut shell = 0.0;
                    for (pp, o) in &points {
                        shell += 1.0 / (dist_sqr(p, *pp) + *o as f64).sqrt() + noise_offset;
                    }
                    if shell < outer_crust {
                        continue;
                    }
                    if shell >= inner_air {
                        let s = self.filling.get(region, random, p);
                        self.safe_set(region, p, s);
                        continue;
                    }
                    let mut crack = 0.0;
                    for cp in &crack_points {
                        crack +=
                            1.0 / (dist_sqr(p, *cp) + self.crack_point_offset as f64).sqrt() + noise_offset;
                    }
                    if generate_crack && crack >= crack_size {
                        self.safe_set(region, p, BlockState::AIR);
                        for d in Direction::ALL {
                            let a = p.relative(d, 1);
                            let f = region.fluid(a);
                            if f != Fluid::Empty {
                                region.schedule_fluid_tick(a, f);
                            }
                        }
                    } else if shell >= innermost {
                        let alternate = (random.next_float() as f64) < self.use_alternate_layer0_chance;
                        let s = if alternate {
                            self.alternate_inner.get(region, random, p)
                        } else {
                            self.inner.get(region, random, p)
                        };
                        self.safe_set(region, p, s);
                        if (!self.placements_require_layer0_alternate || alternate)
                            && (random.next_float() as f64) < self.use_potential_placements_chance
                        {
                            crystals.push(p);
                        }
                    } else if shell >= inner_crust {
                        let s = self.middle.get(region, random, p);
                        self.safe_set(region, p, s);
                    } else if shell >= outer_crust {
                        let s = self.outer.get(region, random, p);
                        self.safe_set(region, p, s);
                    }
                }
            }
        }
        let blocks = &region.env.blocks;
        for c in crystals {
            let mut state =
                self.inner_placements[random.next_int_bounded(self.inner_placements.len() as i32) as usize];
            for d in Direction::ALL {
                if blocks.has_property(state, "facing") {
                    state = blocks.with_property(state, "facing", d.name()).unwrap_or(state);
                }
                let place = c.relative(d, 1);
                let ps = region.get(place);
                let pf = blocks.flags(ps);
                if blocks.has_property(state, "waterlogged") {
                    state = blocks
                        .with_property(
                            state,
                            "waterlogged",
                            if pf.is_fluid_source() { "true" } else { "false" },
                        )
                        .unwrap();
                }
                // `BuddingAmethystBlock.canClusterGrowAtState`
                if pf.is_air() || (blocks.block_of(ps) == region.env.b.water && pf.fluid_amount() == 8) {
                    self.safe_set(region, place, state);
                    break;
                }
            }
        }
        true
    }
}
