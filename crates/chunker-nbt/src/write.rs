use std::io::{self, Write};

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::{Compound, List, Tag, id, mutf8};

/// Streaming NBT writer into a byte buffer.
pub struct NbtWriter {
    pub buf: Vec<u8>,
}

impl NbtWriter {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
        }
    }

    #[inline]
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    #[inline]
    fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    #[inline]
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    #[inline]
    fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    fn string(&mut self, s: &str) {
        let len = mutf8::encoded_len(s);
        assert!(len <= u16::MAX as usize, "NBT string too long ({len} bytes)");
        self.buf.extend_from_slice(&(len as u16).to_be_bytes());
        mutf8::encode(s, &mut self.buf);
    }

    pub fn write_named(&mut self, name: &str, tag: &Tag) {
        self.u8(tag.type_id());
        self.string(name);
        self.payload(tag);
    }

    fn payload(&mut self, tag: &Tag) {
        match tag {
            Tag::Byte(v) => self.u8(*v as u8),
            Tag::Short(v) => self.i16(*v),
            Tag::Int(v) => self.i32(*v),
            Tag::Long(v) => self.i64(*v),
            Tag::Float(v) => self.i32(v.to_bits() as i32),
            Tag::Double(v) => self.i64(v.to_bits() as i64),
            Tag::ByteArray(v) => {
                self.i32(v.len() as i32);
                self.buf.extend(v.iter().map(|&b| b as u8));
            }
            Tag::String(v) => self.string(v),
            Tag::List(list) => self.list(list),
            Tag::Compound(c) => self.compound(c),
            Tag::IntArray(v) => {
                self.i32(v.len() as i32);
                self.buf.reserve(v.len() * 4);
                for &x in v {
                    self.i32(x);
                }
            }
            Tag::LongArray(v) => {
                self.i32(v.len() as i32);
                self.buf.reserve(v.len() * 8);
                for &x in v {
                    self.i64(x);
                }
            }
        }
    }

    fn list(&mut self, list: &List) {
        self.u8(list.element_type());
        self.i32(list.len() as i32);
        for item in list.iter() {
            self.payload(item);
        }
    }

    fn compound(&mut self, c: &Compound) {
        for (k, v) in c.iter() {
            self.write_named(k, v);
        }
        self.u8(id::END);
    }
}

/// Writes a named root tag (`type, name, payload`).
pub fn write_named(name: &str, root: &Compound) -> Vec<u8> {
    let mut w = NbtWriter::with_capacity(4096);
    w.u8(id::COMPOUND);
    w.string(name);
    w.compound(root);
    w.buf
}

/// Writes a root compound with an empty name, which is what vanilla `NbtIo.write` produces.
pub fn write_root(root: &Compound) -> Vec<u8> {
    write_named("", root)
}

/// Writes a gzip-compressed root compound (format of `level.dat` and saved data files).
pub fn write_compressed(root: &Compound, out: impl Write) -> io::Result<()> {
    let mut encoder = GzEncoder::new(out, Compression::default());
    encoder.write_all(&write_root(root))?;
    encoder.finish()?;
    Ok(())
}
