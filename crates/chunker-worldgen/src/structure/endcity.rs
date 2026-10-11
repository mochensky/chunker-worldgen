//! `EndCityStructure` + `EndCityPieces` (recursive template layout and NBT).

use chunker_data::Identifier;
use chunker_nbt::Compound;

use super::geom::{BlockPos, Direction, Mirror, Rotation};
use super::start::{PieceBase, PiecesBuilder};
use super::template::{TemplateManager, transform};
use super::{GenContext, Stub};
use crate::piece_boilerplate;
use crate::random::RandomSource;

/// `EndCityPieces.EndCityPiece`.
#[derive(Clone)]
pub struct EcPiece {
    pub base: PieceBase,
    pub template_name: &'static str,
    pub template_pos: BlockPos,
    pub rotation: Rotation,
    pub overwrite: bool,
}

impl super::start::Piece for EcPiece {
    piece_boilerplate!();
    fn type_id(&self) -> &'static str {
        "minecraft:ecp"
    }
    fn save_extra(&self, tag: &mut Compound) {
        tag.insert("TPX", self.template_pos.x);
        tag.insert("TPY", self.template_pos.y);
        tag.insert("TPZ", self.template_pos.z);
        tag.insert("Template", self.template_name);
        tag.insert("Rot", self.rotation.name());
        tag.insert("OW", self.overwrite);
    }
    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.move_by(dx, dy, dz);
        self.template_pos = self.template_pos.offset(dx, dy, dz);
    }
    fn post_process_ported(&self) -> bool {
        true
    }
    /// `TemplateStructurePiece.postProcess` with `EndCityPiece.makeSettings` / `handleDataMarker`.
    fn post_process(&self, ctx: &mut super::build::PieceCtx, random: &mut dyn RandomSource) {
        let b = &ctx.region.env.b;
        let ignore = if self.overwrite {
            vec![b.structure_block]
        } else {
            vec![b.air, b.structure_block]
        };
        let mut settings = super::place::PlaceSettings {
            ignore_entities: true,
            rotation: self.rotation,
            processors: vec![std::sync::Arc::new(super::processor::Processor::BlockIgnore(
                ignore,
            ))],
            ..Default::default()
        };
        let id = Identifier::vanilla(&format!("end_city/{}", self.template_name));
        let rotation = self.rotation;
        super::templated::template_post_process(
            ctx,
            random,
            &id,
            self.template_pos,
            &mut settings,
            &mut |ctx, random, marker, pos| {
                let bb = ctx.chunk_bb;
                if marker.starts_with("Chest") {
                    let chest = pos.below();
                    if bb.is_inside(chest.x, chest.y, chest.z) {
                        crate::feature::kinds::set_loot_table(
                            ctx.region,
                            random,
                            chest,
                            "minecraft:chests/end_city_treasure",
                        );
                    }
                } else if bb.is_inside(pos.x, pos.y, pos.z) {
                    if marker.starts_with("Sentry") {
                        let mut tag = super::templated::structure_mob("minecraft:shulker", pos);
                        tag.remove("PersistenceRequired");
                        tag.insert(
                            "Rotation",
                            crate::entity::initial_rotation(ctx.region.env.seed, pos),
                        );
                        ctx.region.add_entity(pos.x, pos.z, tag);
                    } else if marker.starts_with("Elytra") {
                        let frame = super::templated::item_frame(
                            pos,
                            rotation.rotate(Direction::South),
                            "minecraft:elytra",
                        );
                        ctx.region.add_entity(pos.x, pos.z, frame);
                    }
                }
            },
        );
    }
}

#[derive(Clone, Copy)]
enum Section {
    HouseTower,
    Tower,
    TowerBridge,
    FatTower,
}

const TOWER_BRIDGES: [(Rotation, BlockPos); 4] = [
    (Rotation::None, BlockPos::new(1, -1, 0)),
    (Rotation::Clockwise90, BlockPos::new(6, -1, 1)),
    (Rotation::CounterClockwise90, BlockPos::new(0, -1, 5)),
    (Rotation::Clockwise180, BlockPos::new(5, -1, 6)),
];

const FAT_TOWER_BRIDGES: [(Rotation, BlockPos); 4] = [
    (Rotation::None, BlockPos::new(4, -1, 0)),
    (Rotation::Clockwise90, BlockPos::new(12, -1, 4)),
    (Rotation::CounterClockwise90, BlockPos::new(0, -1, 8)),
    (Rotation::Clockwise180, BlockPos::new(8, -1, 12)),
];

struct Gen<'a> {
    tm: &'a TemplateManager,
    random: &'a mut dyn RandomSource,
    ship_created: bool,
}

impl Gen<'_> {
    fn piece(&self, name: &'static str, pos: BlockPos, rotation: Rotation, overwrite: bool) -> EcPiece {
        let id = Identifier::vanilla(&format!("end_city/{name}"));
        let bbox = self
            .tm
            .get(&id)
            .bounding_box(pos, rotation, BlockPos::ZERO, Mirror::None);
        EcPiece {
            base: PieceBase::new(0, bbox, Some(Direction::North)),
            template_name: name,
            template_pos: pos,
            rotation,
            overwrite,
        }
    }

    /// `addPiece`: a child positioned relative to `parent` by `offset` (in the parent's rotation).
    fn add_piece(
        &self,
        parent: &EcPiece,
        offset: BlockPos,
        name: &'static str,
        rotation: Rotation,
        overwrite: bool,
    ) -> EcPiece {
        let mut child = self.piece(name, parent.template_pos, rotation, overwrite);
        let origin = transform(offset, Mirror::None, parent.rotation, BlockPos::ZERO);
        super::start::Piece::move_by(&mut child, origin.x, origin.y, origin.z);
        child
    }

    /// `addHelper`: appends and returns the index.
    fn push(pieces: &mut Vec<EcPiece>, p: EcPiece) -> usize {
        pieces.push(p);
        pieces.len() - 1
    }

    fn recursive_children(
        &mut self,
        section: Section,
        gen_depth: i32,
        parent: &EcPiece,
        offset: Option<BlockPos>,
        pieces: &mut Vec<EcPiece>,
    ) -> bool {
        if gen_depth > 8 {
            return false;
        }
        let mut children = Vec::new();
        if !self.generate(section, gen_depth, parent, offset, &mut children) {
            return false;
        }
        let tag = self.random.next_int();
        let mut collision = false;
        for child in &mut children {
            child.base.gen_depth = tag;
            if let Some(c) = pieces.iter().find(|p| p.base.bbox.intersects(&child.base.bbox))
                && c.base.gen_depth != parent.base.gen_depth
            {
                collision = true;
                break;
            }
        }
        if collision {
            return false;
        }
        pieces.extend(children);
        true
    }

    fn generate(
        &mut self,
        section: Section,
        gen_depth: i32,
        parent: &EcPiece,
        offset: Option<BlockPos>,
        pieces: &mut Vec<EcPiece>,
    ) -> bool {
        let rotation = parent.rotation;
        match section {
            Section::HouseTower => {
                if gen_depth > 8 {
                    return false;
                }
                let o = offset.unwrap_or(BlockPos::ZERO);
                let mut last = Self::push(pieces, self.add_piece(parent, o, "base_floor", rotation, true));
                let floors = self.random.next_int_bounded(3);
                if floors == 0 {
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-1, 4, -1),
                        "base_roof",
                        rotation,
                        true,
                    );
                    Self::push(pieces, p);
                } else if floors == 1 {
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-1, 0, -1),
                        "second_floor_2",
                        rotation,
                        false,
                    );
                    last = Self::push(pieces, p);
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-1, 8, -1),
                        "second_roof",
                        rotation,
                        false,
                    );
                    last = Self::push(pieces, p);
                    let parent = pieces[last].clone();
                    self.recursive_children(Section::Tower, gen_depth + 1, &parent, None, pieces);
                } else if floors == 2 {
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-1, 0, -1),
                        "second_floor_2",
                        rotation,
                        false,
                    );
                    last = Self::push(pieces, p);
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-1, 4, -1),
                        "third_floor_2",
                        rotation,
                        false,
                    );
                    last = Self::push(pieces, p);
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-1, 8, -1),
                        "third_roof",
                        rotation,
                        true,
                    );
                    last = Self::push(pieces, p);
                    let parent = pieces[last].clone();
                    self.recursive_children(Section::Tower, gen_depth + 1, &parent, None, pieces);
                }
                true
            }
            Section::Tower => {
                let (a, b) = (self.random.next_int_bounded(2), self.random.next_int_bounded(2));
                let p = self.add_piece(
                    parent,
                    BlockPos::new(3 + a, -3, 3 + b),
                    "tower_base",
                    rotation,
                    true,
                );
                let mut last = Self::push(pieces, p);
                let p = self.add_piece(
                    &pieces[last],
                    BlockPos::new(0, 7, 0),
                    "tower_piece",
                    rotation,
                    true,
                );
                last = Self::push(pieces, p);
                let mut bridge = if self.random.next_int_bounded(3) == 0 {
                    Some(last)
                } else {
                    None
                };
                let height = 1 + self.random.next_int_bounded(3);
                for i in 0..height {
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(0, 4, 0),
                        "tower_piece",
                        rotation,
                        true,
                    );
                    last = Self::push(pieces, p);
                    if i < height - 1 && self.random.next_bool() {
                        bridge = Some(last);
                    }
                }
                if let Some(bridge) = bridge {
                    for (rot, pos) in TOWER_BRIDGES {
                        if self.random.next_bool() {
                            let p =
                                self.add_piece(&pieces[bridge], pos, "bridge_end", rotation.then(rot), true);
                            let start = Self::push(pieces, p);
                            let parent = pieces[start].clone();
                            self.recursive_children(
                                Section::TowerBridge,
                                gen_depth + 1,
                                &parent,
                                None,
                                pieces,
                            );
                        }
                    }
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-1, 4, -1),
                        "tower_top",
                        rotation,
                        true,
                    );
                    Self::push(pieces, p);
                } else {
                    if gen_depth != 7 {
                        let parent = pieces[last].clone();
                        return self.recursive_children(
                            Section::FatTower,
                            gen_depth + 1,
                            &parent,
                            None,
                            pieces,
                        );
                    }
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-1, 4, -1),
                        "tower_top",
                        rotation,
                        true,
                    );
                    Self::push(pieces, p);
                }
                true
            }
            Section::TowerBridge => {
                let length = self.random.next_int_bounded(4) + 1;
                let p = self.add_piece(parent, BlockPos::new(0, 0, -4), "bridge_piece", rotation, true);
                let mut last = Self::push(pieces, p);
                pieces[last].base.gen_depth = -1;
                let mut next_y = 0;
                for _ in 0..length {
                    if self.random.next_bool() {
                        let p = self.add_piece(
                            &pieces[last],
                            BlockPos::new(0, next_y, -4),
                            "bridge_piece",
                            rotation,
                            true,
                        );
                        last = Self::push(pieces, p);
                        next_y = 0;
                    } else {
                        let p = if self.random.next_bool() {
                            self.add_piece(
                                &pieces[last],
                                BlockPos::new(0, next_y, -4),
                                "bridge_steep_stairs",
                                rotation,
                                true,
                            )
                        } else {
                            self.add_piece(
                                &pieces[last],
                                BlockPos::new(0, next_y, -8),
                                "bridge_gentle_stairs",
                                rotation,
                                true,
                            )
                        };
                        last = Self::push(pieces, p);
                        next_y = 4;
                    }
                }
                if !self.ship_created && self.random.next_int_bounded(10 - gen_depth) == 0 {
                    let (sx, sz) = (self.random.next_int_bounded(8), self.random.next_int_bounded(10));
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(-8 + sx, next_y, -70 + sz),
                        "ship",
                        rotation,
                        true,
                    );
                    Self::push(pieces, p);
                    self.ship_created = true;
                } else {
                    let parent = pieces[last].clone();
                    if !self.recursive_children(
                        Section::HouseTower,
                        gen_depth + 1,
                        &parent,
                        Some(BlockPos::new(-3, next_y + 1, -11)),
                        pieces,
                    ) {
                        return false;
                    }
                }
                let p = self.add_piece(
                    &pieces[last],
                    BlockPos::new(4, next_y, 0),
                    "bridge_end",
                    rotation.then(Rotation::Clockwise180),
                    true,
                );
                last = Self::push(pieces, p);
                pieces[last].base.gen_depth = -1;
                true
            }
            Section::FatTower => {
                let p = self.add_piece(parent, BlockPos::new(-3, 4, -3), "fat_tower_base", rotation, true);
                let mut last = Self::push(pieces, p);
                let p = self.add_piece(
                    &pieces[last],
                    BlockPos::new(0, 4, 0),
                    "fat_tower_middle",
                    rotation,
                    true,
                );
                last = Self::push(pieces, p);
                let mut i = 0;
                while i < 2 && self.random.next_int_bounded(3) != 0 {
                    let p = self.add_piece(
                        &pieces[last],
                        BlockPos::new(0, 8, 0),
                        "fat_tower_middle",
                        rotation,
                        true,
                    );
                    last = Self::push(pieces, p);
                    for (rot, pos) in FAT_TOWER_BRIDGES {
                        if self.random.next_bool() {
                            let p =
                                self.add_piece(&pieces[last], pos, "bridge_end", rotation.then(rot), true);
                            let start = Self::push(pieces, p);
                            let parent = pieces[start].clone();
                            self.recursive_children(
                                Section::TowerBridge,
                                gen_depth + 1,
                                &parent,
                                None,
                                pieces,
                            );
                        }
                    }
                    i += 1;
                }
                let p = self.add_piece(
                    &pieces[last],
                    BlockPos::new(-2, 8, -2),
                    "fat_tower_top",
                    rotation,
                    true,
                );
                Self::push(pieces, p);
                true
            }
        }
    }
}

/// `EndCityStructure.findGenerationPoint`.
pub fn find_generation_point<'c>(ctx: &mut GenContext) -> Option<Stub<'c>> {
    let (bx, bz) = (ctx.min_block_x() + 7, ctx.min_block_z() + 7);
    if !ctx.could_valid_biome_exist_in_terrain_column(bx, bz) {
        return None;
    }
    let rotation = Rotation::random(&mut ctx.random);
    let (ox, oz) = match rotation {
        Rotation::Clockwise90 => (-5, 5),
        Rotation::Clockwise180 => (-5, -5),
        Rotation::CounterClockwise90 => (5, -5),
        Rotation::None => (5, 5),
    };
    let start = BlockPos::new(bx, ctx.lowest_y_at(bx, bz, ox, oz), bz);
    if start.y < 60 {
        return None;
    }
    Some(Stub::lazy(
        start,
        move |ctx: &mut GenContext, b: &mut PiecesBuilder| {
            let tm = ctx.structures.templates.clone();
            let mut g = Gen {
                tm: &tm,
                random: &mut ctx.random,
                ship_created: false,
            };
            let mut pieces = Vec::new();
            let mut last = Gen::push(&mut pieces, g.piece("base_floor", start, rotation, true));
            let p = g.add_piece(
                &pieces[last],
                BlockPos::new(-1, 0, -1),
                "second_floor_1",
                rotation,
                false,
            );
            last = Gen::push(&mut pieces, p);
            let p = g.add_piece(
                &pieces[last],
                BlockPos::new(-1, 4, -1),
                "third_floor_1",
                rotation,
                false,
            );
            last = Gen::push(&mut pieces, p);
            let p = g.add_piece(
                &pieces[last],
                BlockPos::new(-1, 8, -1),
                "third_roof",
                rotation,
                true,
            );
            last = Gen::push(&mut pieces, p);
            let parent = pieces[last].clone();
            g.recursive_children(Section::Tower, 1, &parent, None, &mut pieces);
            for p in pieces {
                b.add(Box::new(p));
            }
        },
    ))
}
