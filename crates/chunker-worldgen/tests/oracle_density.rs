//! Density functions compared bit-for-bit with vanilla via the Java oracle.

mod common;

use chunker_data::Identifier;
use chunker_worldgen::density::{DensityVolume, Df, SamplerContext};
use chunker_worldgen::random_state::RandomState;
use common::{Oracle, java_hash, vanilla_registries};
use std::sync::Arc;

/// Checks one oracle answer.
type Check = Box<dyn Fn(&str)>;

const ROUTER_FIELDS: &[&str] = &[
    "temperature",
    "vegetation",
    "continents",
    "erosion",
    "depth",
    "ridges",
    "chunk_surface_level",
    "final_density",
];

fn router_fn(settings: &chunker_worldgen::settings::NoiseGeneratorSettings, field: &str) -> Df {
    let r = &settings.router;
    match field {
        "temperature" => r.temperature.clone(),
        "vegetation" => r.vegetation.clone(),
        "continents" => r.continents.clone(),
        "erosion" => r.erosion.clone(),
        "depth" => r.depth.clone(),
        "ridges" => r.ridges.clone(),
        "chunk_surface_level" => r.chunk_surface_level.clone(),
        "final_density" => r.final_density.clone(),
        _ => unreachable!(),
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, bound: i32) -> i32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) % bound as u64) as i32
    }
}

fn check_settings(settings_id: &str, seed: i64, points: usize, volumes: &[[i32; 9]]) {
    let Some(reg) = vanilla_registries() else { return };
    let Some(mut oracle) = Oracle::start() else { return };
    let settings = reg.noise_settings(&Identifier::new(settings_id)).unwrap();
    let state = Arc::new(RandomState::new(reg.clone(), seed, settings.legacy_random_source));
    let (min_y, height) = (settings.noise.min_y, settings.noise.height);

    let mut rng = Lcg(seed as u64 ^ 0x9E3779B97F4A7C15);
    let mut queries = Vec::new();
    let mut expected: Vec<Check> = Vec::new();
    for field in ROUTER_FIELDS {
        let f = router_fn(&settings, field);
        let sampler = state.sampler(&f);
        for _ in 0..points {
            let x = rng.next(60000) - 30000;
            let y = min_y + rng.next(height);
            let z = rng.next(60000) - 30000;
            queries.push(format!("density {seed} {settings_id} router.{field} {x} {y} {z}"));
            let uncached = sampler.sample_value(&mut SamplerContext::uncached(), x, y, z);
            let cached = sampler.sample_value(&mut SamplerContext::cached(Default::default()), x, y, z);
            let field = field.to_string();
            expected.push(Box::new(move |line: &str| {
                let want = format!("ok {} {}", uncached.to_bits() as i32, cached.to_bits() as i32);
                assert_eq!(line, want, "density {field} at {x} {y} {z}");
            }));
        }
        for v in volumes {
            let vol = DensityVolume::new(v[3], v[4], v[5], v[0], v[1], v[2], v[6], v[7], v[8]);
            let mut buf = vec![0.0f32; vol.size()];
            sampler.sample_volume(&mut SamplerContext::cached(Default::default()), &mut buf, &vol);
            queries.push(format!(
                "volume {seed} {settings_id} router.{field} {} {} {} {} {} {} {} {} {}",
                v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8]
            ));
            let field = field.to_string();
            let v = *v;
            expected.push(Box::new(move |line: &str| {
                let mut parts = line.split(' ').skip(1);
                let hash: i64 = parts.next().unwrap().parse().unwrap();
                for p in parts {
                    let (idx, bits) = p.split_once(':').unwrap();
                    let idx: usize = idx.parse().unwrap();
                    let bits: i32 = bits.parse().unwrap();
                    assert_eq!(
                        buf[idx].to_bits() as i32,
                        bits,
                        "volume {field} {v:?} index {idx}"
                    );
                }
                assert_eq!(java_hash(&buf), hash, "volume {field} {v:?} hash");
            }));
        }
    }
    let answers = oracle.query_batch(&queries);
    for (line, check) in answers.iter().zip(expected.iter()) {
        check(line);
    }
}

#[test]
fn overworld_density_matches_vanilla() {
    check_settings(
        "minecraft:overworld",
        12345,
        40,
        &[
            [0, -64, 0, 16, 384, 16, 1, 1, 1],
            [-4096, -64, 3200, 16, 384, 16, 1, 1, 1],
            [160, -64, -320, 5, 49, 5, 4, 8, 4],
            [-37, 13, 71, 7, 9, 5, 1, 1, 1],
            [512, 0, 512, 5, 1, 5, 4, 1, 4],
        ],
    );
}

#[test]
fn other_overworld_presets_match_vanilla() {
    let vols = [
        [1024, -64, -2048, 16, 384, 16, 1, 1, 1],
        [32, -64, 32, 3, 49, 3, 4, 8, 4],
    ];
    check_settings("minecraft:amplified", -987654321, 15, &vols);
    check_settings("minecraft:large_biomes", 42, 15, &vols);
}

#[test]
fn nether_and_end_density_match_vanilla() {
    check_settings("minecraft:nether", 777, 20, &[[48, 0, -96, 16, 128, 16, 1, 1, 1]]);
    check_settings(
        "minecraft:end",
        777,
        20,
        &[
            [1000, 0, -64, 16, 128, 16, 1, 1, 1],
            [0, 0, 0, 16, 128, 16, 1, 1, 1],
        ],
    );
}
