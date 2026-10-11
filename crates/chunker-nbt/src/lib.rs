//! NBT (Named Binary Tag) reader and writer for Minecraft: Java Edition.
//!
//! Matches the Java Edition binary format (big endian, Modified UTF-8 strings).
//! Compounds keep insertion order, which makes the written output deterministic.

mod mutf8;
mod read;
mod write;

use std::fmt;

pub use read::{NbtReadError, read_compressed, read_named, read_root};
pub use write::{NbtWriter, write_compressed, write_named, write_root};

/// NBT tag type ids.
pub mod id {
    pub const END: u8 = 0;
    pub const BYTE: u8 = 1;
    pub const SHORT: u8 = 2;
    pub const INT: u8 = 3;
    pub const LONG: u8 = 4;
    pub const FLOAT: u8 = 5;
    pub const DOUBLE: u8 = 6;
    pub const BYTE_ARRAY: u8 = 7;
    pub const STRING: u8 = 8;
    pub const LIST: u8 = 9;
    pub const COMPOUND: u8 = 10;
    pub const INT_ARRAY: u8 = 11;
    pub const LONG_ARRAY: u8 = 12;
}

/// A single NBT value.
#[derive(Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(List),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn type_id(&self) -> u8 {
        match self {
            Tag::Byte(_) => id::BYTE,
            Tag::Short(_) => id::SHORT,
            Tag::Int(_) => id::INT,
            Tag::Long(_) => id::LONG,
            Tag::Float(_) => id::FLOAT,
            Tag::Double(_) => id::DOUBLE,
            Tag::ByteArray(_) => id::BYTE_ARRAY,
            Tag::String(_) => id::STRING,
            Tag::List(_) => id::LIST,
            Tag::Compound(_) => id::COMPOUND,
            Tag::IntArray(_) => id::INT_ARRAY,
            Tag::LongArray(_) => id::LONG_ARRAY,
        }
    }

    /// Numeric value as i64 if this is any integral tag (Java `NumericTag` semantics).
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Tag::Byte(v) => Some(v as i64),
            Tag::Short(v) => Some(v as i64),
            Tag::Int(v) => Some(v as i64),
            Tag::Long(v) => Some(v),
            Tag::Float(v) => Some(v as i64),
            Tag::Double(v) => Some(v as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Tag::Byte(v) => Some(v as f64),
            Tag::Short(v) => Some(v as f64),
            Tag::Int(v) => Some(v as f64),
            Tag::Long(v) => Some(v as f64),
            Tag::Float(v) => Some(v as f64),
            Tag::Double(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_compound(&self) -> Option<&Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_compound_mut(&mut self) -> Option<&mut Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&List> {
        match self {
            Tag::List(l) => Some(l),
            _ => None,
        }
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tag::Byte(v) => write!(f, "{v}b"),
            Tag::Short(v) => write!(f, "{v}s"),
            Tag::Int(v) => write!(f, "{v}"),
            Tag::Long(v) => write!(f, "{v}L"),
            Tag::Float(v) => write!(f, "{v}f"),
            Tag::Double(v) => write!(f, "{v}d"),
            Tag::ByteArray(v) => write!(f, "[B; {} bytes]", v.len()),
            Tag::String(v) => write!(f, "{v:?}"),
            Tag::List(v) => v.fmt(f),
            Tag::Compound(v) => v.fmt(f),
            Tag::IntArray(v) => write!(f, "[I; {v:?}]"),
            Tag::LongArray(v) => write!(f, "[L; {} longs]", v.len()),
        }
    }
}

/// A homogeneous NBT list. An empty list may carry any element type (vanilla writes `END`).
#[derive(Clone, PartialEq, Default)]
pub struct List {
    element_type: u8,
    items: Vec<Tag>,
}

impl List {
    pub fn new() -> Self {
        Self {
            element_type: id::END,
            items: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            element_type: id::END,
            items: Vec::with_capacity(capacity),
        }
    }

    /// Appends a value. Panics when the element type does not match the list type,
    /// mirroring vanilla's `ListTag` homogeneity requirement.
    pub fn push(&mut self, tag: impl Into<Tag>) {
        let tag = tag.into();
        if self.items.is_empty() {
            self.element_type = tag.type_id();
        } else {
            assert_eq!(self.element_type, tag.type_id(), "heterogeneous NBT list");
        }
        self.items.push(tag);
    }

    pub fn element_type(&self) -> u8 {
        if self.items.is_empty() {
            id::END
        } else {
            self.element_type
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Tag> {
        self.items.iter()
    }

    pub fn get(&self, index: usize) -> Option<&Tag> {
        self.items.get(index)
    }

    /// Mutable access to an element (its type cannot change through `Tag` methods that keep the variant).
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Tag> {
        self.items.get_mut(index)
    }

    pub fn compounds(&self) -> impl Iterator<Item = &Compound> {
        self.items.iter().filter_map(Tag::as_compound)
    }

    pub fn compounds_mut(&mut self) -> impl Iterator<Item = &mut Compound> {
        self.items.iter_mut().filter_map(Tag::as_compound_mut)
    }

    pub(crate) fn from_raw(element_type: u8, items: Vec<Tag>) -> Self {
        Self { element_type, items }
    }
}

impl fmt::Debug for List {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.items.iter()).finish()
    }
}

impl<T: Into<Tag>> FromIterator<T> for List {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = List::new();
        for item in iter {
            list.push(item);
        }
        list
    }
}

/// An ordered NBT compound.
#[derive(Clone, PartialEq, Default)]
pub struct Compound {
    entries: Vec<(String, Tag)>,
}

impl Compound {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    /// Inserts or replaces a value, keeping the original position on replace.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Tag>) {
        let key = key.into();
        let value = value.into();
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.entries.push((key, value));
        }
    }

    /// Appends without checking for duplicates. Callers must guarantee key uniqueness.
    pub fn push_unchecked(&mut self, key: impl Into<String>, value: impl Into<Tag>) {
        self.entries.push((key.into(), value.into()));
    }

    pub fn get(&self, key: &str) -> Option<&Tag> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Tag> {
        self.entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn remove(&mut self, key: &str) -> Option<Tag> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(index).1)
    }

    /// `CompoundTag.merge`: copies `other` into `self`, merging nested compounds.
    pub fn merge(&mut self, other: &Compound) {
        for (k, v) in other.iter() {
            match (self.get_mut(k), v) {
                (Some(Tag::Compound(mine)), Tag::Compound(theirs)) => mine.merge(theirs),
                _ => self.insert(k, v.clone()),
            }
        }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Tag)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn get_compound(&self, key: &str) -> Option<&Compound> {
        self.get(key).and_then(Tag::as_compound)
    }

    pub fn get_list(&self, key: &str) -> Option<&List> {
        self.get(key).and_then(Tag::as_list)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Tag::as_str)
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Tag::as_i64)
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get_i64(key).map(|v| v as i32)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_i64(key).map(|v| v != 0)
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Tag::as_f64)
    }

    pub fn get_long_array(&self, key: &str) -> Option<&[i64]> {
        match self.get(key) {
            Some(Tag::LongArray(v)) => Some(v),
            _ => None,
        }
    }

    pub fn get_int_array(&self, key: &str) -> Option<&[i32]> {
        match self.get(key) {
            Some(Tag::IntArray(v)) => Some(v),
            _ => None,
        }
    }
}

impl fmt::Debug for Compound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();
        for (k, v) in &self.entries {
            map.entry(k, v);
        }
        map.finish()
    }
}

macro_rules! impl_from {
    ($($t:ty => $variant:ident),* $(,)?) => {
        $(impl From<$t> for Tag {
            fn from(value: $t) -> Self { Tag::$variant(value) }
        })*
    };
}

impl_from! {
    i8 => Byte,
    i16 => Short,
    i32 => Int,
    i64 => Long,
    f32 => Float,
    f64 => Double,
    Vec<i8> => ByteArray,
    String => String,
    List => List,
    Compound => Compound,
    Vec<i32> => IntArray,
    Vec<i64> => LongArray,
}

impl From<bool> for Tag {
    fn from(value: bool) -> Self {
        Tag::Byte(value as i8)
    }
}

impl From<&str> for Tag {
    fn from(value: &str) -> Self {
        Tag::String(value.to_owned())
    }
}

impl From<u8> for Tag {
    fn from(value: u8) -> Self {
        Tag::Byte(value as i8)
    }
}

/// Canonical text form (keys sorted, typed scalars, floats as raw bits) matching the Java oracle's
/// `Structures.canonical`, for exact comparisons in tests.
pub fn canonical(tag: &Tag) -> String {
    fn quote(out: &mut String, s: &str) {
        out.push('"');
        for ch in s.chars() {
            if ch == '"' || ch == '\\' {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
    }
    fn write(out: &mut String, tag: &Tag) {
        use std::fmt::Write as _;
        match tag {
            Tag::Compound(c) => {
                let mut keys: Vec<&str> = c.iter().map(|(k, _)| k).collect();
                keys.sort_unstable();
                out.push('{');
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote(out, k);
                    out.push(':');
                    write(out, c.get(k).unwrap());
                }
                out.push('}');
            }
            Tag::List(l) => {
                out.push('[');
                for (i, t) in l.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(out, t);
                }
                out.push(']');
            }
            Tag::Byte(v) => write!(out, "{v}b").unwrap(),
            Tag::Short(v) => write!(out, "{v}s").unwrap(),
            Tag::Int(v) => write!(out, "{v}").unwrap(),
            Tag::Long(v) => write!(out, "{v}L").unwrap(),
            Tag::Float(v) => write!(out, "f#{:x}", v.to_bits()).unwrap(),
            Tag::Double(v) => write!(out, "d#{:x}", v.to_bits()).unwrap(),
            Tag::String(s) => quote(out, s),
            Tag::ByteArray(a) => {
                out.push_str("[B;");
                out.push_str(&a.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
                out.push(']');
            }
            Tag::IntArray(a) => {
                out.push_str("[I;");
                out.push_str(&a.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
                out.push(']');
            }
            Tag::LongArray(a) => {
                out.push_str("[L;");
                out.push_str(&a.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
                out.push(']');
            }
        }
    }
    let mut out = String::new();
    write(&mut out, tag);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_nested() {
        let mut root = Compound::new();
        root.insert("DataVersion", 5023);
        root.insert("Status", "minecraft:full");
        root.insert("name\u{0}\u{1F600}", "unicode \u{0} \u{1F600} ok");
        let mut list = List::new();
        for i in 0..3 {
            let mut c = Compound::new();
            c.insert("Y", i as i8);
            c.insert("data", vec![1i64, -2, i64::MAX]);
            list.push(c);
        }
        root.insert("sections", list);
        root.insert("empty", List::new());
        root.insert("f", 1.5f32);
        root.insert("d", -0.25f64);
        root.insert("ints", vec![1, 2, 3]);
        root.insert("bytes", vec![-1i8, 0, 1]);

        let bytes = write_root(&root);
        let (name, back) = read_named(&mut bytes.as_slice()).unwrap();
        assert_eq!(name, "");
        assert_eq!(back, root);
    }
}
