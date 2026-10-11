//! Java collection semantics that leak into worldgen results.
//!
//! Vanilla iterates `HashSet<BlockPos>`s while consuming randomness or mutating the world (tree
//! decorators, leaf distance propagation), so the iteration order of `java.util.HashMap` has to be
//! reproduced: power-of-two tables, `h ^ (h >>> 16)` spreading, tail insertion, order preserving
//! splits on resize.

use crate::structure::geom::BlockPos;

/// `Object.hashCode()` of a key.
pub trait JavaHash {
    fn java_hash(&self) -> i32;
}

impl JavaHash for BlockPos {
    /// `Vec3i.hashCode`.
    #[inline]
    fn java_hash(&self) -> i32 {
        (self.y.wrapping_add(self.z.wrapping_mul(31)))
            .wrapping_mul(31)
            .wrapping_add(self.x)
    }
}

/// `HashMap.hash`.
#[inline]
fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// `java.util.HashSet` (backed by `HashMap` with default load factor 0.75).
///
/// Tree bins (chains of 8+ entries in tables of 64+ buckets) reorder iteration in vanilla; they
/// cannot occur for the small, spatially compact sets worldgen builds, so they are not modelled and
/// a debug assertion guards the assumption.
#[derive(Clone)]
pub struct JavaHashSet<T> {
    table: Vec<Vec<(u32, T)>>,
    len: usize,
    threshold: usize,
}

impl<T: JavaHash + PartialEq + Copy> Default for JavaHashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: JavaHash + PartialEq + Copy> JavaHashSet<T> {
    /// `new HashSet<>()`: the table is allocated on first insert with 16 buckets.
    pub fn new() -> Self {
        Self {
            table: Vec::new(),
            len: 0,
            threshold: 0,
        }
    }

    /// `new HashSet<>(initialCapacity)`.
    pub fn with_capacity(initial: usize) -> Self {
        let cap = initial.max(1).next_power_of_two();
        Self {
            table: Vec::new(),
            len: 0,
            threshold: cap,
        }
    }

    /// `new HashSet<>(collection)`.
    pub fn from_iter_java(items: impl IntoIterator<Item = T>, size: usize) -> Self {
        let mut set = Self::with_capacity(((size as f32 / 0.75) as usize + 1).max(16));
        for i in items {
            set.insert(i);
        }
        set
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn resize(&mut self) {
        let old_cap = self.table.len();
        let (new_cap, new_thr) = if old_cap > 0 {
            (old_cap * 2, (old_cap * 2) * 3 / 4)
        } else if self.threshold > 0 {
            // Initial capacity was stored in the threshold (`HashMap(int)`).
            (self.threshold, self.threshold * 3 / 4)
        } else {
            (16, 12)
        };
        let mut table: Vec<Vec<(u32, T)>> = (0..new_cap).map(|_| Vec::new()).collect();
        for bucket in std::mem::take(&mut self.table) {
            for (h, v) in bucket {
                table[(h as usize) & (new_cap - 1)].push((h, v));
            }
        }
        self.table = table;
        self.threshold = new_thr;
    }

    /// `add`; returns false when already present.
    pub fn insert(&mut self, value: T) -> bool {
        if self.table.is_empty() {
            self.resize();
        }
        let h = spread(value.java_hash());
        let n = self.table.len();
        let bucket = &mut self.table[(h as usize) & (n - 1)];
        if bucket.iter().any(|(bh, v)| *bh == h && *v == value) {
            return false;
        }
        bucket.push((h, value));
        let chain = bucket.len();
        if chain > 8 {
            if n < 64 {
                // `treeifyBin` resizes small tables instead of creating a tree bin.
                self.len += 1;
                self.resize();
                if self.len > self.threshold {
                    self.resize();
                }
                return true;
            }
            debug_assert!(false, "JavaHashSet: tree bins are not modelled");
        }
        self.len += 1;
        if self.len > self.threshold {
            self.resize();
        }
        true
    }

    pub fn contains(&self, value: &T) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let h = spread(value.java_hash());
        self.table[(h as usize) & (self.table.len() - 1)]
            .iter()
            .any(|(bh, v)| *bh == h && v == value)
    }

    pub fn remove(&mut self, value: &T) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let h = spread(value.java_hash());
        let n = self.table.len();
        let bucket = &mut self.table[(h as usize) & (n - 1)];
        if let Some(i) = bucket.iter().position(|(bh, v)| *bh == h && v == value) {
            bucket.remove(i);
            self.len -= 1;
            true
        } else {
            false
        }
    }

    /// `iterator().next()` followed by `iterator.remove()`.
    pub fn pop_first(&mut self) -> Option<T> {
        for bucket in &mut self.table {
            if !bucket.is_empty() {
                let (_, v) = bucket.remove(0);
                self.len -= 1;
                return Some(v);
            }
        }
        None
    }

    /// Iteration in `HashMap` order.
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.table.iter().flat_map(|b| b.iter().map(|(_, v)| *v))
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iteration_follows_java_hash_order() {
        // java: new HashSet<>(List.of(new BlockPos(0,0,0), new BlockPos(1,0,0), new BlockPos(0,1,0), new BlockPos(0,0,1)))
        // hashes: 0, 1, 31, 961 -> buckets (16): 0, 1, 15, 1 (961 & 15 = 1, after 1)
        let mut s = JavaHashSet::new();
        for p in [
            BlockPos::new(0, 0, 0),
            BlockPos::new(1, 0, 0),
            BlockPos::new(0, 1, 0),
            BlockPos::new(0, 0, 1),
        ] {
            s.insert(p);
        }
        let order: Vec<_> = s.iter().collect();
        assert_eq!(
            order,
            vec![
                BlockPos::new(0, 0, 0),
                BlockPos::new(1, 0, 0),
                BlockPos::new(0, 0, 1),
                BlockPos::new(0, 1, 0)
            ]
        );
    }
}
