//! `SamplerContext`: per-evaluation state (cache cells, user fields, scratch buffers).

use std::sync::Arc;

use super::DensityVolume;
use super::sampler::Sampler;

/// Context fields bound by `ContextBoundSampler` (`Beardifier.CONTEXT_KEY`, blender alpha/offset).
#[derive(Default, Clone)]
pub struct UserFields {
    pub beardifier: Option<Arc<Sampler>>,
    pub blend_alpha: Option<Arc<Sampler>>,
    pub blend_offset: Option<Arc<Sampler>>,
}

#[derive(Default)]
struct CacheCell {
    volume: Option<DensityVolume>,
    buffer: Vec<f32>,
    has_buffer: bool,
    value_key: i64,
    value: f32,
    has_value: bool,
}

pub struct SamplerContext {
    pub fields: UserFields,
    caches_enabled: bool,
    cells: Vec<CacheCell>,
    pool: Vec<Vec<f32>>,
}

const POOL_LIMIT: usize = 64;

thread_local! {
    /// Scratch buffers handed from one `SamplerContext` to the next on the same thread, so a worker
    /// does not allocate, zero and free megabytes of buffers for every chunk.
    static THREAD_POOL: std::cell::RefCell<Vec<Vec<f32>>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn take_thread_pool() -> Vec<Vec<f32>> {
    THREAD_POOL.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

impl Drop for SamplerContext {
    fn drop(&mut self) {
        let mut pool = std::mem::take(&mut self.pool);
        for cell in &mut self.cells {
            if pool.len() >= POOL_LIMIT {
                break;
            }
            let b = std::mem::take(&mut cell.buffer);
            if b.capacity() > 0 {
                pool.push(b);
            }
        }
        let _ = THREAD_POOL.try_with(|p| {
            let mut p = p.borrow_mut();
            if p.len() < pool.len() {
                *p = pool;
            }
        });
    }
}

/// `BlockPos.asLong` packing.
#[inline(always)]
pub fn pack_block_pos(x: i32, y: i32, z: i32) -> i64 {
    ((x as i64 & 0x3FF_FFFF) << 38) | ((y as i64) & 0xFFF) | ((z as i64 & 0x3FF_FFFF) << 12)
}

impl SamplerContext {
    /// A context with caches enabled (what `NoiseChunk`, biome generation, etc. use).
    pub fn cached(fields: UserFields) -> Self {
        Self {
            fields,
            caches_enabled: true,
            cells: Vec::new(),
            pool: take_thread_pool(),
        }
    }

    /// `SamplerContext.EMPTY_UNCACHED`.
    pub fn uncached() -> Self {
        Self {
            fields: UserFields::default(),
            caches_enabled: false,
            cells: Vec::new(),
            pool: take_thread_pool(),
        }
    }

    /// Takes a scratch buffer of exactly `len` floats (contents unspecified).
    #[inline]
    pub fn acquire(&mut self, len: usize) -> Vec<f32> {
        // Prefer a buffer that is already long enough (no reallocation, no zeroing).
        let pick = self
            .pool
            .iter()
            .rposition(|v| v.len() >= len)
            .or(if self.pool.is_empty() {
                None
            } else {
                Some(self.pool.len() - 1)
            });
        match pick.map(|i| self.pool.swap_remove(i)) {
            Some(mut v) => {
                v.resize(len, 0.0);
                v
            }
            None => vec![0.0; len],
        }
    }

    #[inline]
    pub fn release(&mut self, buf: Vec<f32>) {
        if self.pool.len() < POOL_LIMIT && buf.capacity() > 0 {
            self.pool.push(buf);
        }
    }

    fn cell_index(&mut self, id: usize) -> Option<usize> {
        if !self.caches_enabled {
            return None;
        }
        if id >= self.cells.len() {
            self.cells.resize_with(id + 1, CacheCell::default);
        }
        Some(id)
    }

    /// `SamplerContext.sampleVolumeCached`.
    pub(crate) fn sample_volume_cached(
        &mut self,
        id: usize,
        input: &Sampler,
        out: &mut [f32],
        volume: &DensityVolume,
    ) {
        let Some(ci) = self.cell_index(id) else {
            input.sample_volume(self, out, volume);
            return;
        };
        let valid = {
            let cell = &self.cells[ci];
            cell.has_buffer && cell.volume.as_ref() == Some(volume)
        };
        if !valid {
            let old = std::mem::take(&mut self.cells[ci].buffer);
            if self.cells[ci].has_buffer {
                self.release(old);
            }
            self.cells[ci].has_buffer = false;
            let mut buf = self.acquire(volume.size());
            input.sample_volume(self, &mut buf, volume);
            let cell = &mut self.cells[ci];
            cell.volume = Some(*volume);
            cell.buffer = buf;
            cell.has_buffer = true;
        }
        out.copy_from_slice(&self.cells[ci].buffer[..volume.size()]);
    }

    /// `SamplerContext.sampleValueCached`.
    pub(crate) fn sample_value_cached(&mut self, id: usize, input: &Sampler, x: i32, y: i32, z: i32) -> f32 {
        let Some(ci) = self.cell_index(id) else {
            return input.sample_value(self, x, y, z);
        };
        let key = pack_block_pos(x, y, z);
        {
            let cell = &self.cells[ci];
            if cell.has_value && cell.value_key == key && !cell.value.is_nan() {
                return cell.value;
            }
            if cell.has_buffer
                && let Some(index) = cell.volume.as_ref().and_then(|v| v.index_of_block(x, y, z))
            {
                return cell.buffer[index];
            }
        }
        let value = input.sample_value(self, x, y, z);
        let cell = &mut self.cells[ci];
        cell.value_key = key;
        cell.value = value;
        cell.has_value = true;
        value
    }
}
