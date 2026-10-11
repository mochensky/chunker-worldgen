//! Noise generators (`net.minecraft.world.level.levelgen.synth`, 26.3 float-based rewrite).

mod gradient;
mod normal;
mod perlin;
mod simplex;
mod stack;

pub use gradient::GradientNoise;
pub use normal::{NormalNoise, NormalNoiseParams, Normalization};
pub use perlin::{PerlinNoise, SmearedPerlinNoise};
pub use simplex::SimplexNoise;
pub use stack::{NoiseLayer, NoiseStack, OctaveNoise};

/// `PerlinNoise.STANDARD_DEVIATION`.
pub const PERLIN_STANDARD_DEVIATION: f64 = 0.2702247831245211;

mod blended;
pub use blended::{blended_noise_range, compile_blended_noise};
