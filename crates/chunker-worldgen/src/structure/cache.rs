//! Structure starts per chunk shared by a generation job, plus `structure_references` and the
//! beardifier input derived from them.

use std::sync::Arc;

use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use super::StructureGenerator;
use super::registry::TerrainAdjustment;
use super::start::StructureStart;
use crate::terrain::TerrainGenerator;
use crate::terrain::beardifier::Beardifier;

/// Starts of one chunk (usually empty).
pub type ChunkStarts = Arc<Vec<StructureStart>>;

const SHARDS: usize = 64;

/// `structure_starts` results by chunk, computed once and shared by all workers.
pub struct StartCache {
    shards: Vec<Mutex<FxHashMap<(i32, i32), ChunkStarts>>>,
    empty: ChunkStarts,
    pub min_y: i32,
    pub height: i32,
}

/// A start referenced by a chunk (`structure_references`).
pub struct Reference {
    pub structure: usize,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub starts: ChunkStarts,
    /// Index into `starts`.
    pub index: usize,
}

impl Reference {
    pub fn start(&self) -> &StructureStart {
        &self.starts[self.index]
    }
}

impl StartCache {
    pub fn new(min_y: i32, height: i32) -> Self {
        Self {
            shards: (0..SHARDS).map(|_| Mutex::default()).collect(),
            empty: Arc::new(Vec::new()),
            min_y,
            height,
        }
    }

    fn shard(&self, x: i32, z: i32) -> &Mutex<FxHashMap<(i32, i32), ChunkStarts>> {
        let h = (x as u32).wrapping_mul(0x9E37_79B9) ^ (z as u32).wrapping_mul(0x85EB_CA6B);
        &self.shards[(h >> 26) as usize % SHARDS]
    }

    /// The starts of chunk `(x, z)` (`ChunkGenerator.createStructures`).
    pub fn starts(
        &self,
        structures: &StructureGenerator,
        terrain: &TerrainGenerator,
        x: i32,
        z: i32,
    ) -> ChunkStarts {
        if let Some(s) = self.shard(x, z).lock().get(&(x, z)) {
            return s.clone();
        }
        let list = structures.create_starts(terrain, x, z, self.min_y, self.height);
        let value = if list.is_empty() {
            self.empty.clone()
        } else {
            Arc::new(list)
        };
        self.shard(x, z).lock().entry((x, z)).or_insert(value).clone()
    }

    /// Drops cached chunks outside the given chunk box (keeps memory bounded on long jobs).
    pub fn retain_box(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) {
        for shard in &self.shards {
            shard
                .lock()
                .retain(|&(x, z), _| x >= min_x && x <= max_x && z >= min_z && z <= max_z);
        }
    }

    /// `ChunkGenerator.createReferences`: starts within 8 chunks whose box touches the chunk, in a fixed
    /// order (structure id, then source chunk x, then z).
    pub fn references(
        &self,
        structures: &StructureGenerator,
        terrain: &TerrainGenerator,
        cx: i32,
        cz: i32,
    ) -> Vec<Reference> {
        let (bx, bz) = (cx << 4, cz << 4);
        let mut out = Vec::new();
        for sx in cx - 8..=cx + 8 {
            for sz in cz - 8..=cz + 8 {
                let starts = self.starts(structures, terrain, sx, sz);
                for (index, start) in starts.iter().enumerate() {
                    if start.bbox.intersects_xz(bx, bz, bx + 15, bz + 15) {
                        out.push(Reference {
                            structure: start.structure,
                            chunk_x: sx,
                            chunk_z: sz,
                            starts: starts.clone(),
                            index,
                        });
                    }
                }
            }
        }
        let ids = &structures.registry.structures;
        out.sort_by(|a, b| {
            ids[a.structure]
                .id
                .as_str()
                .cmp(ids[b.structure].id.as_str())
                .then(a.chunk_x.cmp(&b.chunk_x))
                .then(a.chunk_z.cmp(&b.chunk_z))
        });
        out
    }
}

/// `Beardifier.forStructuresInChunk` for the references of a chunk.
pub fn beardifier(
    structures: &StructureGenerator,
    references: &[Reference],
    cx: i32,
    cz: i32,
) -> Option<Beardifier> {
    let ids = &structures.registry.structures;
    Beardifier::for_starts(
        references
            .iter()
            .map(|r| (r.start(), ids[r.structure].terrain_adaptation))
            .filter(|(_, a)| *a != TerrainAdjustment::None),
        cx,
        cz,
    )
}

/// `SerializableChunkData.packStructureData`: the chunk's own starts and its references.
pub fn structures_nbt(
    structures: &StructureGenerator,
    own: &[StructureStart],
    references: &[Reference],
) -> chunker_nbt::Compound {
    let ids = &structures.registry.structures;
    let mut starts = chunker_nbt::Compound::new();
    for start in own {
        let id = ids[start.structure].id.as_str();
        starts.insert(id, start.to_nbt(id));
    }
    let mut refs: Vec<(&str, Vec<i64>)> = Vec::new();
    for r in references {
        let id = ids[r.structure].id.as_str();
        // `ChunkPos.pack`
        let packed = (r.chunk_x as u32 as i64) | ((r.chunk_z as u32 as i64) << 32);
        match refs.iter_mut().find(|(k, _)| *k == id) {
            Some((_, v)) => {
                if !v.contains(&packed) {
                    v.push(packed);
                }
            }
            None => refs.push((id, vec![packed])),
        }
    }
    let mut references_tag = chunker_nbt::Compound::new();
    for (id, v) in refs {
        references_tag.insert(id, v);
    }
    let mut tag = chunker_nbt::Compound::new();
    tag.insert("starts", starts);
    tag.insert("References", references_tag);
    tag
}
