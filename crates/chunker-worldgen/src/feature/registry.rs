//! Placed/configured features from the data pack, biome feature lists and the global feature
//! order (`FeatureSorter.buildFeaturesPerStep`).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use chunker_data::blocks::{BlockId, BlockState, Fluid};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;

use super::kinds::Feature;
use super::placement::Placement;
use super::provider::StateProvider;
use crate::biome::{BiomeId, BiomeRegistry};
use crate::block_set::StateSet;
use crate::codec::{as_array, field};
use crate::registry::WorldgenRegistries;

/// `GenerationStep.Decoration.values().length`.
pub const DECORATION_STEPS: usize = 11;

/// `PlacedFeature`: a feature and its placement modifiers.
pub struct PlacedFeature {
    /// Registry id for top level features (`None` for inline ones).
    pub id: Option<String>,
    pub feature: Arc<Feature>,
    pub placement: Vec<Placement>,
}

impl std::fmt::Debug for PlacedFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PlacedFeature({:?})", self.id)
    }
}

/// Biomes of a `HolderSet<Biome>`, indexed by id.
#[derive(Debug, Clone)]
pub struct BiomeSet(Vec<bool>);

impl BiomeSet {
    pub fn contains(&self, b: BiomeId) -> bool {
        self.0.get(b.0 as usize).copied().unwrap_or(false)
    }
}

/// JSON resolver with caches for registry references (shared holders compare by identity).
pub struct Parser<'a> {
    pub reg: &'a WorldgenRegistries,
    pub biomes: &'a BiomeRegistry,
    providers: RefCell<FxHashMap<Identifier, Arc<StateProvider>>>,
    features: RefCell<FxHashMap<Identifier, Arc<Feature>>>,
    placed: RefCell<FxHashMap<Identifier, Arc<PlacedFeature>>>,
}

/// `Identifier.parse` with an error.
pub fn id(s: &str) -> Result<Identifier> {
    Identifier::parse(s).with_context(|| format!("invalid identifier {s}"))
}

fn strip_hash(s: &str) -> (bool, &str) {
    match s.strip_prefix('#') {
        Some(t) => (true, t),
        None => (false, s),
    }
}

impl<'a> Parser<'a> {
    pub fn new(reg: &'a WorldgenRegistries, biomes: &'a BiomeRegistry) -> Self {
        Self {
            reg,
            biomes,
            providers: Default::default(),
            features: Default::default(),
            placed: Default::default(),
        }
    }

    pub fn tag(&self, name: &str) -> Result<Arc<StateSet>> {
        self.reg.block_tag(&id(strip_hash(name).1)?)
    }

    pub fn block(&self, name: &str) -> Result<BlockId> {
        self.reg
            .blocks
            .block_by_name(&id(name)?)
            .with_context(|| format!("unknown block {name}"))
    }

    pub fn state(&self, json: &Value) -> Result<BlockState> {
        self.reg.blocks.parse_state_json(json)
    }

    /// `HolderSet<Block>` as a state set.
    pub fn block_set(&self, json: &Value) -> Result<Arc<StateSet>> {
        if let Some(s) = json.as_str() {
            let (is_tag, name) = strip_hash(s);
            if is_tag {
                return self.tag(name);
            }
        }
        let mut set = StateSet::empty(&self.reg.blocks);
        for b in self.block_list(json)? {
            set.insert_block(&self.reg.blocks, &self.reg.blocks.block(b).name);
        }
        Ok(Arc::new(set))
    }

    /// `HolderSet<Block>` in holder order.
    pub fn block_list(&self, json: &Value) -> Result<Vec<BlockId>> {
        let ids: Vec<Identifier> = match json {
            Value::String(s) => {
                let (is_tag, name) = strip_hash(s);
                if is_tag {
                    let blocks = &self.reg.blocks;
                    self.reg
                        .packs
                        .resolve_tag("block", &id(name)?, &|i| blocks.block_by_name(i).is_some())?
                } else {
                    vec![id(name)?]
                }
            }
            Value::Array(a) => a
                .iter()
                .map(|v| id(crate::codec::as_str(v)?))
                .collect::<Result<_>>()?,
            other => bail!("invalid block holder set {other}"),
        };
        ids.iter()
            .map(|i| {
                self.reg
                    .blocks
                    .block_by_name(i)
                    .with_context(|| format!("unknown block {i}"))
            })
            .collect()
    }

    pub fn fluid_set(&self, json: &Value) -> Result<Vec<Fluid>> {
        let ids: Vec<Identifier> = match json {
            Value::String(s) => {
                let (is_tag, name) = strip_hash(s);
                if is_tag {
                    self.reg
                        .packs
                        .resolve_tag("fluid", &id(name)?, &|i| Fluid::from_id(&i.to_string()).is_some())?
                } else {
                    vec![id(name)?]
                }
            }
            Value::Array(a) => a
                .iter()
                .map(|v| id(crate::codec::as_str(v)?))
                .collect::<Result<_>>()?,
            other => bail!("invalid fluid holder set {other}"),
        };
        ids.iter()
            .map(|i| Fluid::from_id(&i.to_string()).with_context(|| format!("unknown fluid {i}")))
            .collect()
    }

    pub fn biome_set(&self, json: &Value) -> Result<BiomeSet> {
        let biomes = self.biomes;
        let ids: Vec<Identifier> = match json {
            Value::String(s) => {
                let (is_tag, name) = strip_hash(s);
                if is_tag {
                    self.reg
                        .packs
                        .resolve_tag("worldgen/biome", &id(name)?, &|i| biomes.get(i).is_some())?
                } else {
                    vec![id(name)?]
                }
            }
            Value::Array(a) => a
                .iter()
                .map(|v| id(crate::codec::as_str(v)?))
                .collect::<Result<_>>()?,
            other => bail!("invalid biome holder set {other}"),
        };
        let mut set = vec![false; biomes.len()];
        for i in ids {
            set[biomes.require(&i)?.0 as usize] = true;
        }
        Ok(BiomeSet(set))
    }

    /// `Holder<BlockStateProvider>`.
    pub fn provider(&self, json: &Value) -> Result<Arc<StateProvider>> {
        if let Some(s) = json.as_str() {
            let id = id(s)?;
            if let Some(p) = self.providers.borrow().get(&id) {
                return Ok(p.clone());
            }
            let j = self
                .reg
                .packs
                .require_json("worldgen/block_state_provider", &id)?;
            let p = Arc::new(
                StateProvider::parse_direct(self, &j)
                    .with_context(|| format!("block state provider {id}"))?,
            );
            self.providers.borrow_mut().insert(id, p.clone());
            return Ok(p);
        }
        Ok(Arc::new(StateProvider::parse_direct(self, json)?))
    }

    /// `Holder<Feature>`.
    pub fn feature(&self, json: &Value) -> Result<Arc<Feature>> {
        if let Some(s) = json.as_str() {
            let id = id(s)?;
            if let Some(f) = self.features.borrow().get(&id) {
                return Ok(f.clone());
            }
            let j = self.reg.packs.require_json("worldgen/feature", &id)?;
            let f = Arc::new(Feature::parse(self, &j).with_context(|| format!("feature {id}"))?);
            self.features.borrow_mut().insert(id, f.clone());
            return Ok(f);
        }
        Ok(Arc::new(Feature::parse(self, json)?))
    }

    /// `Holder<PlacedFeature>`.
    pub fn placed(&self, json: &Value) -> Result<Arc<PlacedFeature>> {
        if let Some(s) = json.as_str() {
            let id = id(s)?;
            if let Some(f) = self.placed.borrow().get(&id) {
                return Ok(f.clone());
            }
            let j = self.reg.packs.require_json("worldgen/placed_feature", &id)?;
            let f = Arc::new(
                self.placed_direct(&j, Some(id.to_string()))
                    .with_context(|| format!("placed feature {id}"))?,
            );
            self.placed.borrow_mut().insert(id, f.clone());
            return Ok(f);
        }
        Ok(Arc::new(self.placed_direct(json, None)?))
    }

    fn placed_direct(&self, json: &Value, id: Option<String>) -> Result<PlacedFeature> {
        let feature = self.feature(field(json, "feature")?)?;
        let placement = match json.get("placement") {
            Some(v) => as_array(v)?
                .iter()
                .map(|m| Placement::parse(self, m))
                .collect::<Result<_>>()?,
            None => Vec::new(),
        };
        Ok(PlacedFeature {
            id,
            feature,
            placement,
        })
    }

    /// `HolderSet<PlacedFeature>` of one biome generation step.
    fn placed_set(&self, json: &Value) -> Result<Vec<Arc<PlacedFeature>>> {
        match json {
            Value::String(s) => {
                let (is_tag, name) = strip_hash(s);
                if is_tag {
                    let ids = self
                        .reg
                        .packs
                        .resolve_tag("worldgen/placed_feature", &id(name)?, &|_| true)?;
                    ids.iter()
                        .map(|i| self.placed(&Value::String(i.to_string())))
                        .collect()
                } else {
                    Ok(vec![self.placed(json)?])
                }
            }
            Value::Array(a) => a.iter().map(|v| self.placed(v)).collect(),
            other => bail!("invalid placed feature holder set {other}"),
        }
    }
}

/// `FeatureSorter.StepFeatureData`.
pub struct StepData {
    pub features: Vec<Arc<PlacedFeature>>,
    pub index: FxHashMap<*const PlacedFeature, usize>,
}

// Raw pointers are only used as identity keys.
unsafe impl Send for StepData {}
unsafe impl Sync for StepData {}

/// Features of a dimension: per-biome step lists and the global order.
pub struct FeatureRegistry {
    /// Biome -> step -> placed features (`BiomeGenerationSettings.features()`).
    pub biome_steps: Vec<Vec<Vec<Arc<PlacedFeature>>>>,
    /// `BiomeGenerationSettings.featureSet` per biome.
    biome_sets: Vec<FxHashSet<usize>>,
    pub steps: Vec<StepData>,
}

impl FeatureRegistry {
    /// Loads the features of every biome and sorts them for the biome source's possible biomes
    /// (in `possibleBiomes()` order).
    pub fn load(reg: &WorldgenRegistries, biomes: &BiomeRegistry, possible: &[BiomeId]) -> Result<Self> {
        let parser = Parser::new(reg, biomes);
        let mut biome_steps = Vec::with_capacity(biomes.len());
        let mut biome_sets = Vec::with_capacity(biomes.len());
        for i in 0..biomes.len() {
            let id = BiomeId(i as u16);
            let json = &biomes.data(id).json;
            let mut steps = Vec::new();
            if let Some(list) = json.get("features") {
                for step in as_array(list)? {
                    steps.push(
                        parser
                            .placed_set(step)
                            .with_context(|| format!("features of biome {}", biomes.name(id)))?,
                    );
                }
            }
            let set: FxHashSet<usize> = steps.iter().flatten().map(|f| Arc::as_ptr(f) as usize).collect();
            biome_steps.push(steps);
            biome_sets.push(set);
        }
        let sources: Vec<&Vec<Vec<Arc<PlacedFeature>>>> =
            possible.iter().map(|b| &biome_steps[b.0 as usize]).collect();
        let steps = sort_features(&sources)?;
        Ok(Self {
            biome_steps,
            biome_sets,
            steps,
        })
    }

    /// `BiomeGenerationSettings.hasFeature`.
    pub fn biome_has_feature(&self, biome: BiomeId, feature: &PlacedFeature) -> bool {
        self.biome_sets[biome.0 as usize].contains(&(feature as *const PlacedFeature as usize))
    }
}

/// `FeatureSorter.buildFeaturesPerStep` (without the cycle diagnostics).
fn sort_features(sources: &[&Vec<Vec<Arc<PlacedFeature>>>]) -> Result<Vec<StepData>> {
    type Node = (usize, usize);
    let mut feature_index: FxHashMap<usize, usize> = FxHashMap::default();
    let mut node_feature: FxHashMap<Node, Arc<PlacedFeature>> = FxHashMap::default();
    let mut edges: BTreeMap<Node, BTreeSet<Node>> = BTreeMap::new();
    let mut max_step = 0;
    for source in sources {
        max_step = max_step.max(source.len());
        let mut list: Vec<Node> = Vec::new();
        for (step, features) in source.iter().enumerate() {
            for f in features {
                let next = feature_index.len();
                let index = *feature_index.entry(Arc::as_ptr(f) as usize).or_insert(next);
                node_feature.entry((step, index)).or_insert_with(|| f.clone());
                list.push((step, index));
            }
        }
        for i in 0..list.len() {
            let e = edges.entry(list[i]).or_default();
            if i + 1 < list.len() {
                e.insert(list[i + 1]);
            }
        }
    }
    fn dfs(
        edges: &BTreeMap<Node, BTreeSet<Node>>,
        discovered: &mut BTreeSet<Node>,
        visiting: &mut BTreeSet<Node>,
        out: &mut Vec<Node>,
        current: Node,
    ) -> bool {
        if discovered.contains(&current) {
            return false;
        }
        if visiting.contains(&current) {
            return true;
        }
        visiting.insert(current);
        if let Some(next) = edges.get(&current) {
            for &n in next {
                if dfs(edges, discovered, visiting, out, n) {
                    return true;
                }
            }
        }
        visiting.remove(&current);
        discovered.insert(current);
        out.push(current);
        false
    }
    let mut discovered = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    let mut sorted = Vec::new();
    for &node in edges.keys() {
        if !discovered.contains(&node) && dfs(&edges, &mut discovered, &mut visiting, &mut sorted, node) {
            bail!("feature order cycle found");
        }
    }
    sorted.reverse();
    let mut steps = Vec::with_capacity(max_step);
    for step in 0..max_step {
        let features: Vec<Arc<PlacedFeature>> = sorted
            .iter()
            .filter(|n| n.0 == step)
            .map(|n| node_feature[n].clone())
            .collect();
        let index = features
            .iter()
            .enumerate()
            .map(|(i, f)| (Arc::as_ptr(f), i))
            .collect();
        steps.push(StepData { features, index });
    }
    Ok(steps)
}
