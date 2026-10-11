//! `Beardifier`: terrain adaptation around structures (bound into density functions through the
//! `beardifier` context key).

use std::sync::{Arc, OnceLock};

use crate::math::Interval;
use crate::structure::geom::BoundingBox;
use crate::structure::jigsaw::{JigsawPiece, Junction, Projection};
use crate::structure::registry::TerrainAdjustment;
use crate::structure::start::StructureStart;

/// `Beardifier.RANGE`.
pub const RANGE: Interval = Interval::INFINITE;

/// `Beardifier.Rigid`.
#[derive(Clone, Debug)]
pub struct Rigid {
    pub bbox: BoundingBox,
    pub adjustment: TerrainAdjustment,
    pub ground_level_delta: i32,
}

#[derive(Debug, Default)]
pub struct Beardifier {
    pub rigids: Vec<Rigid>,
    pub junctions: Vec<Junction>,
    pub affected: Option<BoundingBox>,
}

/// `BEARD_KERNEL`: `(float) Math.pow(E, -distanceSqr / 16.0)` over a 24³ grid.
fn kernel() -> &'static [f32; 13824] {
    static KERNEL: OnceLock<Box<[f32; 13824]>> = OnceLock::new();
    KERNEL.get_or_init(|| {
        let mut k = Box::new([0.0f32; 13824]);
        for zi in 0..24 {
            for xi in 0..24 {
                for yi in 0..24 {
                    let (dx, dy, dz) = ((xi - 12) as f64, (yi - 12) as f64 + 0.5, (zi - 12) as f64);
                    let d = dx * dx + dy * dy + dz * dz;
                    k[(zi * 24 * 24 + xi * 24 + yi) as usize] = std::f64::consts::E.powf(-d / 16.0) as f32;
                }
            }
        }
        k
    })
}

/// The beard kernel table (exposed for the oracle comparison).
pub fn beard_kernel() -> &'static [f32; 13824] {
    kernel()
}

/// `Mth.fastInvSqrt(double)`.
fn fast_inv_sqrt(x: f64) -> f64 {
    let xhalf = 0.5 * x;
    let i = 6910469410427058090i64.wrapping_sub((x.to_bits() as i64) >> 1);
    let x = f64::from_bits(i as u64);
    x * (1.5 - xhalf * x * x)
}

fn bury_contribution(dx: f32, dy: f32, dz: f32) -> f32 {
    let d = dx * dx + dy * dy + dz * dz;
    if d >= 36.0 {
        0.0
    } else {
        1.0 - (d as f64).sqrt() as f32 / 6.0
    }
}

fn beard_contribution(dx: i32, dy: i32, dz: i32, y_to_ground: i32) -> f32 {
    let (xi, yi, zi) = (dx + 12, dy + 12, dz + 12);
    let in_range = |v: i32| (0..24).contains(&v);
    if !(in_range(xi) && in_range(yi) && in_range(zi)) {
        return 0.0;
    }
    let dy_offset = y_to_ground as f32 + 0.5;
    let distance = (dx as f32) * (dx as f32) + dy_offset * dy_offset + (dz as f32) * (dz as f32);
    let value = -dy_offset * fast_inv_sqrt((distance / 2.0) as f64) as f32 / 2.0;
    value * kernel()[(zi * 24 * 24 + xi * 24 + yi) as usize]
}

impl Beardifier {
    /// `Beardifier.forStructuresInChunk` over the starts referenced by the chunk (in a fixed order:
    /// vanilla iterates an identity-hash map here, which is itself not deterministic).
    pub fn for_starts<'a>(
        starts: impl Iterator<Item = (&'a StructureStart, TerrainAdjustment)>,
        chunk_x: i32,
        chunk_z: i32,
    ) -> Option<Beardifier> {
        let (cx, cz) = (chunk_x << 4, chunk_z << 4);
        let mut rigids = Vec::new();
        let mut junctions = Vec::new();
        let mut any: Option<BoundingBox> = None;
        let include = |b: BoundingBox, any: &mut Option<BoundingBox>| {
            *any = Some(match any {
                Some(a) => {
                    let mut a = *a;
                    a.encapsulate(&b);
                    a
                }
                None => b,
            });
        };
        for (start, adjustment) in starts {
            if adjustment == TerrainAdjustment::None {
                continue;
            }
            for piece in &start.pieces {
                let bb = *piece.bbox();
                // `isCloseToChunk(chunkPos, 12)`
                if !bb.intersects_xz(cx - 12, cz - 12, cx + 15 + 12, cz + 15 + 12) {
                    continue;
                }
                if let Some(pool) = piece.as_any().downcast_ref::<JigsawPiece>() {
                    if pool.element.projection == Projection::Rigid {
                        rigids.push(Rigid {
                            bbox: bb,
                            adjustment,
                            ground_level_delta: pool.ground_level_delta,
                        });
                        include(bb, &mut any);
                    }
                    for j in &pool.junctions {
                        if j.source_x > cx - 12
                            && j.source_z > cz - 12
                            && j.source_x < cx + 15 + 12
                            && j.source_z < cz + 15 + 12
                        {
                            junctions.push(j.clone());
                            let jb = BoundingBox::new(
                                j.source_x,
                                j.source_ground_y,
                                j.source_z,
                                j.source_x,
                                j.source_ground_y,
                                j.source_z,
                            );
                            include(jb, &mut any);
                        }
                    }
                } else {
                    rigids.push(Rigid {
                        bbox: bb,
                        adjustment,
                        ground_level_delta: 0,
                    });
                    include(bb, &mut any);
                }
            }
        }
        let affected = any?.inflated_by(24);
        Some(Beardifier {
            rigids,
            junctions,
            affected: Some(affected),
        })
    }

    pub fn into_sampler(self) -> Arc<crate::density::sampler::Sampler> {
        Arc::new(crate::density::sampler::Sampler::Beardifier(Arc::new(self)))
    }

    /// `sampleValueUnchecked`.
    pub fn value_unchecked(&self, x: i32, y: i32, z: i32) -> f32 {
        let mut noise = 0.0f32;
        for r in &self.rigids {
            let b = r.bbox;
            let dx = 0.max((b.min_x - x).max(x - b.max_x));
            let dz = 0.max((b.min_z - z).max(z - b.max_z));
            let ground_y = b.min_y + r.ground_level_delta;
            let dy_to_ground = y - ground_y;
            let dy = match r.adjustment {
                TerrainAdjustment::None => 0,
                TerrainAdjustment::Bury | TerrainAdjustment::BeardThin => dy_to_ground,
                TerrainAdjustment::BeardBox => 0.max((ground_y - y).max(y - b.max_y)),
                TerrainAdjustment::Encapsulate => 0.max((b.min_y - y).max(y - b.max_y)),
            };
            noise += match r.adjustment {
                TerrainAdjustment::None => 0.0,
                TerrainAdjustment::Bury => bury_contribution(dx as f32, dy as f32 / 2.0, dz as f32),
                TerrainAdjustment::BeardThin | TerrainAdjustment::BeardBox => {
                    beard_contribution(dx, dy, dz, dy_to_ground) * 0.8
                }
                TerrainAdjustment::Encapsulate => {
                    bury_contribution(dx as f32 / 2.0, dy as f32 / 2.0, dz as f32 / 2.0) * 0.8
                }
            };
        }
        for j in &self.junctions {
            let (dx, dy, dz) = (x - j.source_x, y - j.source_ground_y, z - j.source_z);
            noise += beard_contribution(dx, dy, dz, dy) * 0.4;
        }
        noise
    }

    /// `sampleValue`.
    pub fn sample_value(&self, x: i32, y: i32, z: i32) -> f32 {
        match &self.affected {
            Some(a) if a.is_inside(x, y, z) => self.value_unchecked(x, y, z),
            _ => 0.0,
        }
    }

    /// `sampleVolume`.
    pub fn sample_volume(&self, out: &mut [f32], vol: &crate::density::DensityVolume) {
        out.fill(0.0);
        let Some(a) = self.affected else { return };
        if !vol.intersects_box(a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z) {
            return;
        }
        let fd = crate::math::floor_div;
        let min_x = fd(0.max(a.min_x - vol.min_x), vol.step_x);
        let min_y = fd(0.max(a.min_y - vol.min_y), vol.step_y);
        let min_z = fd(0.max(a.min_z - vol.min_z), vol.step_z);
        let max_x = (vol.size_x - 1).min(fd(a.max_x - vol.min_x, vol.step_x));
        let max_y = (vol.size_y - 1).min(fd(a.max_y - vol.min_y, vol.step_y));
        let max_z = (vol.size_z - 1).min(fd(a.max_z - vol.min_z, vol.step_z));
        for z in min_z..=max_z {
            let bz = vol.block_z(z);
            for x in min_x..=max_x {
                let bx = vol.block_x(x);
                for y in min_y..=max_y {
                    out[vol.index(x, y, z)] = self.value_unchecked(bx, vol.block_y(y), bz);
                }
            }
        }
    }
}
