//! Anvil region files (`r.<x>.<z>.mca`).
//!
//! Layout (see vanilla `RegionFile`):
//! * sector size 4096 bytes; sector 0 = 1024 big-endian location ints
//!   (`sector_index << 8 | sector_count`), sector 1 = 1024 timestamps (epoch seconds);
//! * each chunk: `u32 length` (payload length + 1), `u8 compression`, payload;
//! * chunks needing >= 256 sectors are stored in `c.<x>.<z>.mcc` next to the region file, with
//!   a 5 byte stub whose compression byte has bit 0x80 set.
//!
//! The writer always rewrites a whole region file at once (atomic temp file + rename). Chunks that
//! already exist are copied verbatim, so foreign compression formats (e.g. LZ4) are preserved.

mod compression;

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub use compression::Compression;

pub const SECTOR_BYTES: usize = 4096;
const EXTERNAL_FLAG: u8 = 0x80;
const EXTERNAL_THRESHOLD_SECTORS: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum RegionError {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("unsupported chunk compression id {0}")]
    UnsupportedCompression(u8),
    #[error("corrupt chunk data: {0}")]
    Corrupt(String),
    #[error("nbt error: {0}")]
    Nbt(#[from] chunker_nbt::NbtReadError),
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> RegionError + '_ {
    move |source| RegionError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Region coordinate helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionPos {
    pub x: i32,
    pub z: i32,
}

impl RegionPos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    pub const fn of_chunk(chunk_x: i32, chunk_z: i32) -> Self {
        Self {
            x: chunk_x >> 5,
            z: chunk_z >> 5,
        }
    }

    pub fn file_name(self) -> String {
        format!("r.{}.{}.mca", self.x, self.z)
    }

    pub const fn min_chunk_x(self) -> i32 {
        self.x << 5
    }

    pub const fn min_chunk_z(self) -> i32 {
        self.z << 5
    }

    /// Parses `r.<x>.<z>.mca`.
    pub fn parse_file_name(name: &str) -> Option<Self> {
        let rest = name.strip_prefix("r.")?.strip_suffix(".mca")?;
        let (x, z) = rest.split_once('.')?;
        Some(Self {
            x: x.parse().ok()?,
            z: z.parse().ok()?,
        })
    }
}

/// Index of a chunk inside its region (0..1024).
#[inline]
pub const fn local_index(chunk_x: i32, chunk_z: i32) -> usize {
    ((chunk_x & 31) + (chunk_z & 31) * 32) as usize
}

/// One stored chunk as raw bytes (compression id + compressed payload).
#[derive(Clone)]
pub struct RawChunk {
    pub compression: u8,
    pub payload: Vec<u8>,
    pub timestamp: u32,
}

impl RawChunk {
    pub fn decode_nbt(&self) -> Result<chunker_nbt::Compound, RegionError> {
        let compression = Compression::from_id(self.compression)
            .ok_or(RegionError::UnsupportedCompression(self.compression))?;
        let bytes = compression
            .decompress(&self.payload)
            .map_err(|e| RegionError::Corrupt(e.to_string()))?;
        Ok(chunker_nbt::read_root(&bytes)?)
    }
}

/// An in-memory view of one region file.
pub struct Region {
    pub pos: RegionPos,
    chunks: Vec<Option<RawChunk>>,
}

impl Region {
    pub fn empty(pos: RegionPos) -> Self {
        Self {
            pos,
            chunks: (0..1024).map(|_| None).collect(),
        }
    }

    /// Reads only the location table and returns which chunk slots are occupied.
    /// This is what vanilla `hasChunk` checks; it is cheap (one 4 KiB read).
    pub fn read_presence(path: &Path) -> Result<Option<[bool; 1024]>, RegionError> {
        let mut file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err(path)(e)),
        };
        let mut header = [0u8; SECTOR_BYTES];
        let read = read_fully(&mut file, &mut header).map_err(io_err(path))?;
        let mut out = [false; 1024];
        if read < SECTOR_BYTES {
            return Ok(Some(out));
        }
        for (i, slot) in out.iter_mut().enumerate() {
            let loc = u32::from_be_bytes(header[i * 4..i * 4 + 4].try_into().unwrap());
            *slot = loc != 0 && (loc >> 8) >= 2 && (loc & 0xFF) != 0;
        }
        Ok(Some(out))
    }

    /// Loads a region file fully (all chunk payloads). Missing file -> empty region.
    pub fn load(dir: &Path, pos: RegionPos) -> Result<Self, RegionError> {
        let path = dir.join(pos.file_name());
        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::empty(pos)),
            Err(e) => return Err(io_err(&path)(e)),
        };
        let mut region = Self::empty(pos);
        if data.len() < SECTOR_BYTES * 2 {
            return Ok(region);
        }
        for i in 0..1024 {
            let loc = u32::from_be_bytes(data[i * 4..i * 4 + 4].try_into().unwrap());
            if loc == 0 {
                continue;
            }
            let timestamp = u32::from_be_bytes(
                data[SECTOR_BYTES + i * 4..SECTOR_BYTES + i * 4 + 4]
                    .try_into()
                    .unwrap(),
            );
            let sector = (loc >> 8) as usize;
            let count = (loc & 0xFF) as usize;
            if sector < 2 || count == 0 || sector * SECTOR_BYTES + 5 > data.len() {
                continue;
            }
            let start = sector * SECTOR_BYTES;
            let length = u32::from_be_bytes(data[start..start + 4].try_into().unwrap()) as usize;
            let compression = data[start + 4];
            if length == 0 {
                continue;
            }
            if compression & EXTERNAL_FLAG != 0 {
                let ext = dir.join(format!(
                    "c.{}.{}.mcc",
                    pos.min_chunk_x() + (i as i32 & 31),
                    pos.min_chunk_z() + (i as i32 >> 5)
                ));
                match fs::read(&ext) {
                    Ok(payload) => {
                        region.chunks[i] = Some(RawChunk {
                            compression: compression & !EXTERNAL_FLAG,
                            payload,
                            timestamp,
                        })
                    }
                    Err(_) => continue,
                }
                continue;
            }
            let payload_len = length - 1;
            let end = start + 5 + payload_len;
            if end > data.len() || payload_len > count * SECTOR_BYTES {
                continue;
            }
            region.chunks[i] = Some(RawChunk {
                compression,
                payload: data[start + 5..end].to_vec(),
                timestamp,
            });
        }
        Ok(region)
    }

    pub fn get(&self, index: usize) -> Option<&RawChunk> {
        self.chunks[index].as_ref()
    }

    pub fn has(&self, index: usize) -> bool {
        self.chunks[index].is_some()
    }

    pub fn set(&mut self, index: usize, chunk: RawChunk) {
        self.chunks[index] = Some(chunk);
    }

    /// Serializes and compresses an NBT chunk into a slot.
    pub fn set_nbt(
        &mut self,
        index: usize,
        nbt: &chunker_nbt::Compound,
        compression: Compression,
        timestamp: u32,
    ) {
        let raw = chunker_nbt::write_root(nbt);
        let payload = compression.compress(&raw);
        self.chunks[index] = Some(RawChunk {
            compression: compression.id(),
            payload,
            timestamp,
        });
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.iter().filter(|c| c.is_some()).count()
    }

    /// Writes the region atomically into `dir`. Oversized chunks go to `.mcc` files.
    pub fn save(&self, dir: &Path) -> Result<(), RegionError> {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
        let path = dir.join(self.pos.file_name());
        let mut header = vec![0u8; SECTOR_BYTES * 2];
        let mut body: Vec<u8> = Vec::new();
        let mut next_sector = 2usize;
        let mut externals: Vec<(PathBuf, &[u8])> = Vec::new();

        for (i, chunk) in self.chunks.iter().enumerate() {
            let Some(chunk) = chunk else { continue };
            let total = chunk.payload.len() + 5;
            let mut sectors = total.div_ceil(SECTOR_BYTES);
            let chunk_x = self.pos.min_chunk_x() + (i as i32 & 31);
            let chunk_z = self.pos.min_chunk_z() + (i as i32 >> 5);
            let ext_path = dir.join(format!("c.{chunk_x}.{chunk_z}.mcc"));
            if sectors >= EXTERNAL_THRESHOLD_SECTORS {
                sectors = 1;
                body.extend_from_slice(&1u32.to_be_bytes());
                body.push(chunk.compression | EXTERNAL_FLAG);
                externals.push((ext_path, &chunk.payload));
            } else {
                body.extend_from_slice(&((chunk.payload.len() + 1) as u32).to_be_bytes());
                body.push(chunk.compression);
                body.extend_from_slice(&chunk.payload);
                // Remove a stale external file for a chunk that now fits.
                let _ = fs::remove_file(&ext_path);
            }
            let padded = (next_sector - 2 + sectors) * SECTOR_BYTES;
            body.resize(padded, 0);
            let loc = ((next_sector as u32) << 8) | sectors as u32;
            header[i * 4..i * 4 + 4].copy_from_slice(&loc.to_be_bytes());
            header[SECTOR_BYTES + i * 4..SECTOR_BYTES + i * 4 + 4]
                .copy_from_slice(&chunk.timestamp.to_be_bytes());
            next_sector += sectors;
        }

        for (ext_path, payload) in externals {
            let tmp = ext_path.with_extension("mcc.tmp");
            fs::write(&tmp, payload).map_err(io_err(&tmp))?;
            fs::rename(&tmp, &ext_path).map_err(io_err(&ext_path))?;
        }

        let tmp = dir.join(format!("{}.chunker-tmp", self.pos.file_name()));
        {
            let mut file = fs::File::create(&tmp).map_err(io_err(&tmp))?;
            file.write_all(&header).map_err(io_err(&tmp))?;
            file.write_all(&body).map_err(io_err(&tmp))?;
            file.sync_all().map_err(io_err(&tmp))?;
        }
        fs::rename(&tmp, &path).map_err(io_err(&path))?;
        Ok(())
    }
}

fn read_fully(file: &mut fs::File, buf: &mut [u8]) -> io::Result<usize> {
    file.seek(SeekFrom::Start(0))?;
    let mut read = 0;
    while read < buf.len() {
        let n = file.read(&mut buf[read..])?;
        if n == 0 {
            break;
        }
        read += n;
    }
    Ok(read)
}

/// Seconds since the Unix epoch, as vanilla stores chunk timestamps.
pub fn now_timestamp() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chunker_nbt::Compound;

    #[test]
    fn save_and_reload() {
        let dir = std::env::temp_dir().join(format!("chunker-anvil-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pos = RegionPos::new(-1, 2);
        let mut region = Region::empty(pos);
        let mut nbt = Compound::new();
        nbt.insert("xPos", -32);
        nbt.insert("Status", "minecraft:full");
        region.set_nbt(local_index(-32, 64), &nbt, Compression::Zlib, 7);
        // A large, incompressible chunk to exercise the external path.
        let big: Vec<u8> = (0..(SECTOR_BYTES * 300))
            .map(|i| ((i * 2654435761usize) >> 13) as u8)
            .collect();
        region.set(
            local_index(-31, 64),
            RawChunk {
                compression: 3,
                payload: big.clone(),
                timestamp: 9,
            },
        );
        region.save(&dir).unwrap();

        let presence = Region::read_presence(&dir.join(pos.file_name()))
            .unwrap()
            .unwrap();
        assert!(presence[local_index(-32, 64)]);
        assert!(presence[local_index(-31, 64)]);
        assert!(!presence[local_index(-30, 64)]);

        let back = Region::load(&dir, pos).unwrap();
        assert_eq!(back.chunk_count(), 2);
        assert_eq!(back.get(local_index(-32, 64)).unwrap().decode_nbt().unwrap(), nbt);
        assert_eq!(back.get(local_index(-31, 64)).unwrap().payload, big);
        let _ = fs::remove_dir_all(&dir);
    }
}
