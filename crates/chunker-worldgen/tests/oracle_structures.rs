//! Structure starts compared with vanilla (`ChunkGenerator.createStructures`).
//!
//! `CHUNKER_STRUCTURE_GRID=n` widens the sampled placement grid (default 4 cells per axis and
//! direction); `CHUNKER_STRUCTURE_SETS=villages,mineshafts` limits it to some structure sets;
//! `CHUNKER_STRUCTURE_CHUNKS="x,z;x,z"` checks specific chunks only.

mod common;

use std::sync::Arc;

use chunker_data::Identifier;
use chunker_worldgen::biome::{BiomeRegistry, BiomeSource};
use chunker_worldgen::chunk::BlockEnv;
use chunker_worldgen::random_state::RandomState;
use chunker_worldgen::structure::placement::PlacementKind;
use chunker_worldgen::structure::registry::StructureRegistry;
use chunker_worldgen::structure::template::TemplateManager;
use chunker_worldgen::structure::{StructureGenerator, kind_implemented};
use chunker_worldgen::terrain::TerrainGenerator;
use common::{Oracle, vanilla_registries};
use rustc_hash::FxHashMap;
use serde_json::{Value, json};

const SEED: i64 = 12345;

struct Dim {
    source: Value,
    settings: &'static str,
    oracle_source: &'static str,
    id: &'static str,
    min_y: i32,
    height: i32,
}

fn overworld() -> Dim {
    Dim {
        source: json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"}),
        settings: "overworld",
        oracle_source: "overworld",
        id: "minecraft:overworld",
        min_y: -64,
        height: 384,
    }
}

fn nether() -> Dim {
    Dim {
        source: json!({"type": "minecraft:multi_noise", "preset": "minecraft:nether"}),
        settings: "nether",
        oracle_source: "nether",
        id: "minecraft:the_nether",
        min_y: 0,
        height: 256,
    }
}

fn end() -> Dim {
    Dim {
        source: json!({"type": "minecraft:the_end"}),
        settings: "end",
        oracle_source: "the_end",
        id: "minecraft:the_end",
        min_y: 0,
        height: 256,
    }
}

struct Setup {
    terrain: TerrainGenerator,
    structures: StructureGenerator,
}

fn setup(dim: &Dim) -> Option<Setup> {
    let reg = vanilla_registries()?;
    let biomes = BiomeRegistry::load(&reg.packs).unwrap();
    let env = BlockEnv::load(&reg).unwrap();
    let settings = reg.noise_settings(&Identifier::vanilla(dim.settings)).unwrap();
    let state = Arc::new(RandomState::new(reg.clone(), SEED, settings.legacy_random_source));
    let source = Arc::new(BiomeSource::parse(&reg, &biomes, &dim.source).unwrap());
    let terrain = TerrainGenerator::new(&reg, biomes.clone(), settings, state, source, env).unwrap();
    let registry = Arc::new(StructureRegistry::load(&reg, &biomes).unwrap());
    let templates = Arc::new(TemplateManager::new(
        Arc::new(reg.packs.clone()),
        reg.blocks.clone(),
    ));
    let structures = StructureGenerator::new(registry, templates, &terrain);
    Some(Setup { terrain, structures })
}

#[test]
fn stronghold_rings_match_vanilla() {
    let Some(s) = setup(&overworld()) else { return };
    let Some(mut oracle) = Oracle::start() else { return };
    let set = s
        .structures
        .registry
        .sets
        .iter()
        .position(|x| x.id.as_str() == "minecraft:strongholds")
        .unwrap();
    let ours = &s.structures.ring_positions(&s.terrain)[&set];
    let expected = oracle.query_batch(&[format!(
        "rings {SEED} overworld minecraft:overworld minecraft:strongholds"
    )]);
    let ours: Vec<String> = ours.iter().map(|(x, z)| format!("{x},{z}")).collect();
    assert_eq!(format!("ok {}", ours.join(" ")), expected[0]);
}

/// Chunks where random-spread placements fire (several grid cells per set).
fn candidate_chunks(s: &Setup) -> Vec<(i32, i32)> {
    if let Ok(list) = std::env::var("CHUNKER_STRUCTURE_CHUNKS") {
        return list
            .split(';')
            .filter_map(|c| c.split_once(','))
            .map(|(x, z)| (x.trim().parse().unwrap(), z.trim().parse().unwrap()))
            .collect();
    }
    let r = std::env::var("CHUNKER_STRUCTURE_GRID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let mut out = Vec::new();
    let only = std::env::var("CHUNKER_STRUCTURE_SETS").ok();
    for (si, set) in s.structures.registry.sets.iter().enumerate() {
        let PlacementKind::RandomSpread { spacing, .. } = set.placement.kind else {
            continue;
        };
        if only
            .as_ref()
            .is_some_and(|o| !o.split(',').any(|n| set.id.path() == n))
        {
            continue;
        }
        for gz in -r..r {
            for gx in -r..r {
                if let Some(c) = set.placement.potential_chunk(SEED, gx * spacing, gz * spacing)
                    && s.structures.is_structure_chunk(&s.terrain, si, c.0, c.1)
                    && !out.contains(&c)
                {
                    out.push(c);
                }
            }
        }
    }
    out
}

fn check_dimension(dim: Dim) {
    let Some(s) = setup(&dim) else { return };
    let chunks = candidate_chunks(&s);
    let queries: Vec<String> = chunks
        .iter()
        .map(|(x, z)| {
            format!(
                "structures {SEED} {} minecraft:{} {} {x} {z} {} {}",
                dim.oracle_source, dim.settings, dim.id, dim.min_y, dim.height
            )
        })
        .collect();
    // Several oracle processes in parallel (vanilla jigsaw generation is slow), within the slot limit.
    let workers = common::oracle_slots().min(queries.len().max(1));
    let parts: Vec<Vec<String>> = (0..workers)
        .map(|w| queries.iter().skip(w).step_by(workers).cloned().collect())
        .collect();
    let results: Vec<Option<Vec<String>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = parts
            .iter()
            .map(|p| scope.spawn(move || Oracle::start().map(|mut o| o.query_batch(p))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let Some(results) = results.into_iter().collect::<Option<Vec<_>>>() else {
        return;
    };
    let mut answers = vec![String::new(); queries.len()];
    for (w, part) in results.into_iter().enumerate() {
        for (i, a) in part.into_iter().enumerate() {
            answers[w + i * workers] = a;
        }
    }
    let mut compared = FxHashMap::<String, usize>::default();
    let mut failures = Vec::new();
    for (&(cx, cz), answer) in chunks.iter().zip(&answers) {
        let mut expected: FxHashMap<String, String> = FxHashMap::default();
        let body = answer.strip_prefix("ok ").unwrap();
        if body != "none" {
            for item in body.split(' ') {
                let (id, canon) = item.split_once('=').unwrap();
                expected.insert(id.to_owned(), canon.to_owned());
            }
        }
        let starts = s
            .structures
            .create_starts(&s.terrain, cx, cz, dim.min_y, dim.height);
        let mut ours: FxHashMap<String, String> = FxHashMap::default();
        for st in &starts {
            let id = s.structures.structure_id(st).to_string();
            ours.insert(
                id.clone(),
                chunker_nbt::canonical(&chunker_nbt::Tag::Compound(st.to_nbt(&id))),
            );
        }
        for (id, canon) in &expected {
            let idx = s.structures.registry.by_id[&Identifier::parse(id).unwrap()];
            if !kind_implemented(&s.structures.registry.structures[idx].kind) {
                continue;
            }
            *compared.entry(id.clone()).or_default() += 1;
            match ours.get(id) {
                Some(o) if o == canon => {}
                Some(o) => failures.push(format!("chunk {cx},{cz} {id}:\n  vanilla {canon}\n  chunker {o}")),
                None => failures.push(format!("chunk {cx},{cz} {id}: missing (vanilla {canon})")),
            }
        }
        for id in ours.keys() {
            if !expected.contains_key(id) {
                failures.push(format!("chunk {cx},{cz} {id}: extra start {}", ours[id]));
            }
        }
    }
    let mut compared: Vec<_> = compared.into_iter().collect();
    compared.sort();
    eprintln!(
        "{}: {} chunks, compared starts: {compared:?}",
        dim.id,
        chunks.len()
    );
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn overworld_structure_starts_match_vanilla() {
    check_dimension(overworld());
}

#[test]
fn nether_structure_starts_match_vanilla() {
    check_dimension(nether());
}

#[test]
fn end_structure_starts_match_vanilla() {
    check_dimension(end());
}
