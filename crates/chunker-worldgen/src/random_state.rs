//! `RandomState`: seed-derived noises, positional randoms and the density function compiler.

use std::sync::Arc;

use chunker_data::Identifier;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::density::compiler::{CompileEnv, DensityCompiler};
use crate::density::sampler::SamplerRef;
use crate::density::{Df, NoiseHolder};
use crate::noise::NoiseStack;
use crate::random::{AnyRandom, LegacyRandom, PositionalRandomFactory, RandomAlgorithm, RandomSource};
use crate::registry::WorldgenRegistries;

pub struct RandomState {
    pub seed: i64,
    pub legacy_random_source: bool,
    pub random: PositionalRandomFactory,
    pub registries: Arc<WorldgenRegistries>,
    noise_instances: Mutex<FxHashMap<Identifier, Arc<NoiseStack>>>,
    positional_randoms: Mutex<FxHashMap<String, PositionalRandomFactory>>,
    compiler: Mutex<DensityCompiler>,
}

impl RandomState {
    pub fn new(registries: Arc<WorldgenRegistries>, seed: i64, legacy_random_source: bool) -> Self {
        let algorithm = if legacy_random_source {
            RandomAlgorithm::Legacy
        } else {
            RandomAlgorithm::Xoroshiro
        };
        let random = algorithm.new_instance(seed).fork_positional();
        Self {
            seed,
            legacy_random_source,
            random,
            registries,
            noise_instances: Mutex::default(),
            positional_randoms: Mutex::default(),
            compiler: Mutex::new(DensityCompiler::new()),
        }
    }

    /// `getOrCreateNoise(key)`.
    pub fn noise(&self, id: &Identifier) -> anyhow::Result<Arc<NoiseStack>> {
        if let Some(n) = self.noise_instances.lock().get(id) {
            return Ok(n.clone());
        }
        let params = self.registries.noise(id)?;
        let stack = Arc::new(params.create(&mut self.random.from_hash_of(id.as_str())));
        Ok(self
            .noise_instances
            .lock()
            .entry(id.clone())
            .or_insert(stack)
            .clone())
    }

    /// `getOrCreateRandomFactory(name)`.
    pub fn random_factory(&self, name: &str) -> PositionalRandomFactory {
        *self
            .positional_randoms
            .lock()
            .entry(name.to_owned())
            .or_insert_with(|| self.random.from_hash_of(name).fork_positional())
    }

    /// `getSampler(function)`.
    pub fn sampler(&self, function: &Df) -> SamplerRef {
        let mut compiler = self.compiler.lock();
        compiler.sampler(self, function)
    }

    pub fn cache_count(&self) -> usize {
        self.compiler.lock().cache_count()
    }
}

impl CompileEnv for RandomState {
    fn noise(&self, holder: &NoiseHolder) -> Arc<NoiseStack> {
        match holder.id.as_str() {
            "minecraft:nether/temperature" => Arc::new(
                holder
                    .noise
                    .create_for_legacy_nether_biome(&mut LegacyRandom::new(self.seed)),
            ),
            "minecraft:nether/vegetation" => Arc::new(
                holder
                    .noise
                    .create_for_legacy_nether_biome(&mut LegacyRandom::new(self.seed.wrapping_add(1))),
            ),
            _ => RandomState::noise(self, &holder.id).expect("noise was resolved while parsing"),
        }
    }

    fn random(&self, id: &str) -> AnyRandom {
        if self.legacy_random_source && id == "minecraft:terrain" {
            AnyRandom::Legacy(LegacyRandom::new(self.seed))
        } else {
            self.random.from_hash_of(id)
        }
    }

    fn end_island_random(&self) -> LegacyRandom {
        LegacyRandom::new(self.seed)
    }
}
