//! Worldgen registries loaded from data packs (vanilla jar + world data packs).
//!
//! Entries are parsed lazily and memoised. A registry reference always resolves to the same
//! `Arc`, which keeps vanilla `Holder` identity semantics where they matter.

use std::sync::Arc;

use anyhow::{Context, Result};
use chunker_data::blocks::BlockRegistry;
use chunker_data::{DataPacks, Identifier};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::density::{DensityFunction, Df};
use crate::noise::{NormalNoise, NormalNoiseParams};

pub struct WorldgenRegistries {
    pub packs: DataPacks,
    pub blocks: Arc<BlockRegistry>,
    density_functions: Mutex<FxHashMap<Identifier, Df>>,
    noises: Mutex<FxHashMap<Identifier, Arc<NormalNoise>>>,
    pub(crate) block_tags: Mutex<FxHashMap<Identifier, Arc<crate::block_set::StateSet>>>,
}

impl WorldgenRegistries {
    pub fn new(packs: DataPacks, blocks: Arc<BlockRegistry>) -> Self {
        Self {
            packs,
            blocks,
            density_functions: Mutex::default(),
            noises: Mutex::default(),
            block_tags: Mutex::default(),
        }
    }

    /// A `worldgen/density_function` entry as a reference node (`HolderHolder`).
    pub fn density_function(&self, id: &Identifier) -> Result<Df> {
        if let Some(df) = self.density_functions.lock().get(id) {
            return Ok(df.clone());
        }
        let json = self.packs.require_json("worldgen/density_function", id)?;
        let target = self
            .parse_density_function(&json)
            .with_context(|| format!("density function {id}"))?;
        let node = Arc::new(DensityFunction::Reference {
            id: id.clone(),
            target,
        });
        Ok(self
            .density_functions
            .lock()
            .entry(id.clone())
            .or_insert(node)
            .clone())
    }

    /// A `worldgen/noise` entry.
    pub fn noise(&self, id: &Identifier) -> Result<Arc<NormalNoise>> {
        if let Some(n) = self.noises.lock().get(id) {
            return Ok(n.clone());
        }
        let json = self.packs.require_json("worldgen/noise", id)?;
        let params = NormalNoiseParams::from_json(&json).with_context(|| format!("noise {id}"))?;
        let noise = Arc::new(NormalNoise::new(params));
        Ok(self.noises.lock().entry(id.clone()).or_insert(noise).clone())
    }
}
