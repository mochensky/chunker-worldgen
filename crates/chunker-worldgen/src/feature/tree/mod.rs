//! `TreeFeature` with its trunk/foliage/root placers and decorators.

mod decorator;
mod fallen;
mod foliage;
mod root;
mod trunk;

use std::sync::Arc;

use anyhow::{Result, bail};
use chunker_data::blocks::BlockState;
use serde_json::Value;

pub use decorator::Decorator;
pub use fallen::FallenTree;
pub use foliage::{Attachment, FoliagePlacer};
pub use root::RootPlacer;
pub use trunk::TrunkPlacer;

use super::Region;
use super::provider::StateProvider;
use super::registry::Parser;
use crate::codec::{as_array, field, opt_bool, opt_i32, type_name};
use crate::java::JavaHashSet;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, BoundingBox, Direction};

/// `setBlockKnownShape` flags (`UPDATE_ALL | UPDATE_KNOWN_SHAPE`).
const TREE_FLAGS: u32 = 19;

/// `FeatureSize`.
pub enum FeatureSize {
    TwoLayers {
        limit: i32,
        lower: i32,
        upper: i32,
        min_clipped: Option<i32>,
    },
    ThreeLayers {
        limit: i32,
        upper_limit: i32,
        lower: i32,
        middle: i32,
        upper: i32,
        min_clipped: Option<i32>,
    },
}

impl FeatureSize {
    fn parse(json: &Value) -> Result<Self> {
        let min_clipped = json
            .get("min_clipped_height")
            .map(crate::codec::as_i32)
            .transpose()?;
        Ok(match type_name(json)?.as_str() {
            "two_layers_feature_size" => FeatureSize::TwoLayers {
                limit: opt_i32(json, "limit", 1)?,
                lower: opt_i32(json, "lower_size", 0)?,
                upper: opt_i32(json, "upper_size", 1)?,
                min_clipped,
            },
            "three_layers_feature_size" => FeatureSize::ThreeLayers {
                limit: opt_i32(json, "limit", 1)?,
                upper_limit: opt_i32(json, "upper_limit", 1)?,
                lower: opt_i32(json, "lower_size", 0)?,
                middle: opt_i32(json, "middle_size", 1)?,
                upper: opt_i32(json, "upper_size", 1)?,
                min_clipped,
            },
            other => bail!("unknown feature size {other}"),
        })
    }

    fn size_at(&self, tree_height: i32, yo: i32) -> i32 {
        match *self {
            FeatureSize::TwoLayers {
                limit, lower, upper, ..
            } => {
                if yo < limit {
                    lower
                } else {
                    upper
                }
            }
            FeatureSize::ThreeLayers {
                limit,
                upper_limit,
                lower,
                middle,
                upper,
                ..
            } => {
                if yo < limit {
                    lower
                } else if yo >= tree_height - upper_limit {
                    upper
                } else {
                    middle
                }
            }
        }
    }

    fn min_clipped(&self) -> Option<i32> {
        match *self {
            FeatureSize::TwoLayers { min_clipped, .. } | FeatureSize::ThreeLayers { min_clipped, .. } => {
                min_clipped
            }
        }
    }
}

pub struct Tree {
    pub trunk_provider: Arc<StateProvider>,
    pub trunk: TrunkPlacer,
    pub foliage_provider: Arc<StateProvider>,
    pub foliage: FoliagePlacer,
    pub root: Option<RootPlacer>,
    pub size: FeatureSize,
    pub decorators: Vec<Decorator>,
    pub ignore_vines: bool,
    pub below_trunk_provider: Arc<StateProvider>,
}

/// Positions placed by one tree (`TreeFeature.place` sets, `HashSet` iteration order).
#[derive(Default)]
pub struct Placed {
    pub roots: JavaHashSet<BlockPos>,
    pub trunks: JavaHashSet<BlockPos>,
    pub foliage: JavaHashSet<BlockPos>,
    pub decorations: JavaHashSet<BlockPos>,
}

/// State shared by the placers of one tree.
pub struct TreeCtx<'r, 'a> {
    pub region: &'r mut Region<'a>,
    pub tree: &'r Tree,
    pub placed: Placed,
}

impl TreeCtx<'_, '_> {
    /// `TreeFeature.validTreePos`.
    pub fn valid_tree_pos(&self, p: BlockPos) -> bool {
        let s = self.region.get(p);
        self.region.env.blocks.flags(s).is_air() || self.region.env.t.replaceable_by_trees.contains(s)
    }

    pub fn is_air_or_leaves(&self, p: BlockPos) -> bool {
        let s = self.region.get(p);
        self.region.env.blocks.flags(s).is_air() || self.region.env.t.leaves.contains(s)
    }

    pub fn set_trunk(&mut self, p: BlockPos, state: BlockState) {
        self.placed.trunks.insert(p);
        self.region.set(p, state, TREE_FLAGS);
    }

    pub fn set_root(&mut self, p: BlockPos, state: BlockState) {
        self.placed.roots.insert(p);
        self.region.set(p, state, TREE_FLAGS);
    }

    pub fn set_foliage(&mut self, p: BlockPos, state: BlockState) {
        self.placed.foliage.insert(p);
        self.region.set(p, state, TREE_FLAGS);
    }

    pub fn set_decoration(&mut self, p: BlockPos, state: BlockState) {
        self.placed.decorations.insert(p);
        self.region.set(p, state, TREE_FLAGS);
    }

    /// `trySetValue(AXIS, axis)`.
    pub fn with_axis(&self, state: BlockState, axis: crate::structure::geom::Axis) -> BlockState {
        self.region
            .env
            .blocks
            .with_property(state, "axis", axis.name())
            .unwrap_or(state)
    }
}

impl Tree {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        let decorators = match json.get("decorators") {
            Some(v) => as_array(v)?
                .iter()
                .map(|d| Decorator::parse(p, d))
                .collect::<Result<_>>()?,
            None => Vec::new(),
        };
        Ok(Self {
            trunk_provider: p.provider(field(json, "trunk_provider")?)?,
            trunk: TrunkPlacer::parse(p, field(json, "trunk_placer")?)?,
            foliage_provider: p.provider(field(json, "foliage_provider")?)?,
            foliage: FoliagePlacer::parse(field(json, "foliage_placer")?)?,
            root: json
                .get("root_placer")
                .map(|r| RootPlacer::parse(p, r))
                .transpose()?,
            size: FeatureSize::parse(field(json, "minimum_size")?)?,
            decorators,
            ignore_vines: opt_bool(json, "ignore_vines", false)?,
            below_trunk_provider: p.provider(field(json, "below_trunk_provider")?)?,
        })
    }

    /// `TreeFeature.place`.
    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let mut ctx = TreeCtx {
            region,
            tree: self,
            placed: Placed::default(),
        };
        let result = self.do_place(&mut ctx, random, origin);
        if !result || (ctx.placed.trunks.is_empty() && ctx.placed.foliage.is_empty()) {
            return false;
        }
        if !self.decorators.is_empty() {
            let mut d = decorator::Context::new(&mut ctx);
            for dec in &self.decorators {
                dec.place(&mut d, random);
            }
        }
        let placed = &ctx.placed;
        let all: Vec<BlockPos> = placed
            .roots
            .iter()
            .chain(placed.trunks.iter())
            .chain(placed.foliage.iter())
            .chain(placed.decorations.iter())
            .collect();
        let Some(bounds) = BoundingBox::encapsulating_positions(&all) else {
            return false;
        };
        let shape = update_leaves(&mut ctx, &bounds);
        update_shape_at_edge(ctx.region, &shape, bounds.min_x, bounds.min_y, bounds.min_z);
        true
    }

    fn do_place(&self, ctx: &mut TreeCtx, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let tree_height = self.trunk.tree_height(random);
        let foliage_height = self.foliage.foliage_height(random, tree_height);
        let trunk_height = tree_height - foliage_height;
        let leaf_radius = self.foliage.foliage_radius(random, trunk_height);
        let trunk_origin = match &self.root {
            Some(r) => r.trunk_origin(origin, random),
            None => origin,
        };
        let min_y = origin.y.min(trunk_origin.y);
        let max_y = origin.y.max(trunk_origin.y) + tree_height + 1;
        if min_y < ctx.region.min_y() + 1 || max_y > ctx.region.max_y() + 1 {
            return false;
        }
        let clipped = self.max_free_tree_height(ctx, tree_height, trunk_origin);
        if !(clipped >= tree_height || self.size.min_clipped().is_some_and(|m| clipped >= m)) {
            return false;
        }
        if let Some(r) = &self.root
            && !r.place_roots(ctx, random, origin, trunk_origin)
        {
            return false;
        }
        let attachments = self.trunk.place_trunk(ctx, random, clipped, trunk_origin);
        for a in &attachments {
            self.foliage
                .create_foliage(ctx, random, clipped, a, foliage_height, leaf_radius);
        }
        true
    }

    /// `getMaxFreeTreeHeight`.
    fn max_free_tree_height(&self, ctx: &TreeCtx, max_tree_height: i32, pos: BlockPos) -> i32 {
        let vine = ctx.region.env.b.vine;
        for y in 0..=max_tree_height + 1 {
            let r = self.size.size_at(max_tree_height, y);
            for x in -r..=r {
                for z in -r..=r {
                    let p = pos.offset(x, y, z);
                    if !self.trunk.is_free(ctx, p) || (!self.ignore_vines && ctx.region.is(p, vine)) {
                        return y - 2;
                    }
                }
            }
        }
        max_tree_height
    }
}

/// `BitSetDiscreteVoxelShape` over a bounding box.
pub struct Shape {
    pub sx: i32,
    pub sy: i32,
    pub sz: i32,
    bits: Vec<bool>,
}

impl Shape {
    pub fn new(sx: i32, sy: i32, sz: i32) -> Self {
        Self {
            sx,
            sy,
            sz,
            bits: vec![false; (sx * sy * sz) as usize],
        }
    }

    #[inline]
    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        ((x * self.sy + y) * self.sz + z) as usize
    }

    pub fn fill(&mut self, x: i32, y: i32, z: i32) {
        let i = self.index(x, y, z);
        self.bits[i] = true;
    }

    pub fn is_full(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0
            && y >= 0
            && z >= 0
            && x < self.sx
            && y < self.sy
            && z < self.sz
            && self.bits[self.index(x, y, z)]
    }
}

/// `TreeFeature.updateLeaves`.
fn update_leaves(ctx: &mut TreeCtx, bounds: &BoundingBox) -> Shape {
    let mut shape = Shape::new(bounds.x_span(), bounds.y_span(), bounds.z_span());
    let env = ctx.region.env;
    for p in ctx.placed.decorations.iter().chain(ctx.placed.roots.iter()) {
        if bounds.contains(p) {
            shape.fill(p.x - bounds.min_x, p.y - bounds.min_y, p.z - bounds.min_z);
        }
    }
    let mut to_check: Vec<JavaHashSet<BlockPos>> = (0..7).map(|_| JavaHashSet::new()).collect();
    for p in ctx.placed.trunks.iter() {
        to_check[0].insert(p);
    }
    let mut smallest = 0usize;
    loop {
        while smallest >= 7 || !to_check[smallest].is_empty() {
            if smallest >= 7 {
                return shape;
            }
            let pos = to_check[smallest].pop_first().unwrap();
            if !bounds.contains(pos) {
                continue;
            }
            if smallest != 0 {
                let state = ctx.region.get(pos);
                let s = env
                    .blocks
                    .with_property(state, "distance", &smallest.to_string())
                    .unwrap_or(state);
                ctx.region.set(pos, s, TREE_FLAGS);
            }
            shape.fill(pos.x - bounds.min_x, pos.y - bounds.min_y, pos.z - bounds.min_z);
            for d in Direction::ALL {
                let n = pos.relative(d, 1);
                if !bounds.contains(n) {
                    continue;
                }
                if shape.is_full(n.x - bounds.min_x, n.y - bounds.min_y, n.z - bounds.min_z) {
                    continue;
                }
                let cs = ctx.region.get(n);
                if let Some(distance) = super::behaviour::leaf_distance(env, cs) {
                    let nd = (distance as usize).min(smallest + 1);
                    if nd < 7 {
                        to_check[nd].insert(n);
                        smallest = smallest.min(nd);
                    }
                }
            }
        }
        smallest += 1;
    }
}

/// `StructureTemplate.updateShapeAtEdge(level, 3, shape, x, y, z)`: faces in `forAllFaces` order
/// (z faces, then y faces, then x faces).
pub fn update_shape_at_edge(region: &mut Region, shape: &Shape, sx: i32, sy: i32, sz: i32) {
    update_shape_at_edge_with(region, shape, sx, sy, sz, 2);
}

/// `StructureTemplate.updateShapeAtEdge` with `flags` = `updateMode & ~UPDATE_NEIGHBORS`.
pub fn update_shape_at_edge_with(region: &mut Region, shape: &Shape, sx: i32, sy: i32, sz: i32, flags: u32) {
    let mut faces: Vec<(Direction, i32, i32, i32)> = Vec::new();
    // NONE: c = z.
    for a in 0..shape.sx {
        for b in 0..shape.sy {
            let mut last = false;
            for c in 0..=shape.sz {
                let full = c != shape.sz && shape.is_full(a, b, c);
                if !last && full {
                    faces.push((Direction::North, a, b, c));
                }
                if last && !full {
                    faces.push((Direction::South, a, b, c - 1));
                }
                last = full;
            }
        }
    }
    // FORWARD: a = z, b = x, c = y.
    for a in 0..shape.sz {
        for b in 0..shape.sx {
            let mut last = false;
            for c in 0..=shape.sy {
                let full = c != shape.sy && shape.is_full(b, c, a);
                if !last && full {
                    faces.push((Direction::Down, b, c, a));
                }
                if last && !full {
                    faces.push((Direction::Up, b, c - 1, a));
                }
                last = full;
            }
        }
    }
    // BACKWARD: a = y, b = z, c = x.
    for a in 0..shape.sy {
        for b in 0..shape.sz {
            let mut last = false;
            for c in 0..=shape.sx {
                let full = c != shape.sx && shape.is_full(c, a, b);
                if !last && full {
                    faces.push((Direction::West, c, a, b));
                }
                if last && !full {
                    faces.push((Direction::East, c - 1, a, b));
                }
                last = full;
            }
        }
    }
    for (d, x, y, z) in faces {
        let pos = BlockPos::new(sx + x, sy + y, sz + z);
        let n = pos.relative(d, 1);
        let state = region.get(pos);
        let neighbour = region.get(n);
        let new_state = super::behaviour::update_shape(region, state, pos, d, n, neighbour);
        if state != new_state {
            region.set(pos, new_state, flags);
        }
        let new_neighbour =
            super::behaviour::update_shape(region, neighbour, n, d.opposite(), pos, new_state);
        if neighbour != new_neighbour {
            region.set(n, new_neighbour, flags);
        }
    }
}
