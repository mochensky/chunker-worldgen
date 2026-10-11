use crate::random::RandomSource;

/// `GradientNoise.GRADIENT` (x, y, z).
pub(crate) const GRADIENT: [[i32; 3]; 16] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
    [1, 1, 0],
    [0, -1, 1],
    [-1, 1, 0],
    [0, -1, -1],
];

/// Float gradients for the hot path (`int * float` in Java converts the int exactly).
pub(crate) const GRADIENT_F32: [[f32; 3]; 16] = {
    let mut out = [[0.0f32; 3]; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = [
            GRADIENT[i][0] as f32,
            GRADIENT[i][1] as f32,
            GRADIENT[i][2] as f32,
        ];
        i += 1;
    }
    out
};

/// `HALF_ROUND_OFF = Math.nextDown(1.6777216E7)`.
const HALF_ROUND_OFF: f64 = 16_777_215.999_999_998;
const ROUND_OFF: f64 = 33_554_432.0;

/// Shared state of `GradientNoise`: permutation table and coordinate offsets.
#[derive(Clone)]
pub struct GradientNoise {
    pub(crate) perms: [u8; 256],
    pub offset_x: f64,
    pub offset_y: f64,
    pub offset_z: f64,
}

impl GradientNoise {
    pub fn new(random: &mut dyn RandomSource, offset_scale: f64) -> Self {
        let offset_x = random.next_double() * offset_scale;
        let offset_y = random.next_double() * offset_scale;
        let offset_z = random.next_double() * offset_scale;
        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let offset = random.next_int_bounded(256 - i as i32) as usize;
            perms.swap(i, offset + i);
        }
        Self {
            perms,
            offset_x,
            offset_y,
            offset_z,
        }
    }

    #[inline(always)]
    pub(crate) fn permute(&self, x: i32) -> i32 {
        self.perms[(x & 0xFF) as usize] as i32
    }

    #[inline(always)]
    pub(crate) fn grad(&self, x: i32) -> &'static [f32; 3] {
        &GRADIENT_F32[(self.permute(x) & 15) as usize]
    }

    /// `GradientNoise.wrap`.
    #[inline(always)]
    pub(crate) fn wrap(x: f64) -> f64 {
        if (-HALF_ROUND_OFF..HALF_ROUND_OFF).contains(&x) {
            x
        } else {
            x - (x / ROUND_OFF + 0.5).floor() * ROUND_OFF
        }
    }
}

/// `gradDot(hash, x, y, z)`.
#[inline(always)]
pub(crate) fn grad_dot(hash: i32, x: f32, y: f32, z: f32) -> f32 {
    let g = &GRADIENT_F32[(hash & 15) as usize];
    g[0] * x + g[1] * y + g[2] * z
}
