//! `RootPlacer` (mangrove roots).

use std::sync::Arc;

use anyhow::{Result, bail};
use chunker_data::blocks::BlockState;
use serde_json::Value;

use super::TreeCtx;
use crate::block_set::StateSet;
use crate::codec::{f32_field, field, i32_field, type_name};
use crate::feature::provider::StateProvider;
use crate::feature::registry::Parser;
use crate::providers::IntProvider;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

pub struct RootPlacer {
    trunk_offset_y: IntProvider,
    root_provider: Arc<StateProvider>,
    above: Option<(Arc<StateProvider>, f32)>,
    can_grow_through: Arc<StateSet>,
    muddy_roots_in: Arc<StateSet>,
    muddy_roots_provider: Arc<StateProvider>,
    max_root_width: i32,
    max_root_length: i32,
    random_skew_chance: f32,
}

impl RootPlacer {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        if type_name(json)? != "mangrove_root_placer" {
            bail!("unknown root placer {json}");
        }
        let m = field(json, "mangrove_root_placement")?;
        let above = match json.get("above_root_placement") {
            Some(a) => Some((
                p.provider(field(a, "above_root_provider")?)?,
                f32_field(a, "above_root_placement_chance")?,
            )),
            None => None,
        };
        Ok(Self {
            trunk_offset_y: IntProvider::parse(field(json, "trunk_offset_y")?)?,
            root_provider: p.provider(field(json, "root_provider")?)?,
            above,
            can_grow_through: p.block_set(field(m, "can_grow_through")?)?,
            muddy_roots_in: p.block_set(field(m, "muddy_roots_in")?)?,
            muddy_roots_provider: p.provider(field(m, "muddy_roots_provider")?)?,
            max_root_width: i32_field(m, "max_root_width")?,
            max_root_length: i32_field(m, "max_root_length")?,
            random_skew_chance: f32_field(m, "random_skew_chance")?,
        })
    }

    pub fn trunk_origin(&self, origin: BlockPos, random: &mut dyn RandomSource) -> BlockPos {
        origin.above_n(self.trunk_offset_y.sample(random))
    }

    fn can_place_root(&self, ctx: &TreeCtx, p: BlockPos) -> bool {
        ctx.valid_tree_pos(p) || self.can_grow_through.contains(ctx.region.get(p))
    }

    fn waterlogged(ctx: &TreeCtx, p: BlockPos, s: BlockState) -> BlockState {
        let blocks = &ctx.region.env.blocks;
        if blocks.has_property(s, "waterlogged") {
            let w = ctx.region.fluid(p).is_water();
            blocks
                .with_property(s, "waterlogged", if w { "true" } else { "false" })
                .unwrap_or(s)
        } else {
            s
        }
    }

    fn place_root(&self, ctx: &mut TreeCtx, random: &mut dyn RandomSource, p: BlockPos) {
        if self.muddy_roots_in.contains(ctx.region.get(p)) {
            let s = self.muddy_roots_provider.get(ctx.region, random, p);
            let s = Self::waterlogged(ctx, p, s);
            ctx.set_root(p, s);
            return;
        }
        if self.can_place_root(ctx, p) {
            let s = self.root_provider.get(ctx.region, random, p);
            let s = Self::waterlogged(ctx, p, s);
            ctx.set_root(p, s);
            if let Some((provider, chance)) = &self.above {
                let above = p.above();
                if random.next_float() < *chance && ctx.region.flags(above).is_air() {
                    let s = provider.get(ctx.region, random, above);
                    let s = Self::waterlogged(ctx, above, s);
                    ctx.set_root(above, s);
                }
            }
        }
    }

    /// `MangroveRootPlacer.placeRoots`.
    pub fn place_roots(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        trunk_origin: BlockPos,
    ) -> bool {
        let mut column = origin;
        while column.y < trunk_origin.y {
            if !self.can_place_root(ctx, column) {
                return false;
            }
            column = column.above();
        }
        let mut positions = vec![trunk_origin.below()];
        for dir in Direction::HORIZONTAL {
            let pos = trunk_origin.relative(dir, 1);
            let mut in_dir = Vec::new();
            if !self.simulate(ctx, random, pos, dir, trunk_origin, &mut in_dir, 0) {
                return false;
            }
            positions.extend(in_dir);
            positions.push(trunk_origin.relative(dir, 1));
        }
        for p in positions {
            self.place_root(ctx, random, p);
        }
        true
    }

    fn simulate(
        &self,
        ctx: &TreeCtx,
        random: &mut dyn RandomSource,
        root: BlockPos,
        dir: Direction,
        origin: BlockPos,
        out: &mut Vec<BlockPos>,
        layer: i32,
    ) -> bool {
        if layer == self.max_root_length || out.len() as i32 > self.max_root_length {
            return false;
        }
        for p in self.potential(root, dir, random, origin) {
            if self.can_place_root(ctx, p) {
                out.push(p);
                if !self.simulate(ctx, random, p, dir, origin, out, layer + 1) {
                    return false;
                }
            }
        }
        true
    }

    fn potential(
        &self,
        pos: BlockPos,
        dir: Direction,
        random: &mut dyn RandomSource,
        origin: BlockPos,
    ) -> Vec<BlockPos> {
        let below = pos.below();
        let next = pos.relative(dir, 1);
        let width = pos.dist_manhattan(origin);
        let max = self.max_root_width;
        if width > max - 3 && width <= max {
            if random.next_float() < self.random_skew_chance {
                vec![below, next.below()]
            } else {
                vec![below]
            }
        } else if width > max || random.next_float() < self.random_skew_chance {
            vec![below]
        } else if random.next_bool() {
            vec![next]
        } else {
            vec![below]
        }
    }
}
