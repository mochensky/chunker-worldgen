//! Terrain step compared with vanilla.

mod common;

use std::sync::Arc;

use chunker_data::Identifier;
use chunker_worldgen::biome::manager::BiomeManager;
use chunker_worldgen::biome::{BiomeId, BiomeRegistry, BiomeSource, ClimateSampler};
use chunker_worldgen::chunk::{BlockEnv, ChunkStatus, ProtoChunk};
use chunker_worldgen::density::SamplerContext;
use chunker_worldgen::random_state::RandomState;
use chunker_worldgen::terrain::material::{GenerationContext, MaterialParser, MaterialSystem, SurfaceInputs};
use chunker_worldgen::terrain::noise::NoiseTerrain;
use chunker_worldgen::terrain::{NeighborBiomes, TerrainGenerator};
use common::{Oracle, describe_chunk as describe, vanilla_registries};
use rustc_hash::FxHashSet;
use serde_json::{Value, json};

fn check(
    source_name: &str,
    source_json: Value,
    settings_id: &str,
    seed: i64,
    stage: &str,
    dim: (i32, i32),
    chunks: &[(i32, i32)],
) {
    let Some(reg) = vanilla_registries() else { return };
    let Some(mut oracle) = Oracle::start() else { return };
    let biomes = BiomeRegistry::load(&reg.packs).unwrap();
    let settings = reg.noise_settings(&Identifier::new(settings_id)).unwrap();
    let state = Arc::new(RandomState::new(reg.clone(), seed, settings.legacy_random_source));
    let env = BlockEnv::load(&reg).unwrap();
    let climate = ClimateSampler::new(&state, &settings.router);
    let source = BiomeSource::parse(&reg, &biomes, &source_json).unwrap();
    let terrain = NoiseTerrain::new(settings.clone(), state.clone(), env.clone()).unwrap();
    let parser = MaterialParser::new(&reg, &biomes);
    let material = MaterialSystem::new(state.clone(), biomes.clone(), &settings, &parser).unwrap();
    let queries: Vec<String> = chunks
        .iter()
        .map(|(x, z)| {
            format!(
                "terrain {seed} {source_name} {settings_id} {x} {z} {stage} {} {}",
                dim.0, dim.1
            )
        })
        .collect();
    let answers = oracle.query_batch(&queries);
    for (&(cx, cz), answer) in chunks.iter().zip(answers) {
        let mut chunk = ProtoChunk::new(env.clone(), cx, cz, dim.0, dim.1);
        let r = source.resolve_chunk(
            &climate,
            &mut SamplerContext::cached(Default::default()),
            cx,
            cz,
            chunk.min_y,
            chunk.height,
        );
        chunk.fill_biomes(&r);
        chunk.status = ChunkStatus::Biomes;
        if stage == "carvers" {
            let generator = TerrainGenerator::new(
                &reg,
                biomes.clone(),
                settings.clone(),
                state.clone(),
                Arc::new(BiomeSource::parse(&reg, &biomes, &source_json).unwrap()),
                env.clone(),
            )
            .unwrap();
            let resolvers: Vec<_> = (0..9)
                .map(|i| generator.resolve_biomes(cx + i % 3 - 1, cz + i / 3 - 1, dim.0, dim.1))
                .collect();
            let neighbors = NeighborBiomes {
                center_x: cx,
                center_z: cz,
                chunks: std::array::from_fn(|i| &resolvers[i]),
                beardifier: None,
            };
            generator.build_terrain(&mut chunk, &neighbors);
            let ours = describe(&mut chunk);
            assert_eq!(
                ours.split('|').next().unwrap(),
                answer.split('|').next().unwrap(),
                "{settings_id} chunk {cx},{cz} carvers"
            );
            continue;
        }
        let mut nc = terrain.noise_chunk(cx, cz, dim.0, dim.1, None).unwrap();
        terrain.do_fill(&mut nc, &mut chunk);
        if stage == "surface" {
            let mut neighbors = Vec::new();
            let mut possible = FxHashSet::default();
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let nr = source.resolve_chunk(
                        &climate,
                        &mut SamplerContext::cached(Default::default()),
                        cx + dx,
                        cz + dz,
                        dim.0,
                        dim.1,
                    );
                    possible.extend(nr.biomes.iter().copied());
                    neighbors.push(((cx + dx, cz + dz), nr));
                }
            }
            let manager = BiomeManager::new(seed);
            let mut uncached = SamplerContext::uncached();
            let mut getter = |x: i32, y: i32, z: i32| -> BiomeId {
                manager.get_biome(x, y, z, &mut |qx, qy, qz| match neighbors
                    .iter()
                    .find(|((ncx, ncz), _)| *ncx == qx >> 2 && *ncz == qz >> 2)
                {
                    Some((_, r)) => r.get(qx, qy, qz),
                    None => source.noise_biome(&climate, &mut uncached, qx, qy, qz),
                })
            };
            let gen_ctx = GenerationContext {
                min_y: chunk.min_y.max(settings.noise.min_y),
                height: chunk.height.min(settings.noise.height),
                sea_level: settings.sea_level,
            };
            let volume = nc.volume;
            material.build_surface(
                &mut chunk,
                &mut nc.ctx,
                &volume,
                SurfaceInputs {
                    biome_getter: &mut getter,
                    possible_biomes: &possible,
                    gen_ctx,
                },
            );
        }
        let ours = describe(&mut chunk);
        if ours != answer {
            let ours_sections: Vec<&str> = ours.split('|').nth(1).unwrap().split_whitespace().collect();
            let theirs_sections: Vec<&str> = answer.split('|').nth(1).unwrap().split_whitespace().collect();
            let first_bad = ours_sections
                .iter()
                .zip(&theirs_sections)
                .position(|(a, b)| a != b);
            panic!(
                "{settings_id} chunk {cx},{cz} differs (first differing section index {first_bad:?})\n ours:  {}\n vanilla: {}",
                ours.split('|').next().unwrap(),
                answer.split('|').next().unwrap()
            );
        }
    }
}

fn chunks() -> Vec<(i32, i32)> {
    let mut v = vec![(0, 0), (-1, -1), (7, -13), (100, -250), (-3000, 1500)];
    for i in 0..60 {
        v.push((i * 397 - 12000, 9500 - i * 311));
    }
    v
}

const OVERWORLD: (i32, i32) = (-64, 384);
const NETHER: (i32, i32) = (0, 256);
const END: (i32, i32) = (0, 256);

#[test]
fn overworld_fill_matches_vanilla() {
    let src = json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"});
    check(
        "overworld",
        src,
        "minecraft:overworld",
        12345,
        "fill",
        OVERWORLD,
        &chunks(),
    );
}

#[test]
fn nether_and_end_fill_match_vanilla() {
    check(
        "nether",
        json!({"type": "minecraft:multi_noise", "preset": "minecraft:nether"}),
        "minecraft:nether",
        5,
        "fill",
        NETHER,
        &chunks(),
    );
    check(
        "the_end",
        json!({"type": "minecraft:the_end"}),
        "minecraft:end",
        5,
        "fill",
        END,
        &chunks(),
    );
}

#[test]
fn overworld_surface_matches_vanilla() {
    let src = json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"});
    check(
        "overworld",
        src,
        "minecraft:overworld",
        12345,
        "surface",
        OVERWORLD,
        &chunks(),
    );
}

#[test]
fn nether_and_end_surface_match_vanilla() {
    check(
        "nether",
        json!({"type": "minecraft:multi_noise", "preset": "minecraft:nether"}),
        "minecraft:nether",
        5,
        "surface",
        NETHER,
        &chunks(),
    );
    check(
        "the_end",
        json!({"type": "minecraft:the_end"}),
        "minecraft:end",
        5,
        "surface",
        END,
        &chunks(),
    );
}

#[test]
fn overworld_full_terrain_matches_vanilla() {
    let src = json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"});
    check(
        "overworld",
        src,
        "minecraft:overworld",
        12345,
        "carvers",
        OVERWORLD,
        &chunks(),
    );
}

#[test]
fn nether_full_terrain_matches_vanilla() {
    check(
        "nether",
        json!({"type": "minecraft:multi_noise", "preset": "minecraft:nether"}),
        "minecraft:nether",
        5,
        "carvers",
        NETHER,
        &chunks(),
    );
}
