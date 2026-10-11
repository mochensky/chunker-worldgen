//! `StructurePiece` / `StructureStart` and their NBT form (`StructurePiece.createTag`).

use std::any::Any;

use chunker_nbt::{Compound, List};

use super::geom::{BoundingBox, Direction, Mirror, Rotation};

/// Fields every `StructurePiece` has.
#[derive(Clone, Debug)]
pub struct PieceBase {
    pub bbox: BoundingBox,
    pub orientation: Option<Direction>,
    pub gen_depth: i32,
}

impl PieceBase {
    pub fn new(gen_depth: i32, bbox: BoundingBox, orientation: Option<Direction>) -> Self {
        Self {
            bbox,
            orientation,
            gen_depth,
        }
    }

    /// The mirror implied by `setOrientation`.
    pub fn mirror(&self) -> Mirror {
        match self.orientation {
            Some(Direction::South) | Some(Direction::West) => Mirror::LeftRight,
            _ => Mirror::None,
        }
    }

    /// The rotation implied by `setOrientation`.
    pub fn rotation(&self) -> Rotation {
        match self.orientation {
            Some(Direction::West) | Some(Direction::East) => Rotation::Clockwise90,
            _ => Rotation::None,
        }
    }

    /// `getWorldX(x, z)`.
    pub fn world_x(&self, x: i32, z: i32) -> i32 {
        match self.orientation {
            Some(Direction::North) | Some(Direction::South) => self.bbox.min_x + x,
            Some(Direction::West) => self.bbox.max_x - z,
            Some(Direction::East) => self.bbox.min_x + z,
            _ => x,
        }
    }

    /// `getWorldY(y)`.
    pub fn world_y(&self, y: i32) -> i32 {
        if self.orientation.is_none() {
            y
        } else {
            y + self.bbox.min_y
        }
    }

    /// `getWorldZ(x, z)`.
    pub fn world_z(&self, x: i32, z: i32) -> i32 {
        match self.orientation {
            Some(Direction::North) => self.bbox.max_z - z,
            Some(Direction::South) => self.bbox.min_z + z,
            Some(Direction::West) | Some(Direction::East) => self.bbox.min_z + x,
            _ => z,
        }
    }

    /// `makeBoundingBox(x, y, z, direction, width, height, depth)`.
    pub fn make_bounding_box(
        x: i32,
        y: i32,
        z: i32,
        d: Direction,
        width: i32,
        height: i32,
        depth: i32,
    ) -> BoundingBox {
        if d.axis() == super::geom::Axis::Z {
            BoundingBox::new(x, y, z, x + width - 1, y + height - 1, z + depth - 1)
        } else {
            BoundingBox::new(x, y, z, x + depth - 1, y + height - 1, z + width - 1)
        }
    }
}

/// A generated structure piece. Implementations mirror one vanilla `StructurePiece` subclass.
pub trait Piece: Send + Sync + 'static {
    fn base(&self) -> &PieceBase;
    fn base_mut(&mut self) -> &mut PieceBase;
    /// Registry id (`StructurePieceType`), e.g. `minecraft:msroom`.
    fn type_id(&self) -> &'static str;
    /// `addAdditionalSaveData`.
    fn save_extra(&self, tag: &mut Compound);
    /// `StructurePiece.move` (template pieces also move their template position).
    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base_mut().bbox.move_by(dx, dy, dz);
    }
    /// `postProcess(level, structureManager, generator, random, chunkBB, chunkPos, referencePos)`.
    fn post_process(&self, _ctx: &mut super::build::PieceCtx, _random: &mut dyn crate::random::RandomSource) {
    }
    /// Whether `post_process` is ported (unported pieces place nothing).
    fn post_process_ported(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn bbox(&self) -> &BoundingBox {
        &self.base().bbox
    }

    /// The bounding box as `postProcess` of an earlier chunk may have moved it (scattered pieces).
    fn current_bbox(&self) -> BoundingBox {
        *self.bbox()
    }

    /// `createTag`.
    fn to_nbt(&self) -> Compound {
        let b = self.base();
        let mut tag = Compound::new();
        tag.insert("id", self.type_id());
        tag.insert("BB", b.bbox.to_array());
        tag.insert("O", b.orientation.map_or(-1, Direction::data_2d));
        tag.insert("GD", b.gen_depth);
        self.save_extra(&mut tag);
        tag
    }
}

/// Implements the boilerplate accessors of [`Piece`] for a struct with a `base: PieceBase` field.
#[macro_export]
macro_rules! piece_boilerplate {
    () => {
        fn base(&self) -> &$crate::structure::start::PieceBase {
            &self.base
        }
        fn base_mut(&mut self) -> &mut $crate::structure::start::PieceBase {
            &mut self.base
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}

/// `StructurePiecesBuilder`.
#[derive(Default)]
pub struct PiecesBuilder {
    pub pieces: Vec<Box<dyn Piece>>,
}

impl PiecesBuilder {
    pub fn add(&mut self, piece: Box<dyn Piece>) {
        self.pieces.push(piece);
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// `findCollisionPiece`: the first piece whose box intersects `bbox`.
    pub fn find_collision(&self, bbox: &BoundingBox) -> Option<usize> {
        self.pieces.iter().position(|p| p.bbox().intersects(bbox))
    }

    /// `getBoundingBox` (`StructurePiece.createBoundingBox`).
    pub fn bounding_box(&self) -> BoundingBox {
        BoundingBox::encapsulating(self.pieces.iter().map(|p| p.bbox()))
            .expect("Unable to calculate boundingbox without pieces")
    }

    /// `offsetPiecesVertically`.
    pub fn offset_vertically(&mut self, dy: i32) {
        for p in &mut self.pieces {
            p.move_by(0, dy, 0);
        }
    }

    /// `moveBelowSeaLevel`.
    pub fn move_below_sea_level(
        &mut self,
        sea_level: i32,
        min_y: i32,
        random: &mut dyn crate::random::RandomSource,
        offset: i32,
    ) -> i32 {
        let max_y = sea_level - offset;
        let bbox = self.bounding_box();
        let mut y1 = bbox.y_span() + min_y + 1;
        if y1 < max_y {
            y1 += random.next_int_bounded(max_y - y1);
        }
        let dy = y1 - bbox.max_y;
        self.offset_vertically(dy);
        dy
    }

    /// `moveInsideHeights`.
    pub fn move_inside_heights(
        &mut self,
        random: &mut dyn crate::random::RandomSource,
        lowest: i32,
        highest: i32,
    ) {
        let bbox = self.bounding_box();
        let span = highest - lowest + 1 - bbox.y_span();
        let y0 = if span > 1 {
            lowest + random.next_int_bounded(span)
        } else {
            lowest
        };
        let dy = y0 - bbox.min_y;
        self.offset_vertically(dy);
    }
}

/// A valid `StructureStart` (invalid starts are simply not stored).
pub struct StructureStart {
    /// Index into [`super::registry::StructureRegistry::structures`].
    pub structure: usize,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub references: i32,
    pub pieces: Vec<Box<dyn Piece>>,
    /// `getBoundingBox()` (pieces box, inflated by 12 for terrain-adapting structures).
    pub bbox: BoundingBox,
}

impl StructureStart {
    pub fn new(
        structure: usize,
        adapts_terrain: bool,
        chunk_x: i32,
        chunk_z: i32,
        references: i32,
        pieces: Vec<Box<dyn Piece>>,
    ) -> Self {
        let raw = BoundingBox::encapsulating(pieces.iter().map(|p| p.bbox())).expect("start without pieces");
        let bbox = if adapts_terrain { raw.inflated_by(12) } else { raw };
        Self {
            structure,
            chunk_x,
            chunk_z,
            references,
            pieces,
            bbox,
        }
    }

    /// `createTag(context, chunkPos)`.
    pub fn to_nbt(&self, id: &str) -> Compound {
        let mut tag = Compound::new();
        tag.insert("id", id);
        tag.insert("ChunkX", self.chunk_x);
        tag.insert("ChunkZ", self.chunk_z);
        tag.insert("references", self.references);
        let mut children = List::new();
        for p in &self.pieces {
            children.push(p.to_nbt());
        }
        tag.insert("Children", children);
        tag
    }
}
