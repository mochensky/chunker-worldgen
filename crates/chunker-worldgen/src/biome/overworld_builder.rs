//! Port of `OverworldBiomeBuilder.addBiomes` (the `minecraft:overworld` multi-noise preset).
//!
//! The emission order matters: it defines the R-tree layout and therefore tie-breaking.

use super::climate::{Parameter as P, ParameterPoint, quantize};

type Out = Vec<(ParameterPoint, &'static str)>;

const N: Option<&str> = None;

struct Builder {
    full: P,
    temperatures: [P; 5],
    humidities: [P; 5],
    erosions: [P; 7],
    frozen: P,
    unfrozen: P,
    mushroom_fields_c: P,
    deep_ocean_c: P,
    ocean_c: P,
    coast_c: P,
    inland_c: P,
    near_inland_c: P,
    mid_inland_c: P,
    far_inland_c: P,
}

const OCEANS: [[&str; 5]; 2] = [
    [
        "deep_frozen_ocean",
        "deep_cold_ocean",
        "deep_ocean",
        "deep_lukewarm_ocean",
        "warm_ocean",
    ],
    [
        "frozen_ocean",
        "cold_ocean",
        "ocean",
        "lukewarm_ocean",
        "warm_ocean",
    ],
];

const MIDDLE: [[&str; 5]; 5] = [
    [
        "snowy_plains",
        "snowy_plains",
        "snowy_plains",
        "snowy_taiga",
        "taiga",
    ],
    ["plains", "plains", "forest", "taiga", "old_growth_spruce_taiga"],
    ["flower_forest", "plains", "forest", "birch_forest", "dark_forest"],
    ["savanna", "savanna", "forest", "jungle", "jungle"],
    ["desert", "desert", "desert", "desert", "desert"],
];

const MIDDLE_VARIANT: [[Option<&str>; 5]; 5] = [
    [Some("ice_spikes"), N, Some("snowy_taiga"), N, N],
    [Some("dappled_forest"), N, N, N, Some("old_growth_pine_taiga")],
    [Some("sunflower_plains"), N, N, Some("old_growth_birch_forest"), N],
    [N, N, Some("plains"), Some("sparse_jungle"), Some("bamboo_jungle")],
    [N, N, N, N, N],
];

const PLATEAU: [[&str; 5]; 5] = [
    [
        "snowy_plains",
        "snowy_plains",
        "snowy_plains",
        "snowy_taiga",
        "snowy_taiga",
    ],
    ["meadow", "meadow", "forest", "taiga", "old_growth_spruce_taiga"],
    ["meadow", "meadow", "meadow", "meadow", "pale_garden"],
    ["savanna_plateau", "savanna_plateau", "forest", "forest", "jungle"],
    [
        "badlands",
        "badlands",
        "badlands",
        "wooded_badlands",
        "wooded_badlands",
    ],
];

const PLATEAU_VARIANT: [[Option<&str>; 5]; 5] = [
    [Some("ice_spikes"), N, N, N, N],
    [
        Some("cherry_grove"),
        N,
        Some("meadow"),
        Some("meadow"),
        Some("old_growth_pine_taiga"),
    ],
    [
        Some("cherry_grove"),
        Some("cherry_grove"),
        Some("forest"),
        Some("birch_forest"),
        N,
    ],
    [N, N, N, N, N],
    [Some("eroded_badlands"), Some("eroded_badlands"), N, N, N],
];

const SHATTERED: [[Option<&str>; 5]; 5] = [
    [
        Some("windswept_gravelly_hills"),
        Some("windswept_gravelly_hills"),
        Some("windswept_hills"),
        Some("windswept_forest"),
        Some("windswept_forest"),
    ],
    [
        Some("windswept_gravelly_hills"),
        Some("windswept_gravelly_hills"),
        Some("windswept_hills"),
        Some("windswept_forest"),
        Some("windswept_forest"),
    ],
    [
        Some("windswept_hills"),
        Some("windswept_hills"),
        Some("windswept_hills"),
        Some("windswept_forest"),
        Some("windswept_forest"),
    ],
    [N, N, N, N, N],
    [N, N, N, N, N],
];

fn span(a: f32, b: f32) -> P {
    P::span(a, b)
}

fn sp(a: P, b: P) -> P {
    P::span_params(a, b)
}

fn point(t: P, h: P, c: P, e: P, d: P, w: P, offset: f32) -> ParameterPoint {
    ParameterPoint {
        temperature: t,
        humidity: h,
        continentalness: c,
        erosion: e,
        depth: d,
        weirdness: w,
        offset: quantize(offset),
    }
}

impl Builder {
    fn new() -> Self {
        let temperatures = [
            span(-1.0, -0.45),
            span(-0.45, -0.15),
            span(-0.15, 0.2),
            span(0.2, 0.55),
            span(0.55, 1.0),
        ];
        Self {
            full: span(-1.0, 1.0),
            temperatures,
            humidities: [
                span(-1.0, -0.35),
                span(-0.35, -0.1),
                span(-0.1, 0.1),
                span(0.1, 0.3),
                span(0.3, 1.0),
            ],
            erosions: [
                span(-1.0, -0.78),
                span(-0.78, -0.375),
                span(-0.375, -0.2225),
                span(-0.2225, 0.05),
                span(0.05, 0.45),
                span(0.45, 0.55),
                span(0.55, 1.0),
            ],
            frozen: temperatures[0],
            unfrozen: sp(temperatures[1], temperatures[4]),
            mushroom_fields_c: span(-1.2, -1.05),
            deep_ocean_c: span(-1.05, -0.455),
            ocean_c: span(-0.455, -0.19),
            coast_c: span(-0.19, -0.11),
            inland_c: span(-0.11, 0.55),
            near_inland_c: span(-0.11, 0.03),
            mid_inland_c: span(0.03, 0.3),
            far_inland_c: span(0.3, 1.0),
        }
    }

    fn surface(&self, out: &mut Out, t: P, h: P, c: P, e: P, w: P, offset: f32, biome: &'static str) {
        out.push((point(t, h, c, e, P::point(0.0), w, offset), biome));
        out.push((point(t, h, c, e, P::point(1.0), w, offset), biome));
    }

    fn underground(&self, out: &mut Out, t: P, h: P, c: P, e: P, w: P, offset: f32, biome: &'static str) {
        out.push((point(t, h, c, e, span(0.2, 0.9), w, offset), biome));
    }

    fn bottom(&self, out: &mut Out, t: P, h: P, c: P, e: P, w: P, offset: f32, biome: &'static str) {
        out.push((point(t, h, c, e, P::point(1.1), w, offset), biome));
    }

    fn add_biomes(&self, out: &mut Out) {
        self.off_coast(out);
        self.inland(out);
        self.underground_biomes(out);
    }

    fn off_coast(&self, out: &mut Out) {
        self.surface(
            out,
            self.full,
            self.full,
            self.mushroom_fields_c,
            self.full,
            self.full,
            0.0,
            "mushroom_fields",
        );
        for ti in 0..5 {
            let t = self.temperatures[ti];
            self.surface(
                out,
                t,
                self.full,
                self.deep_ocean_c,
                self.full,
                self.full,
                0.0,
                OCEANS[0][ti],
            );
            self.surface(
                out,
                t,
                self.full,
                self.ocean_c,
                self.full,
                self.full,
                0.0,
                OCEANS[1][ti],
            );
        }
    }

    fn inland(&self, out: &mut Out) {
        self.mid_slice(out, span(-1.0, -0.93333334));
        self.high_slice(out, span(-0.93333334, -0.7666667));
        self.peaks(out, span(-0.7666667, -0.56666666));
        self.high_slice(out, span(-0.56666666, -0.4));
        self.mid_slice(out, span(-0.4, -0.26666668));
        self.low_slice(out, span(-0.26666668, -0.05));
        self.valleys(out, span(-0.05, 0.05));
        self.low_slice(out, span(0.05, 0.26666668));
        self.mid_slice(out, span(0.26666668, 0.4));
        self.high_slice(out, span(0.4, 0.56666666));
        self.peaks(out, span(0.56666666, 0.7666667));
        self.high_slice(out, span(0.7666667, 0.93333334));
        self.mid_slice(out, span(0.93333334, 1.0));
    }

    fn peaks(&self, out: &mut Out, w: P) {
        let e = &self.erosions;
        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope =
                    self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let shattered_or_savanna = self.maybe_windswept_savanna(ti, hi, w, shattered);
                let peak = self.pick_peak(ti, hi, w);
                self.surface(out, t, h, sp(self.coast_c, self.far_inland_c), e[0], w, 0.0, peak);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.near_inland_c),
                    e[1],
                    w,
                    0.0,
                    middle_or_badlands_or_slope,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[1],
                    w,
                    0.0,
                    peak,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.near_inland_c),
                    sp(e[2], e[3]),
                    w,
                    0.0,
                    middle,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[2],
                    w,
                    0.0,
                    plateau,
                );
                self.surface(out, t, h, self.mid_inland_c, e[3], w, 0.0, middle_or_badlands);
                self.surface(out, t, h, self.far_inland_c, e[3], w, 0.0, plateau);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.far_inland_c),
                    e[4],
                    w,
                    0.0,
                    middle,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.near_inland_c),
                    e[5],
                    w,
                    0.0,
                    shattered_or_savanna,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[5],
                    w,
                    0.0,
                    shattered,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.far_inland_c),
                    e[6],
                    w,
                    0.0,
                    middle,
                );
            }
        }
    }

    fn high_slice(&self, out: &mut Out, w: P) {
        let e = &self.erosions;
        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope =
                    self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let slope = self.pick_slope(ti, hi, w);
                let peak = self.pick_peak(ti, hi, w);
                self.surface(out, t, h, self.coast_c, sp(e[0], e[1]), w, 0.0, middle);
                self.surface(out, t, h, self.near_inland_c, e[0], w, 0.0, slope);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[0],
                    w,
                    0.0,
                    peak,
                );
                self.surface(
                    out,
                    t,
                    h,
                    self.near_inland_c,
                    e[1],
                    w,
                    0.0,
                    middle_or_badlands_or_slope,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[1],
                    w,
                    0.0,
                    slope,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.near_inland_c),
                    sp(e[2], e[3]),
                    w,
                    0.0,
                    middle,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[2],
                    w,
                    0.0,
                    plateau,
                );
                self.surface(out, t, h, self.mid_inland_c, e[3], w, 0.0, middle_or_badlands);
                self.surface(out, t, h, self.far_inland_c, e[3], w, 0.0, plateau);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.far_inland_c),
                    e[4],
                    w,
                    0.0,
                    middle,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.near_inland_c),
                    e[5],
                    w,
                    0.0,
                    middle_or_savanna,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[5],
                    w,
                    0.0,
                    shattered,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.far_inland_c),
                    e[6],
                    w,
                    0.0,
                    middle,
                );
            }
        }
    }

    fn swamps(&self, out: &mut Out, w: P) {
        let e = &self.erosions;
        self.surface(
            out,
            self.full,
            self.full,
            self.coast_c,
            sp(e[0], e[2]),
            w,
            0.0,
            "stony_shore",
        );
        self.surface(
            out,
            sp(self.temperatures[1], self.temperatures[2]),
            self.full,
            sp(self.near_inland_c, self.far_inland_c),
            e[6],
            w,
            0.0,
            "swamp",
        );
        self.surface(
            out,
            sp(self.temperatures[3], self.temperatures[4]),
            self.full,
            sp(self.near_inland_c, self.far_inland_c),
            e[6],
            w,
            0.0,
            "mangrove_swamp",
        );
    }

    fn mid_slice(&self, out: &mut Out, w: P) {
        let e = &self.erosions;
        self.swamps(out, w);
        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope =
                    self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let beach = self.pick_beach(ti, hi);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let shattered_coast = self.pick_shattered_coast(ti, hi, w);
                let slope = self.pick_slope(ti, hi, w);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.near_inland_c, self.far_inland_c),
                    e[0],
                    w,
                    0.0,
                    slope,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.near_inland_c, self.mid_inland_c),
                    e[1],
                    w,
                    0.0,
                    middle_or_badlands_or_slope,
                );
                self.surface(
                    out,
                    t,
                    h,
                    self.far_inland_c,
                    e[1],
                    w,
                    0.0,
                    if ti == 0 { slope } else { plateau },
                );
                self.surface(out, t, h, self.near_inland_c, e[2], w, 0.0, middle);
                self.surface(out, t, h, self.mid_inland_c, e[2], w, 0.0, middle_or_badlands);
                self.surface(out, t, h, self.far_inland_c, e[2], w, 0.0, plateau);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.coast_c, self.near_inland_c),
                    e[3],
                    w,
                    0.0,
                    middle,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[3],
                    w,
                    0.0,
                    middle_or_badlands,
                );
                if w.max < 0 {
                    self.surface(out, t, h, self.coast_c, e[4], w, 0.0, beach);
                    self.surface(
                        out,
                        t,
                        h,
                        sp(self.near_inland_c, self.far_inland_c),
                        e[4],
                        w,
                        0.0,
                        middle,
                    );
                } else {
                    self.surface(
                        out,
                        t,
                        h,
                        sp(self.coast_c, self.far_inland_c),
                        e[4],
                        w,
                        0.0,
                        middle,
                    );
                }
                self.surface(out, t, h, self.coast_c, e[5], w, 0.0, shattered_coast);
                self.surface(out, t, h, self.near_inland_c, e[5], w, 0.0, middle_or_savanna);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[5],
                    w,
                    0.0,
                    shattered,
                );
                if w.max < 0 {
                    self.surface(out, t, h, self.coast_c, e[6], w, 0.0, beach);
                } else {
                    self.surface(out, t, h, self.coast_c, e[6], w, 0.0, middle);
                }
                if ti == 0 {
                    self.surface(
                        out,
                        t,
                        h,
                        sp(self.near_inland_c, self.far_inland_c),
                        e[6],
                        w,
                        0.0,
                        middle,
                    );
                }
            }
        }
    }

    fn low_slice(&self, out: &mut Out, w: P) {
        let e = &self.erosions;
        self.swamps(out, w);
        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope =
                    self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let beach = self.pick_beach(ti, hi);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let shattered_coast = self.pick_shattered_coast(ti, hi, w);
                self.surface(
                    out,
                    t,
                    h,
                    self.near_inland_c,
                    sp(e[0], e[1]),
                    w,
                    0.0,
                    middle_or_badlands,
                );
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    sp(e[0], e[1]),
                    w,
                    0.0,
                    middle_or_badlands_or_slope,
                );
                self.surface(out, t, h, self.near_inland_c, sp(e[2], e[3]), w, 0.0, middle);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    sp(e[2], e[3]),
                    w,
                    0.0,
                    middle_or_badlands,
                );
                self.surface(out, t, h, self.coast_c, sp(e[3], e[4]), w, 0.0, beach);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.near_inland_c, self.far_inland_c),
                    e[4],
                    w,
                    0.0,
                    middle,
                );
                self.surface(out, t, h, self.coast_c, e[5], w, 0.0, shattered_coast);
                self.surface(out, t, h, self.near_inland_c, e[5], w, 0.0, middle_or_savanna);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    e[5],
                    w,
                    0.0,
                    middle,
                );
                self.surface(out, t, h, self.coast_c, e[6], w, 0.0, beach);
                if ti == 0 {
                    self.surface(
                        out,
                        t,
                        h,
                        sp(self.near_inland_c, self.far_inland_c),
                        e[6],
                        w,
                        0.0,
                        middle,
                    );
                }
            }
        }
    }

    fn valleys(&self, out: &mut Out, w: P) {
        let e = &self.erosions;
        let neg = w.max < 0;
        self.surface(
            out,
            self.frozen,
            self.full,
            self.coast_c,
            sp(e[0], e[1]),
            w,
            0.0,
            if neg { "stony_shore" } else { "frozen_river" },
        );
        self.surface(
            out,
            self.unfrozen,
            self.full,
            self.coast_c,
            sp(e[0], e[1]),
            w,
            0.0,
            if neg { "stony_shore" } else { "river" },
        );
        self.surface(
            out,
            self.frozen,
            self.full,
            self.near_inland_c,
            sp(e[0], e[1]),
            w,
            0.0,
            "frozen_river",
        );
        self.surface(
            out,
            self.unfrozen,
            self.full,
            self.near_inland_c,
            sp(e[0], e[1]),
            w,
            0.0,
            "river",
        );
        self.surface(
            out,
            self.frozen,
            self.full,
            sp(self.coast_c, self.far_inland_c),
            sp(e[2], e[5]),
            w,
            0.0,
            "frozen_river",
        );
        self.surface(
            out,
            self.unfrozen,
            self.full,
            sp(self.coast_c, self.far_inland_c),
            sp(e[2], e[5]),
            w,
            0.0,
            "river",
        );
        self.surface(
            out,
            self.frozen,
            self.full,
            self.coast_c,
            e[6],
            w,
            0.0,
            "frozen_river",
        );
        self.surface(out, self.unfrozen, self.full, self.coast_c, e[6], w, 0.0, "river");
        self.surface(
            out,
            sp(self.temperatures[1], self.temperatures[2]),
            self.full,
            sp(self.inland_c, self.far_inland_c),
            e[6],
            w,
            0.0,
            "swamp",
        );
        self.surface(
            out,
            sp(self.temperatures[3], self.temperatures[4]),
            self.full,
            sp(self.inland_c, self.far_inland_c),
            e[6],
            w,
            0.0,
            "mangrove_swamp",
        );
        self.surface(
            out,
            self.frozen,
            self.full,
            sp(self.inland_c, self.far_inland_c),
            e[6],
            w,
            0.0,
            "frozen_river",
        );
        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let b = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                self.surface(
                    out,
                    t,
                    h,
                    sp(self.mid_inland_c, self.far_inland_c),
                    sp(e[0], e[1]),
                    w,
                    0.0,
                    b,
                );
            }
        }
    }

    fn underground_biomes(&self, out: &mut Out) {
        let e = &self.erosions;
        self.underground(
            out,
            self.full,
            self.full,
            span(0.8, 1.0),
            self.full,
            self.full,
            0.0,
            "dripstone_caves",
        );
        self.underground(
            out,
            self.full,
            span(0.7, 1.0),
            self.full,
            self.full,
            self.full,
            0.0,
            "lush_caves",
        );
        self.underground(
            out,
            self.full,
            self.full,
            sp(self.coast_c, self.inland_c),
            sp(e[5], e[6]),
            span(-1.1, -0.85),
            0.0,
            "sulfur_caves",
        );
        self.bottom(
            out,
            self.full,
            self.full,
            self.full,
            sp(e[0], e[1]),
            self.full,
            0.0,
            "deep_dark",
        );
    }

    fn pick_middle(&self, ti: usize, hi: usize, w: P) -> &'static str {
        if w.max < 0 {
            MIDDLE[ti][hi]
        } else {
            MIDDLE_VARIANT[ti][hi].unwrap_or(MIDDLE[ti][hi])
        }
    }

    fn pick_middle_or_badlands_if_hot(&self, ti: usize, hi: usize, w: P) -> &'static str {
        if ti == 4 {
            self.pick_badlands(hi, w)
        } else {
            self.pick_middle(ti, hi, w)
        }
    }

    fn pick_middle_or_badlands_if_hot_or_slope_if_cold(&self, ti: usize, hi: usize, w: P) -> &'static str {
        if ti == 0 {
            self.pick_slope(ti, hi, w)
        } else {
            self.pick_middle_or_badlands_if_hot(ti, hi, w)
        }
    }

    fn maybe_windswept_savanna(&self, ti: usize, hi: usize, w: P, underlying: &'static str) -> &'static str {
        if ti > 1 && hi < 4 && w.max >= 0 {
            "windswept_savanna"
        } else {
            underlying
        }
    }

    fn pick_shattered_coast(&self, ti: usize, hi: usize, w: P) -> &'static str {
        let b = if w.max >= 0 {
            self.pick_middle(ti, hi, w)
        } else {
            self.pick_beach(ti, hi)
        };
        self.maybe_windswept_savanna(ti, hi, w, b)
    }

    fn pick_beach(&self, ti: usize, _hi: usize) -> &'static str {
        if ti == 0 {
            "snowy_beach"
        } else if ti == 4 {
            "desert"
        } else {
            "beach"
        }
    }

    fn pick_badlands(&self, hi: usize, w: P) -> &'static str {
        if hi < 2 {
            if w.max < 0 { "badlands" } else { "eroded_badlands" }
        } else if hi < 3 {
            "badlands"
        } else {
            "wooded_badlands"
        }
    }

    fn pick_plateau(&self, ti: usize, hi: usize, w: P) -> &'static str {
        if w.max >= 0
            && let Some(v) = PLATEAU_VARIANT[ti][hi]
        {
            return v;
        }
        PLATEAU[ti][hi]
    }

    fn pick_peak(&self, ti: usize, hi: usize, w: P) -> &'static str {
        if ti <= 2 {
            if w.max < 0 { "jagged_peaks" } else { "frozen_peaks" }
        } else if ti == 3 {
            "stony_peaks"
        } else {
            self.pick_badlands(hi, w)
        }
    }

    fn pick_slope(&self, ti: usize, hi: usize, w: P) -> &'static str {
        if ti >= 3 {
            self.pick_plateau(ti, hi, w)
        } else if hi <= 1 {
            "snowy_slopes"
        } else {
            "grove"
        }
    }

    fn pick_shattered(&self, ti: usize, hi: usize, w: P) -> &'static str {
        SHATTERED[ti][hi].unwrap_or_else(|| self.pick_middle(ti, hi, w))
    }
}

/// The `minecraft:overworld` preset (biome paths without namespace).
pub fn overworld_parameters() -> Vec<(ParameterPoint, &'static str)> {
    let mut out = Vec::with_capacity(8000);
    Builder::new().add_biomes(&mut out);
    out
}

/// The `minecraft:nether` preset.
pub fn nether_parameters() -> Vec<(ParameterPoint, &'static str)> {
    let p = |t: f32, h: f32, offset: f32| {
        point(
            P::point(t),
            P::point(h),
            P::point(0.0),
            P::point(0.0),
            P::point(0.0),
            P::point(0.0),
            offset,
        )
    };
    vec![
        (p(0.0, 0.0, 0.0), "nether_wastes"),
        (p(0.0, -0.5, 0.0), "soul_sand_valley"),
        (p(0.4, 0.0, 0.0), "crimson_forest"),
        (p(0.0, 0.5, 0.375), "warped_forest"),
        (p(-0.5, 0.0, 0.175), "basalt_deltas"),
    ]
}
