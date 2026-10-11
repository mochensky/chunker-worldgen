//! `Heightmap`: per-column first free y above the topmost "opaque" block of a predicate.

use chunker_data::blocks::BlockState;

use super::ProtoChunk;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeightmapKind {
    WorldSurfaceWg,
    WorldSurface,
    OceanFloorWg,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

impl HeightmapKind {
    pub const WORLDGEN: [HeightmapKind; 2] = [HeightmapKind::OceanFloorWg, HeightmapKind::WorldSurfaceWg];
    pub const FINAL: [HeightmapKind; 4] = [
        HeightmapKind::OceanFloor,
        HeightmapKind::WorldSurface,
        HeightmapKind::MotionBlocking,
        HeightmapKind::MotionBlockingNoLeaves,
    ];
    /// Enum order, used for `EnumMap` iteration (serialization order).
    pub const ALL: [HeightmapKind; 6] = [
        HeightmapKind::WorldSurfaceWg,
        HeightmapKind::WorldSurface,
        HeightmapKind::OceanFloorWg,
        HeightmapKind::OceanFloor,
        HeightmapKind::MotionBlocking,
        HeightmapKind::MotionBlockingNoLeaves,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Parses a `Heightmap.Types` serialization key (`"OCEAN_FLOOR"`).
    pub fn from_key(key: &str) -> anyhow::Result<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.key() == key)
            .ok_or_else(|| anyhow::anyhow!("unknown heightmap type {key}"))
    }

    pub fn key(self) -> &'static str {
        match self {
            HeightmapKind::WorldSurfaceWg => "WORLD_SURFACE_WG",
            HeightmapKind::WorldSurface => "WORLD_SURFACE",
            HeightmapKind::OceanFloorWg => "OCEAN_FLOOR_WG",
            HeightmapKind::OceanFloor => "OCEAN_FLOOR",
            HeightmapKind::MotionBlocking => "MOTION_BLOCKING",
            HeightmapKind::MotionBlockingNoLeaves => "MOTION_BLOCKING_NO_LEAVES",
        }
    }
}

/// All heightmaps of a chunk, stored as `height - minY` per column like vanilla's bit storage.
#[derive(Clone)]
pub struct Heightmaps {
    min_y: i32,
    pub height: i32,
    data: [Option<Box<[i32; 256]>>; 6],
}

impl Heightmaps {
    pub fn new(min_y: i32, height: i32) -> Self {
        Self {
            min_y,
            height,
            data: Default::default(),
        }
    }

    pub fn is_primed(&self, kind: HeightmapKind) -> bool {
        self.data[kind.index()].is_some()
    }

    /// `getOrCreateHeightmapUnprimed`.
    pub fn ensure(&mut self, kind: HeightmapKind) {
        if self.data[kind.index()].is_none() {
            self.data[kind.index()] = Some(Box::new([0; 256]));
        }
    }

    #[inline]
    pub fn first_available(&self, kind: HeightmapKind, x: i32, z: i32) -> i32 {
        self.data[kind.index()]
            .as_ref()
            .map_or(0, |d| d[(x + z * 16) as usize])
            + self.min_y
    }

    #[inline]
    pub fn set_height(&mut self, kind: HeightmapKind, x: i32, z: i32, height: i32) {
        let min_y = self.min_y;
        self.data[kind.index()].as_mut().expect("heightmap exists")[(x + z * 16) as usize] = height - min_y;
    }

    /// Raw `height - minY` values for serialization.
    pub fn raw(&self, kind: HeightmapKind) -> Option<&[i32; 256]> {
        self.data[kind.index()].as_deref()
    }
}

#[inline]
pub(crate) fn is_opaque(chunk: &ProtoChunk, kind: HeightmapKind, state: BlockState) -> bool {
    is_opaque_in(&chunk.env, kind, state)
}

/// `Heightmap.Types.isOpaque()` predicate of `kind`.
#[inline]
pub fn is_opaque_in(env: &super::BlockEnv, kind: HeightmapKind, state: BlockState) -> bool {
    let f = env.blocks.flags(state);
    match kind {
        HeightmapKind::WorldSurfaceWg | HeightmapKind::WorldSurface => !f.is_air(),
        HeightmapKind::OceanFloorWg | HeightmapKind::OceanFloor => env.motion_blocking.contains(state),
        HeightmapKind::MotionBlocking => env.motion_blocking.contains(state) || f.has_fluid(),
        HeightmapKind::MotionBlockingNoLeaves => {
            env.motion_blocking_no_leaves.contains(state) || f.has_fluid()
        }
    }
}

/// `Heightmap.update(localX, y, localZ, state)`.
pub(crate) fn update(
    chunk: &mut ProtoChunk,
    kind: HeightmapKind,
    x: i32,
    y: i32,
    z: i32,
    state: BlockState,
) -> bool {
    let first = chunk.heightmaps.first_available(kind, x, z);
    if y <= first - 2 {
        return false;
    }
    if is_opaque(chunk, kind, state) {
        if y >= first {
            chunk.heightmaps.set_height(kind, x, z, y + 1);
            return true;
        }
    } else if first - 1 == y {
        let mut yy = y - 1;
        while yy >= chunk.min_y {
            if is_opaque(chunk, kind, chunk.get_block(x, yy, z)) {
                chunk.heightmaps.set_height(kind, x, z, yy + 1);
                return true;
            }
            yy -= 1;
        }
        chunk.heightmaps.set_height(kind, x, z, chunk.min_y);
        return true;
    }
    false
}

/// `Heightmap.primeHeightmaps(chunk, types)`.
pub(crate) fn prime(chunk: &mut ProtoChunk, kinds: &[HeightmapKind]) {
    if kinds.is_empty() {
        return;
    }
    for &k in kinds {
        chunk.heightmaps.ensure(k);
    }
    let top = chunk.highest_section_position() + 16;
    let mut remaining: Vec<HeightmapKind> = Vec::with_capacity(kinds.len());
    for x in 0..16 {
        for z in 0..16 {
            remaining.clear();
            remaining.extend_from_slice(kinds);
            let mut y = top - 1;
            while y >= chunk.min_y {
                let state = chunk.get_block(x, y, z);
                if state != BlockState::AIR {
                    let mut i = 0;
                    while i < remaining.len() {
                        if is_opaque(chunk, remaining[i], state) {
                            chunk.heightmaps.set_height(remaining[i], x, z, y + 1);
                            remaining.remove(i);
                        } else {
                            i += 1;
                        }
                    }
                    if remaining.is_empty() {
                        break;
                    }
                }
                y -= 1;
            }
        }
    }
}
