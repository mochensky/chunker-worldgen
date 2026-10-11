//! Terrain next to structures (beardifier) compared with vanilla.

mod common;

use std::sync::Arc;

use chunker_data::Identifier;
use chunker_worldgen::biome::{BiomeRegistry, BiomeSource};
use chunker_worldgen::chunk::{BlockEnv, ChunkStatus, ProtoChunk};
use chunker_worldgen::random_state::RandomState;
use chunker_worldgen::structure::StructureGenerator;
use chunker_worldgen::structure::cache::{StartCache, beardifier};
use chunker_worldgen::structure::placement::PlacementKind;
use chunker_worldgen::structure::registry::{StructureRegistry, TerrainAdjustment};
use chunker_worldgen::structure::template::TemplateManager;
use chunker_worldgen::terrain::{NeighborBiomes, TerrainGenerator};
use common::{Oracle, describe_chunk, vanilla_registries};
use serde_json::json;

const SEED: i64 = 12345;

#[test]
fn beard_kernel_matches_vanilla() {
    let Some(mut oracle) = Oracle::start() else { return };
    let answer = oracle.query_batch(&["beardkernel".to_owned()]).remove(0);
    let vanilla: Vec<u32> = answer
        .split(' ')
        .skip(1)
        .map(|h| u32::from_str_radix(h, 16).unwrap())
        .collect();
    let ours = chunker_worldgen::terrain::beardifier::beard_kernel();
    let bad = vanilla
        .iter()
        .zip(ours.iter())
        .filter(|(v, o)| **v != o.to_bits())
        .count();
    assert_eq!(vanilla.len(), ours.len());
    assert_eq!(bad, 0, "{bad} beard kernel entries differ");
}

#[test]
fn terrain_around_structures_matches_vanilla() {
    let Some(reg) = vanilla_registries() else { return };
    let Some(mut oracle) = Oracle::start() else { return };
    let biomes = BiomeRegistry::load(&reg.packs).unwrap();
    let env = BlockEnv::load(&reg).unwrap();
    let settings = reg.noise_settings(&Identifier::vanilla("overworld")).unwrap();
    let state = Arc::new(RandomState::new(reg.clone(), SEED, settings.legacy_random_source));
    let source = Arc::new(
        BiomeSource::parse(
            &reg,
            &biomes,
            &json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"}),
        )
        .unwrap(),
    );
    let terrain = TerrainGenerator::new(&reg, biomes.clone(), settings, state, source, env.clone()).unwrap();
    let registry = Arc::new(StructureRegistry::load(&reg, &biomes).unwrap());
    let templates = Arc::new(TemplateManager::new(
        Arc::new(reg.packs.clone()),
        reg.blocks.clone(),
    ));
    let structures = StructureGenerator::new(registry.clone(), templates, &terrain);
    let cache = StartCache::new(-64, 384);

    // A few starts of every terrain-adapting kind, from the placement grids around the origin.
    let mut targets: Vec<(String, i32, i32)> = Vec::new();
    for (si, set) in registry.sets.iter().enumerate() {
        let PlacementKind::RandomSpread { spacing, .. } = set.placement.kind else {
            continue;
        };
        let mut found = 0;
        'grid: for gz in -3..3 {
            for gx in -3..3 {
                let Some((x, z)) = set.placement.potential_chunk(SEED, gx * spacing, gz * spacing) else {
                    continue;
                };
                if !structures.is_structure_chunk(&terrain, si, x, z) {
                    continue;
                }
                for start in cache.starts(&structures, &terrain, x, z).iter() {
                    let s = &registry.structures[start.structure];
                    if s.terrain_adaptation == TerrainAdjustment::None {
                        continue;
                    }
                    // The chunk under the first piece: there the beardifier contributes most.
                    let b = start.pieces[0].bbox();
                    targets.push((s.id.to_string(), b.min_x >> 4, b.min_z >> 4));
                    found += 1;
                    if found >= 2 {
                        break 'grid;
                    }
                }
            }
        }
    }
    let ring = &structures.ring_positions(&terrain)[&registry
        .sets
        .iter()
        .position(|s| s.id.path() == "strongholds")
        .unwrap()];
    for &(x, z) in ring.iter().take(2) {
        for start in cache.starts(&structures, &terrain, x, z).iter() {
            let b = start.pieces[0].bbox();
            targets.push(("minecraft:stronghold".into(), b.min_x >> 4, b.min_z >> 4));
        }
    }
    assert!(!targets.is_empty());

    let queries: Vec<String> = targets
        .iter()
        .map(|(_, x, z)| {
            format!(
                "terrain {SEED} overworld minecraft:overworld {x} {z} carvers -64 384 minecraft:overworld"
            )
        })
        .collect();
    let answers = oracle.query_batch(&queries);
    let mut failures = Vec::new();
    for ((id, cx, cz), answer) in targets.iter().zip(answers) {
        let refs = cache.references(&structures, &terrain, *cx, *cz);
        let beard = beardifier(&structures, &refs, *cx, *cz)
            .map(|b| b.into_sampler())
            .filter(|_| std::env::var("CHUNKER_SABOTAGE").is_err());
        let resolvers: Vec<_> = (0..9)
            .map(|i| terrain.resolve_biomes(cx + i % 3 - 1, cz + i / 3 - 1, -64, 384))
            .collect();
        let neighbors = NeighborBiomes {
            center_x: *cx,
            center_z: *cz,
            chunks: std::array::from_fn(|i| &resolvers[i]),
            beardifier: beard,
        };
        let mut chunk = ProtoChunk::new(env.clone(), *cx, *cz, -64, 384);
        chunk.fill_biomes(&resolvers[4]);
        chunk.status = ChunkStatus::Biomes;
        terrain.build_terrain(&mut chunk, &neighbors);
        let ours = describe_chunk(&mut chunk);
        let (ours, theirs) = (
            ours.split('|').next().unwrap().to_owned(),
            answer.split('|').next().unwrap().to_owned(),
        );
        if ours != theirs {
            failures.push(format!(
                "{id} chunk {cx},{cz}:\n  ours    {ours}\n  vanilla {theirs}"
            ));
        }
    }
    eprintln!("checked {} chunks next to structures", targets.len());
    assert!(
        failures.is_empty(),
        "{} chunks differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
