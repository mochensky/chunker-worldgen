//! Top-down region previews: 4x4 pixels per chunk (128x128 per region) in vanilla map colours and
//! shading, plus the surface biome of every pixel.
//!
//! Tiles of chunks chunker generates are filled in while a job runs; tiles of chunks that already
//! existed are rendered from the region files on demand. Every change bumps a per-region version
//! so viewers can refetch exactly the tiles that changed.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use crate::biome::BiomeRegistry;
use crate::chunk::{HeightmapKind, ProtoChunk};
use anyhow::{Context, Result, bail};
use chunker_anvil::{Region, RegionPos, local_index};
use chunker_data::Identifier;
use chunker_data::blocks::{BlockRegistry, BlockState};
use chunker_nbt::{Compound, Tag};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

pub const PIXELS_PER_CHUNK: usize = 4;
pub const TILE_SIZE: usize = 32 * PIXELS_PER_CHUNK;
const PIXELS: usize = TILE_SIZE * TILE_SIZE;
const STEP: i32 = 16 / PIXELS_PER_CHUNK as i32;
/// Biome value of pixels without a chunk (or with an unknown biome).
pub const NO_BIOME: u16 = u16::MAX;
/// Colour of existing chunks that have no blocks yet (proto chunks before `terrain`).
const EMPTY_CHUNK: [u8; 3] = [24, 26, 29];

/// One region preview. `rgba` alpha is 0 where no chunk exists.
#[derive(Clone)]
pub struct Tile {
    pub rgba: Vec<u8>,
    pub biomes: Vec<u16>,
}

/// The preview pixels of one chunk (row-major z then x).
pub struct ChunkPixels {
    pub rgb: [[u8; 3]; PIXELS_PER_CHUNK * PIXELS_PER_CHUNK],
    pub biomes: [u16; PIXELS_PER_CHUNK * PIXELS_PER_CHUNK],
}

impl Tile {
    fn empty() -> Self {
        Self {
            rgba: vec![0; PIXELS * 4],
            biomes: vec![NO_BIOME; PIXELS],
        }
    }

    fn put(&mut self, chunk_x: i32, chunk_z: i32, pixels: &ChunkPixels) {
        let base_x = (chunk_x & 31) as usize * PIXELS_PER_CHUNK;
        let base_z = (chunk_z & 31) as usize * PIXELS_PER_CHUNK;
        for pz in 0..PIXELS_PER_CHUNK {
            for px in 0..PIXELS_PER_CHUNK {
                let p = (base_z + pz) * TILE_SIZE + base_x + px;
                let c = pixels.rgb[pz * PIXELS_PER_CHUNK + px];
                self.rgba[p * 4..p * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                self.biomes[p] = pixels.biomes[pz * PIXELS_PER_CHUNK + px];
            }
        }
    }

    pub fn surface_png(&self) -> Vec<u8> {
        encode_png(TILE_SIZE as u32, TILE_SIZE as u32, &self.rgba)
    }

    /// Biome layer coloured with `palette` (indexed by biome id).
    pub fn biome_png(&self, palette: &[[u8; 3]]) -> Vec<u8> {
        let mut rgba = vec![0u8; PIXELS * 4];
        for (i, &b) in self.biomes.iter().enumerate() {
            if let Some(c) = palette.get(b as usize) {
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        encode_png(TILE_SIZE as u32, TILE_SIZE as u32, &rgba)
    }

    // On-disk format: magic, biome name table, then lz4(rgba ++ biome indices as u16 LE).
    fn encode(&self, biomes: &BiomeRegistry) -> Vec<u8> {
        let mut local: FxHashMap<u16, u16> = FxHashMap::default();
        let mut names: Vec<&str> = Vec::new();
        let mut payload = self.rgba.clone();
        payload.reserve(PIXELS * 2);
        for &b in &self.biomes {
            let v = if b == NO_BIOME || b as usize >= biomes.len() {
                NO_BIOME
            } else {
                *local.entry(b).or_insert_with(|| {
                    names.push(biomes.name(crate::biome::BiomeId(b)).as_str());
                    (names.len() - 1) as u16
                })
            };
            payload.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = b"CKT1".to_vec();
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        for n in names {
            out.extend_from_slice(&(n.len() as u16).to_le_bytes());
            out.extend_from_slice(n.as_bytes());
        }
        out.extend_from_slice(&lz4_flex::compress_prepend_size(&payload));
        out
    }

    fn decode(bytes: &[u8], biomes: &BiomeRegistry) -> Result<Self> {
        let rest = bytes.strip_prefix(b"CKT1").context("not a tile")?;
        let read_u16 = |b: &[u8], at: usize| -> Result<u16> {
            Ok(u16::from_le_bytes(
                b.get(at..at + 2).context("truncated tile")?.try_into().unwrap(),
            ))
        };
        let count = read_u16(rest, 0)? as usize;
        let mut at = 2;
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let len = read_u16(rest, at)? as usize;
            let name = std::str::from_utf8(rest.get(at + 2..at + 2 + len).context("truncated tile")?)?;
            at += 2 + len;
            ids.push(
                Identifier::parse(name)
                    .and_then(|id| biomes.get(&id))
                    .map_or(NO_BIOME, |b| b.0),
            );
        }
        let payload = lz4_flex::decompress_size_prepended(&rest[at..])?;
        if payload.len() != PIXELS * 6 {
            bail!("bad tile size");
        }
        let rgba = payload[..PIXELS * 4].to_vec();
        let biomes = payload[PIXELS * 4..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                ids.get(u16::from_le_bytes([c[0], c[1]]) as usize)
                    .copied()
                    .unwrap_or(NO_BIOME)
            })
            .collect();
        Ok(Self { rgba, biomes })
    }
}

// ---------------------------------------------------------------------------------------------
// Rendering

/// Column access shared by the renderers of in-memory and stored chunks (chunk-local x/z).
trait Columns {
    fn min_y(&self) -> i32;
    /// Y of the highest non-air block (`WORLD_SURFACE` height - 1).
    fn top(&mut self, x: i32, z: i32) -> i32;
    fn block(&mut self, x: i32, y: i32, z: i32) -> BlockState;
    fn biome(&mut self, x: i32, y: i32, z: i32) -> u16;
}

/// The topmost block with a map colour: `(colour, water depth above it, y)`.
fn surface(c: &mut impl Columns, reg: &BlockRegistry, x: i32, z: i32) -> (u32, i32, i32) {
    let min_y = c.min_y();
    let mut y = c.top(x, z);
    let mut water: Option<(u32, i32)> = None;
    let mut depth = 0;
    while y >= min_y {
        let s = c.block(x, y, z);
        let f = reg.flags(s);
        if f.has_fluid() && f.is_water_fluid() {
            if water.is_none() {
                water = Some((reg.map_color(s), y));
            }
            depth += 1;
            if depth > 10 {
                break;
            }
        } else {
            let colour = reg.map_color(s);
            if colour != 0 {
                return match water {
                    Some((wc, wy)) => (wc, depth, wy),
                    None => (colour, 0, y),
                };
            }
        }
        y -= 1;
    }
    match water {
        Some((wc, wy)) => (wc, depth, wy),
        None => (0, 0, min_y),
    }
}

/// `MapColor.Brightness` multipliers (LOW, NORMAL, HIGH).
fn shade(colour: u32, brightness: u32) -> [u8; 3] {
    let f = |v: u32| (v * brightness / 255) as u8;
    [
        f((colour >> 16) & 0xFF),
        f((colour >> 8) & 0xFF),
        f(colour & 0xFF),
    ]
}

fn render(c: &mut impl Columns, reg: &BlockRegistry) -> ChunkPixels {
    let mut out = ChunkPixels {
        rgb: [[0; 3]; 16],
        biomes: [NO_BIOME; 16],
    };
    for pz in 0..PIXELS_PER_CHUNK as i32 {
        for px in 0..PIXELS_PER_CHUNK as i32 {
            let (x, z) = (px * STEP + STEP / 2, pz * STEP + STEP / 2);
            let (colour, depth, y) = surface(c, reg, x, z);
            let i = (pz * PIXELS_PER_CHUNK as i32 + px) as usize;
            if colour == 0 {
                out.rgb[i] = EMPTY_CHUNK;
                continue;
            }
            // Shading like `MapItem.update` at scale 0: water by depth, land by the step to the north.
            let checker = ((x + z) & 1) as f64;
            let brightness = if depth > 0 {
                let d = depth as f64 * 0.1 + checker * 0.2;
                if d < 0.5 {
                    255
                } else if d > 0.9 {
                    180
                } else {
                    220
                }
            } else {
                let north = surface(c, reg, x, z - 1).2;
                let d = (y - north) as f64 + (checker - 0.5) * 0.4;
                if d > 0.6 {
                    255
                } else if d < -0.6 {
                    180
                } else {
                    220
                }
            };
            out.rgb[i] = shade(colour, brightness);
            out.biomes[i] = c.biome(x, y, z);
        }
    }
    out
}

struct ProtoColumns<'a>(&'a mut ProtoChunk);

impl Columns for ProtoColumns<'_> {
    fn min_y(&self) -> i32 {
        self.0.min_y
    }
    fn top(&mut self, x: i32, z: i32) -> i32 {
        self.0.height_at(HeightmapKind::WorldSurface, x, z)
    }
    fn block(&mut self, x: i32, y: i32, z: i32) -> BlockState {
        self.0.get_block(x, y, z)
    }
    fn biome(&mut self, x: i32, y: i32, z: i32) -> u16 {
        self.0.noise_biome(x >> 2, y >> 2, z >> 2).0
    }
}

/// Preview pixels of a freshly generated chunk.
pub fn render_chunk(chunk: &mut ProtoChunk) -> ChunkPixels {
    let reg = chunk.blocks.clone();
    render(&mut ProtoColumns(chunk), &reg)
}

/// Decoded section of a stored chunk.
struct StoredSection {
    blocks: Vec<BlockState>,
    biomes: [u16; 64],
}

/// A chunk read from a region file, decoded lazily section by section.
struct StoredChunk<'a> {
    min_y: i32,
    height: i32,
    sections: Vec<Option<&'a Compound>>,
    decoded: Vec<Option<StoredSection>>,
    heights: Option<Vec<i32>>,
    blocks: &'a BlockRegistry,
    biomes: &'a BiomeRegistry,
    states: &'a mut FxHashMap<String, BlockState>,
}

fn ceil_log2(n: usize) -> u32 {
    if n <= 1 {
        0
    } else {
        usize::BITS - (n - 1).leading_zeros()
    }
}

/// Reads entry `i` of a `SimpleBitStorage` long array.
fn unpack(data: &[i64], bits: u32, i: usize) -> usize {
    let per_long = (64 / bits) as usize;
    let word = data.get(i / per_long).copied().unwrap_or(0) as u64;
    ((word >> ((i % per_long) as u32 * bits)) & ((1u64 << bits) - 1)) as usize
}

impl<'a> StoredChunk<'a> {
    fn new(
        nbt: &'a Compound,
        min_y: i32,
        height: i32,
        blocks: &'a BlockRegistry,
        biomes: &'a BiomeRegistry,
        states: &'a mut FxHashMap<String, BlockState>,
    ) -> Self {
        let count = (height >> 4) as usize;
        let mut sections = vec![None; count];
        for s in nbt.get_list("sections").into_iter().flat_map(|l| l.compounds()) {
            let index = s.get_i32("Y").unwrap_or(i32::MIN) - (min_y >> 4);
            if (0..count as i32).contains(&index) {
                sections[index as usize] = Some(s);
            }
        }
        let heights = nbt
            .get_compound("Heightmaps")
            .and_then(|h| h.get_long_array("WORLD_SURFACE"))
            .map(|data| {
                let bits = ceil_log2(height as usize + 1);
                (0..256).map(|i| unpack(data, bits, i) as i32).collect()
            });
        Self {
            min_y,
            height,
            sections,
            decoded: (0..count).map(|_| None).collect(),
            heights,
            blocks,
            biomes,
            states,
        }
    }

    fn state_of(&mut self, entry: &Tag) -> BlockState {
        let (id, props) = match entry {
            Tag::String(s) => (s.as_str(), None),
            Tag::Compound(c) => match c.get("") {
                Some(Tag::String(s)) => (s.as_str(), None),
                _ => (
                    c.get_str("id")
                        .or_else(|| c.get_str("Name"))
                        .unwrap_or("minecraft:air"),
                    c.get_compound("properties")
                        .or_else(|| c.get_compound("Properties")),
                ),
            },
            _ => ("minecraft:air", None),
        };
        let mut key = id.to_owned();
        if let Some(p) = props.filter(|p| !p.is_empty()) {
            let kv: Vec<String> = p
                .iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                .collect();
            key = format!("{id}[{}]", kv.join(","));
        }
        if let Some(s) = self.states.get(&key) {
            return *s;
        }
        let state = self
            .blocks
            .parse_state_str(&key)
            .or_else(|_| self.blocks.parse_state_str(id))
            .unwrap_or(BlockState::AIR);
        self.states.insert(key, state);
        state
    }

    fn section(&mut self, index: usize) -> Option<&StoredSection> {
        if self.decoded[index].is_none() {
            let nbt = self.sections[index]?;
            let mut blocks = vec![BlockState::AIR; 4096];
            if let Some(bs) = nbt.get_compound("block_states") {
                let palette: Vec<BlockState> = bs
                    .get_list("palette")
                    .map(|l| l.iter().map(|e| self.state_of(e)).collect())
                    .unwrap_or_default();
                match bs.get_long_array("data") {
                    Some(data) if palette.len() > 1 => {
                        let bits = ceil_log2(palette.len()).max(4);
                        for (i, b) in blocks.iter_mut().enumerate() {
                            *b = palette
                                .get(unpack(data, bits, i))
                                .copied()
                                .unwrap_or(BlockState::AIR);
                        }
                    }
                    _ => blocks.fill(palette.first().copied().unwrap_or(BlockState::AIR)),
                }
            }
            let mut biomes = [NO_BIOME; 64];
            if let Some(bi) = nbt.get_compound("biomes") {
                let palette: Vec<u16> = bi
                    .get_list("palette")
                    .map(|l| {
                        l.iter()
                            .map(|e| {
                                e.as_str()
                                    .and_then(Identifier::parse)
                                    .and_then(|id| self.biomes.get(&id))
                                    .map_or(NO_BIOME, |b| b.0)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                match bi.get_long_array("data") {
                    Some(data) if palette.len() > 1 => {
                        let bits = ceil_log2(palette.len());
                        for (i, b) in biomes.iter_mut().enumerate() {
                            *b = palette.get(unpack(data, bits, i)).copied().unwrap_or(NO_BIOME);
                        }
                    }
                    _ => biomes.fill(palette.first().copied().unwrap_or(NO_BIOME)),
                }
            }
            self.decoded[index] = Some(StoredSection { blocks, biomes });
        }
        self.decoded[index].as_ref()
    }

    fn section_index(&self, y: i32) -> Option<usize> {
        let i = (y >> 4) - (self.min_y >> 4);
        (i >= 0 && (i as usize) < self.sections.len()).then_some(i as usize)
    }
}

impl Columns for StoredChunk<'_> {
    fn min_y(&self) -> i32 {
        self.min_y
    }

    fn top(&mut self, x: i32, z: i32) -> i32 {
        if let Some(h) = &self.heights {
            return self.min_y + h[(z * 16 + x) as usize] - 1;
        }
        let mut y = self.min_y + self.height - 1;
        while y >= self.min_y {
            if !self.blocks.flags(self.block(x, y, z)).is_air() {
                return y;
            }
            y -= 1;
        }
        self.min_y - 1
    }

    fn block(&mut self, x: i32, y: i32, z: i32) -> BlockState {
        let Some(si) = self.section_index(y) else {
            return BlockState::AIR;
        };
        match self.section(si) {
            Some(s) => s.blocks[(((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize],
            None => BlockState::AIR,
        }
    }

    fn biome(&mut self, x: i32, y: i32, z: i32) -> u16 {
        let y = y.clamp(self.min_y, self.min_y + self.height - 1);
        let Some(si) = self.section_index(y) else {
            return NO_BIOME;
        };
        match self.section(si) {
            Some(s) => s.biomes[((((y >> 2) & 3) << 4) | (((z >> 2) & 3) << 2) | ((x >> 2) & 3)) as usize],
            None => NO_BIOME,
        }
    }
}

/// What the store needs to render chunks of its dimension from region files.
pub struct RenderContext {
    pub blocks: Arc<BlockRegistry>,
    pub biomes: Arc<BiomeRegistry>,
    pub min_y: i32,
    pub height: i32,
    pub region_dir: PathBuf,
}

/// Renders every chunk of a region file (`None` if the file does not exist).
pub fn render_region_file(ctx: &RenderContext, pos: RegionPos) -> Result<Option<Tile>> {
    if !ctx.region_dir.join(pos.file_name()).exists() {
        return Ok(None);
    }
    let region = Region::load(&ctx.region_dir, pos)?;
    let mut tile = Tile::empty();
    let mut states = FxHashMap::default();
    for lz in 0..32 {
        for lx in 0..32 {
            let Some(raw) = region.get(local_index(lx, lz)) else {
                continue;
            };
            let Ok(nbt) = raw.decode_nbt() else { continue };
            let mut chunk =
                StoredChunk::new(&nbt, ctx.min_y, ctx.height, &ctx.blocks, &ctx.biomes, &mut states);
            let pixels = render(&mut chunk, &ctx.blocks);
            tile.put(lx, lz, &pixels);
        }
    }
    Ok(Some(tile))
}

// ---------------------------------------------------------------------------------------------
// Store

pub struct PreviewStore {
    dir: PathBuf,
    ctx: RenderContext,
    /// Tiles of regions a job is generating right now.
    live: Mutex<FxHashMap<RegionPos, Tile>>,
    versions: Mutex<FxHashMap<RegionPos, u64>>,
    seq: AtomicU64,
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl PreviewStore {
    pub fn new(dir: PathBuf, ctx: RenderContext) -> Self {
        Self {
            dir,
            ctx,
            live: Mutex::default(),
            versions: Mutex::default(),
            seq: AtomicU64::new(1),
        }
    }

    fn tile_path(&self, pos: RegionPos) -> PathBuf {
        self.dir.join(format!("r.{}.{}.ckt", pos.x, pos.z))
    }

    fn bump(&self, pos: RegionPos) {
        let v = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.versions.lock().insert(pos, v);
    }

    /// Current version of a region's tile (0 = unchanged this session).
    pub fn version(&self, pos: RegionPos) -> u64 {
        self.versions.lock().get(&pos).copied().unwrap_or(0)
    }

    /// Regions whose tiles changed after sequence number `since`, and the new sequence number.
    pub fn changes_since(&self, since: u64) -> (u64, Vec<(RegionPos, u64)>) {
        let now = self.seq.load(Ordering::Relaxed);
        let changed = self
            .versions
            .lock()
            .iter()
            .filter(|(_, v)| **v > since)
            .map(|(p, v)| (*p, *v))
            .collect();
        (now, changed)
    }

    /// The saved tile, if it is at least as new as the region file.
    fn load_fresh(&self, pos: RegionPos) -> Option<Tile> {
        let tile_path = self.tile_path(pos);
        let tile_time = modified(&tile_path)?;
        let region_time = modified(&self.ctx.region_dir.join(pos.file_name()))?;
        if tile_time < region_time {
            return None;
        }
        Tile::decode(&fs::read(&tile_path).ok()?, &self.ctx.biomes).ok()
    }

    fn save(&self, pos: RegionPos, tile: &Tile) {
        let _ = fs::create_dir_all(&self.dir);
        let path = self.tile_path(pos);
        let tmp = path.with_extension("ckt.tmp");
        if fs::write(&tmp, tile.encode(&self.ctx.biomes)).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }

    /// The tile of a region: live, saved, or rendered from the region file (then saved).
    pub fn tile(&self, pos: RegionPos) -> Option<Tile> {
        if let Some(t) = self.live.lock().get(&pos) {
            return Some(t.clone());
        }
        if let Some(t) = self.load_fresh(pos) {
            return Some(t);
        }
        match render_region_file(&self.ctx, pos) {
            Ok(Some(t)) => {
                self.save(pos, &t);
                Some(t)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("preview of {}: {e:#}", pos.file_name());
                None
            }
        }
    }

    /// Called before a job generates chunks of `pos`: starts from the chunks that already exist.
    pub fn begin_region(&self, pos: RegionPos) {
        if self.live.lock().contains_key(&pos) {
            return;
        }
        let tile = self.tile(pos).unwrap_or_else(Tile::empty);
        self.live.lock().insert(pos, tile);
        self.bump(pos);
    }

    pub fn put_chunk(&self, chunk_x: i32, chunk_z: i32, pixels: &ChunkPixels) {
        let pos = RegionPos::of_chunk(chunk_x, chunk_z);
        self.live
            .lock()
            .entry(pos)
            .or_insert_with(Tile::empty)
            .put(chunk_x, chunk_z, pixels);
        self.bump(pos);
    }

    /// Persists the tile of a region whose region file was just written.
    pub fn finish_region(&self, pos: RegionPos) {
        let tile = self.live.lock().remove(&pos);
        if let Some(tile) = tile {
            self.save(pos, &tile);
            self.bump(pos);
        }
    }

    /// Regions that have a region file or a tile in progress: `(pos, stored chunk count)`.
    pub fn regions(&self) -> Vec<(RegionPos, usize)> {
        let mut out: FxHashMap<RegionPos, usize> = FxHashMap::default();
        if let Ok(entries) = fs::read_dir(&self.ctx.region_dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(pos) = RegionPos::parse_file_name(&name) {
                    let count = Region::read_presence(&e.path())
                        .ok()
                        .flatten()
                        .map_or(0, |p| p.iter().filter(|b| **b).count());
                    if count > 0 {
                        out.insert(pos, count);
                    }
                }
            }
        }
        for pos in self.live.lock().keys() {
            out.entry(*pos).or_insert(0);
        }
        out.into_iter().collect()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

// ---------------------------------------------------------------------------------------------
// Biome colours

/// Map colours of the vanilla biomes (close to the usual seed-map palettes).
const BIOME_COLOURS: &[(&str, u32)] = &[
    ("ocean", 0x000070),
    ("deep_ocean", 0x000030),
    ("warm_ocean", 0x0000ac),
    ("lukewarm_ocean", 0x000090),
    ("deep_lukewarm_ocean", 0x000040),
    ("cold_ocean", 0x202070),
    ("deep_cold_ocean", 0x202038),
    ("frozen_ocean", 0x7070d6),
    ("deep_frozen_ocean", 0x404090),
    ("river", 0x0000ff),
    ("frozen_river", 0xa0a0ff),
    ("beach", 0xfade55),
    ("snowy_beach", 0xfaf0c0),
    ("stony_shore", 0xa2a284),
    ("plains", 0x8db360),
    ("sunflower_plains", 0xb5db88),
    ("snowy_plains", 0xf0f8ff),
    ("ice_spikes", 0xb4dcdc),
    ("desert", 0xfa9418),
    ("swamp", 0x07f9b2),
    ("mangrove_swamp", 0x2ccc8e),
    ("forest", 0x056621),
    ("flower_forest", 0x2d8e49),
    ("birch_forest", 0x307444),
    ("old_growth_birch_forest", 0x589c6c),
    ("dark_forest", 0x40511a),
    ("pale_garden", 0x8a9389),
    ("dappled_forest", 0x4f7f2e),
    ("taiga", 0x0b6659),
    ("old_growth_pine_taiga", 0x596651),
    ("old_growth_spruce_taiga", 0x818e79),
    ("snowy_taiga", 0x31554a),
    ("savanna", 0xbdb25f),
    ("savanna_plateau", 0xa79d64),
    ("windswept_savanna", 0xe5da87),
    ("windswept_hills", 0x606060),
    ("windswept_gravelly_hills", 0x888888),
    ("windswept_forest", 0x507050),
    ("jungle", 0x537b09),
    ("sparse_jungle", 0x628b17),
    ("bamboo_jungle", 0x768e14),
    ("badlands", 0xd94515),
    ("eroded_badlands", 0xff6d3d),
    ("wooded_badlands", 0xb09765),
    ("meadow", 0x83bb6d),
    ("cherry_grove", 0xffb7d5),
    ("grove", 0x47726c),
    ("snowy_slopes", 0xc4c4c4),
    ("frozen_peaks", 0xdcdcff),
    ("jagged_peaks", 0xdcdcc8),
    ("stony_peaks", 0x7b8f74),
    ("mushroom_fields", 0xff00ff),
    ("dripstone_caves", 0x866043),
    ("lush_caves", 0x7ba331),
    ("deep_dark", 0x031f29),
    ("sulfur_caves", 0xd8d23a),
    ("nether_wastes", 0xbf3b3b),
    ("soul_sand_valley", 0x5e3830),
    ("crimson_forest", 0xdd0808),
    ("warped_forest", 0x49907b),
    ("basalt_deltas", 0x403636),
    ("the_end", 0x8080ff),
    ("end_highlands", 0xb5b5ff),
    ("end_midlands", 0x9a9aff),
    ("small_end_islands", 0x6f6fcf),
    ("end_barrens", 0x7575e0),
    ("the_void", 0x101010),
];

/// One distinct colour per biome id (unknown biomes get a stable hashed colour).
pub fn biome_palette(biomes: &BiomeRegistry) -> Vec<[u8; 3]> {
    let mut used = rustc_hash::FxHashSet::default();
    let mut out = Vec::with_capacity(biomes.len());
    for i in 0..biomes.len() {
        let id = biomes.name(crate::biome::BiomeId(i as u16));
        let mut c = match (id.namespace() == "minecraft")
            .then(|| BIOME_COLOURS.iter().find(|(n, _)| *n == id.path()))
            .flatten()
        {
            Some((_, c)) => *c,
            None => {
                let h = id
                    .as_str()
                    .bytes()
                    .fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
                h & 0xffffff
            }
        };
        while !used.insert(c) {
            c = (c + 0x010203) & 0xffffff;
        }
        out.push([(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// PNG

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xedb88320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for part in parts {
        for &b in *part {
            c = CRC_TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
    }
    c ^ 0xffff_ffff
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// Minimal RGBA PNG encoder (filter 0, zlib).
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    z.write_all(&raw).expect("in-memory write");
    let idat = z.finish().expect("in-memory write");
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &idat);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_reference() {
        assert_eq!(crc32(&[b"123456789"]), 0xcbf43926);
    }

    #[test]
    fn unpack_reads_simple_bit_storage() {
        // 9-bit entries, seven per long.
        let data = [(0x155i64 << 9) | 3];
        assert_eq!(unpack(&data, 9, 0), 3);
        assert_eq!(unpack(&data, 9, 1), 0x155);
    }
}
