//! Bit sets over block states, used for block tags and block predicates on hot paths.

use anyhow::Result;
use chunker_data::Identifier;
use chunker_data::blocks::{BlockRegistry, BlockState};

use crate::registry::WorldgenRegistries;

/// Set of block states (all states of a block are members when the block is).
#[derive(Clone, Debug, Default)]
pub struct StateSet {
    bits: Vec<u64>,
}

impl StateSet {
    pub fn empty(reg: &BlockRegistry) -> Self {
        Self {
            bits: vec![0; reg.state_count().div_ceil(64)],
        }
    }

    #[inline(always)]
    pub fn contains(&self, state: BlockState) -> bool {
        let i = state.0 as usize;
        self.bits.get(i >> 6).is_some_and(|w| w & (1 << (i & 63)) != 0)
    }

    pub fn insert_block(&mut self, reg: &BlockRegistry, block: &Identifier) {
        let Some(b) = reg.block_by_name(block) else { return };
        let info = reg.block(b);
        for s in info.first_state..info.first_state + info.state_count {
            self.bits[(s as usize) >> 6] |= 1 << (s as usize & 63);
        }
    }
}

impl WorldgenRegistries {
    /// Resolves `#ns:tag` of `tags/block` into a state set (cached).
    pub fn block_tag(&self, tag: &Identifier) -> Result<std::sync::Arc<StateSet>> {
        if let Some(s) = self.block_tags.lock().get(tag) {
            return Ok(s.clone());
        }
        let blocks = &self.blocks;
        let members = self
            .packs
            .resolve_tag("block", tag, &|id| blocks.block_by_name(id).is_some())?;
        let mut set = StateSet::empty(blocks);
        for m in &members {
            set.insert_block(blocks, m);
        }
        let set = std::sync::Arc::new(set);
        self.block_tags.lock().insert(tag.clone(), set.clone());
        Ok(set)
    }
}
