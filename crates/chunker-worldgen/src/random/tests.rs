//! Reference values produced by running the vanilla 26.3 classes through the Java oracle.

use super::*;

#[test]
fn xoroshiro_matches_vanilla() {
    let mut x = XoroshiroRandom::new(12345);
    assert_eq!(x.next_long(), -8118485274630516485);
    assert_eq!(x.next_int(), -778892930);
    assert_eq!(x.next_int_bounded(100), 87);
    assert_eq!(x.next_int_bounded(7), 0);
    assert_eq!(x.next_float().to_bits(), 1058029652);
    assert_eq!(x.next_double().to_bits() as i64, 4596064642917773428);
    assert_eq!(x.next_gaussian().to_bits() as i64, 4607015788133847531);
    let pf = x.fork_positional();
    assert_eq!(pf.at(10, -20, 30).next_long(), 6981475222541544058);
    assert_eq!(
        pf.from_hash_of("minecraft:terrain").next_long(),
        619033806811502851
    );
}

#[test]
fn legacy_matches_vanilla() {
    let mut l = LegacyRandom::new(-98765);
    assert_eq!(l.next_long(), 5643789298210576898);
    assert_eq!(l.next_int_bounded(100), 80);
    assert_eq!(l.next_int_bounded(64), 35);
    assert_eq!(l.next_double().to_bits() as i64, 4605557496302129941);
    assert_eq!(l.next_gaussian().to_bits() as i64, 4592370792183230468);
    let pf = l.fork_positional();
    assert_eq!(
        pf.from_hash_of("minecraft:terrain").next_long(),
        7594408597133209584
    );
}

#[test]
fn worldgen_random_matches_vanilla() {
    let mut w = WorldgenRandom::new(AnyRandom::Xoroshiro(XoroshiroRandom::new(777)));
    w.set_large_feature_seed(12345, -3, 7);
    assert_eq!(w.next_int_bounded(10), 4);
    assert_eq!(w.next_double().to_bits() as i64, 4601308650074168712);
    assert_eq!(w.set_decoration_seed(12345, -48, 112), 5237629200623607001);
    assert_eq!(w.next_float().to_bits(), 1043038416);
}

#[test]
fn helpers_match_vanilla() {
    assert_eq!(mix_stafford13(123456789), -1000775671988880032);
    assert_eq!(crate::math::get_seed(123, -45, 678), -49831574764955);
}
