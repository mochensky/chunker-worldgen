//! Small vanilla helpers shared by structure code (`Util.shuffle`, `WorldgenRandom` constructors).

use crate::random::{AnyRandom, LegacyRandom, RandomSource, WorldgenRandom};

/// `Util.shuffle(list, random)`.
pub fn shuffle<T>(list: &mut [T], random: &mut dyn RandomSource) {
    let mut i = list.len();
    while i > 1 {
        let swap_to = random.next_int_bounded(i as i32) as usize;
        list.swap(i - 1, swap_to);
        i -= 1;
    }
}

/// `new WorldgenRandom(new LegacyRandomSource(0L))`.
pub fn legacy_worldgen_random() -> WorldgenRandom {
    WorldgenRandom::new(AnyRandom::Legacy(LegacyRandom::new(0)))
}

/// `Util.getRandom(list, random)`.
pub fn pick<'a, T>(list: &'a [T], random: &mut dyn RandomSource) -> &'a T {
    &list[random.next_int_bounded(list.len() as i32) as usize]
}
