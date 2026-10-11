//! `FoliagePlacer`s.

use anyhow::{Result, bail};
use chunker_data::blocks::Fluid;
use serde_json::Value;

use super::TreeCtx;
use crate::codec::{f32_field, field, i32_field, type_name};
use crate::math;
use crate::providers::IntProvider;
use crate::random::RandomSource;
use crate::structure::geom::{Axis, BlockPos, Direction};

/// `FoliagePlacer.FoliageAttachment`.
#[derive(Clone, Copy, Debug)]
pub struct Attachment {
    pub pos: BlockPos,
    pub radius_offset: i32,
    pub height_offset: i32,
    pub double_trunk: bool,
}

impl Attachment {
    pub fn new(pos: BlockPos, radius_offset: i32, double_trunk: bool) -> Self {
        Self {
            pos,
            radius_offset,
            height_offset: 0,
            double_trunk,
        }
    }
}

pub enum Kind {
    Blob {
        height: i32,
    },
    Fancy {
        height: i32,
    },
    Bush {
        height: i32,
    },
    Spruce {
        trunk_height: IntProvider,
    },
    Pine {
        height: IntProvider,
    },
    Acacia,
    DarkOak,
    MegaPine {
        crown_height: IntProvider,
    },
    MegaJungle {
        height: i32,
    },
    RandomSpread {
        foliage_height: IntProvider,
        attempts: i32,
    },
    Cherry {
        height: IntProvider,
        wide_bottom_hole: f32,
        corner_hole: f32,
        hanging: f32,
        hanging_extension: f32,
    },
    Poplar {
        height: IntProvider,
        side_hole: f32,
    },
}

pub struct FoliagePlacer {
    radius: IntProvider,
    offset: IntProvider,
    kind: Kind,
}

impl FoliagePlacer {
    pub fn parse(json: &Value) -> Result<Self> {
        let int = |n: &str| IntProvider::parse(field(json, n)?);
        let kind = match type_name(json)?.as_str() {
            "blob_foliage_placer" => Kind::Blob {
                height: i32_field(json, "height")?,
            },
            "fancy_foliage_placer" => Kind::Fancy {
                height: i32_field(json, "height")?,
            },
            "bush_foliage_placer" => Kind::Bush {
                height: i32_field(json, "height")?,
            },
            "spruce_foliage_placer" => Kind::Spruce {
                trunk_height: int("trunk_height")?,
            },
            "pine_foliage_placer" => Kind::Pine {
                height: int("height")?,
            },
            "acacia_foliage_placer" => Kind::Acacia,
            "dark_oak_foliage_placer" => Kind::DarkOak,
            "mega_pine_foliage_placer" => Kind::MegaPine {
                crown_height: int("crown_height")?,
            },
            "jungle_foliage_placer" => Kind::MegaJungle {
                height: i32_field(json, "height")?,
            },
            "random_spread_foliage_placer" => Kind::RandomSpread {
                foliage_height: int("foliage_height")?,
                attempts: i32_field(json, "leaf_placement_attempts")?,
            },
            "cherry_foliage_placer" => Kind::Cherry {
                height: int("height")?,
                wide_bottom_hole: f32_field(json, "wide_bottom_layer_hole_chance")?,
                corner_hole: f32_field(json, "corner_hole_chance")?,
                hanging: f32_field(json, "hanging_leaves_chance")?,
                hanging_extension: f32_field(json, "hanging_leaves_extension_chance")?,
            },
            "poplar_foliage_placer" => Kind::Poplar {
                height: int("height")?,
                side_hole: f32_field(json, "side_hole_chance")?,
            },
            other => bail!("unknown foliage placer {other}"),
        };
        Ok(Self {
            radius: int("radius")?,
            offset: int("offset")?,
            kind,
        })
    }

    /// `foliageHeight(random, treeHeight, tree)`.
    pub fn foliage_height(&self, random: &mut dyn RandomSource, tree_height: i32) -> i32 {
        match &self.kind {
            Kind::Blob { height }
            | Kind::Fancy { height }
            | Kind::Bush { height }
            | Kind::MegaJungle { height } => *height,
            Kind::Spruce { trunk_height } => 4.max(tree_height - trunk_height.sample(random)),
            Kind::Pine { height } | Kind::Cherry { height, .. } | Kind::Poplar { height, .. } => {
                height.sample(random)
            }
            Kind::Acacia => 0,
            Kind::DarkOak => 4,
            Kind::MegaPine { crown_height } => crown_height.sample(random),
            Kind::RandomSpread { foliage_height, .. } => foliage_height.sample(random),
        }
    }

    /// `foliageRadius(random, trunkHeight)`.
    pub fn foliage_radius(&self, random: &mut dyn RandomSource, trunk_height: i32) -> i32 {
        let r = self.radius.sample(random);
        match self.kind {
            Kind::Pine { .. } => r + random.next_int_bounded((trunk_height + 1).max(1)),
            _ => r,
        }
    }

    /// `shouldSkipLocation` (unsigned distances).
    fn skip(
        &self,
        random: &mut dyn RandomSource,
        dx: i32,
        y: i32,
        dz: i32,
        r: i32,
        double_trunk: bool,
    ) -> bool {
        match &self.kind {
            Kind::Blob { .. } => dx == r && dz == r && (random.next_int_bounded(2) == 0 || y == 0),
            Kind::Fancy { .. } => {
                math::square_f32(dx as f32 + 0.5) + math::square_f32(dz as f32 + 0.5) > (r * r) as f32
            }
            Kind::Bush { .. } => dx == r && dz == r && random.next_int_bounded(2) == 0,
            Kind::Spruce { .. } | Kind::Pine { .. } => dx == r && dz == r && r > 0,
            Kind::Acacia => {
                if y == 0 {
                    (dx > 1 || dz > 1) && dx != 0 && dz != 0
                } else {
                    dx == r && dz == r && r > 0
                }
            }
            Kind::DarkOak => {
                if y == -1 && !double_trunk {
                    dx == r && dz == r
                } else {
                    y == 1 && dx + dz > r * 2 - 2
                }
            }
            Kind::MegaPine { .. } | Kind::MegaJungle { .. } => {
                if dx + dz >= 7 {
                    true
                } else {
                    dx * dx + dz * dz > r * r
                }
            }
            Kind::RandomSpread { .. } => false,
            Kind::Cherry {
                wide_bottom_hole,
                corner_hole,
                ..
            } => {
                if y == -1 && (dx == r || dz == r) && random.next_float() < *wide_bottom_hole {
                    return true;
                }
                let corner = dx == r && dz == r;
                if r > 2 {
                    corner || (dx + dz > r * 2 - 2 && random.next_float() < *corner_hole)
                } else {
                    corner && random.next_float() < *corner_hole
                }
            }
            Kind::Poplar { .. } => unreachable!("poplar uses its own skip"),
        }
    }

    /// `shouldSkipLocationSigned`.
    fn skip_signed(
        &self,
        random: &mut dyn RandomSource,
        dx: i32,
        y: i32,
        dz: i32,
        r: i32,
        double_trunk: bool,
    ) -> bool {
        if matches!(self.kind, Kind::DarkOak)
            && !(y != 0 || !double_trunk || (dx != -r && dx < r) || (dz != -r && dz < r))
        {
            return true;
        }
        let (mx, mz) = if double_trunk {
            (dx.abs().min((dx - 1).abs()), dz.abs().min((dz - 1).abs()))
        } else {
            (dx.abs(), dz.abs())
        };
        self.skip(random, mx, y, mz, r, double_trunk)
    }

    /// `placeLeavesRow`.
    fn leaves_row(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        r: i32,
        y: i32,
        double_trunk: bool,
    ) {
        let offset = if double_trunk { 1 } else { 0 };
        for dx in -r..=r + offset {
            for dz in -r..=r + offset {
                if !self.skip_signed(random, dx, y, dz, r, double_trunk) {
                    try_place_leaf(ctx, random, origin.offset(dx, y, dz));
                }
            }
        }
    }

    /// `placeLeavesRowWithHangingLeavesBelow`.
    fn leaves_row_hanging(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        origin: BlockPos,
        r: i32,
        y: i32,
        double_trunk: bool,
        chance: f32,
        extension: f32,
    ) {
        self.leaves_row(ctx, random, origin, r, y, double_trunk);
        let offset = if double_trunk { 1 } else { 0 };
        let log = origin.below();
        for along in Direction::HORIZONTAL {
            let to_edge = along.clockwise();
            let offset_to_edge = if to_edge.is_positive() { r + offset } else { r };
            let mut pos = origin
                .offset(0, y - 1, 0)
                .relative(to_edge, offset_to_edge)
                .relative(along, -r);
            let mut i = -r;
            while i < r + offset {
                let above = ctx.placed.foliage.contains(&pos.above());
                if above && try_place_extension(ctx, random, chance, log, pos) {
                    try_place_extension(ctx, random, extension, log, pos.below());
                }
                i += 1;
                pos = pos.relative(along, 1);
            }
        }
    }

    /// `createFoliage` (samples the offset first).
    pub fn create_foliage(
        &self,
        ctx: &mut TreeCtx,
        random: &mut dyn RandomSource,
        tree_height: i32,
        a: &Attachment,
        foliage_height: i32,
        leaf_radius: i32,
    ) {
        let offset = self.offset.sample(random);
        let _ = tree_height;
        let dt = a.double_trunk;
        let fh = foliage_height + a.height_offset;
        match &self.kind {
            Kind::Blob { .. } => {
                let mut yo = offset;
                while yo >= offset - fh {
                    let r = (leaf_radius + a.radius_offset - 1 - yo / 2).max(0);
                    self.leaves_row(ctx, random, a.pos, r, yo, dt);
                    yo -= 1;
                }
            }
            Kind::Fancy { .. } => {
                let mut yo = offset;
                while yo >= offset - foliage_height {
                    let r = leaf_radius
                        + if yo != offset && yo != offset - foliage_height {
                            1
                        } else {
                            0
                        };
                    self.leaves_row(ctx, random, a.pos, r, yo, dt);
                    yo -= 1;
                }
            }
            Kind::Bush { .. } => {
                let mut yo = offset;
                while yo >= offset - fh {
                    let r = leaf_radius + a.radius_offset - 1 - yo;
                    self.leaves_row(ctx, random, a.pos, r, yo, dt);
                    yo -= 1;
                }
            }
            Kind::Spruce { .. } => {
                let mut r = random.next_int_bounded(2);
                let mut max_r = 1;
                let mut min_r = 0;
                let mut yo = offset;
                while yo >= -fh {
                    self.leaves_row(ctx, random, a.pos, r, yo, dt);
                    if r >= max_r {
                        r = min_r;
                        min_r = 1;
                        max_r = (max_r + 1).min(leaf_radius + a.radius_offset);
                    } else {
                        r += 1;
                    }
                    yo -= 1;
                }
            }
            Kind::Pine { .. } => {
                let mut r = 0;
                let mut yo = offset;
                while yo >= offset - fh {
                    self.leaves_row(ctx, random, a.pos, r, yo, dt);
                    if r >= 1 && yo == offset - fh + 1 {
                        r -= 1;
                    } else if r < leaf_radius + a.radius_offset {
                        r += 1;
                    }
                    yo -= 1;
                }
            }
            Kind::Acacia => {
                let p = a.pos.above_n(offset);
                self.leaves_row(ctx, random, p, leaf_radius + a.radius_offset, -1 - fh, dt);
                self.leaves_row(ctx, random, p, leaf_radius - 1, -fh, dt);
                self.leaves_row(ctx, random, p, leaf_radius + a.radius_offset - 1, 0, dt);
            }
            Kind::DarkOak => {
                let p = a.pos.above_n(offset);
                if dt {
                    self.leaves_row(ctx, random, p, leaf_radius + 2, -1, dt);
                    self.leaves_row(ctx, random, p, leaf_radius + 3, 0, dt);
                    self.leaves_row(ctx, random, p, leaf_radius + 2, 1, dt);
                    if random.next_bool() {
                        self.leaves_row(ctx, random, p, leaf_radius, 2, dt);
                    }
                } else {
                    self.leaves_row(ctx, random, p, leaf_radius + 2, -1, dt);
                    self.leaves_row(ctx, random, p, leaf_radius + 1, 0, dt);
                }
            }
            Kind::MegaPine { .. } => {
                let fp = a.pos;
                let mut prev = 0;
                let mut yy = fp.y - fh + offset;
                while yy <= fp.y + offset {
                    let yo = fp.y - yy;
                    let smooth = leaf_radius + a.radius_offset + math::floor_f32(yo as f32 / fh as f32 * 3.5);
                    let jagged = if yo > 0 && smooth == prev && (yy & 1) == 0 {
                        smooth + 1
                    } else {
                        smooth
                    };
                    self.leaves_row(ctx, random, BlockPos::new(fp.x, yy, fp.z), jagged, 0, dt);
                    prev = smooth;
                    yy += 1;
                }
            }
            Kind::MegaJungle { .. } => {
                let leaf_height = (if dt {
                    foliage_height
                } else {
                    1 + random.next_int_bounded(2)
                }) + a.height_offset;
                let mut yo = offset;
                while yo >= offset - leaf_height {
                    let r = leaf_radius + a.radius_offset + 1 - yo;
                    self.leaves_row(ctx, random, a.pos, r, yo, dt);
                    yo -= 1;
                }
            }
            Kind::RandomSpread { attempts, .. } => {
                for _ in 0..*attempts {
                    let dx = random.next_int_bounded(leaf_radius) - random.next_int_bounded(leaf_radius);
                    let dy =
                        random.next_int_bounded(foliage_height) - random.next_int_bounded(foliage_height);
                    let dz = random.next_int_bounded(leaf_radius) - random.next_int_bounded(leaf_radius);
                    try_place_leaf(ctx, random, a.pos.offset(dx, dy, dz));
                }
            }
            Kind::Cherry {
                hanging,
                hanging_extension,
                ..
            } => {
                let p = a.pos.above_n(offset);
                let r = leaf_radius + a.radius_offset - 1;
                self.leaves_row(ctx, random, p, r - 2, fh - 3, dt);
                self.leaves_row(ctx, random, p, r - 1, fh - 4, dt);
                let mut y = fh - 5;
                while y >= 0 {
                    self.leaves_row(ctx, random, p, r, y, dt);
                    y -= 1;
                }
                self.leaves_row_hanging(ctx, random, p, r, -1, dt, *hanging, *hanging_extension);
                self.leaves_row_hanging(ctx, random, p, r - 1, -2, dt, *hanging, *hanging_extension);
            }
            Kind::Poplar { side_hole, .. } => {
                let p = a.pos.above_n(offset);
                let r = leaf_radius + a.radius_offset - 1;
                let flip = random.next_bool();
                let row = |ctx: &mut TreeCtx, random: &mut dyn RandomSource, radius: i32, y: i32| {
                    poplar_row(ctx, random, p, radius, y, dt, fh, flip, *side_hole);
                };
                row(ctx, random, r - 2, fh - 1);
                row(ctx, random, r - 1, fh - 2);
                row(ctx, random, r - 1, fh - 3);
                let mut y = fh - 4;
                while y >= 1 {
                    row(ctx, random, r, y);
                    y -= 1;
                }
                poplar_logs(ctx, random, p, r, fh - 4, dt, fh, flip);
                row(ctx, random, r - 1, 0);
                row(ctx, random, math::clamp_i32(r - 2, 1, 2), -1);
            }
        }
    }
}

/// `FoliagePlacer.tryPlaceLeaf`.
pub fn try_place_leaf(ctx: &mut TreeCtx, random: &mut dyn RandomSource, pos: BlockPos) -> bool {
    let env = ctx.region.env;
    let existing = ctx.region.get(pos);
    let persistent = env.blocks.get_property(existing, "persistent") == Some("true");
    if persistent || !ctx.valid_tree_pos(pos) {
        return false;
    }
    let mut s = ctx.tree.foliage_provider.get(ctx.region, random, pos);
    if env.blocks.has_property(s, "waterlogged") {
        let water = env.blocks.flags(existing).fluid() == Fluid::Water;
        s = env
            .blocks
            .with_property(s, "waterlogged", if water { "true" } else { "false" })
            .unwrap_or(s);
    }
    ctx.set_foliage(pos, s);
    true
}

fn try_place_extension(
    ctx: &mut TreeCtx,
    random: &mut dyn RandomSource,
    chance: f32,
    log: BlockPos,
    pos: BlockPos,
) -> bool {
    if pos.dist_manhattan(log) >= 7 {
        return false;
    }
    if random.next_float() > chance {
        return false;
    }
    try_place_leaf(ctx, random, pos)
}

fn poplar_partial_row(foliage_height: i32, y: i32) -> bool {
    foliage_height - 1 == y || foliage_height - 2 == y
}

fn poplar_corner_cut(dx: i32, dz: i32, r: i32, partial: bool, flip: bool) -> i32 {
    let small = if flip {
        (dx > 0 && dz > 0) || (dz < 0 && dx < 0)
    } else {
        (dx > 0 && dz < 0) || (dz > 0 && dx < 0)
    };
    if small {
        r - 1
    } else if partial {
        r + 1
    } else {
        r
    }
}

fn within_rhombus(r: i32, adx: i32, adz: i32, cut: i32, extra: i32) -> bool {
    adx + adz <= r * 2 - (cut + extra)
}

fn poplar_row(
    ctx: &mut TreeCtx,
    random: &mut dyn RandomSource,
    origin: BlockPos,
    r: i32,
    y: i32,
    dt: bool,
    fh: i32,
    flip: bool,
    side_hole: f32,
) {
    let offset = if dt { 1 } else { 0 };
    for dx in -r..=r + offset {
        for dz in -r..=r + offset {
            let partial = poplar_partial_row(fh, y);
            let cut = poplar_corner_cut(dx, dz, r, partial, flip);
            let adx = dx.abs();
            let adz = dz.abs();
            let edge = adx == r || adz == r;
            let skip = if partial && edge {
                true
            } else {
                let extra = if random.next_float() <= side_hole { 1 } else { 0 };
                !within_rhombus(r, adx, adz, cut, extra)
            };
            if !skip {
                try_place_leaf(ctx, random, origin.offset(dx, y, dz));
            }
        }
    }
}

/// `PoplarFoliagePlacer.replaceLeavesWithLog`.
fn poplar_logs(
    ctx: &mut TreeCtx,
    random: &mut dyn RandomSource,
    origin: BlockPos,
    r: i32,
    y: i32,
    dt: bool,
    fh: i32,
    flip: bool,
) {
    let offset = if dt { 1 } else { 0 };
    for dx in -r..=r + offset {
        for dz in -r..=r + offset {
            let adz = dz.abs();
            let adx = dx.abs();
            let cut = poplar_corner_cut(dx, dz, r, poplar_partial_row(fh, y), flip);
            if within_rhombus(r, adx, adz, cut, 2)
                && ((adz == 0 && r - adx >= 4) || (adx == 0 && r - adz >= 4))
            {
                let pos = origin.offset(dx, y, dz);
                let axis = if adz == 0 { Axis::X } else { Axis::Z };
                // `isStateAtPosition(pos, state -> state.equals(foliageProvider.getState(...)))`
                let existing = ctx.region.get(pos);
                let leaf = ctx.tree.foliage_provider.get(ctx.region, random, pos);
                if existing == leaf {
                    let log = ctx.tree.trunk_provider.get(ctx.region, random, pos);
                    let log = ctx.with_axis(log, axis);
                    ctx.set_foliage(pos, log);
                }
            }
        }
    }
}
