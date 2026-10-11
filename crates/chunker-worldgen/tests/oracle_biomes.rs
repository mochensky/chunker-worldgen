//! Biome generation compared with vanilla.

mod common;

use std::sync::Arc;

use chunker_data::Identifier;
use chunker_worldgen::biome::climate::unquantize;
use chunker_worldgen::biome::{BiomeRegistry, BiomeSource, ClimateSampler, overworld_parameters};
use chunker_worldgen::density::SamplerContext;
use chunker_worldgen::random_state::RandomState;
use common::{Oracle, vanilla_registries, workspace_root};
use serde_json::{Value, json};

/// The ported `OverworldBiomeBuilder` must reproduce the data generator report entry by entry.
#[test]
fn overworld_preset_matches_datagen_report() {
    let path =
        workspace_root().join("vanilla/26.3/generated/reports/biome_parameters/minecraft/overworld.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipping: {} missing", path.display());
        return;
    };
    let report: Value = serde_json::from_str(&text).unwrap();
    let entries = report["biomes"].as_array().unwrap();
    let ours = overworld_parameters();
    assert_eq!(ours.len(), entries.len());
    let param = |p: chunker_worldgen::biome::climate::Parameter| -> Value {
        if p.min == p.max {
            json!(unquantize(p.min))
        } else {
            json!([unquantize(p.min), unquantize(p.max)])
        }
    };
    let to_f32 = |v: &Value| -> Value {
        match v {
            Value::Array(a) => Value::Array(
                a.iter()
                    .map(|x| json!(x.to_string().parse::<f32>().unwrap()))
                    .collect(),
            ),
            x => json!(x.to_string().parse::<f32>().unwrap()),
        }
    };
    for (i, ((p, biome), e)) in ours.iter().zip(entries).enumerate() {
        assert_eq!(
            e["biome"].as_str().unwrap(),
            format!("minecraft:{biome}"),
            "entry {i}"
        );
        let pr = &e["parameters"];
        for (name, ours) in [
            ("temperature", p.temperature),
            ("humidity", p.humidity),
            ("continentalness", p.continentalness),
            ("erosion", p.erosion),
            ("depth", p.depth),
            ("weirdness", p.weirdness),
        ] {
            assert_eq!(param(ours), to_f32(&pr[name]), "entry {i} {name}");
        }
        assert_eq!(
            json!(unquantize(p.offset)),
            to_f32(&pr["offset"]),
            "entry {i} offset"
        );
    }
}

fn rle(names: &[String]) -> String {
    let mut out = String::from("ok");
    let mut i = 0;
    while i < names.len() {
        let mut j = i;
        while j < names.len() && names[j] == names[i] {
            j += 1;
        }
        out.push_str(&format!(" {}*{}", names[i], j - i));
        i = j;
    }
    out
}

fn check(source_name: &str, source_json: Value, settings_id: &str, seed: i64, chunks: &[(i32, i32)]) {
    let Some(reg) = vanilla_registries() else { return };
    let Some(mut oracle) = Oracle::start() else { return };
    let biomes = BiomeRegistry::load(&reg.packs).unwrap();
    let settings = reg.noise_settings(&Identifier::new(settings_id)).unwrap();
    let state = Arc::new(RandomState::new(reg.clone(), seed, settings.legacy_random_source));
    let climate = ClimateSampler::new(&state, &settings.router);
    let source = BiomeSource::parse(&reg, &biomes, &source_json).unwrap();
    let queries: Vec<String> = chunks
        .iter()
        .map(|(x, z)| format!("biomes {seed} {source_name} {settings_id} {x} {z}"))
        .collect();
    let answers = oracle.query_batch(&queries);
    for ((cx, cz), answer) in chunks.iter().zip(answers) {
        let mut ctx = SamplerContext::cached(Default::default());
        let r = source.resolve_chunk(
            &climate,
            &mut ctx,
            *cx,
            *cz,
            settings.noise.min_y,
            settings.noise.height,
        );
        let mut names = Vec::new();
        for section in 0..r.size_y / 4 {
            for x in 0..4 {
                for y in 0..4 {
                    for z in 0..4 {
                        let b = r.get(r.min_qx + x, r.min_qy + section * 4 + y, r.min_qz + z);
                        names.push(biomes.name(b).path().to_owned());
                    }
                }
            }
        }
        assert_eq!(rle(&names), answer, "{source_name} chunk {cx},{cz}");
    }
}

fn chunks() -> Vec<(i32, i32)> {
    let mut v = vec![(0, 0), (-1, -1), (100, -250), (-3000, 1500), (12345, 678)];
    for i in 0..40 {
        v.push((i * 97 - 2000, 1500 - i * 61));
    }
    v
}

#[test]
fn overworld_biomes_match_vanilla() {
    let src = json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"});
    check("overworld", src.clone(), "minecraft:overworld", 12345, &chunks());
    check("overworld", src, "minecraft:large_biomes", -42, &chunks());
}

#[test]
fn nether_and_end_biomes_match_vanilla() {
    check(
        "nether",
        json!({"type": "minecraft:multi_noise", "preset": "minecraft:nether"}),
        "minecraft:nether",
        99,
        &chunks(),
    );
    check(
        "the_end",
        json!({"type": "minecraft:the_end"}),
        "minecraft:end",
        99,
        &chunks(),
    );
}
