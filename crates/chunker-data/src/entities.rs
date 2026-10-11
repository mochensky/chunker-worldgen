//! Code-defined entity and item data that worldgen needs for `Mob.finalizeSpawn`: default attribute
//! values per entity type (`DefaultAttributes`) and the `enchantable` value of items.

use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use rustc_hash::FxHashMap;

use crate::McVersion;

pub struct EntityTables {
    /// Entity type -> (attribute -> default base value).
    attributes: FxHashMap<String, FxHashMap<String, f64>>,
    /// Item -> `DataComponents.ENCHANTABLE` value.
    enchantable: FxHashMap<String, i32>,
}

static TABLES_26_3: OnceLock<Arc<EntityTables>> = OnceLock::new();

impl EntityTables {
    /// The embedded tables for a version.
    pub fn for_version(version: McVersion) -> Arc<EntityTables> {
        match version {
            McVersion::V26_3 => TABLES_26_3
                .get_or_init(|| {
                    Arc::new(
                        Self::parse(
                            include_str!("../data/26.3/entity_attributes.txt"),
                            include_str!("../data/26.3/item_enchantable.txt"),
                        )
                        .expect("embedded entity tables"),
                    )
                })
                .clone(),
        }
    }

    fn parse(attributes_text: &str, enchantable_text: &str) -> Result<Self> {
        let mut attributes = FxHashMap::default();
        for line in attributes_text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let (entity, list) = line.split_once('|').with_context(|| format!("bad line {line}"))?;
            let mut map = FxHashMap::default();
            for entry in list.split(',').filter(|e| !e.is_empty()) {
                let (attribute, bits) = entry
                    .split_once('=')
                    .with_context(|| format!("bad entry {entry}"))?;
                let bits = u64::from_str_radix(bits, 16).with_context(|| format!("bad value {entry}"))?;
                map.insert(attribute.to_owned(), f64::from_bits(bits));
            }
            attributes.insert(entity.to_owned(), map);
        }
        let mut enchantable = FxHashMap::default();
        for line in enchantable_text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let Some((item, value)) = line.split_once('=') else {
                bail!("bad line {line}");
            };
            enchantable.insert(item.to_owned(), value.parse()?);
        }
        Ok(Self {
            attributes,
            enchantable,
        })
    }

    /// `DefaultAttributes.getSupplier(type).getBaseValue(attribute)`; `None` if the type lacks it.
    pub fn attribute_base(&self, entity: &str, attribute: &str) -> Option<f64> {
        self.attributes.get(entity)?.get(attribute).copied()
    }

    /// The default `enchantable` component of an item, `None` for items that can't be enchanted.
    pub fn enchantable(&self, item: &str) -> Option<i32> {
        self.enchantable.get(item).copied()
    }
}
