//! The `features` step of single chunks compared with vanilla (center chunk decorated, all 9
//! chunks of the region compared: blocks, heightmaps, post-processing, block entities, ticks).
//!
//! `CHUNKER_FEATURES` = comma separated placed feature ids to restrict the comparison (default: all
//! fully ported features), `CHUNKER_FEATURE_CHUNKS` = `x,z;x,z` chunk list. Spawned mobs see
//! `CHUNKER_DIFFICULTY` (default `normal`), `CHUNKER_CLOCK` and `CHUNKER_GAME_TIME` (default 0);
//! `CHUNKER_ENTITY_LOG=<file>` appends vanilla's entities of every compared chunk.

mod common;

use std::sync::Arc;

use chunker_data::Identifier;
use chunker_worldgen::biome::{BiomeRegistry, BiomeSource};
use chunker_worldgen::chunk::{BlockEnv, ChunkStatus, ProtoChunk};
use chunker_worldgen::entity::Difficulty;
use chunker_worldgen::feature::{FeatureEnv, FeatureRegistry, Region, decorate};
use chunker_worldgen::random_state::RandomState;
use chunker_worldgen::registry::WorldgenRegistries;
use chunker_worldgen::structure::StructureGenerator;
use chunker_worldgen::structure::cache::{StartCache, beardifier};
use chunker_worldgen::structure::registry::StructureRegistry;
use chunker_worldgen::terrain::{NeighborBiomes, TerrainGenerator};
use common::{Oracle, describe_chunk, vanilla_registries};
use serde_json::json;

const SEED: i64 = 12345;

const BIOME_CHUNKS: &str = "0,3;-255,-255;-84,0;0,147;0,-42;343,0;0,42;-343,0;0,-35;215,215;100,-100;-110,110;0,-147;-310,-310;-56,0;0,931;185,-185;-25,-25;55,55;-42,0;98,0;-35,0;45,-45;0,427;-91,0;-105,105;7,0;658,0;0,-119;0,728;0,0;-125,-125;25,25;-445,-445;-20,20;0,-14;-63,0;-590,590;80,80;0,84;0,441;40,40;-40,-40;-49,0;-15,15;600,-600;-25,25;0,126;35,35;-315,-315;-100,-100;-270,270;0,112;-655,655;-270,-270";

/// Dimension under test (`CHUNKER_DIM` = overworld | nether | end).
struct Dim {
    /// Oracle biome source name.
    source: &'static str,
    settings: &'static str,
    dimension: &'static str,
    min_y: i32,
    height: i32,
    skylight: bool,
    source_json: serde_json::Value,
}

fn dim() -> Dim {
    match std::env::var("CHUNKER_DIM").as_deref() {
        Ok("nether") => Dim {
            source: "nether",
            settings: "minecraft:nether",
            dimension: "minecraft:the_nether",
            min_y: 0,
            height: 256,
            skylight: false,
            source_json: json!({"type": "minecraft:multi_noise", "preset": "minecraft:nether"}),
        },
        Ok("end") => Dim {
            source: "the_end",
            settings: "minecraft:end",
            dimension: "minecraft:the_end",
            min_y: 0,
            height: 256,
            skylight: false,
            source_json: json!({"type": "minecraft:the_end"}),
        },
        _ => Dim {
            source: "overworld",
            settings: "minecraft:overworld",
            dimension: "minecraft:overworld",
            min_y: -64,
            height: 384,
            skylight: true,
            source_json: json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"}),
        },
    }
}

struct Setup {
    dim: Dim,
    reg: Arc<WorldgenRegistries>,
    biomes: Arc<BiomeRegistry>,
    env: Arc<BlockEnv>,
    terrain: TerrainGenerator,
    features: FeatureEnv,
    structures: StructureGenerator,
    starts: StartCache,
    /// `CHUNKER_STRUCTURES`: "1" (all) or comma separated structure ids placed in the features step.
    structs: Option<String>,
}

fn setup() -> Option<Setup> {
    let reg = vanilla_registries()?;
    let biomes = BiomeRegistry::load(&reg.packs).unwrap();
    let env = BlockEnv::load(&reg).unwrap();
    let dim = dim();
    let settings = reg
        .noise_settings(&Identifier::parse(dim.settings).unwrap())
        .unwrap();
    let state = Arc::new(RandomState::new(reg.clone(), SEED, settings.legacy_random_source));
    let source = Arc::new(BiomeSource::parse(&reg, &biomes, &dim.source_json).unwrap());
    let terrain = TerrainGenerator::new(&reg, biomes.clone(), settings, state, source, env.clone()).unwrap();
    let registry =
        Arc::new(FeatureRegistry::load(&reg, &biomes, &terrain.biome_source.possible_biomes()).unwrap());
    let features = FeatureEnv::new(
        reg.clone(),
        &terrain,
        env.clone(),
        registry,
        dim.min_y,
        dim.height,
        dim.skylight,
        Identifier::parse(dim.dimension).unwrap(),
        &reg.packs
            .require_json("dimension_type", &Identifier::parse(dim.dimension).unwrap())
            .unwrap(),
    )
    .unwrap();
    let mut features = features;
    let var = |name: &str| std::env::var(name).ok();
    if let Some(d) = var("CHUNKER_DIFFICULTY") {
        features.spawn.difficulty = Difficulty::by_name(&d).expect("CHUNKER_DIFFICULTY");
    }
    features.spawn.clock_time = var("CHUNKER_CLOCK").map_or(0, |v| v.parse().unwrap());
    features.spawn.game_time = var("CHUNKER_GAME_TIME").map_or(0, |v| v.parse().unwrap());
    let registry = Arc::new(StructureRegistry::load(&reg, &biomes).unwrap());
    let structures = StructureGenerator::new(registry, features.templates.clone(), &terrain);
    let starts = StartCache::new(dim.min_y, dim.height);
    let structs = std::env::var("CHUNKER_STRUCTURES")
        .ok()
        .filter(|v| !v.is_empty() && v != "0");
    Some(Setup {
        dim,
        reg,
        biomes,
        env,
        terrain,
        features,
        structures,
        starts,
        structs,
    })
}

/// An oracle with the spawn settings of `s`.
fn start_oracle(s: &Setup) -> Option<Oracle> {
    let mut oracle = Oracle::start()?;
    let sp = &s.features.spawn;
    let name = match sp.difficulty {
        Difficulty::Peaceful => "peaceful",
        Difficulty::Easy => "easy",
        Difficulty::Normal => "normal",
        Difficulty::Hard => "hard",
    };
    oracle.query_batch(&[format!("spawnsettings {name} {} {}", sp.clock_time, sp.game_time)]);
    Some(oracle)
}

fn query(s: &Setup, cx: i32, cz: i32, filter: &str) -> String {
    let d = &s.dim;
    let structs = s.structs.as_deref().unwrap_or("0");
    format!(
        "features {SEED} {} {} {} {cx} {cz} {} {} {structs} {filter}",
        d.source, d.settings, d.dimension, d.min_y, d.height
    )
}

fn terrain_chunk(s: &Setup, x: i32, z: i32) -> ProtoChunk {
    let resolvers: Vec<_> = (0..9)
        .map(|i| {
            s.terrain
                .resolve_biomes(x + i % 3 - 1, z + i / 3 - 1, s.dim.min_y, s.dim.height)
        })
        .collect();
    let beard = s.structs.as_ref().and_then(|_| {
        let refs = s.starts.references(&s.structures, &s.terrain, x, z);
        beardifier(&s.structures, &refs, x, z).map(|b| b.into_sampler())
    });
    let neighbors = NeighborBiomes {
        center_x: x,
        center_z: z,
        chunks: std::array::from_fn(|i| &resolvers[i]),
        beardifier: beard,
    };
    let mut chunk = ProtoChunk::new(s.env.clone(), x, z, s.dim.min_y, s.dim.height);
    chunk.fill_biomes(&resolvers[4]);
    chunk.status = ChunkStatus::Biomes;
    s.terrain.build_terrain(&mut chunk, &neighbors);
    chunk.status = ChunkStatus::Terrain;
    chunk
}

/// Block entities, ticks and entities in the oracle's text form (entities normalized by the oracle).
fn extras(s: &Setup, chunk: &ProtoChunk, oracle: &mut Oracle) -> String {
    let mut be: Vec<(&(i32, i32, i32), &chunker_nbt::Compound)> = chunk.block_entities.iter().collect();
    be.sort_by_key(|((x, y, z), _)| (*y, *z, *x));
    let be: Vec<String> = be
        .iter()
        .map(|(_, t)| chunker_nbt::canonical(&chunker_nbt::Tag::Compound((*t).clone())))
        .collect();
    let mut bt: Vec<String> = chunk
        .block_ticks
        .list
        .iter()
        .map(|(b, x, y, z, p)| format!("{}@{x},{y},{z}:0:{p}", s.reg.blocks.block(*b).name))
        .collect();
    bt.sort();
    let mut ft: Vec<String> = chunk
        .fluid_ticks
        .list
        .iter()
        .map(|(f, x, y, z, p)| format!("{}@{x},{y},{z}:0:{p}", f.id()))
        .collect();
    ft.sort();
    let queries: Vec<String> = chunk
        .entities
        .iter()
        .map(|e| {
            let bytes = chunker_nbt::write_root(e);
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            format!("normentity {hex}")
        })
        .collect();
    let entities: Vec<String> = oracle
        .query_batch(&queries)
        .into_iter()
        .map(|a| a.strip_prefix("ok ").unwrap_or(&a).to_owned())
        .collect();
    format!(
        "be=[{}] bt=[{}] ft=[{}] entities=[{}]",
        be.join(","),
        bt.join(","),
        ft.join(","),
        entities.join(",")
    )
}

/// `applyBiomeDecoration` with the structures of `CHUNKER_STRUCTURES` (if set) and a feature filter.
fn decorate_full(s: &Setup, region: &mut Region, filter: Option<&dyn Fn(&str) -> bool>) {
    let Some(structs) = s.structs.as_deref() else {
        return decorate(region, |_, _, _, _| {}, filter);
    };
    let refs = s
        .starts
        .references(&s.structures, &s.terrain, region.center_x, region.center_z);
    let allowed: Vec<&str> = structs.split(',').collect();
    let sfilter = |id: &str| structs == "1" || allowed.contains(&id);
    let mut chunk_bb = chunker_worldgen::structure::build::writable_area(region);
    decorate(
        region,
        |region, step, random, seed| {
            chunker_worldgen::structure::build::decorate_step(
                region,
                &s.structures,
                &refs,
                step,
                random,
                seed,
                &mut chunk_bb,
                Some(&sfilter),
            )
        },
        filter,
    );
}

fn run_region(s: &Setup, cx: i32, cz: i32, apply: &dyn Fn(&mut Region)) -> Vec<ProtoChunk> {
    run_region_logged(s, cx, cz, apply).0
}

fn run_region_logged(
    s: &Setup,
    cx: i32,
    cz: i32,
    apply: &dyn Fn(&mut Region),
) -> (Vec<ProtoChunk>, Vec<String>) {
    let mut chunks: Vec<ProtoChunk> = (0..9)
        .map(|i| terrain_chunk(s, cx + i % 3 - 1, cz + i / 3 - 1))
        .collect();
    let log;
    {
        let refs: Vec<&mut ProtoChunk> = chunks.iter_mut().collect();
        let mut region = Region::new(&s.features, cx, cz, refs, [true; 9]);
        region.log = Some(Vec::new());
        apply(&mut region);
        log = region.log.take().unwrap();
    }
    (chunks, log)
}

fn describe_all(s: &Setup, chunks: &mut [ProtoChunk], oracle: &mut Oracle) -> Vec<String> {
    chunks
        .iter_mut()
        .map(|c| {
            let d = describe_chunk(c);
            let extras = extras(s, c, oracle);
            format!("{} @ {}", d.strip_prefix("ok ").unwrap(), extras)
        })
        .collect()
}

fn decorate_chunk(s: &Setup, oracle: &mut Oracle, cx: i32, cz: i32, allowed: &[String]) -> Vec<String> {
    let filter = |id: &str| allowed.iter().any(|a| a == id);
    let mut chunks = run_region(s, cx, cz, &|region| decorate_full(s, region, Some(&filter)));
    describe_all(s, &mut chunks, oracle)
}

/// Compares our region after `apply` with the oracle's after `java_filter`; returns failure reports.
fn compare_region(
    s: &Setup,
    oracle: &mut Oracle,
    cx: i32,
    cz: i32,
    java_filter: &str,
    apply: &dyn Fn(&mut Region),
) -> Vec<String> {
    let q = query(s, cx, cz, java_filter);
    let answer = oracle.query_batch(std::slice::from_ref(&q)).remove(0);
    let Some(answer) = answer.strip_prefix("ok ") else {
        panic!("oracle: {answer}")
    };
    let (answer, their_log) = match answer.split_once(" ## ") {
        Some((a, l)) => (a, Some(l)),
        None => (answer, None),
    };
    let theirs: Vec<&str> = answer.split(" ;; ").collect();
    // `CHUNKER_ENTITY_LOG=<file>`: append vanilla's entities (to see what a run covered).
    if let Ok(path) = std::env::var("CHUNKER_ENTITY_LOG") {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for t in &theirs {
            if let Some((_, e)) = t.split_once(" entities=") {
                writeln!(f, "{cx},{cz} {e}").unwrap();
            }
        }
    }
    let (mut our_chunks, our_log) = run_region_logged(s, cx, cz, apply);
    let ours = describe_all(s, &mut our_chunks, oracle);
    let mut failures = Vec::new();
    if let Some(their_log) = their_log {
        let theirs_log: Vec<&str> = if their_log.is_empty() {
            Vec::new()
        } else {
            their_log.split(';').collect()
        };
        if let Some(i) = (0..theirs_log.len().max(our_log.len()))
            .find(|&i| theirs_log.get(i).copied() != our_log.get(i).map(|s| s.as_str()))
        {
            failures.push(format!(
                "setBlock #{i} differs (ours {} calls, vanilla {}): ours {:?} vanilla {:?}; previous {:?}",
                our_log.len(),
                theirs_log.len(),
                our_log.get(i),
                theirs_log.get(i),
                if i > 0 {
                    theirs_log.get(i - 1).copied()
                } else {
                    None
                }
            ));
            // `CHUNKER_LOG_WINDOW=n`: n calls before and after the first difference.
            if let Some(n) = std::env::var("CHUNKER_LOG_WINDOW")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
            {
                let range = i.saturating_sub(n)..(i + n).min(theirs_log.len().max(our_log.len()));
                for j in range {
                    failures.push(format!(
                        "  #{j}: ours {:?} vanilla {:?}",
                        our_log.get(j),
                        theirs_log.get(j)
                    ));
                }
            }
        }
    }
    for (i, (o, t)) in ours.iter().zip(theirs.iter()).enumerate() {
        if o == t {
            continue;
        }
        let (dx, dz) = (i as i32 % 3 - 1, i as i32 / 3 - 1);
        let mut msg = format!("chunk {},{} (decorating {cx},{cz}):", cx + dx, cz + dz);
        let (od, oe) = o.split_once(" @ ").unwrap();
        let (td, te) = t.split_once(" @ ").unwrap();
        if oe != te {
            msg.push_str(&format!(
                "
  extras ours    {}
  extras vanilla {}",
                oe, te
            ));
        }
        let (oh, os) = od.split_once(" |").unwrap();
        let (th, ts) = td.split_once(" |").unwrap();
        if oh != th {
            msg.push_str(&format!(
                "
  ours    {oh}
  vanilla {th}"
            ));
        }
        let osecs: Vec<&str> = os.split_whitespace().collect();
        let tsecs: Vec<&str> = ts.split_whitespace().collect();
        let mut printed = 0;
        for (si, (a, b)) in osecs.iter().zip(tsecs.iter()).enumerate() {
            if a == b
                || printed
                    >= std::env::var("CHUNKER_SECTIONS")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(3)
            {
                continue;
            }
            printed += 1;
            let sq = format!("featuresection {} {i} {si}", q.strip_prefix("features ").unwrap());
            let ids = oracle.query_batch(&[sq]).remove(0);
            let vanilla: Vec<u16> = ids
                .strip_prefix("ok ")
                .unwrap()
                .split(' ')
                .map(|v| v.parse().unwrap())
                .collect();
            let sec = &our_chunks[i].sections[si];
            let mut shown = 0;
            for (j, v) in vanilla.iter().enumerate() {
                let (x, y, z) = (j & 15, (j >> 8) & 15, (j >> 4) & 15);
                let our = sec.get(x, y, z);
                if our.0 != *v
                    && shown
                        < std::env::var("CHUNKER_SHOW")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(12)
                {
                    shown += 1;
                    let wx = ((cx + dx) << 4) + x as i32;
                    let wz = ((cz + dz) << 4) + z as i32;
                    let wy = s.dim.min_y + (si as i32) * 16 + y as i32;
                    msg.push_str(&format!(
                        "
    {wx},{wy},{wz}: ours {} vanilla {}",
                        s.reg.blocks.describe(our),
                        s.reg.blocks.describe(chunker_data::blocks::BlockState(*v))
                    ));
                }
            }
        }
        failures.push(msg);
    }
    failures
}

/// Placed feature ids of the overworld whose features are fully ported.
fn ported(s: &Setup) -> Vec<String> {
    let mut out = Vec::new();
    for step in &s.features.features.steps {
        for f in &step.features {
            if f.feature.deep_implemented() {
                out.push(f.id.clone().unwrap());
            }
        }
    }
    out
}

#[test]
fn feature_order_matches_vanilla() {
    let Some(s) = setup() else { return };
    let Some(mut oracle) = start_oracle(&s) else {
        return;
    };
    let answer = oracle
        .query_batch(&[format!("featureorder {SEED} {} {}", s.dim.source, s.dim.settings)])
        .remove(0);
    let theirs: Vec<Vec<String>> = answer
        .strip_prefix("ok ")
        .unwrap()
        .split(" | ")
        .map(|step| {
            step.split(' ')
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .collect();
    let ours: Vec<Vec<String>> = s
        .features
        .features
        .steps
        .iter()
        .map(|st| st.features.iter().map(|f| f.id.clone().unwrap()).collect())
        .collect();
    assert_eq!(ours, theirs);
    let _ = (&s.biomes, &s.reg);
}

#[test]
fn chunk_features_match_vanilla() {
    let Some(s) = setup() else { return };
    let Some(mut oracle) = start_oracle(&s) else {
        return;
    };
    let allowed: Vec<String> = match std::env::var("CHUNKER_FEATURES") {
        Ok(v) => v.split(',').map(str::to_owned).collect(),
        Err(_) => ported(&s),
    };
    eprintln!(
        "comparing {} placed features: {}",
        allowed.len(),
        allowed.join(" ")
    );
    let chunks: Vec<(i32, i32)> = match std::env::var("CHUNKER_FEATURE_CHUNKS") {
        Ok(v) => v
            .split(';')
            .map(|p| {
                let (x, z) = p.split_once(',').unwrap();
                (x.trim().parse().unwrap(), z.trim().parse().unwrap())
            })
            .collect(),
        // One chunk per overworld biome (see `find_biome_chunks`).
        Err(_) => BIOME_CHUNKS
            .split(';')
            .map(|p| {
                let (x, z) = p.split_once(',').unwrap();
                (x.parse().unwrap(), z.parse().unwrap())
            })
            .collect(),
    };
    let filter = if allowed.is_empty() {
        "none".to_owned()
    } else {
        allowed.join(",")
    };
    let mut failures = Vec::new();
    for &(cx, cz) in &chunks {
        // CHUNKER_SABOTAGE drops one feature on our side only: the test must fail then.
        let ours_allowed: Vec<String> = if std::env::var("CHUNKER_SABOTAGE").is_ok() {
            allowed.iter().skip(1).cloned().collect()
        } else {
            allowed.clone()
        };
        let f = |id: &str| ours_allowed.iter().any(|a| a == id);
        failures.extend(compare_region(&s, &mut oracle, cx, cz, &filter, &|region| {
            decorate_full(&s, region, Some(&f))
        }));
    }
    assert!(
        failures.is_empty(),
        "{} chunks differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
#[ignore]
fn list_unported_features() {
    let Some(s) = setup() else { return };
    fn walk(f: &chunker_worldgen::feature::kinds::Feature, out: &mut std::collections::BTreeSet<String>) {
        use chunker_worldgen::feature::kinds::Feature;
        match f {
            Feature::Unimplemented(t) => {
                out.insert(t.clone());
            }
            Feature::RandomSelector(r) => {
                walk(&r.default.feature, out);
                for (p, _) in &r.features {
                    walk(&p.feature, out);
                }
            }
            Feature::SimpleRandomSelector(l) | Feature::Sequence(l) => {
                l.iter().for_each(|p| walk(&p.feature, out))
            }
            Feature::RandomBooleanSelector(a, b) => {
                walk(&a.feature, out);
                walk(&b.feature, out);
            }
            Feature::WeightedRandomSelector(l) => l.entries.iter().for_each(|(p, _)| walk(&p.feature, out)),
            Feature::Tree(t) if !t.decorators.iter().all(|d| d.is_implemented()) => {
                out.insert("tree decorators".into());
            }
            _ => {}
        }
    }
    let mut missing = std::collections::BTreeSet::new();
    for step in &s.features.features.steps {
        for f in &step.features {
            let mut m = std::collections::BTreeSet::new();
            walk(&f.feature, &mut m);
            if !m.is_empty() {
                eprintln!("{} needs {:?}", f.id.as_ref().unwrap(), m);
                missing.extend(m);
            }
        }
    }
    eprintln!("missing types: {missing:?}");
}

#[test]
#[ignore]
fn find_biome_chunks() {
    let Some(s) = setup() else { return };
    let mut found: std::collections::BTreeMap<String, (i32, i32)> = Default::default();
    let mut step = 0;
    'outer: for r in 0..400i32 {
        for (x, z) in [
            (r * 7, 0),
            (0, r * 7),
            (-r * 7, 0),
            (0, -r * 7),
            (r * 5, r * 5),
            (-r * 5, r * 5),
            (r * 5, -r * 5),
            (-r * 5, -r * 5),
        ] {
            let res = s.terrain.resolve_biomes(x, z, s.dim.min_y, s.dim.height);
            // Biome at the chunk center, near sea level.
            let b = res.get(res.min_qx + 2, 63 >> 2, res.min_qz + 2);
            let name = s.biomes.name(b).to_string();
            found.entry(name).or_insert((x, z));
            step += 1;
            if found.len() >= 54 {
                break 'outer;
            }
        }
    }
    let list: Vec<String> = found.values().map(|(x, z)| format!("{x},{z}")).collect();
    for (k, v) in &found {
        eprintln!("{k}: {},{}", v.0, v.1);
    }
    eprintln!("{} biomes after {step} probes: {}", found.len(), list.join(";"));
}

/// `CHUNKER_FEATURE_CHUNKS=x,z`: finds the first placed feature (in decoration order) after which
/// the chunk differs from vanilla.
#[test]
#[ignore]
fn bisect_features() {
    let Some(s) = setup() else { return };
    let Some(mut oracle) = start_oracle(&s) else {
        return;
    };
    let (cx, cz) = {
        let v = std::env::var("CHUNKER_FEATURE_CHUNKS").expect("set CHUNKER_FEATURE_CHUNKS=x,z");
        let (x, z) = v.split_once(',').unwrap();
        (x.trim().parse::<i32>().unwrap(), z.trim().parse::<i32>().unwrap())
    };
    let all = ported(&s);
    let mut matches = |n: usize| -> bool {
        let list: Vec<String> = all[..n].to_vec();
        let filter = if list.is_empty() {
            "none".to_owned()
        } else {
            list.join(",")
        };
        let q = query(&s, cx, cz, &filter);
        let answer = oracle.query_batch(&[q]).remove(0);
        let theirs: Vec<String> = answer
            .strip_prefix("ok ")
            .unwrap()
            .split(" ;; ")
            .map(str::to_owned)
            .collect();
        decorate_chunk(&s, &mut oracle, cx, cz, &list) == theirs
    };
    let (mut lo, mut hi) = (0usize, all.len());
    assert!(!matches(hi), "chunk matches with all features");
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if matches(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    eprintln!("first differing feature: {} (after {} matching)", all[hi - 1], lo);
}

/// `CHUNKER_PLACE=<configured feature>@x,y,z@seed` with `CHUNKER_FEATURE_CHUNKS=cx,cz`: one feature
/// placed with a fresh legacy random on both sides.
#[test]
#[ignore]
fn single_feature() {
    let Some(s) = setup() else { return };
    let Some(mut oracle) = start_oracle(&s) else {
        return;
    };
    let place = std::env::var("CHUNKER_PLACE").expect("set CHUNKER_PLACE=feature@x,y,z@seed");
    let (cx, cz) = {
        let v = std::env::var("CHUNKER_FEATURE_CHUNKS").expect("set CHUNKER_FEATURE_CHUNKS=x,z");
        let (x, z) = v.split_once(',').unwrap();
        (x.trim().parse::<i32>().unwrap(), z.trim().parse::<i32>().unwrap())
    };
    let parts: Vec<&str> = place.split('@').collect();
    let xyz: Vec<i32> = parts[1].split(',').map(|v| v.parse().unwrap()).collect();
    let seed: i64 = parts[2].parse().unwrap();
    let parser = chunker_worldgen::feature::registry::Parser::new(&s.reg, &s.biomes);
    let feature = parser
        .feature(&serde_json::Value::String(parts[0].to_owned()))
        .unwrap();
    let failures = compare_region(&s, &mut oracle, cx, cz, &format!("place:{place}"), &|region| {
        use chunker_worldgen::random::{AnyRandom, LegacyRandom, WorldgenRandom};
        let mut random = WorldgenRandom::new(AnyRandom::Legacy(LegacyRandom::new(seed)));
        feature.place(
            region,
            &mut random,
            chunker_worldgen::structure::geom::BlockPos::new(xyz[0], xyz[1], xyz[2]),
        );
    });
    assert!(
        failures.is_empty(),
        "{} chunks differ:
{}",
        failures.len(),
        failures.join(
            "
"
        )
    );
}

/// `CHUNKER_FEATURES=a,b`: searches chunks where vanilla places one of these features (output
/// differs from placing none) and compares those. `CHUNKER_HITS` = how many (default 3).
#[test]
#[ignore]
fn feature_hits() {
    let Some(s) = setup() else { return };
    let Some(mut oracle) = start_oracle(&s) else {
        return;
    };
    let allowed: Vec<String> = std::env::var("CHUNKER_FEATURES")
        .expect("set CHUNKER_FEATURES")
        .split(',')
        .map(str::to_owned)
        .collect();
    let wanted: usize = std::env::var("CHUNKER_HITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let filter = if allowed.is_empty() {
        "none".to_owned()
    } else {
        allowed.join(",")
    };
    let mut hits = 0;
    let mut failures = Vec::new();
    let mut probes = 0;
    // Biome sample chunks and their neighbours first (one per biome), then a spiral.
    let mut candidates: Vec<(i32, i32)> = Vec::new();
    for d in [(0, 0), (2, 1), (-1, 2), (3, -2), (-3, -1)] {
        for p in BIOME_CHUNKS.split(';') {
            let (x, z) = p.split_once(',').unwrap();
            candidates.push((x.parse::<i32>().unwrap() + d.0, z.parse::<i32>().unwrap() + d.1));
        }
    }
    for r in 0..200i32 {
        candidates.extend([(r * 3, r * 2), (-r * 2, r * 3), (r * 5, -r), (-r * 3, -r * 4)]);
    }
    'search: for chunk in candidates.chunks(1) {
        for &(cx, cz) in chunk {
            probes += 1;
            let q = |f: &str| query(&s, cx, cz, f);
            let with = oracle.query_batch(&[q(&filter)]).remove(0);
            let without = oracle.query_batch(&[q("none")]).remove(0);
            if with == without {
                continue;
            }
            hits += 1;
            let f = |id: &str| allowed.iter().any(|a| a == id);
            failures.extend(compare_region(&s, &mut oracle, cx, cz, &filter, &|region| {
                decorate_full(&s, region, Some(&f))
            }));
            if hits >= wanted {
                break 'search;
            }
        }
    }
    eprintln!("{hits} chunks with the features after {probes} probes");
    assert!(hits > 0, "feature never placed");
    assert!(
        failures.is_empty(),
        "{} chunks differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `CHUNKER_STRUCTURES=a,b` (structure ids): chunks under pieces of these structures (found on the
/// placement grids near the origin, `CHUNKER_HITS` chunks per structure, default 3) are decorated
/// with only those structures and compared with vanilla.
#[test]
#[ignore]
fn structure_pieces_match_vanilla() {
    let Some(s) = setup() else { return };
    let Some(mut oracle) = start_oracle(&s) else {
        return;
    };
    let wanted: Vec<String> = s
        .structs
        .as_deref()
        .expect("set CHUNKER_STRUCTURES")
        .split(',')
        .map(str::to_owned)
        .collect();
    let hits: usize = std::env::var("CHUNKER_HITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let reg = &s.structures.registry;
    let mut chunks: Vec<(String, i32, i32)> = Vec::new();
    let ring_sets = s.structures.ring_positions(&s.terrain);
    for (si, set) in reg.sets.iter().enumerate() {
        if !set
            .structures
            .iter()
            .any(|(i, _)| wanted.iter().any(|w| w == reg.structures[*i].id.as_str()))
        {
            continue;
        }
        let mut candidates: Vec<(i32, i32)> = Vec::new();
        match ring_sets.get(&si) {
            Some(ring) => candidates.extend(ring.iter().take(3).copied()),
            None => {
                let spacing = match set.placement.kind {
                    chunker_worldgen::structure::placement::PlacementKind::RandomSpread {
                        spacing, ..
                    } => spacing,
                    _ => continue,
                };
                // Rings of grid cells until about 40000 cells (rare sets like buried treasure).
                let max_r: i32 = std::env::var("CHUNKER_GRID_R")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or((100 / spacing.max(1)).max(12));
                for r in 0..max_r {
                    for gx in -r..=r {
                        for gz in -r..=r {
                            if gx.abs().max(gz.abs()) != r {
                                continue;
                            }
                            if let Some(c) = set.placement.potential_chunk(SEED, gx * spacing, gz * spacing) {
                                candidates.push(c);
                            }
                        }
                    }
                }
            }
        }
        let mut found = 0;
        for (x, z) in candidates {
            if found >= hits {
                break;
            }
            if !s.structures.is_structure_chunk(&s.terrain, si, x, z) {
                continue;
            }
            for start in s.starts.starts(&s.structures, &s.terrain, x, z).iter() {
                let id = reg.structures[start.structure].id.to_string();
                if !wanted.contains(&id) {
                    continue;
                }
                // Chunks of distinct pieces, first pieces first (`CHUNKER_PIECE`: only that piece type).
                let piece_filter = std::env::var("CHUNKER_PIECE").ok();
                for p in start
                    .pieces
                    .iter()
                    .filter(|p| piece_filter.as_deref().is_none_or(|f| p.type_id() == f))
                {
                    let b = p.bbox();
                    // `CHUNKER_ALL_CHUNKS=1`: every chunk under the piece, else its center chunk.
                    let all = std::env::var("CHUNKER_ALL_CHUNKS").is_ok();
                    let mut cs = vec![((b.min_x + b.max_x) >> 5, (b.min_z + b.max_z) >> 5)];
                    if all {
                        cs = (b.min_x >> 4..=b.max_x >> 4)
                            .flat_map(|x| (b.min_z >> 4..=b.max_z >> 4).map(move |z| (x, z)))
                            .collect();
                    }
                    for (x, z) in cs {
                        let c = (id.clone(), x, z);
                        if found < hits && !chunks.contains(&c) {
                            chunks.push(c);
                            found += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(!chunks.is_empty(), "no chunks with {wanted:?}");
    let mut failures = Vec::new();
    for (id, cx, cz) in &chunks {
        eprintln!("{id} at chunk {cx},{cz}");
        failures.extend(compare_region(&s, &mut oracle, *cx, *cz, "none", &|region| {
            decorate_full(&s, region, Some(&|_: &str| false))
        }));
    }
    assert!(
        failures.is_empty(),
        "{} chunks differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The whole `features` step (every feature and structure) against vanilla's own
/// `applyBiomeDecoration` on the biome sample chunks (`CHUNKER_FEATURE_CHUNKS` overrides).
/// Run with `CHUNKER_STRUCTURES=1`.
#[test]
#[ignore]
fn full_decoration_matches_vanilla() {
    let Some(s) = setup() else { return };
    let Some(mut oracle) = start_oracle(&s) else {
        return;
    };
    assert_eq!(s.structs.as_deref(), Some("1"), "set CHUNKER_STRUCTURES=1");
    let list = std::env::var("CHUNKER_FEATURE_CHUNKS").unwrap_or_else(|_| BIOME_CHUNKS.to_owned());
    let mut failures = Vec::new();
    for p in list.split(';') {
        let (x, z) = p.split_once(',').unwrap();
        let (cx, cz) = (x.trim().parse().unwrap(), z.trim().parse().unwrap());
        failures.extend(compare_region(&s, &mut oracle, cx, cz, "vanilla", &|region| {
            decorate_full(&s, region, None)
        }));
    }
    assert!(
        failures.is_empty(),
        "{} chunks differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
