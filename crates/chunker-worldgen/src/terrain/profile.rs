//! Cheap global phase timers for finding generation hot spots (see the `bench_terrain` example).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub enum Phase {
    Biomes,
    NoiseSetup,
    Fill,
    Surface,
    Carvers,
    Heightmaps,
    Serialize,
    FillDensity,
    SurfaceCompile,
    Features,
}

pub const PHASES: [Phase; 10] = [
    Phase::Biomes,
    Phase::NoiseSetup,
    Phase::Fill,
    Phase::Surface,
    Phase::Carvers,
    Phase::Heightmaps,
    Phase::Serialize,
    Phase::FillDensity,
    Phase::SurfaceCompile,
    Phase::Features,
];

static NANOS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

pub struct Timer(Instant);

impl Timer {
    pub fn start() -> Self {
        Timer(Instant::now())
    }

    /// Adds the time since the last lap to `phase` and restarts.
    pub fn lap(self, phase: Phase) -> Timer {
        let now = Instant::now();
        NANOS[phase as usize].fetch_add(now.duration_since(self.0).as_nanos() as u64, Ordering::Relaxed);
        Timer(now)
    }
}

/// Accumulated CPU seconds per phase since start (summed over threads).
pub fn snapshot() -> Vec<(Phase, f64)> {
    PHASES
        .iter()
        .map(|&p| (p, NANOS[p as usize].load(Ordering::Relaxed) as f64 / 1e9))
        .collect()
}

pub fn reset() {
    for n in &NANOS {
        n.store(0, Ordering::Relaxed);
    }
}
