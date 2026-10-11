//! Generates the terrain of 4x4 Overworld chunks, prints the surface block in the middle of each
//! chunk and writes the chunks to an Anvil region file.
//!
//! ```text
//! cargo run --release --example terrain -- [seed] [chunk x] [chunk z] [output dir]
//! ```
//!
//! The vanilla server jar is looked up in `vanilla/26.3/` and downloaded from Mojang into
//! `target/server-jars/` when it is missing.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use chunker_anvil::{Compression, Region, RegionPos};
use chunker_data::blocks::BlockRegistry;
use chunker_data::{DataPacks, Identifier, McVersion, vanilla};
use chunker_worldgen::biome::{BiomeRegistry, BiomeSource};
use chunker_worldgen::chunk::serialize::write_chunk;
use chunker_worldgen::chunk::{BlockEnv, ChunkStatus, HeightmapKind, ProtoChunk};
use chunker_worldgen::random_state::RandomState;
use chunker_worldgen::registry::WorldgenRegistries;
use chunker_worldgen::terrain::{NeighborBiomes, TerrainGenerator};

const MIN_Y: i32 = -64;
const HEIGHT: i32 = 384;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let seed: i64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(12345);
    let x0: i32 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(-12);
    let z0: i32 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(-56);
    let out_dir = args.next().unwrap_or_else(|| "target/terrain-example".into());

    // 1. Vanilla data: the worldgen JSON of the official server jar.
    let version = McVersion::V26_3;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let jar = vanilla::locate(
        version,
        None,
        Some(&root.join("vanilla").join(version.id())),
        &root.join("target/server-jars"),
        true,
    )?;
    let mut packs = DataPacks::new();
    packs.push(jar.vanilla_pack()?);
    let registries = Arc::new(WorldgenRegistries::new(
        packs,
        BlockRegistry::for_version(version),
    ));

    // 2. A terrain generator for the Overworld, like `minecraft:noise` with the overworld settings.
    let biomes = BiomeRegistry::load(&registries.packs)?;
    let env = BlockEnv::load(&registries)?;
    let settings = registries.noise_settings(&Identifier::vanilla("overworld"))?;
    let random_state = Arc::new(RandomState::new(registries.clone(), seed, false));
    let source = Arc::new(BiomeSource::parse(
        &registries,
        &biomes,
        &serde_json::json!({ "type": "minecraft:multi_noise", "preset": "minecraft:overworld" }),
    )?);
    let generator = TerrainGenerator::new(
        &registries,
        biomes.clone(),
        settings,
        random_state,
        source,
        env.clone(),
    )?;

    // 3. Biomes, then terrain (noise, aquifers, surface rules, carvers), chunk by chunk.
    let region_pos = RegionPos::of_chunk(x0, z0);
    let mut region = Region::empty(region_pos);
    for cz in z0..z0 + 4 {
        for cx in x0..x0 + 4 {
            // Surface rules and carvers look at the biomes of the neighbouring chunks too.
            let resolvers: Vec<_> = (0..9)
                .map(|k| generator.resolve_biomes(cx + k % 3 - 1, cz + k / 3 - 1, MIN_Y, HEIGHT))
                .collect();
            let neighbors = NeighborBiomes {
                center_x: cx,
                center_z: cz,
                chunks: std::array::from_fn(|k| &resolvers[k]),
                beardifier: None,
            };
            let mut chunk = ProtoChunk::new(env.clone(), cx, cz, MIN_Y, HEIGHT);
            chunk.fill_biomes(&resolvers[4]);
            chunk.status = ChunkStatus::Biomes;
            generator.build_terrain(&mut chunk, &neighbors);
            chunk.status = ChunkStatus::Terrain;

            let y = chunk.height_at(HeightmapKind::WorldSurfaceWg, 8, 8);
            let block = chunk.blocks.describe(chunk.get_block(8, y, 8));
            println!("chunk {cx:>4} {cz:>4}: surface at y={y:<4} {block}");

            // 4. Vanilla chunk NBT, stored in the region file like the server does.
            let nbt = write_chunk(&chunk, &biomes, version.data_version(), 0);
            let index = (cx & 31) as usize + (cz & 31) as usize * 32;
            region.set_nbt(index, &nbt, Compression::Zlib, chunker_anvil::now_timestamp());
        }
    }
    std::fs::create_dir_all(&out_dir)?;
    region.save(Path::new(&out_dir))?;
    println!("wrote {out_dir}/{}", region_pos.file_name());
    Ok(())
}
