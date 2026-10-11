use std::io::Read;

use flate2::read::GzDecoder;

use crate::{Compound, List, Tag, id, mutf8};

#[derive(Debug, thiserror::Error)]
pub enum NbtReadError {
    #[error("unexpected end of NBT data")]
    UnexpectedEof,
    #[error("invalid NBT tag type {0}")]
    InvalidTagType(u8),
    #[error("root tag must be a compound, got type {0}")]
    RootNotCompound(u8),
    #[error("invalid modified UTF-8 string")]
    InvalidString,
    #[error("negative length {0}")]
    NegativeLength(i32),
    #[error("NBT nesting too deep")]
    TooDeep,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

const MAX_DEPTH: usize = 512;

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    #[inline]
    fn take(&mut self, n: usize) -> Result<&'a [u8], NbtReadError> {
        if self.data.len() < n {
            return Err(NbtReadError::UnexpectedEof);
        }
        let (head, tail) = self.data.split_at(n);
        self.data = tail;
        Ok(head)
    }

    #[inline]
    fn u8(&mut self) -> Result<u8, NbtReadError> {
        Ok(self.take(1)?[0])
    }

    #[inline]
    fn i16(&mut self) -> Result<i16, NbtReadError> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    #[inline]
    fn i32(&mut self) -> Result<i32, NbtReadError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    #[inline]
    fn i64(&mut self) -> Result<i64, NbtReadError> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn len(&mut self) -> Result<usize, NbtReadError> {
        let len = self.i32()?;
        if len < 0 {
            return Err(NbtReadError::NegativeLength(len));
        }
        Ok(len as usize)
    }

    fn string(&mut self) -> Result<String, NbtReadError> {
        let len = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        let bytes = self.take(len)?;
        mutf8::decode(bytes).ok_or(NbtReadError::InvalidString)
    }

    fn payload(&mut self, ty: u8, depth: usize) -> Result<Tag, NbtReadError> {
        if depth > MAX_DEPTH {
            return Err(NbtReadError::TooDeep);
        }
        Ok(match ty {
            id::BYTE => Tag::Byte(self.u8()? as i8),
            id::SHORT => Tag::Short(self.i16()?),
            id::INT => Tag::Int(self.i32()?),
            id::LONG => Tag::Long(self.i64()?),
            id::FLOAT => Tag::Float(f32::from_bits(self.i32()? as u32)),
            id::DOUBLE => Tag::Double(f64::from_bits(self.i64()? as u64)),
            id::BYTE_ARRAY => {
                let len = self.len()?;
                Tag::ByteArray(self.take(len)?.iter().map(|&b| b as i8).collect())
            }
            id::STRING => Tag::String(self.string()?),
            id::LIST => {
                let element_type = self.u8()?;
                let len = self.len()?;
                if element_type == id::END && len > 0 {
                    return Err(NbtReadError::InvalidTagType(id::END));
                }
                let mut items = Vec::with_capacity(len.min(65536));
                for _ in 0..len {
                    items.push(self.payload(element_type, depth + 1)?);
                }
                Tag::List(List::from_raw(element_type, items))
            }
            id::COMPOUND => Tag::Compound(self.compound(depth + 1)?),
            id::INT_ARRAY => {
                let len = self.len()?;
                let bytes = self.take(len.checked_mul(4).ok_or(NbtReadError::UnexpectedEof)?)?;
                Tag::IntArray(
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| i32::from_be_bytes(*c))
                        .collect(),
                )
            }
            id::LONG_ARRAY => {
                let len = self.len()?;
                let bytes = self.take(len.checked_mul(8).ok_or(NbtReadError::UnexpectedEof)?)?;
                Tag::LongArray(
                    bytes
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|c| i64::from_be_bytes(*c))
                        .collect(),
                )
            }
            other => return Err(NbtReadError::InvalidTagType(other)),
        })
    }

    fn compound(&mut self, depth: usize) -> Result<Compound, NbtReadError> {
        let mut c = Compound::new();
        loop {
            let ty = self.u8()?;
            if ty == id::END {
                return Ok(c);
            }
            let name = self.string()?;
            let value = self.payload(ty, depth)?;
            c.push_unchecked(name, value);
        }
    }
}

/// Reads a named root compound, advancing the slice past it.
pub fn read_named(data: &mut &[u8]) -> Result<(String, Compound), NbtReadError> {
    let mut r = Reader { data };
    let ty = r.u8()?;
    if ty != id::COMPOUND {
        return Err(NbtReadError::RootNotCompound(ty));
    }
    let name = r.string()?;
    let c = r.compound(0)?;
    *data = r.data;
    Ok((name, c))
}

/// Reads an uncompressed root compound.
pub fn read_root(data: &[u8]) -> Result<Compound, NbtReadError> {
    let mut slice = data;
    read_named(&mut slice).map(|(_, c)| c)
}

/// Reads a gzip-compressed root compound (`level.dat` format).
pub fn read_compressed(input: impl Read) -> Result<Compound, NbtReadError> {
    let mut decoder = GzDecoder::new(input);
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes)?;
    read_root(&bytes)
}
