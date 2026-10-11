use super::perlin::{PerlinNoise, SmearedPerlinNoise};
use crate::density::DensityVolume;
use crate::math::Interval;

/// The octave noise types that appear inside a [`NoiseStack`].
#[derive(Clone)]
pub enum OctaveNoise {
    Perlin(PerlinNoise),
    Smeared(SmearedPerlinNoise),
}

impl OctaveNoise {
    fn range(&self) -> Interval {
        match self {
            OctaveNoise::Perlin(_) => PerlinNoise::range(),
            OctaveNoise::Smeared(s) => SmearedPerlinNoise::range_for(s.fudge_y_scale),
        }
    }

    #[inline(always)]
    pub fn get(&self, x: f64, y: f64, z: f64) -> f32 {
        match self {
            OctaveNoise::Perlin(p) => p.get(x, y, z),
            OctaveNoise::Smeared(s) => s.get(x, y, z),
        }
    }

    #[inline(always)]
    pub fn get2(&self, x: f64, y: f64) -> f32 {
        match self {
            OctaveNoise::Perlin(p) => p.get2(x, y),
            OctaveNoise::Smeared(s) => s.get2(x, y),
        }
    }

    #[inline]
    fn add_to_volume(&self, buf: &mut [f32], volume: &DensityVolume, xz: f64, y: f64, amplitude: f32) {
        match self {
            OctaveNoise::Perlin(p) => p.add_to_volume(buf, volume, xz, y, amplitude),
            OctaveNoise::Smeared(s) => s.add_to_volume(buf, volume, xz, y, amplitude),
        }
    }
}

#[derive(Clone)]
pub struct NoiseLayer {
    pub noise: OctaveNoise,
    pub frequency: f64,
    pub amplitude: f32,
}

/// `NoiseStack`: sum of scaled octaves. All vanilla `Noise` instances used by worldgen are stacks.
#[derive(Clone)]
pub struct NoiseStack {
    pub layers: Vec<NoiseLayer>,
    range: Interval,
}

impl NoiseStack {
    pub fn new(layers: Vec<NoiseLayer>) -> Self {
        let mut range = Interval::of_exact(0.0);
        for layer in &layers {
            let layer_range = Interval::mul(layer.noise.range(), Interval::of_exact(layer.amplitude));
            range = Interval::add(range, layer_range);
        }
        Self { layers, range }
    }

    pub fn range(&self) -> Interval {
        self.range
    }

    #[inline]
    pub fn get(&self, x: f64, y: f64, z: f64) -> f32 {
        let mut value = 0.0f32;
        for layer in &self.layers {
            let f = layer.frequency;
            value += layer.amplitude * layer.noise.get(x * f, y * f, z * f);
        }
        value
    }

    #[inline]
    pub fn get2(&self, x: f64, y: f64) -> f32 {
        let mut value = 0.0f32;
        for layer in &self.layers {
            let f = layer.frequency;
            value += layer.amplitude * layer.noise.get2(x * f, y * f);
        }
        value
    }

    /// `NoiseStack.addToVolume`.
    pub fn add_to_volume(
        &self,
        buf: &mut [f32],
        volume: &DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) {
        for layer in &self.layers {
            let f = layer.frequency;
            layer.noise.add_to_volume(
                buf,
                volume,
                xz_scale * f,
                y_scale * f,
                amplitude * layer.amplitude,
            );
        }
    }

    /// `NoiseStack.Builder.addStack`.
    pub fn push_stack(layers: &mut Vec<NoiseLayer>, stack: &NoiseStack, frequency: f64, amplitude: f32) {
        for layer in &stack.layers {
            layers.push(NoiseLayer {
                noise: layer.noise.clone(),
                frequency: layer.frequency * frequency,
                amplitude: layer.amplitude * amplitude,
            });
        }
    }
}

impl std::fmt::Debug for NoiseStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NoiseStack({} layers)", self.layers.len())
    }
}
