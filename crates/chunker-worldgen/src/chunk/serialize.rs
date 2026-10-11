//! `SerializableChunkData.write()` for proto chunks (26.3 layout).

use chunker_data::blocks::{BlockRegistry, BlockState};
use chunker_nbt::{Compound, List, Tag};

use super::{HeightmapKind, ProtoChunk};
use crate::biome::BiomeRegistry;

/// `Mth.ceillog2`.
fn ceillog2(v: usize) -> u32 {
    if v <= 1 {
        0
    } else {
        usize::BITS - (v - 1).leading_zeros()
    }
}

/// `SimpleBitStorage` raw longs (values never span two longs).
pub fn pack_bits(values: &[u32], bits: u32) -> Vec<i64> {
    let per_long = (64 / bits) as usize;
    let mut out = vec![0i64; values.len().div_ceil(per_long)];
    let mask = (1u64 << bits) - 1;
    for (i, &v) in values.iter().enumerate() {
        let shift = (i % per_long) as u32 * bits;
        out[i / per_long] |= (((v as u64) & mask) << shift) as i64;
    }
    out
}

/// `BlockState.CODEC` (26.x): a default state is the block id string, any other state is
/// `{id, properties}` with every property.
fn block_state_tag(reg: &BlockRegistry, state: BlockState) -> Tag {
    let info = reg.block_info_of(state);
    if info.default_state == state {
        return Tag::String(info.name.to_string());
    }
    let mut c = Compound::with_capacity(2);
    c.insert("id", info.name.as_str());
    let mut p = Compound::with_capacity(info.properties.len());
    for (k, v) in reg.properties(state) {
        p.insert(k, v);
    }
    c.insert("properties", p);
    Tag::Compound(c)
}

/// `NbtOps` list building: heterogeneous lists wrap non-compound elements as `{"": value}`.
fn nbt_list(items: Vec<Tag>) -> List {
    let mixed = items.iter().any(|t| matches!(t, Tag::Compound(_)))
        && items.iter().any(|t| !matches!(t, Tag::Compound(_)));
    let mut list = List::with_capacity(items.len());
    for t in items {
        if mixed && !matches!(t, Tag::Compound(_)) {
            let mut w = Compound::with_capacity(1);
            w.insert("", t);
            list.push(w);
        } else {
            list.push(t);
        }
    }
    list
}

/// Encodes a paletted container: palette in first-appearance order, then packed indices.
fn paletted<T: Copy + PartialEq>(
    values: &[T],
    bits_for: impl Fn(usize) -> u32,
    encode: impl Fn(T) -> Tag,
) -> Compound {
    let mut palette: Vec<T> = Vec::new();
    let mut indices: Vec<u32> = Vec::with_capacity(values.len());
    let mut last: Option<(T, u32)> = None;
    for &v in values {
        let id = match last {
            Some((lv, li)) if lv == v => li,
            _ => {
                let id = match palette.iter().position(|p| *p == v) {
                    Some(i) => i as u32,
                    None => {
                        palette.push(v);
                        (palette.len() - 1) as u32
                    }
                };
                last = Some((v, id));
                id
            }
        };
        indices.push(id);
    }
    let mut c = Compound::with_capacity(2);
    c.insert("palette", nbt_list(palette.iter().map(|p| encode(*p)).collect()));
    let bits = bits_for(palette.len());
    if bits != 0 {
        c.insert("data", pack_bits(&indices, bits));
    }
    c
}

fn block_bits(palette_size: usize) -> u32 {
    match ceillog2(palette_size) {
        0 => 0,
        1..=4 => 4,
        b => b,
    }
}

fn biome_bits(palette_size: usize) -> u32 {
    ceillog2(palette_size)
}

/// Builds the chunk NBT exactly like vanilla's `SerializableChunkData.write()`.
/// `SavedTick.codec` entries (`i`, `x`, `y`, `z`, `t`, `p`).
fn ticks_list<T: Copy + Eq + std::hash::Hash>(ticks: &super::Ticks<T>, name: impl Fn(T) -> String) -> List {
    let mut list = List::with_capacity(ticks.len());
    for &(kind, x, y, z, priority) in &ticks.list {
        let mut t = Compound::with_capacity(6);
        t.insert("i", name(kind));
        t.insert("x", x);
        t.insert("y", y);
        t.insert("z", z);
        t.insert("t", 0i32);
        t.insert("p", priority);
        list.push(t);
    }
    list
}

pub fn write_chunk(
    chunk: &ProtoChunk,
    biomes: &BiomeRegistry,
    data_version: i32,
    game_time: i64,
) -> Compound {
    let reg = &chunk.blocks;
    let mut tag = Compound::with_capacity(16);
    tag.insert("DataVersion", data_version);
    tag.insert("xPos", chunk.x);
    tag.insert("yPos", chunk.min_section_y());
    tag.insert("zPos", chunk.z);
    tag.insert("LastUpdate", game_time);
    tag.insert("InhabitedTime", 0i64);
    tag.insert("Status", chunk.status.id());

    let mut sections = List::with_capacity(chunk.sections.len());
    for (i, section) in chunk.sections.iter().enumerate() {
        let mut s = Compound::with_capacity(3);
        let uniform;
        let blocks: &[BlockState] = match section.raw() {
            Some(b) => b,
            None => {
                uniform = [section.uniform()];
                &uniform
            }
        };
        s.insert(
            "block_states",
            paletted(blocks, block_bits, |st| block_state_tag(reg, st)),
        );
        s.insert(
            "biomes",
            paletted(&section.biomes, biome_bits, |b| {
                Tag::String(biomes.name(b).to_string())
            }),
        );
        s.insert("Y", (chunk.min_section_y() + i as i32) as i8);
        sections.push(s);
    }
    tag.insert("sections", sections);
    let mut block_entities = List::with_capacity(chunk.block_entities.len());
    for be in chunk.block_entities.values() {
        block_entities.push(be.clone());
    }
    tag.insert("block_entities", block_entities);
    let mut entities = List::with_capacity(chunk.entities.len());
    for e in &chunk.entities {
        entities.push(e.clone());
    }
    tag.insert("entities", entities);
    tag.insert(
        "block_ticks",
        ticks_list(&chunk.block_ticks, |b| reg.block(b).name.to_string()),
    );
    tag.insert(
        "fluid_ticks",
        ticks_list(&chunk.fluid_ticks, |f| f.id().to_owned()),
    );

    let mut post = List::with_capacity(chunk.post_processing.len());
    for offsets in &chunk.post_processing {
        let mut l = List::with_capacity(offsets.len());
        for &o in offsets {
            l.push(o);
        }
        post.push(l);
    }
    tag.insert("PostProcessing", post);

    let mut heightmaps = Compound::new();
    let bits = ceillog2(chunk.height as usize + 1);
    // `ChunkStatus.heightmapsAfter()`: the worldgen pair before `features`, the final four after.
    let after_features = chunk.status >= super::ChunkStatus::Features;
    for kind in HeightmapKind::ALL {
        let wanted =
            matches!(kind, HeightmapKind::OceanFloorWg | HeightmapKind::WorldSurfaceWg) != after_features;
        if !wanted {
            continue;
        }
        if let Some(raw) = chunk.heightmaps.raw(kind) {
            let values: Vec<u32> = raw.iter().map(|&v| v as u32).collect();
            heightmaps.insert(kind.key(), pack_bits(&values, bits));
        }
    }
    tag.insert("Heightmaps", heightmaps);

    let structures = chunk.structures_nbt.clone().unwrap_or_else(|| {
        let mut s = Compound::new();
        s.insert("starts", Compound::new());
        s.insert("References", Compound::new());
        s
    });
    tag.insert("structures", structures);
    tag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_packing_matches_simple_bit_storage() {
        // 5 bits -> 12 values per long, 4096 values -> 342 longs.
        let values: Vec<u32> = (0..4096).map(|i| (i % 31) as u32).collect();
        let packed = pack_bits(&values, 5);
        assert_eq!(packed.len(), 342);
        assert_eq!(packed[0] as u64 & 31, 0);
        assert_eq!((packed[0] as u64 >> 5) & 31, 1);
        assert_eq!(packed[1] as u64 & 31, 12);
        assert_eq!(ceillog2(1), 0);
        assert_eq!(ceillog2(17), 5);
        assert_eq!(ceillog2(385), 9);
    }
}
