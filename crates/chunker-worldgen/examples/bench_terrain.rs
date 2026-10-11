//! Single-threaded terrain benchmark: `cargo run --release -p chunker-worldgen --example bench_terrain [chunks]`.
//! Needs `vanilla/26.3/server-26.3.jar` (run `cargo xtask vanilla`).

use std::sync::Arc;
use std::time::Instant;

use chunker_data::blocks::BlockRegistry;
use chunker_data::{DataPacks, Identifier, McVersion, PackSource};
use chunker_worldgen::biome::{BiomeRegistry, BiomeSource};
use chunker_worldgen::chunk::{BlockEnv, ChunkStatus, ProtoChunk};
use chunker_worldgen::random_state::RandomState;
use chunker_worldgen::registry::WorldgenRegistries;
use chunker_worldgen::terrain::{NeighborBiomes, TerrainGenerator, profile};

fn main() -> anyhow::Result<()> {
    let n: i32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(200);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut packs = DataPacks::new();
    packs.push(PackSource::zip_file(&root.join("vanilla/26.3/server-26.3.jar"))?);
    let reg = Arc::new(WorldgenRegistries::new(
        packs,
        BlockRegistry::for_version(McVersion::V26_3),
    ));
    let biomes = BiomeRegistry::load(&reg.packs)?;
    let env = BlockEnv::load(&reg)?;
    let settings = reg.noise_settings(&Identifier::vanilla("overworld"))?;
    let state = Arc::new(RandomState::new(reg.clone(), 12345, false));
    let source = Arc::new(BiomeSource::parse(
        &reg,
        &biomes,
        &serde_json::json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"}),
    )?);
    let generator = TerrainGenerator::new(&reg, biomes.clone(), settings, state, source, env.clone())?;
    let side = (n as f64).sqrt().ceil() as i32;
    let start = Instant::now();
    let mut count = 0;
    for i in 0..n {
        let (cx, cz) = (300 + i % side, -200 + i / side);
        let resolvers: Vec<_> = (0..9)
            .map(|k| generator.resolve_biomes(cx + k % 3 - 1, cz + k / 3 - 1, -64, 384))
            .collect();
        let neighbors = NeighborBiomes {
            center_x: cx,
            center_z: cz,
            chunks: std::array::from_fn(|k| &resolvers[k]),
            beardifier: None,
        };
        let mut chunk = ProtoChunk::new(env.clone(), cx, cz, -64, 384);
        chunk.fill_biomes(&resolvers[4]);
        chunk.status = ChunkStatus::Biomes;
        generator.build_terrain(&mut chunk, &neighbors);
        count += 1;
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{count} chunks in {secs:.2}s = {:.2} ms/chunk ({:.0} chunks/s/thread)",
        secs * 1000.0 / count as f64,
        count as f64 / secs
    );
    for (p, s) in profile::snapshot() {
        println!("  {p:?}: {:.2} ms/chunk", s * 1000.0 / count as f64);
    }
    Ok(())
}
