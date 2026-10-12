//! The disk size of generated chunks, measured on sample chunks: generated, decorated, serialized
//! and compressed exactly like a job does, so the estimate follows the dimension, the data packs and
//! the Minecraft version.

use chunker_anvil::Compression;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use super::pipeline::decorate_chunk;
use crate::biome::BiomeResolver;
use crate::chunk::ProtoChunk;
use crate::chunk::serialize::write_chunk;
use crate::world::Dimension;

/// Region file sector size: chunks take whole sectors.
const SECTOR: u64 = 4096;
/// The 8 KiB region header, spread over the 1024 chunks of a region.
const HEADER_PER_CHUNK: f64 = 8192.0 / 1024.0;

/// Average bytes on disk per chunk of the 3x3 chunks around each of `spots` (chunk positions), at
/// `features` when `decorate` is set and at `terrain` otherwise.
pub fn chunk_bytes(
    dim: &Dimension,
    spots: &[(i32, i32)],
    decorate: bool,
    data_version: i32,
    game_time: i64,
) -> f64 {
    let sizes: Vec<u64> = spots
        .par_iter()
        .flat_map_iter(|&(x, z)| sample(dim, x, z, decorate, data_version, game_time))
        .collect();
    if sizes.is_empty() {
        return 0.0;
    }
    sizes.iter().sum::<u64>() as f64 / sizes.len() as f64 + HEADER_PER_CHUNK
}

/// Sizes of the 3x3 chunks around `(x, z)`: terrain for 5x5 chunks (the decorated ones need their
/// neighbours), decoration of the inner 3x3.
fn sample(dim: &Dimension, x: i32, z: i32, decorate: bool, data_version: i32, game_time: i64) -> Vec<u64> {
    let starts = dim.start_cache();
    let mut biomes: FxHashMap<(i32, i32), BiomeResolver> = FxHashMap::default();
    for bz in z - 3..=z + 3 {
        for bx in x - 3..=x + 3 {
            biomes.insert((bx, bz), dim.resolve_biomes(bx, bz));
        }
    }
    // z-major, (x - 2, z - 2) first.
    let index = |cx: i32, cz: i32| ((cz - z + 2) * 5 + (cx - x + 2)) as usize;
    let mut chunks: Vec<ProtoChunk> = (0..25)
        .map(|i| {
            let (cx, cz) = (x - 2 + i % 5, z - 2 + i / 5);
            dim.generate_terrain(cx, cz, |dx, dz| &biomes[&(cx + dx, cz + dz)], &starts)
        })
        .collect();
    let inner: Vec<(i32, i32)> = (0..9).map(|i| (x - 1 + i % 3, z - 1 + i / 3)).collect();
    if decorate {
        for &(cx, cz) in &inner {
            let wanted: Vec<usize> = (0..9).map(|i| index(cx - 1 + i % 3, cz - 1 + i / 3)).collect();
            // Ascending indices are the z-major order `decorate_chunk` expects.
            let neighbourhood: Vec<&mut ProtoChunk> = chunks
                .iter_mut()
                .enumerate()
                .filter(|(i, _)| wanted.contains(i))
                .map(|(_, c)| c)
                .collect();
            decorate_chunk(dim, &starts, cx, cz, neighbourhood, [true; 9]);
        }
    }
    inner
        .iter()
        .map(|&(cx, cz)| {
            let nbt = write_chunk(&chunks[index(cx, cz)], &dim.biomes, data_version, game_time);
            let payload = Compression::Zlib.compress(&chunker_nbt::write_root(&nbt));
            // Length, compression byte, payload, rounded up to whole sectors.
            (payload.len() as u64 + 5).div_ceil(SECTOR) * SECTOR
        })
        .collect()
}
