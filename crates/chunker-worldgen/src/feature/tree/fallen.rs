//! `FallenTreeFeature`.

use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use super::decorator::{Context, Decorator};
use crate::codec::{as_array, field};
use crate::feature::Region;
use crate::feature::kinds::mark_above_for_post_processing;
use crate::feature::provider::StateProvider;
use crate::feature::registry::Parser;
use crate::java::JavaHashSet;
use crate::providers::IntProvider;
use crate::random::RandomSource;
use crate::structure::geom::{Axis, BlockPos, Direction};

pub struct FallenTree {
    trunk_provider: Arc<StateProvider>,
    log_length: IntProvider,
    stump_decorators: Vec<Decorator>,
    log_decorators: Vec<Decorator>,
}

impl FallenTree {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let decorators = |name: &str| -> Result<Vec<Decorator>> {
            as_array(field(json, name)?)?
                .iter()
                .map(|d| Decorator::parse(p, d))
                .collect()
        };
        Ok(Self {
            trunk_provider: p.provider(field(json, "trunk_provider")?)?,
            log_length: IntProvider::parse(field(json, "log_length")?)?,
            stump_decorators: decorators("stump_decorators")?,
            log_decorators: decorators("log_decorators")?,
        })
    }

    pub fn is_implemented(&self) -> bool {
        self.stump_decorators
            .iter()
            .chain(self.log_decorators.iter())
            .all(|d| d.is_implemented())
    }

    fn valid_tree_pos(region: &Region, p: BlockPos) -> bool {
        let s = region.get(p);
        region.env.blocks.flags(s).is_air() || region.env.t.replaceable_by_trees.contains(s)
    }

    fn over_solid_ground(region: &Region, p: BlockPos) -> bool {
        crate::feature::behaviour::is_face_sturdy(region.env, region.get(p.below()), Direction::Up)
    }

    fn place_log(
        &self,
        region: &mut Region,
        random: &mut dyn RandomSource,
        p: BlockPos,
        axis: Option<Axis>,
    ) -> BlockPos {
        let mut s = self.trunk_provider.get(region, random, p);
        if let Some(a) = axis {
            s = region.env.blocks.with_property(s, "axis", a.name()).unwrap_or(s);
        }
        region.set_and_update(p, s);
        mark_above_for_post_processing(region, p);
        p
    }

    fn decorate(
        region: &mut Region,
        random: &mut dyn RandomSource,
        logs: Vec<BlockPos>,
        decorators: &[Decorator],
    ) {
        if decorators.is_empty() {
            return;
        }
        let mut c = Context::from_lists(region, None, logs, Vec::new(), Vec::new());
        for d in decorators {
            d.place(&mut c, random);
        }
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let stump = self.place_log(region, random, origin, None);
        Self::decorate(region, random, vec![stump], &self.stump_decorators);
        let dir = Direction::random_horizontal(random);
        let length = self.log_length.sample(random) - 2;
        let mut start = origin.relative(dir, 2 + random.next_int_bounded(2));
        // `setGroundHeightForFallenLogStartPos`
        start = start.above();
        for _ in 0..6 {
            if Self::valid_tree_pos(region, start) && Self::over_solid_ground(region, start) {
                break;
            }
            start = start.below();
        }
        // `canPlaceEntireFallenLog`
        let mut gap = 0;
        let mut p = start;
        for _ in 0..length {
            if !Self::valid_tree_pos(region, p) {
                return true;
            }
            if !Self::over_solid_ground(region, p) {
                gap += 1;
                if gap > 2 {
                    return true;
                }
            } else {
                gap = 0;
            }
            p = p.relative(dir, 1);
        }
        let mut logs = JavaHashSet::new();
        let mut p = start;
        for _ in 0..length {
            logs.insert(self.place_log(region, random, p, Some(dir.axis())));
            p = p.relative(dir, 1);
        }
        Self::decorate(region, random, logs.to_vec(), &self.log_decorators);
        true
    }
}
