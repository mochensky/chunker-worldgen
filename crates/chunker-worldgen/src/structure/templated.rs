//! `TemplateStructurePiece` structures: shipwrecks, igloos, ocean ruins, ruined portals, nether fossils.

use chunker_data::Identifier;
use chunker_data::blocks::BlockState;
use chunker_nbt::Compound;

use super::build::PieceCtx;
use super::geom::{BlockPos, BoundingBox, Direction, Mirror, Rotation};
use super::place::{PlaceSettings, filter_blocks, place_in_world};
use super::processor::{BlockEntityModifier, PosRule, Processor, ProcessorRule};
use super::registry::RuinedPortalSetup;
use super::start::{PieceBase, PiecesBuilder};
use super::template::transform;
use super::util::pick;
use super::{GenContext, Stub};
use crate::chunk::HeightmapKind;
use crate::feature::rule::RuleTest;
use crate::piece_boilerplate;
use crate::providers::{mth_next_int, random_between_inclusive};
use crate::random::RandomSource;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum TemplatedKind {
    Shipwreck {
        beached: bool,
        height_adjusted: bool,
    },
    Igloo,
    OceanRuin {
        integrity: f32,
        warm: bool,
        large: bool,
    },
    RuinedPortal {
        placement: String,
        cold: bool,
        mossiness: f32,
        air_pocket: bool,
        overgrown: bool,
        vines: bool,
        blackstone: bool,
    },
    NetherFossil,
}

/// A `TemplateStructurePiece` subclass.
pub struct TemplatedPiece {
    pub base: PieceBase,
    pub template: Identifier,
    pub template_pos: BlockPos,
    pub rotation: Rotation,
    pub mirror: Mirror,
    pub pivot: BlockPos,
    pub kind: TemplatedKind,
    /// Template position and shipwreck `heightAdjusted` as changed by `postProcess` (vanilla mutates
    /// the shared piece, so later chunks see the first chunk's adjustment).
    pub live: parking_lot::Mutex<Option<(BlockPos, bool)>>,
}

impl TemplatedPiece {
    fn new(
        ctx: &GenContext,
        kind: TemplatedKind,
        template: Identifier,
        pos: BlockPos,
        rotation: Rotation,
        mirror: Mirror,
        pivot: BlockPos,
    ) -> Self {
        let bbox = ctx
            .structures
            .templates
            .get(&template)
            .bounding_box(pos, rotation, pivot, mirror);
        Self {
            base: PieceBase::new(0, bbox, Some(Direction::North)),
            template,
            template_pos: pos,
            rotation,
            mirror,
            pivot,
            kind,
            live: parking_lot::Mutex::new(None),
        }
    }
}

impl super::start::Piece for TemplatedPiece {
    piece_boilerplate!();
    fn type_id(&self) -> &'static str {
        match self.kind {
            TemplatedKind::Shipwreck { .. } => "minecraft:shipwreck",
            TemplatedKind::Igloo => "minecraft:iglu",
            TemplatedKind::OceanRuin { .. } => "minecraft:orp",
            TemplatedKind::RuinedPortal { .. } => "minecraft:rupo",
            TemplatedKind::NetherFossil => "minecraft:nefos",
        }
    }
    fn save_extra(&self, tag: &mut Compound) {
        tag.insert("TPX", self.template_pos.x);
        tag.insert("TPY", self.template_pos.y);
        tag.insert("TPZ", self.template_pos.z);
        tag.insert("Template", self.template.as_str());
        match &self.kind {
            TemplatedKind::Shipwreck {
                beached,
                height_adjusted,
            } => {
                tag.insert("isBeached", *beached);
                tag.insert("Rot", self.rotation.name());
                tag.insert("height_adjusted", *height_adjusted);
            }
            TemplatedKind::Igloo | TemplatedKind::NetherFossil => tag.insert("Rot", self.rotation.name()),
            TemplatedKind::OceanRuin {
                integrity,
                warm,
                large,
            } => {
                tag.insert("Rot", self.rotation.name());
                tag.insert("Integrity", *integrity);
                tag.insert("BiomeType", if *warm { "WARM" } else { "COLD" });
                tag.insert("IsLarge", *large);
            }
            TemplatedKind::RuinedPortal {
                placement,
                cold,
                mossiness,
                air_pocket,
                overgrown,
                vines,
                blackstone,
            } => {
                tag.insert("Rotation", self.rotation.name());
                tag.insert("Mirror", self.mirror.name());
                tag.insert("VerticalPlacement", placement.as_str());
                let mut p = Compound::new();
                p.insert("cold", *cold);
                p.insert("mossiness", *mossiness);
                p.insert("air_pocket", *air_pocket);
                p.insert("overgrown", *overgrown);
                p.insert("vines", *vines);
                p.insert("replace_with_blackstone", *blackstone);
                tag.insert("Properties", p);
            }
        }
    }
    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.move_by(dx, dy, dz);
        self.template_pos = self.template_pos.offset(dx, dy, dz);
    }
    fn post_process_ported(&self) -> bool {
        true
    }
    fn post_process(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        self.post_process_impl(ctx, random);
    }
}

fn mc(path: &str) -> Identifier {
    Identifier::vanilla(path)
}

// --- shipwreck ----------------------------------------------------------------------------------

const SHIPWRECKS_BEACHED: [&str; 11] = [
    "shipwreck/with_mast",
    "shipwreck/sideways_full",
    "shipwreck/sideways_fronthalf",
    "shipwreck/sideways_backhalf",
    "shipwreck/rightsideup_full",
    "shipwreck/rightsideup_fronthalf",
    "shipwreck/rightsideup_backhalf",
    "shipwreck/with_mast_degraded",
    "shipwreck/rightsideup_full_degraded",
    "shipwreck/rightsideup_fronthalf_degraded",
    "shipwreck/rightsideup_backhalf_degraded",
];

const SHIPWRECKS_OCEAN: [&str; 20] = [
    "shipwreck/with_mast",
    "shipwreck/upsidedown_full",
    "shipwreck/upsidedown_fronthalf",
    "shipwreck/upsidedown_backhalf",
    "shipwreck/sideways_full",
    "shipwreck/sideways_fronthalf",
    "shipwreck/sideways_backhalf",
    "shipwreck/rightsideup_full",
    "shipwreck/rightsideup_fronthalf",
    "shipwreck/rightsideup_backhalf",
    "shipwreck/with_mast_degraded",
    "shipwreck/upsidedown_full_degraded",
    "shipwreck/upsidedown_fronthalf_degraded",
    "shipwreck/upsidedown_backhalf_degraded",
    "shipwreck/sideways_full_degraded",
    "shipwreck/sideways_fronthalf_degraded",
    "shipwreck/sideways_backhalf_degraded",
    "shipwreck/rightsideup_full_degraded",
    "shipwreck/rightsideup_fronthalf_degraded",
    "shipwreck/rightsideup_backhalf_degraded",
];

/// `ShipwreckStructure.findGenerationPoint`.
pub fn shipwreck<'c>(ctx: &mut GenContext, beached: bool) -> Option<Stub<'c>> {
    if !ctx.could_valid_biome_exist_on_top_of_chunk_center() {
        return None;
    }
    let kind = if beached {
        HeightmapKind::WorldSurfaceWg
    } else {
        HeightmapKind::OceanFloorWg
    };
    let pos = ctx.top_of_chunk_center(kind);
    Some(Stub::lazy(
        pos,
        move |ctx: &mut GenContext, b: &mut PiecesBuilder| {
            let rotation = Rotation::random(&mut ctx.random);
            let offset = BlockPos::new(ctx.min_block_x(), 90, ctx.min_block_z());
            let list: &[&str] = if beached {
                &SHIPWRECKS_BEACHED
            } else {
                &SHIPWRECKS_OCEAN
            };
            let id = mc(pick::<&str>(list, &mut ctx.random));
            let mut piece = TemplatedPiece::new(
                ctx,
                TemplatedKind::Shipwreck {
                    beached,
                    height_adjusted: false,
                },
                id.clone(),
                offset,
                rotation,
                Mirror::None,
                BlockPos::new(4, 0, 15),
            );
            let size = ctx.structures.templates.get(&id).size;
            // `isTooBigToFitInWorldGenRegion`
            if size.0 > 32 || size.1 > 32 {
                let bb = piece.base.bbox;
                let height = if beached {
                    let min_y = ctx.lowest_y_at(bb.min_x, bb.min_z, bb.x_span(), bb.z_span());
                    min_y - size.1 / 2 - ctx.random.next_int_bounded(3)
                } else {
                    ctx.mean_first_occupied_height(bb.min_x, bb.x_span(), bb.min_z, bb.z_span())
                };
                // `adjustPositionHeight` moves the template but (like vanilla) not the bounding box.
                if let TemplatedKind::Shipwreck { height_adjusted, .. } = &mut piece.kind {
                    *height_adjusted = true;
                }
                piece.template_pos = BlockPos::new(piece.template_pos.x, height, piece.template_pos.z);
            }
            b.add(Box::new(piece));
        },
    ))
}

// --- igloo ---------------------------------------------------------------------------------------

/// `IglooStructure.findGenerationPoint`.
pub fn igloo<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    if !ctx.could_valid_biome_exist_on_top_of_chunk_center() {
        return None;
    }
    let pos = ctx.top_of_chunk_center(HeightmapKind::WorldSurfaceWg);
    Some(Stub::lazy(pos, |ctx: &mut GenContext, b: &mut PiecesBuilder| {
        let start = BlockPos::new(ctx.min_block_x(), 90, ctx.min_block_z());
        let rotation = Rotation::random(&mut ctx.random);
        let add = |ctx: &GenContext, b: &mut PiecesBuilder, name: &str, depth: i32| {
            let (pivot, offset) = match name {
                "igloo/top" => (BlockPos::new(3, 5, 5), BlockPos::ZERO),
                "igloo/middle" => (BlockPos::new(1, 3, 1), BlockPos::new(2, -3, 4)),
                _ => (BlockPos::new(3, 6, 7), BlockPos::new(0, -3, -2)),
            };
            let pos = start.add(offset).offset(0, -depth, 0);
            b.add(Box::new(TemplatedPiece::new(
                ctx,
                TemplatedKind::Igloo,
                mc(name),
                pos,
                rotation,
                Mirror::None,
                pivot,
            )));
        };
        if ctx.random.next_double() < 0.5 {
            let depth = ctx.random.next_int_bounded(8) + 4;
            add(ctx, b, "igloo/bottom", depth * 3);
            for i in 0..depth - 1 {
                add(ctx, b, "igloo/middle", i * 3);
            }
        }
        add(ctx, b, "igloo/top", 0);
    }))
}

// --- ocean ruins ---------------------------------------------------------------------------------

fn numbered(prefix: &str, n: &[i32]) -> Vec<String> {
    n.iter()
        .map(|i| format!("underwater_ruin/{prefix}_{i}"))
        .collect()
}

/// `OceanRuinStructure.findGenerationPoint`.
pub fn ocean_ruin<'c>(
    ctx: &mut GenContext,
    warm: bool,
    large_probability: f32,
    cluster_probability: f32,
) -> Option<Stub<'c>> {
    if !ctx.could_valid_biome_exist_on_top_of_chunk_center() {
        return None;
    }
    let pos = ctx.top_of_chunk_center(HeightmapKind::OceanFloorWg);
    Some(Stub::lazy(
        pos,
        move |ctx: &mut GenContext, b: &mut PiecesBuilder| {
            let offset = BlockPos::new(ctx.min_block_x(), 90, ctx.min_block_z());
            let rotation = Rotation::random(&mut ctx.random);
            let large = ctx.random.next_float() <= large_probability;
            let integrity = if large { 0.9 } else { 0.8 };
            add_ruin(ctx, b, offset, rotation, warm, large, integrity);
            if large && ctx.random.next_float() <= cluster_probability {
                add_cluster_ruins(ctx, b, rotation, offset, warm);
            }
        },
    ))
}

fn add_ruin(
    ctx: &mut GenContext,
    b: &mut PiecesBuilder,
    pos: BlockPos,
    rotation: Rotation,
    warm: bool,
    large: bool,
    integrity: f32,
) {
    let piece = |ctx: &GenContext, name: String, integrity: f32| {
        TemplatedPiece::new(
            ctx,
            TemplatedKind::OceanRuin {
                integrity,
                warm,
                large,
            },
            mc(&name),
            pos,
            rotation,
            Mirror::None,
            BlockPos::ZERO,
        )
    };
    if warm {
        let list = if large {
            numbered("big_warm", &[4, 5, 6, 7])
        } else {
            numbered("warm", &[1, 2, 3, 4, 5, 6, 7, 8])
        };
        let name = pick(&list, &mut ctx.random).clone();
        b.add(Box::new(piece(ctx, name, integrity)));
    } else {
        let nums: &[i32] = if large {
            &[1, 2, 3, 8]
        } else {
            &[1, 2, 3, 4, 5, 6, 7, 8]
        };
        let (bricks, cracked, mossy) = if large {
            (
                numbered("big_brick", nums),
                numbered("big_cracked", nums),
                numbered("big_mossy", nums),
            )
        } else {
            (
                numbered("brick", nums),
                numbered("cracked", nums),
                numbered("mossy", nums),
            )
        };
        let i = ctx.random.next_int_bounded(bricks.len() as i32) as usize;
        b.add(Box::new(piece(ctx, bricks[i].clone(), integrity)));
        b.add(Box::new(piece(ctx, cracked[i].clone(), 0.7)));
        b.add(Box::new(piece(ctx, mossy[i].clone(), 0.5)));
    }
}

fn add_cluster_ruins(
    ctx: &mut GenContext,
    b: &mut PiecesBuilder,
    rotation: Rotation,
    p: BlockPos,
    warm: bool,
) {
    let parent = BlockPos::new(p.x, 90, p.z);
    let corner = transform(BlockPos::new(15, 0, 15), Mirror::None, rotation, BlockPos::ZERO).add(parent);
    let parent_bb = BoundingBox::from_corners(parent, corner);
    let bottom_left = BlockPos::new(parent.x.min(corner.x), parent.y, parent.z.min(corner.z));
    let r = &mut ctx.random;
    let o = bottom_left;
    let mut positions = Vec::with_capacity(8);
    let mut push = |dx: i32, dz: i32| positions.push(o.offset(dx, 0, dz));
    let (a, b2) = (-16 + mth_next_int(r, 1, 8), 16 + mth_next_int(r, 1, 7));
    push(a, b2);
    let (a, b2) = (-16 + mth_next_int(r, 1, 8), mth_next_int(r, 1, 7));
    push(a, b2);
    let (a, b2) = (-16 + mth_next_int(r, 1, 8), -16 + mth_next_int(r, 4, 8));
    push(a, b2);
    let (a, b2) = (mth_next_int(r, 1, 7), 16 + mth_next_int(r, 1, 7));
    push(a, b2);
    let (a, b2) = (mth_next_int(r, 1, 7), -16 + mth_next_int(r, 4, 6));
    push(a, b2);
    let (a, b2) = (16 + mth_next_int(r, 1, 7), 16 + mth_next_int(r, 3, 8));
    push(a, b2);
    let (a, b2) = (16 + mth_next_int(r, 1, 7), mth_next_int(r, 1, 7));
    push(a, b2);
    let (a, b2) = (16 + mth_next_int(r, 1, 7), -16 + mth_next_int(r, 4, 8));
    push(a, b2);
    let ruins = mth_next_int(&mut ctx.random, 4, 8);
    for _ in 0..ruins {
        if positions.is_empty() {
            continue;
        }
        let idx = ctx.random.next_int_bounded(positions.len() as i32) as usize;
        let pos = positions.remove(idx);
        let next_rotation = Rotation::random(&mut ctx.random);
        let next_corner = transform(
            BlockPos::new(5, 0, 6),
            Mirror::None,
            next_rotation,
            BlockPos::ZERO,
        )
        .add(pos);
        if !BoundingBox::from_corners(pos, next_corner).intersects(&parent_bb) {
            add_ruin(ctx, b, pos, next_rotation, warm, false, 0.8);
        }
    }
}

// --- ruined portal -------------------------------------------------------------------------------

const PORTALS: [&str; 10] = [
    "ruined_portal/portal_1",
    "ruined_portal/portal_2",
    "ruined_portal/portal_3",
    "ruined_portal/portal_4",
    "ruined_portal/portal_5",
    "ruined_portal/portal_6",
    "ruined_portal/portal_7",
    "ruined_portal/portal_8",
    "ruined_portal/portal_9",
    "ruined_portal/portal_10",
];
const GIANT_PORTALS: [&str; 3] = [
    "ruined_portal/giant_portal_1",
    "ruined_portal/giant_portal_2",
    "ruined_portal/giant_portal_3",
];

fn sample_probability(random: &mut dyn RandomSource, limit: f32) -> bool {
    if limit == 0.0 {
        false
    } else if limit == 1.0 {
        true
    } else {
        random.next_float() < limit
    }
}

/// `RuinedPortalStructure.findGenerationPoint`.
pub fn ruined_portal<'c>(ctx: &mut GenContext, setups: &'c [RuinedPortalSetup]) -> Option<Stub<'c>> {
    let setup = if setups.len() > 1 {
        let total: f32 = setups.iter().fold(0.0f32, |t, s| t + s.weight);
        let mut pick_value = ctx.random.next_float();
        let mut chosen = None;
        for s in setups {
            pick_value -= s.weight / total;
            if pick_value < 0.0 {
                chosen = Some(s);
                break;
            }
        }
        chosen.expect("no ruined portal setup chosen")
    } else {
        &setups[0]
    };
    let air_pocket = sample_probability(&mut ctx.random, setup.air_pocket_probability);
    let name = if ctx.random.next_float() < 0.05 {
        GIANT_PORTALS[ctx.random.next_int_bounded(GIANT_PORTALS.len() as i32) as usize]
    } else {
        PORTALS[ctx.random.next_int_bounded(PORTALS.len() as i32) as usize]
    };
    let id = mc(name);
    let template = ctx.structures.templates.get(&id);
    let rotation = Rotation::random(&mut ctx.random);
    let mirror = if ctx.random.next_float() < 0.5 {
        Mirror::None
    } else {
        Mirror::FrontBack
    };
    let pivot = BlockPos::new(template.size.0 / 2, 0, template.size.2 / 2);
    let base = BlockPos::new(ctx.min_block_x(), 0, ctx.min_block_z());
    let bbox = template.bounding_box(base, rotation, pivot, mirror);
    let center = bbox.center();
    let kind = if setup.placement == "on_ocean_floor" {
        HeightmapKind::OceanFloorWg
    } else {
        HeightmapKind::WorldSurfaceWg
    };
    let surface_y = ctx.base_height(center.x, center.z, kind) - 1;
    let projected = find_suitable_y(ctx, &setup.placement, air_pocket, surface_y, bbox.y_span(), &bbox);
    let origin = BlockPos::new(base.x, projected, base.z);
    Some(Stub::lazy(
        origin,
        move |ctx: &mut GenContext, b: &mut PiecesBuilder| {
            let cold = setup.can_be_cold && {
                let biome = ctx.biome_at_quart(origin.x >> 2, origin.y >> 2, origin.z >> 2);
                let sea = ctx.sea_level();
                ctx.terrain
                    .biomes
                    .data(biome)
                    .cold_enough_to_snow(origin.x, origin.y, origin.z, sea)
            };
            let kind = TemplatedKind::RuinedPortal {
                placement: setup.placement.clone(),
                cold,
                mossiness: setup.mossiness,
                air_pocket,
                overgrown: setup.overgrown,
                vines: setup.vines,
                blackstone: setup.replace_with_blackstone,
            };
            b.add(Box::new(TemplatedPiece::new(
                ctx,
                kind,
                id.clone(),
                origin,
                rotation,
                mirror,
                pivot,
            )));
        },
    ))
}

fn random_within(random: &mut dyn RandomSource, min_preferred: i32, max: i32) -> i32 {
    if min_preferred < max {
        random_between_inclusive(random, min_preferred, max)
    } else {
        max
    }
}

/// `RuinedPortalStructure.findSuitableY`.
fn find_suitable_y(
    ctx: &mut GenContext,
    placement: &str,
    air_pocket: bool,
    surface_y: i32,
    y_span: i32,
    bbox: &BoundingBox,
) -> i32 {
    let min_y = ctx.min_y + 15;
    let r = &mut ctx.random;
    let new_y = match placement {
        "in_nether" => {
            if air_pocket {
                random_between_inclusive(r, 32, 100)
            } else if r.next_float() < 0.5 {
                random_between_inclusive(r, 27, 29)
            } else {
                random_between_inclusive(r, 29, 100)
            }
        }
        "in_mountain" => random_within(r, 70, surface_y - y_span),
        "underground" => random_within(r, min_y, surface_y - y_span),
        "partly_buried" => surface_y - y_span + random_between_inclusive(r, 2, 8),
        _ => surface_y,
    };
    let corners = [
        (bbox.min_x, bbox.min_z),
        (bbox.max_x, bbox.min_z),
        (bbox.min_x, bbox.max_z),
        (bbox.max_x, bbox.max_z),
    ];
    let columns: Vec<(i32, Vec<BlockState>)> = corners.iter().map(|&(x, z)| ctx.base_column(x, z)).collect();
    let kind = if placement == "on_ocean_floor" {
        HeightmapKind::OceanFloorWg
    } else {
        HeightmapKind::WorldSurfaceWg
    };
    let env = &ctx.terrain.noise.env;
    let mut projected = new_y;
    while projected > min_y {
        let mut solid = 0;
        for (cmin, col) in &columns {
            let i = projected - cmin;
            let state = if i >= 0 && (i as usize) < col.len() {
                col[i as usize]
            } else {
                BlockState::AIR
            };
            if crate::chunk::is_opaque_in(env, kind, state) {
                solid += 1;
                if solid == 3 {
                    return projected;
                }
            }
        }
        projected -= 1;
    }
    projected
}

// --- nether fossil -------------------------------------------------------------------------------

/// `NetherFossilStructure.findGenerationPoint`.
pub fn nether_fossil<'c>(
    ctx: &mut GenContext,
    height: &crate::providers::HeightProvider,
) -> Option<Stub<'c>> {
    let x = ctx.min_block_x() + ctx.random.next_int_bounded(16);
    let z = ctx.min_block_z() + ctx.random.next_int_bounded(16);
    let sea_level = ctx.sea_level();
    let gen_ctx = ctx.terrain.generation_context_for(ctx.min_y, ctx.height);
    let mut y = height.sample(&mut ctx.random, &gen_ctx);
    let (cmin, column) = ctx.base_column(x, z);
    let blocks = &ctx.terrain.noise.env.blocks;
    let soul_sand = blocks.block_by_name(&mc("soul_sand"));
    let at = |y: i32| -> BlockState {
        let i = y - cmin;
        if i >= 0 && (i as usize) < column.len() {
            column[i as usize]
        } else {
            BlockState::AIR
        }
    };
    while y > sea_level {
        let current = at(y);
        y -= 1;
        let below = at(y);
        if blocks.flags(current).is_air()
            && (Some(blocks.block_of(below)) == soul_sand || blocks.flags(below).is_sturdy_up())
        {
            break;
        }
    }
    if y <= sea_level {
        return None;
    }
    let position = BlockPos::new(x, y, z);
    Some(Stub::lazy(
        position,
        move |ctx: &mut GenContext, b: &mut PiecesBuilder| {
            let rotation = Rotation::random(&mut ctx.random);
            let n = ctx.random.next_int_bounded(14) + 1;
            let id = mc(&format!("nether_fossils/fossil_{n}"));
            b.add(Box::new(TemplatedPiece::new(
                ctx,
                TemplatedKind::NetherFossil,
                id,
                position,
                rotation,
                Mirror::None,
                BlockPos::ZERO,
            )));
        },
    ))
}

// --- postProcess ---------------------------------------------------------------------------------

/// `RuinedPortalPiece.getBlockReplaceRule(source, probability, target)`.
fn replace_rule(
    source: chunker_data::blocks::BlockId,
    probability: Option<f32>,
    target: BlockState,
) -> ProcessorRule {
    let input = match probability {
        Some(p) => RuleTest::RandomBlock(source, p),
        None => RuleTest::Block(source),
    };
    ProcessorRule::new(
        input,
        RuleTest::AlwaysTrue,
        PosRule::AlwaysTrue,
        target,
        BlockEntityModifier::Passthrough,
    )
}

impl TemplatedPiece {
    /// `makeSettings` of the piece type.
    fn settings(&self, ctx: &PieceCtx) -> PlaceSettings {
        let env = ctx.region.env;
        let b = &env.b;
        let ignore_air = || Arc::new(Processor::BlockIgnore(vec![b.air, b.structure_block]));
        let ignore = || Arc::new(Processor::BlockIgnore(vec![b.structure_block]));
        let mut s = PlaceSettings {
            rotation: self.rotation,
            mirror: self.mirror,
            pivot: self.pivot,
            ..Default::default()
        };
        match &self.kind {
            TemplatedKind::Shipwreck { .. } | TemplatedKind::NetherFossil => s.processors.push(ignore_air()),
            TemplatedKind::Igloo => {
                s.processors.push(ignore());
                s.waterlogging = false;
            }
            TemplatedKind::OceanRuin { integrity, warm, .. } => {
                s.processors.push(Arc::new(Processor::BlockRot {
                    blocks: None,
                    integrity: *integrity,
                }));
                s.processors.push(ignore_air());
                // `archyRuleProcessor`: at most 5 sand/gravel blocks become suspicious.
                let (candidate, replacement, table) = if *warm {
                    (
                        b.sand,
                        env.blocks.vanilla_default("suspicious_sand"),
                        "minecraft:archaeology/ocean_ruin_warm",
                    )
                } else {
                    (
                        b.gravel,
                        env.blocks.vanilla_default("suspicious_gravel"),
                        "minecraft:archaeology/ocean_ruin_cold",
                    )
                };
                let rule = ProcessorRule::new(
                    RuleTest::Block(candidate),
                    RuleTest::AlwaysTrue,
                    PosRule::AlwaysTrue,
                    replacement,
                    BlockEntityModifier::AppendLoot(table.into()),
                );
                s.processors.push(Arc::new(Processor::Capped {
                    delegate: Box::new(Processor::Rule(vec![rule])),
                    limit: crate::providers::IntProvider::Constant(5),
                }));
            }
            TemplatedKind::RuinedPortal {
                placement,
                cold,
                mossiness,
                air_pocket,
                blackstone,
                ..
            } => {
                s.processors
                    .push(if *air_pocket { ignore() } else { ignore_air() });
                let state = |id| env.blocks.block(id).default_state;
                let mut rules = vec![replace_rule(b.gold_block, Some(0.3), BlockState::AIR)];
                rules.push(if placement == "on_ocean_floor" {
                    replace_rule(b.lava, None, state(b.magma_block))
                } else if *cold {
                    replace_rule(b.lava, None, state(b.netherrack))
                } else {
                    replace_rule(b.lava, Some(0.2), state(b.magma_block))
                });
                if !*cold {
                    rules.push(replace_rule(b.netherrack, Some(0.07), state(b.magma_block)));
                }
                s.processors.push(Arc::new(Processor::Rule(rules)));
                s.processors.push(Arc::new(Processor::BlockAge(*mossiness)));
                s.processors.push(Arc::new(Processor::ProtectedBlocks(
                    env.t.features_cannot_replace.clone(),
                )));
                s.processors.push(Arc::new(Processor::LavaSubmerged));
                if *blackstone {
                    s.processors.push(Arc::new(Processor::BlackstoneReplace));
                }
            }
        }
        s
    }

    fn post_process_impl(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        let template = ctx.structures.templates.get(&self.template);
        let mut settings = self.settings(ctx);
        let live = *self.live.lock();
        let (mut pos, mut adjusted) = live.unwrap_or((
            self.template_pos,
            matches!(
                self.kind,
                TemplatedKind::Shipwreck {
                    height_adjusted: true,
                    ..
                }
            ),
        ));
        match &self.kind {
            TemplatedKind::Shipwreck { beached, .. } => {
                let size = template.size;
                if !adjusted && !(size.0 > 32 || size.1 > 32) {
                    let kind = if *beached {
                        HeightmapKind::WorldSurfaceWg
                    } else {
                        HeightmapKind::OceanFloorWg
                    };
                    let mut min_y = ctx.region.max_y() + 1;
                    let mut mean = 0;
                    let base = size.0 * size.2;
                    if base == 0 {
                        mean = ctx.region.height(kind, pos.x, pos.z);
                    } else {
                        for z in pos.z..pos.z + size.2 {
                            for x in pos.x..pos.x + size.0 {
                                let h = ctx.region.height(kind, x, z);
                                mean += h;
                                min_y = min_y.min(h);
                            }
                        }
                        mean /= base;
                    }
                    let y = if *beached {
                        min_y - size.1 / 2 - random.next_int_bounded(3)
                    } else {
                        mean
                    };
                    adjusted = true;
                    pos = BlockPos::new(pos.x, y, pos.z);
                }
                *self.live.lock() = Some((pos, adjusted));
                self.place_template(ctx, random, &mut settings, pos);
            }
            TemplatedKind::Igloo => {
                let offset = match self.template.path() {
                    "igloo/top" => BlockPos::ZERO,
                    "igloo/middle" => BlockPos::new(2, -3, 4),
                    _ => BlockPos::new(0, -3, -2),
                };
                let entrance = pos.add(transform(
                    BlockPos::new(3 - offset.x, 0, -offset.z),
                    Mirror::None,
                    self.rotation,
                    self.pivot,
                ));
                let height = ctx
                    .region
                    .height(HeightmapKind::WorldSurfaceWg, entrance.x, entrance.z);
                let placed_at = pos.offset(0, height - 90 - 1, 0);
                self.place_template(ctx, random, &mut settings, placed_at);
                if self.template.path() == "igloo/top" {
                    let trapdoor = placed_at.add(transform(
                        BlockPos::new(3, 0, 5),
                        Mirror::None,
                        self.rotation,
                        self.pivot,
                    ));
                    let below = ctx.region.get(trapdoor.below());
                    let env = ctx.region.env;
                    if !env.blocks.flags(below).is_air() && env.blocks.block_of(below) != env.b.ladder {
                        let snow = ctx.state(env.b.snow_block);
                        ctx.region.set_and_update(trapdoor, snow);
                    }
                }
            }
            TemplatedKind::OceanRuin { .. } => {
                let h = ctx.region.height(HeightmapKind::OceanFloorWg, pos.x, pos.z);
                pos = BlockPos::new(pos.x, h, pos.z);
                let corner = transform(
                    BlockPos::new(template.size.0 - 1, 0, template.size.2 - 1),
                    Mirror::None,
                    self.rotation,
                    BlockPos::ZERO,
                )
                .add(pos);
                pos = BlockPos::new(pos.x, ocean_ruin_height(ctx, pos, corner), pos.z);
                *self.live.lock() = Some((pos, adjusted));
                self.place_template(ctx, random, &mut settings, pos);
            }
            TemplatedKind::NetherFossil => {
                let fossil_bb = template.bounding_box(pos, self.rotation, self.pivot, self.mirror);
                ctx.chunk_bb.encapsulate(&fossil_bb);
                self.place_template(ctx, random, &mut settings, pos);
                // `placeDriedGhast`
                let c = fossil_bb.center();
                let mut r = crate::random::LegacyRandom::new(ctx.region.env.seed)
                    .fork_positional()
                    .at(c.x, c.y, c.z);
                if r.next_float() < 0.5 {
                    let x = fossil_bb.min_x + r.next_int_bounded(fossil_bb.x_span());
                    let y = fossil_bb.min_y;
                    let z = fossil_bb.min_z + r.next_int_bounded(fossil_bb.z_span());
                    let p = BlockPos::new(x, y, z);
                    if ctx.region.flags(p).is_air() && ctx.chunk_bb.is_inside(p.x, p.y, p.z) {
                        let env = ctx.region.env;
                        let ghast = env.blocks.vanilla_default("dried_ghast");
                        let rotation = Rotation::random(&mut r);
                        ctx.region.set(p, env.blocks.rotate(ghast, rotation as u8), 2);
                    }
                }
            }
            TemplatedKind::RuinedPortal { .. } => {
                let bb = template.bounding_box(pos, self.rotation, self.pivot, self.mirror);
                let c = bb.center();
                if ctx.chunk_bb.is_inside(c.x, c.y, c.z) {
                    ctx.chunk_bb.encapsulate(&bb);
                    self.place_template(ctx, random, &mut settings, pos);
                    self.ruined_portal_extras(ctx, random, bb);
                }
            }
        }
    }

    fn place_template(
        &self,
        ctx: &mut PieceCtx,
        random: &mut dyn RandomSource,
        settings: &mut PlaceSettings,
        pos: BlockPos,
    ) {
        template_post_process(
            ctx,
            random,
            &self.template,
            pos,
            settings,
            &mut |ctx, random, id, p| self.handle_data_marker(ctx, random, id, p),
        );
    }

    /// `handleDataMarker`.
    fn handle_data_marker(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource, id: &str, pos: BlockPos) {
        let env = ctx.region.env;
        match &self.kind {
            TemplatedKind::Shipwreck { .. } => {
                let table = match id {
                    "map_chest" => "minecraft:chests/shipwreck_map",
                    "treasure_chest" => "minecraft:chests/shipwreck_treasure",
                    "supply_chest" => "minecraft:chests/shipwreck_supply",
                    _ => return,
                };
                crate::feature::kinds::set_loot_table(ctx.region, random, pos.below(), table);
            }
            TemplatedKind::Igloo if id == "chest" => {
                ctx.region.set_and_update(pos, BlockState::AIR);
                if ctx.region.is(pos.below(), env.b.chest) {
                    crate::feature::kinds::set_loot_table(
                        ctx.region,
                        random,
                        pos.below(),
                        "minecraft:chests/igloo_chest",
                    );
                }
            }
            TemplatedKind::OceanRuin { large, .. } => match id {
                "chest" => {
                    let water = ctx.region.fluid(pos).is_water();
                    let chest = ctx.state(env.b.chest);
                    let chest = env
                        .blocks
                        .with_property(chest, "waterlogged", if water { "true" } else { "false" })
                        .unwrap();
                    ctx.region.set(pos, chest, 2);
                    let table = if *large {
                        "minecraft:chests/underwater_ruin_big"
                    } else {
                        "minecraft:chests/underwater_ruin_small"
                    };
                    crate::feature::kinds::set_loot_table(ctx.region, random, pos, table);
                }
                "drowned" => {
                    crate::entity::spawn_mob(ctx.region, structure_mob("minecraft:drowned", pos));
                    let s = if pos.y > env.sea_level {
                        BlockState::AIR
                    } else {
                        ctx.state(env.b.water)
                    };
                    ctx.region.set(pos, s, 2);
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// The rest of `RuinedPortalPiece.postProcess`.
    fn ruined_portal_extras(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource, bb: BoundingBox) {
        let TemplatedKind::RuinedPortal {
            placement,
            cold,
            overgrown,
            vines,
            ..
        } = &self.kind
        else {
            return;
        };
        let env = ctx.region.env;
        let b = &env.b;
        let in_nether = placement == "in_nether";
        let can_replace = |ctx: &PieceCtx, p: BlockPos| {
            let s = ctx.region.get(p);
            let id = env.blocks.block_of(s);
            id != b.air
                && id != b.obsidian
                && !env.t.features_cannot_replace.contains(s)
                && (in_nether || id != b.lava)
        };
        let place = |ctx: &mut PieceCtx, random: &mut dyn RandomSource, p: BlockPos| {
            if can_replace(ctx, p) {
                let s = if !*cold && random.next_float() < 0.07 {
                    ctx.state(b.magma_block)
                } else {
                    ctx.state(b.netherrack)
                };
                ctx.region.set_and_update(p, s);
            }
        };
        let drip = |ctx: &mut PieceCtx, random: &mut dyn RandomSource, p: BlockPos| {
            let mut cur = p;
            place(ctx, random, cur);
            let mut cap = 8;
            while cap > 0 && random.next_float() < 0.5 {
                cur = cur.below();
                cap -= 1;
                place(ctx, random, cur);
            }
        };
        let leaves_above = |ctx: &mut PieceCtx, random: &mut dyn RandomSource, p: BlockPos| {
            if random.next_float() < 0.5
                && ctx.region.is(p, b.netherrack)
                && ctx.region.flags(p.above()).is_air()
            {
                let l = env
                    .blocks
                    .with_property(ctx.state(b.jungle_leaves), "persistent", "true")
                    .unwrap();
                ctx.region.set_and_update(p.above(), l);
            }
        };
        // `spreadNetherrack`
        let follow_ground = placement == "on_land_surface" || placement == "on_ocean_floor";
        let c = bb.center();
        let probs: [f32; 14] = [
            1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.9, 0.9, 0.8, 0.7, 0.6, 0.4, 0.2,
        ];
        let max_distance = probs.len() as i32;
        let average_width = (bb.x_span() + bb.z_span()) / 2;
        let adjustment = random.next_int_bounded((8 - average_width / 2).max(1));
        let heightmap = if placement == "on_ocean_floor" {
            HeightmapKind::OceanFloorWg
        } else {
            HeightmapKind::WorldSurfaceWg
        };
        for x in c.x - max_distance..=c.x + max_distance {
            for z in c.z - max_distance..=c.z + max_distance {
                let distance = (x - c.x).abs() + (z - c.z).abs();
                let adjusted = (distance + adjustment).max(0);
                if adjusted < max_distance && random.next_double() < probs[adjusted as usize] as f64 {
                    let surface = ctx.region.height(heightmap, x, z) - 1;
                    let y = if follow_ground {
                        surface
                    } else {
                        bb.min_y.min(surface)
                    };
                    let p = BlockPos::new(x, y, z);
                    if (y - bb.min_y).abs() <= 3 && can_replace(ctx, p) {
                        place(ctx, random, p);
                        if *overgrown {
                            leaves_above(ctx, random, p);
                        }
                        drip(ctx, random, p.below());
                    }
                }
            }
        }
        // `addNetherrackDripColumnsBelowPortal`
        for x in bb.min_x + 1..bb.max_x {
            for z in bb.min_z + 1..bb.max_z {
                let p = BlockPos::new(x, bb.min_y, z);
                if ctx.region.is(p, b.netherrack) {
                    drip(ctx, random, p.below());
                }
            }
        }
        if *vines || *overgrown {
            for z in bb.min_z..=bb.max_z {
                for y in bb.min_y..=bb.max_y {
                    for x in bb.min_x..=bb.max_x {
                        let p = BlockPos::new(x, y, z);
                        if *vines {
                            let s = ctx.region.get(p);
                            if !env.blocks.flags(s).is_air() && env.blocks.block_of(s) != b.vine {
                                let d = Direction::random_horizontal(random);
                                let n = p.relative(d, 1);
                                if ctx.region.flags(n).is_air()
                                    && env.blocks.flags(s).collision_face_full(d.data_3d() as usize)
                                {
                                    let v = env
                                        .blocks
                                        .with_property(ctx.state(b.vine), d.opposite().name(), "true")
                                        .unwrap();
                                    ctx.region.set_and_update(n, v);
                                }
                            }
                        }
                        if *overgrown {
                            leaves_above(ctx, random, p);
                        }
                    }
                }
            }
        }
    }
}

/// `OceanRuinPiece.getHeight`.
fn ocean_ruin_height(ctx: &PieceCtx, pos: BlockPos, corner: BlockPos) -> i32 {
    let env = ctx.region.env;
    let mut new_y = pos.y;
    let mut min_y = 512;
    let top_y = new_y - 1;
    let mut area = 0;
    let (x0, x1) = (pos.x.min(corner.x), pos.x.max(corner.x));
    let (z0, z1) = (pos.z.min(corner.z), pos.z.max(corner.z));
    for z in z0..=z1 {
        for x in x0..=x1 {
            let mut floor_y = pos.y - 1;
            loop {
                let p = BlockPos::new(x, floor_y, z);
                let s = ctx.region.get(p);
                let open =
                    env.blocks.flags(s).is_air() || ctx.region.fluid(p).is_water() || env.t.ice.contains(s);
                if !(open && floor_y > ctx.region.min_y() + 1) {
                    break;
                }
                floor_y -= 1;
            }
            min_y = min_y.min(floor_y);
            if floor_y < top_y - 2 {
                area += 1;
            }
        }
    }
    let width = (pos.x - corner.x).abs();
    if top_y - min_y > 2 && area > width - 2 {
        new_y = min_y + 1;
    }
    new_y
}

/// NBT of a mob spawned by a structure (`setPersistenceRequired`, `snapTo(pos)` at the bottom center of
/// the block); the server fills in defaults and a UUID when it loads the chunk.
pub fn structure_mob(id: &str, pos: BlockPos) -> chunker_nbt::Compound {
    let mut tag = chunker_nbt::Compound::new();
    tag.insert("id", id);
    let mut p = chunker_nbt::List::new();
    p.push(pos.x as f64 + 0.5);
    p.push(pos.y as f64);
    p.push(pos.z as f64 + 0.5);
    tag.insert("Pos", p);
    tag.insert("PersistenceRequired", true);
    tag
}

/// `TemplateStructurePiece.postProcess`: the template inside the chunk box, then its data markers
/// (`markers(metadata, pos)`) and jigsaw blocks (replaced by their final state).
pub fn template_post_process(
    ctx: &mut PieceCtx,
    random: &mut dyn RandomSource,
    template_id: &Identifier,
    pos: BlockPos,
    settings: &mut PlaceSettings,
    markers: &mut dyn FnMut(&mut PieceCtx, &mut dyn RandomSource, &str, BlockPos),
) {
    let template = ctx.structures.templates.get(template_id);
    settings.bounding_box = Some(ctx.chunk_bb);
    if !place_in_world(ctx.region, template, pos, ctx.reference, settings, random, 2) {
        return;
    }
    let env = ctx.region.env;
    for marker in filter_blocks(template, pos, settings, env.b.structure_block, &env.blocks) {
        let Some(nbt) = &marker.nbt else { continue };
        if nbt.get_str("mode") == Some("DATA") {
            let id = nbt.get_str("metadata").unwrap_or("").to_owned();
            markers(ctx, random, &id, marker.pos);
        }
    }
    for jigsaw in filter_blocks(template, pos, settings, env.b.jigsaw, &env.blocks) {
        let Some(nbt) = &jigsaw.nbt else { continue };
        let text = nbt.get_str("final_state").unwrap_or("minecraft:air");
        let text = text.split_once('{').map_or(text, |(s, _)| s);
        let state = env.blocks.parse_state_str(text).unwrap_or(BlockState::AIR);
        ctx.region.set_and_update(jigsaw.pos, state);
    }
}

/// NBT of an `ItemFrame(level, pos, facing)` holding `item` (the server completes it on load).
pub fn item_frame(pos: BlockPos, facing: Direction, item: &str) -> chunker_nbt::Compound {
    let mut tag = chunker_nbt::Compound::new();
    tag.insert("id", "minecraft:item_frame");
    let mut p = chunker_nbt::List::new();
    p.push(pos.x as f64 + 0.5 - facing.step_x() as f64 * 0.46875);
    p.push(pos.y as f64 + 0.5 - facing.step_y() as f64 * 0.46875);
    p.push(pos.z as f64 + 0.5 - facing.step_z() as f64 * 0.46875);
    tag.insert("Pos", p);
    tag.insert("block_pos", vec![pos.x, pos.y, pos.z]);
    tag.insert("Facing", facing.data_3d() as i8);
    let mut it = chunker_nbt::Compound::new();
    it.insert("id", item);
    it.insert("count", 1i32);
    tag.insert("Item", it);
    tag.insert("ItemDropChance", 1.0f32);
    tag.insert("ItemRotation", 0i8);
    tag
}
