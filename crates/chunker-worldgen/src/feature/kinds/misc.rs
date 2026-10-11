//! Assorted feature types: overlays, ice, blobs, spikes, vines, bamboo, huge mushrooms.

use std::sync::Arc;

use anyhow::Result;
use chunker_data::blocks::BlockState;
use serde_json::Value;

use crate::chunk::HeightmapKind;
use crate::codec::{f32_field, field, opt_i32};
use crate::feature::Region;
use crate::feature::predicate::BlockPredicate;
use crate::feature::provider::StateProvider;
use crate::feature::registry::Parser;
use crate::math;
use crate::random::RandomSource;
use crate::structure::geom::{BlockPos, Direction};

/// `BlueIceFeature`.
pub fn blue_ice(region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
    let b = &region.env.b;
    if origin.y > region.env.sea_level - 1 {
        return false;
    }
    if !region.is(origin, b.water) && !region.is(origin.below(), b.water) {
        return false;
    }
    let found = Direction::ALL
        .iter()
        .any(|d| *d != Direction::Down && region.is(origin.relative(*d, 1), b.packed_ice));
    if !found {
        return false;
    }
    let blue = region.env.blocks.block(b.blue_ice).default_state;
    region.set(origin, blue, 2);
    for _ in 0..200 {
        let y_off = random.next_int_bounded(5) - random.next_int_bounded(6);
        let mut xz = 3;
        if y_off < 2 {
            xz += y_off / 2;
        }
        if xz >= 1 {
            let dx = random.next_int_bounded(xz) - random.next_int_bounded(xz);
            let dz = random.next_int_bounded(xz) - random.next_int_bounded(xz);
            let p = origin.offset(dx, y_off, dz);
            let id = region.block_of(p);
            if (region.flags(p).is_air() || id == b.water || id == b.packed_ice || id == b.ice)
                && Direction::ALL
                    .iter()
                    .any(|d| region.is(p.relative(*d, 1), b.blue_ice))
            {
                region.set(p, blue, 2);
            }
        }
    }
    true
}

/// `BlockBlobFeature`.
pub struct BlockBlob {
    state: BlockState,
    can_place_on: BlockPredicate,
}

impl BlockBlob {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            state: p.state(field(json, "state")?)?,
            can_place_on: BlockPredicate::parse(p, field(json, "can_place_on")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, mut origin: BlockPos) -> bool {
        while origin.y > region.min_y() + 3 && !self.can_place_on.test(region, origin.below()) {
            origin = origin.below();
        }
        if origin.y <= region.min_y() + 3 {
            return false;
        }
        for _ in 0..3 {
            let xr = random.next_int_bounded(2);
            let yr = random.next_int_bounded(2);
            let zr = random.next_int_bounded(2);
            let tr = (xr + yr + zr) as f32 * 0.333 + 0.5;
            for z in -zr..=zr {
                for y in -yr..=yr {
                    for x in -xr..=xr {
                        let d = (x * x + y * y + z * z) as f64;
                        if d <= (tr * tr) as f64 {
                            region.set_and_update(origin.offset(x, y, z), self.state);
                        }
                    }
                }
            }
            origin = origin.offset(
                -1 + random.next_int_bounded(2),
                -random.next_int_bounded(2),
                -1 + random.next_int_bounded(2),
            );
        }
        true
    }
}

/// `SpikeFeature` (ice spikes).
pub struct Spike {
    state: BlockState,
    can_place_on: BlockPredicate,
    can_replace: BlockPredicate,
}

impl Spike {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            state: p.state(field(json, "state")?)?,
            can_place_on: BlockPredicate::parse(p, field(json, "can_place_on")?)?,
            can_replace: BlockPredicate::parse(p, field(json, "can_replace")?)?,
        })
    }

    fn replaceable(&self, region: &Region, p: BlockPos) -> bool {
        region.flags(p).is_air() || self.can_replace.test(region, p)
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, mut origin: BlockPos) -> bool {
        while region.is_empty_block(origin) && origin.y > region.min_y() + 2 {
            origin = origin.below();
        }
        if !self.can_place_on.test(region, origin) {
            return false;
        }
        origin = origin.above_n(random.next_int_bounded(4));
        let height = random.next_int_bounded(4) + 7;
        let width = height / 4 + random.next_int_bounded(2);
        if width > 1 && random.next_int_bounded(60) == 0 {
            origin = origin.above_n(10 + random.next_int_bounded(30));
        }
        for y_off in 0..height {
            let scale = (1.0 - y_off as f32 / height as f32) * width as f32;
            let new_width = math::ceil(scale as f64);
            for xo in -new_width..=new_width {
                let dx = xo.abs() as f32 - 0.25;
                for zo in -new_width..=new_width {
                    let dz = zo.abs() as f32 - 0.25;
                    if ((xo == 0 && zo == 0) || dx * dx + dz * dz <= scale * scale)
                        && ((xo != -new_width && xo != new_width && zo != -new_width && zo != new_width)
                            || random.next_float() <= 0.75)
                    {
                        let pos = origin.offset(xo, y_off, zo);
                        if self.replaceable(region, pos) {
                            region.set_and_update(pos, self.state);
                        }
                        if y_off != 0 && new_width > 1 {
                            let neg = origin.offset(xo, -y_off, zo);
                            if self.replaceable(region, neg) {
                                region.set_and_update(neg, self.state);
                            }
                        }
                    }
                }
            }
        }
        let pillar = (width - 1).clamp(0, 1);
        for xo in -pillar..=pillar {
            for zo in -pillar..=pillar {
                let mut cursor = origin.offset(xo, -1, zo);
                let mut run = 50;
                if xo.abs() == 1 && zo.abs() == 1 {
                    run = random.next_int_bounded(5);
                }
                while cursor.y > 50 {
                    let s = region.get(cursor);
                    if !region.env.blocks.flags(s).is_air()
                        && !self.can_replace.test(region, cursor)
                        && s != self.state
                    {
                        break;
                    }
                    region.set_and_update(cursor, self.state);
                    cursor = cursor.below();
                    run -= 1;
                    if run <= 0 {
                        cursor = cursor.offset(0, -(random.next_int_bounded(5) + 1), 0);
                        run = random.next_int_bounded(5);
                    }
                }
            }
        }
        true
    }
}

/// `VinesFeature`.
pub fn vines(region: &mut Region, origin: BlockPos) -> bool {
    if !region.is_empty_block(origin) {
        return false;
    }
    let env = region.env;
    for d in Direction::ALL {
        if d != Direction::Down && crate::feature::behaviour::can_attach_to(region, d, origin.relative(d, 1))
        {
            let vine = env.blocks.block(env.b.vine).default_state;
            let s = env.blocks.with_property(vine, d.name(), "true").unwrap();
            region.set(origin, s, 2);
            return true;
        }
    }
    false
}

/// `BambooFeature`.
pub fn bamboo(
    region: &mut Region,
    random: &mut dyn RandomSource,
    origin: BlockPos,
    probability: f32,
) -> bool {
    let env = region.env;
    let blocks = &env.blocks;
    let mut placed = 0;
    if region.is_empty_block(origin) {
        let bamboo = blocks.block(env.b.bamboo).default_state;
        if crate::feature::behaviour::can_survive(region, bamboo, origin) {
            let height = random.next_int_bounded(12) + 5;
            if random.next_float() < probability {
                let r = random.next_int_bounded(4) + 1;
                let podzol = blocks.block(env.b.podzol).default_state;
                for xx in origin.x - r..=origin.x + r {
                    for zz in origin.z - r..=origin.z + r {
                        let xd = xx - origin.x;
                        let zd = zz - origin.z;
                        if xd * xd + zd * zd <= r * r {
                            let p =
                                BlockPos::new(xx, region.height(HeightmapKind::WorldSurface, xx, zz) - 1, zz);
                            if env.t.beneath_bamboo_podzol_replaceable.contains(region.get(p)) {
                                region.set(p, podzol, 2);
                            }
                        }
                    }
                }
            }
            let with = |s: BlockState, k: &str, v: &str| blocks.with_property(s, k, v).unwrap();
            let trunk = with(with(with(bamboo, "age", "1"), "leaves", "none"), "stage", "0");
            let final_large = with(with(trunk, "leaves", "large"), "stage", "1");
            let top_large = with(trunk, "leaves", "large");
            let top_small = with(trunk, "leaves", "small");
            let mut pos = origin;
            let mut i = 0;
            while i < height && region.is_empty_block(pos) {
                region.set(pos, trunk, 2);
                pos = pos.above();
                i += 1;
            }
            if pos.y - origin.y >= 3 {
                region.set(pos, final_large, 2);
                pos = pos.below();
                region.set(pos, top_large, 2);
                pos = pos.below();
                region.set(pos, top_small, 2);
            }
        }
        placed += 1;
    }
    placed > 0
}

/// `HugeBrownMushroomFeature` / `HugeRedMushroomFeature`.
pub struct HugeMushroom {
    red: bool,
    cap: Arc<StateProvider>,
    stem: Arc<StateProvider>,
    foliage_radius: i32,
    can_place_on: BlockPredicate,
}

impl HugeMushroom {
    pub fn parse(p: &Parser, json: &Value, red: bool) -> Result<Self> {
        Ok(Self {
            red,
            cap: p.provider(field(json, "cap_provider")?)?,
            stem: p.provider(field(json, "stem_provider")?)?,
            foliage_radius: opt_i32(json, "foliage_radius", 2)?,
            can_place_on: BlockPredicate::parse(p, field(json, "can_place_on")?)?,
        })
    }

    fn radius_for_height(&self, trunk_height: i32, tree_height: i32, leaf_radius: i32, yo: i32) -> i32 {
        let _ = trunk_height;
        if self.red {
            if (yo < tree_height && yo >= tree_height - 3) || yo == tree_height {
                leaf_radius
            } else {
                0
            }
        } else if yo <= 3 {
            0
        } else {
            leaf_radius
        }
    }

    fn place_block(region: &mut Region, pos: BlockPos, state: BlockState) {
        let current = region.get(pos);
        if region.env.blocks.flags(current).is_air()
            || region.env.t.replaceable_by_mushrooms.contains(current)
        {
            region.set_and_update(pos, state);
        }
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        let mut tree_height = random.next_int_bounded(3) + 4;
        if random.next_int_bounded(12) == 0 {
            tree_height *= 2;
        }
        // `isValidPosition`
        let y = origin.y;
        if y < region.min_y() + 1 || y + tree_height + 1 > region.max_y() {
            return false;
        }
        if !self.can_place_on.test(region, origin.below()) {
            return false;
        }
        for dy in 0..=tree_height {
            let radius = self.radius_for_height(-1, -1, self.foliage_radius, dy);
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let s = region.get(origin.offset(dx, dy, dz));
                    if !region.env.blocks.flags(s).is_air() && !region.env.t.leaves.contains(s) {
                        return false;
                    }
                }
            }
        }
        let blocks = &region.env.blocks;
        let set_faces = |s: BlockState, faces: &[(&str, bool)]| -> BlockState {
            if faces.iter().all(|(k, _)| blocks.has_property(s, k)) {
                let mut r = s;
                for (k, v) in faces {
                    r = blocks
                        .with_property(r, k, if *v { "true" } else { "false" })
                        .unwrap();
                }
                r
            } else {
                s
            }
        };
        let fr = self.foliage_radius;
        if self.red {
            for dy in tree_height - 3..=tree_height {
                let radius = if dy < tree_height { fr } else { fr - 1 };
                let center = fr - 2;
                for dx in -radius..=radius {
                    for dz in -radius..=radius {
                        let x_edge = dx == -radius || dx == radius;
                        let z_edge = dz == -radius || dz == radius;
                        if dy >= tree_height || x_edge != z_edge {
                            let s = self.cap.get(region, random, origin);
                            let s = set_faces(
                                s,
                                &[
                                    ("up", dy >= tree_height - 1),
                                    ("west", dx < -center),
                                    ("east", dx > center),
                                    ("north", dz < -center),
                                    ("south", dz > center),
                                ],
                            );
                            Self::place_block(region, origin.offset(dx, dy, dz), s);
                        }
                    }
                }
            }
        } else {
            for dx in -fr..=fr {
                for dz in -fr..=fr {
                    let min_x = dx == -fr;
                    let max_x = dx == fr;
                    let min_z = dz == -fr;
                    let max_z = dz == fr;
                    let x_edge = min_x || max_x;
                    let z_edge = min_z || max_z;
                    if !x_edge || !z_edge {
                        let west = min_x || (z_edge && dx == 1 - fr);
                        let east = max_x || (z_edge && dx == fr - 1);
                        let north = min_z || (x_edge && dz == 1 - fr);
                        let south = max_z || (x_edge && dz == fr - 1);
                        let s = self.cap.get(region, random, origin);
                        let s = set_faces(
                            s,
                            &[("west", west), ("east", east), ("north", north), ("south", south)],
                        );
                        Self::place_block(region, origin.offset(dx, tree_height, dz), s);
                    }
                }
            }
        }
        for dy in 0..tree_height {
            let s = self.stem.get(region, random, origin);
            Self::place_block(region, origin.offset(0, dy, 0), s);
        }
        true
    }
}

pub fn bamboo_probability(json: &Value) -> Result<f32> {
    f32_field(json, "probability")
}

/// `CoralClawFeature`.
pub fn coral_claw(
    region: &mut Region,
    random: &mut dyn RandomSource,
    origin: BlockPos,
    f: &crate::feature::PlacedFeature,
) -> bool {
    use crate::feature::placement::place;
    if !place(region, f, None, random, origin) {
        return false;
    }
    let claw = Direction::random_horizontal(random);
    let branches = random.next_int_bounded(2) + 2;
    let mut dirs = vec![claw, claw.clockwise(), claw.counter_clockwise()];
    crate::structure::util::shuffle(&mut dirs, random);
    for &branch in &dirs[..branches as usize] {
        let mut pos = origin;
        let sideway = random.next_int_bounded(2) + 1;
        pos = pos.relative(branch, 1);
        let (segment, inway) = if branch == claw {
            (claw, random.next_int_bounded(3) + 2)
        } else {
            pos = pos.above();
            let options = [branch, Direction::Up];
            (
                options[random.next_int_bounded(2) as usize],
                random.next_int_bounded(3) + 3,
            )
        };
        let mut i = 0;
        while i < sideway && place(region, f, None, random, pos) {
            pos = pos.relative(segment, 1);
            i += 1;
        }
        pos = pos.relative(segment.opposite(), 1).above();
        for _ in 0..inway {
            pos = pos.relative(claw, 1);
            if !place(region, f, None, random, pos) {
                break;
            }
            if random.next_float() < 0.25 {
                pos = pos.above();
            }
        }
    }
    true
}

/// `CoralTreeFeature`.
pub fn coral_tree(
    region: &mut Region,
    random: &mut dyn RandomSource,
    origin: BlockPos,
    f: &crate::feature::PlacedFeature,
) -> bool {
    use crate::feature::placement::place;
    let mut pos = origin;
    let trunk = random.next_int_bounded(3) + 1;
    for _ in 0..trunk {
        if !place(region, f, None, random, pos) {
            return true;
        }
        pos = pos.above();
    }
    let top = pos;
    let branches = random.next_int_bounded(3) + 2;
    let mut dirs = Direction::HORIZONTAL.to_vec();
    crate::structure::util::shuffle(&mut dirs, random);
    for &d in &dirs[..branches as usize] {
        let mut p = top.relative(d, 1);
        let height = random.next_int_bounded(5) + 2;
        let mut segment = 0;
        let mut j = 0;
        while j < height && place(region, f, None, random, p) {
            segment += 1;
            p = p.above();
            if j == 0 || (segment >= 2 && random.next_float() < 0.25) {
                p = p.relative(d, 1);
                segment = 0;
            }
            j += 1;
        }
    }
    true
}

/// `BlockPileFeature`.
pub struct BlockPile {
    provider: Arc<StateProvider>,
}

impl BlockPile {
    pub fn parse(p: &Parser, json: &Value) -> Result<Self> {
        Ok(Self {
            provider: p.provider(field(json, "state_provider")?)?,
        })
    }

    pub fn place(&self, region: &mut Region, random: &mut dyn RandomSource, origin: BlockPos) -> bool {
        if origin.y < region.min_y() + 5 {
            return false;
        }
        let xr = 2 + random.next_int_bounded(2);
        let zr = 2 + random.next_int_bounded(2);
        // `BlockPos.betweenClosed`: x fastest, then y, then z.
        for z in origin.z - zr..=origin.z + zr {
            for y in origin.y..=origin.y + 1 {
                for x in origin.x - xr..=origin.x + xr {
                    let p = BlockPos::new(x, y, z);
                    let (xd, zd) = (origin.x - x, origin.z - z);
                    let a = random.next_float() * 10.0;
                    let threshold = a - random.next_float() * 6.0;
                    if ((xd * xd + zd * zd) as f32) <= threshold || (random.next_float() as f64) < 0.031 {
                        self.try_place(region, random, p);
                    }
                }
            }
        }
        true
    }

    fn try_place(&self, region: &mut Region, random: &mut dyn RandomSource, p: BlockPos) {
        if !region.is_empty_block(p) {
            return;
        }
        let below = p.below();
        let ok = if region.is(below, region.env.b.dirt_path) {
            random.next_bool()
        } else {
            region.flags(below).is_face_sturdy(1)
        };
        if ok {
            let s = self.provider.get(region, random, p);
            region.set(p, s, 260);
        }
    }
}
