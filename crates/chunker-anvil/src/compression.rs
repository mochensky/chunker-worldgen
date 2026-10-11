use std::io::{self, Read, Write};

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};

/// Region chunk compression (`RegionFileVersion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Gzip,
    /// Vanilla default ("deflate"), a zlib stream.
    Zlib,
    None,
}

impl Compression {
    pub fn id(self) -> u8 {
        match self {
            Compression::Gzip => 1,
            Compression::Zlib => 2,
            Compression::None => 3,
        }
    }

    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            1 => Some(Compression::Gzip),
            2 => Some(Compression::Zlib),
            3 => Some(Compression::None),
            // 4 = LZ4 (lz4-java block stream) is preserved verbatim but not decoded yet.
            _ => None,
        }
    }

    pub fn compress(self, data: &[u8]) -> Vec<u8> {
        match self {
            Compression::None => data.to_vec(),
            Compression::Zlib => {
                // Level 6 matches java.util.zip.Deflater's default; level 1-3 would be faster
                // but produce larger worlds.
                let mut enc =
                    ZlibEncoder::new(Vec::with_capacity(data.len() / 4), flate2::Compression::new(6));
                enc.write_all(data).expect("in-memory write");
                enc.finish().expect("in-memory write")
            }
            Compression::Gzip => {
                let mut enc = GzEncoder::new(Vec::with_capacity(data.len() / 4), flate2::Compression::new(6));
                enc.write_all(data).expect("in-memory write");
                enc.finish().expect("in-memory write")
            }
        }
    }

    pub fn decompress(self, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(data.len() * 4);
        match self {
            Compression::None => out.extend_from_slice(data),
            Compression::Zlib => {
                ZlibDecoder::new(data).read_to_end(&mut out)?;
            }
            Compression::Gzip => {
                GzDecoder::new(data).read_to_end(&mut out)?;
            }
        }
        Ok(out)
    }
}
