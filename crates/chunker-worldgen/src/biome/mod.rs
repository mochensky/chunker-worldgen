//! Biomes: registry, climate parameter lists and biome sources.

pub mod climate;
pub mod data;
pub mod manager;
mod overworld_builder;
mod source;

use std::sync::Arc;

use anyhow::{Context, Result};
use chunker_data::{DataPacks, Identifier};
use rustc_hash::FxHashMap;

pub use overworld_builder::{nether_parameters, overworld_parameters};
pub use source::{BiomeResolver, BiomeSource, ClimateSampler, parse_biome_holder_set};

/// Index into [`BiomeRegistry`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BiomeId(pub u16);

/// All biomes known to the loaded data packs.
pub struct BiomeRegistry {
    ids: Vec<Identifier>,
    by_id: FxHashMap<Identifier, BiomeId>,
    data: Vec<data::BiomeData>,
}

impl BiomeRegistry {
    pub fn load(packs: &DataPacks) -> Result<Arc<Self>> {
        let ids = packs.list("worldgen/biome");
        anyhow::ensure!(!ids.is_empty(), "no biomes in data packs");
        let by_id = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), BiomeId(i as u16)))
            .collect();
        let data = ids
            .iter()
            .map(|id| {
                data::BiomeData::parse(packs.require_json("worldgen/biome", id)?)
                    .with_context(|| format!("biome {id}"))
            })
            .collect::<Result<_>>()?;
        Ok(Arc::new(Self { ids, by_id, data }))
    }

    pub fn get(&self, id: &Identifier) -> Option<BiomeId> {
        self.by_id.get(id).copied()
    }

    pub fn require(&self, id: &Identifier) -> Result<BiomeId> {
        self.get(id).with_context(|| format!("unknown biome {id}"))
    }

    pub fn data(&self, id: BiomeId) -> &data::BiomeData {
        &self.data[id.0 as usize]
    }

    pub fn name(&self, id: BiomeId) -> &Identifier {
        &self.ids[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}
