//! Single-piece structures: buried treasure and the `ScatteredFeaturePiece` family
//! (swamp hut, desert pyramid, jungle temple).

use chunker_nbt::Compound;

use super::build::PieceCtx;
use super::geom::{BlockPos, BoundingBox, Direction};
use super::start::{PieceBase, PiecesBuilder};
use super::{GenContext, Stub};
use crate::chunk::HeightmapKind;
use crate::piece_boilerplate;
use crate::random::RandomSource;

/// `BuriedTreasurePieces.BuriedTreasurePiece`.
pub struct BuriedTreasurePiece {
    pub base: PieceBase,
}

impl super::start::Piece for BuriedTreasurePiece {
    piece_boilerplate!();
    fn type_id(&self) -> &'static str {
        "minecraft:btp"
    }
    fn save_extra(&self, _tag: &mut Compound) {}
    fn post_process_ported(&self) -> bool {
        true
    }

    /// `BuriedTreasurePiece.postProcess` (the box is only one block, so its update to the chest
    /// position never matters for another chunk).
    fn post_process(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        let bb = self.base.bbox;
        let y = ctx.region.height(HeightmapKind::OceanFloorWg, bb.min_x, bb.min_z);
        let mut pos = BlockPos::new(bb.min_x, y, bb.min_z);
        let env = ctx.region.env;
        let b = &env.b;
        let is_liquid = |id| id == b.water || id == b.lava;
        while pos.y > ctx.region.min_y() {
            let current = ctx.region.get(pos);
            let below = ctx.region.get(pos.below());
            let below_id = env.blocks.block_of(below);
            if [b.sandstone, b.stone, b.andesite, b.granite, b.diorite].contains(&below_id) {
                let cur_id = env.blocks.block_of(current);
                let soft = if !env.blocks.flags(current).is_air() && !is_liquid(cur_id) {
                    current
                } else {
                    ctx.state(b.sand)
                };
                for d in Direction::ALL {
                    let rel = pos.relative(d, 1);
                    let rs = ctx.region.get(rel);
                    if env.blocks.flags(rs).is_air() || is_liquid(env.blocks.block_of(rs)) {
                        let brs = ctx.region.get(rel.below());
                        if (env.blocks.flags(brs).is_air() || is_liquid(env.blocks.block_of(brs)))
                            && d != Direction::Up
                        {
                            ctx.region.set_and_update(rel, below);
                        } else {
                            ctx.region.set_and_update(rel, soft);
                        }
                    }
                }
                ctx.create_chest_at(random, pos, "minecraft:chests/buried_treasure", None);
                return;
            }
            pos = pos.below();
        }
    }
}

/// `BuriedTreasureStructure.findGenerationPoint`.
pub fn buried_treasure<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    if !ctx.could_valid_biome_exist_on_top_of_chunk_center() {
        return None;
    }
    let pos = ctx.top_of_chunk_center(HeightmapKind::OceanFloorWg);
    Some(Stub::lazy(pos, |ctx: &mut GenContext, b: &mut PiecesBuilder| {
        let offset = BlockPos::new(ctx.min_block_x() + 9, 90, ctx.min_block_z() + 9);
        b.add(Box::new(BuriedTreasurePiece {
            base: PieceBase::new(0, BoundingBox::of_pos(offset), None),
        }));
    }))
}

#[derive(Clone, Copy, Debug)]
pub enum ScatteredKind {
    SwampHut {
        witch: bool,
        cat: bool,
    },
    DesertPyramid {
        chests: [bool; 4],
    },
    JungleTemple {
        main_chest: bool,
        hidden_chest: bool,
        trap1: bool,
        trap2: bool,
    },
}

/// `ScatteredFeaturePiece` and its subclasses.
pub struct ScatteredPiece {
    pub base: PieceBase,
    pub width: i32,
    pub height: i32,
    pub depth: i32,
    pub height_position: i32,
    pub kind: ScatteredKind,
    /// Height, box and placed-chest flags as changed by `postProcess`.
    pub live: parking_lot::Mutex<Option<super::scattered::ScatteredLive>>,
    /// Desert pyramids: potential suspicious sand positions and the collapsed roof position.
    pub sand: parking_lot::Mutex<Option<(Vec<BlockPos>, BlockPos)>>,
}

impl ScatteredPiece {
    fn new(
        kind: ScatteredKind,
        west: i32,
        floor: i32,
        north: i32,
        width: i32,
        height: i32,
        depth: i32,
        d: Direction,
    ) -> Self {
        let bbox = PieceBase::make_bounding_box(west, floor, north, d, width, height, depth);
        Self {
            base: PieceBase::new(0, bbox, Some(d)),
            width,
            height,
            depth,
            height_position: -1,
            kind,
            live: parking_lot::Mutex::new(None),
            sand: parking_lot::Mutex::new(None),
        }
    }
}

impl super::start::Piece for ScatteredPiece {
    piece_boilerplate!();
    fn post_process_ported(&self) -> bool {
        true
    }
    fn post_process(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        self.post_process_scattered(ctx, random);
    }
    fn current_bbox(&self) -> BoundingBox {
        self.live.lock().as_ref().map_or(self.base.bbox, |l| l.bbox)
    }
    fn type_id(&self) -> &'static str {
        match self.kind {
            ScatteredKind::SwampHut { .. } => "minecraft:tesh",
            ScatteredKind::DesertPyramid { .. } => "minecraft:tedp",
            ScatteredKind::JungleTemple { .. } => "minecraft:tejp",
        }
    }
    fn save_extra(&self, tag: &mut Compound) {
        tag.insert("Width", self.width);
        tag.insert("Height", self.height);
        tag.insert("Depth", self.depth);
        tag.insert("HPos", self.height_position);
        match self.kind {
            ScatteredKind::SwampHut { witch, cat } => {
                tag.insert("Witch", witch);
                tag.insert("Cat", cat);
            }
            ScatteredKind::DesertPyramid { chests } => {
                for (i, c) in chests.iter().enumerate() {
                    tag.insert(format!("hasPlacedChest{i}"), *c);
                }
            }
            ScatteredKind::JungleTemple {
                main_chest,
                hidden_chest,
                trap1,
                trap2,
            } => {
                tag.insert("placedMainChest", main_chest);
                tag.insert("placedHiddenChest", hidden_chest);
                tag.insert("placedTrap1", trap1);
                tag.insert("placedTrap2", trap2);
            }
        }
    }
}

/// `SwampHutStructure.findGenerationPoint`.
pub fn swamp_hut<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    if !ctx.could_valid_biome_exist_on_top_of_chunk_center() {
        return None;
    }
    let pos = ctx.top_of_chunk_center(HeightmapKind::WorldSurfaceWg);
    Some(Stub::lazy(pos, |ctx: &mut GenContext, b: &mut PiecesBuilder| {
        let d = Direction::random_horizontal(&mut ctx.random);
        let kind = ScatteredKind::SwampHut {
            witch: false,
            cat: false,
        };
        b.add(Box::new(ScatteredPiece::new(
            kind,
            ctx.min_block_x(),
            64,
            ctx.min_block_z(),
            7,
            7,
            9,
            d,
        )));
    }))
}

/// `SinglePieceStructure.findGenerationPoint`.
fn single_piece<'c>(
    ctx: &mut GenContext,
    width: i32,
    depth: i32,
    make: fn(&mut GenContext) -> ScatteredPiece,
) -> Option<Stub<'c>> {
    if !ctx.could_valid_biome_exist_on_top_of_chunk_center() {
        return None;
    }
    if ctx.lowest_y(width, depth) < ctx.sea_level() {
        return None;
    }
    let pos = ctx.top_of_chunk_center(HeightmapKind::WorldSurfaceWg);
    Some(Stub::lazy(
        pos,
        move |ctx: &mut GenContext, b: &mut PiecesBuilder| b.add(Box::new(make(ctx))),
    ))
}

pub fn desert_pyramid<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    single_piece(ctx, 21, 21, |ctx| {
        let d = Direction::random_horizontal(&mut ctx.random);
        ScatteredPiece::new(
            ScatteredKind::DesertPyramid { chests: [false; 4] },
            ctx.min_block_x(),
            64,
            ctx.min_block_z(),
            21,
            15,
            21,
            d,
        )
    })
}

pub fn jungle_temple<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    single_piece(ctx, 12, 15, |ctx| {
        let d = Direction::random_horizontal(&mut ctx.random);
        let kind = ScatteredKind::JungleTemple {
            main_chest: false,
            hidden_chest: false,
            trap1: false,
            trap2: false,
        };
        ScatteredPiece::new(kind, ctx.min_block_x(), 64, ctx.min_block_z(), 12, 10, 15, d)
    })
}

/// `OceanMonumentPieces.MonumentBuilding` (only the top piece is stored; vanilla regenerates the
/// rooms from the seed after loading).
pub struct MonumentPiece {
    pub base: PieceBase,
    /// The building's rooms (built by vanilla's constructor from the start random).
    pub layout: std::sync::Arc<super::monument::Layout>,
}

impl super::start::Piece for MonumentPiece {
    piece_boilerplate!();
    fn type_id(&self) -> &'static str {
        "minecraft:omb"
    }
    fn save_extra(&self, _tag: &mut Compound) {}
    fn post_process_ported(&self) -> bool {
        true
    }
    fn post_process(&self, ctx: &mut PieceCtx, random: &mut dyn RandomSource) {
        super::monument::post_process_building(ctx, &self.base, random, &self.layout);
    }
}

/// `OceanMonumentStructure.findGenerationPoint`.
pub fn ocean_monument<'c>(
    ctx: &mut GenContext,
    surrounding: &rustc_hash::FxHashSet<crate::biome::BiomeId>,
) -> Option<Stub<'c>> {
    let (x, z) = (ctx.min_block_x() + 9, ctx.min_block_z() + 9);
    let y = ctx.sea_level();
    let r = 29;
    for qz in (z - r) >> 2..=(z + r) >> 2 {
        for qx in (x - r) >> 2..=(x + r) >> 2 {
            for qy in (y - r) >> 2..=(y + r) >> 2 {
                let b = ctx.biome_at_quart(qx, qy, qz);
                if !surrounding.contains(&b) {
                    return None;
                }
            }
        }
    }
    if !ctx.could_valid_biome_exist_on_top_of_chunk_center() {
        return None;
    }
    let pos = ctx.top_of_chunk_center(HeightmapKind::OceanFloorWg);
    Some(Stub::lazy(pos, |ctx: &mut GenContext, b: &mut PiecesBuilder| {
        let (west, north) = (ctx.min_block_x() - 29, ctx.min_block_z() - 29);
        let d = Direction::random_horizontal(&mut ctx.random);
        let bbox = PieceBase::make_bounding_box(west, 39, north, d, 58, 23, 58);
        let base = PieceBase::new(0, bbox, Some(d));
        let layout = std::sync::Arc::new(super::monument::build_layout(&mut ctx.random, &base));
        b.add(Box::new(MonumentPiece { base, layout }));
    }))
}
