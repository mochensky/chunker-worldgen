//! Java "Modified UTF-8" used by `DataOutput.writeUTF`.
//!
//! Differences from UTF-8: NUL is encoded as `C0 80`, and supplementary characters
//! are encoded as two 3-byte surrogate sequences (CESU-8 style).

pub fn encode(s: &str, out: &mut Vec<u8>) {
    if s.bytes().all(|b| b != 0 && b < 0x80) {
        out.extend_from_slice(s.as_bytes());
        return;
    }
    for unit in s.encode_utf16() {
        match unit {
            0x0001..=0x007F => out.push(unit as u8),
            0x0000 | 0x0080..=0x07FF => {
                out.push((0xC0 | ((unit >> 6) & 0x1F)) as u8);
                out.push((0x80 | (unit & 0x3F)) as u8);
            }
            _ => {
                out.push((0xE0 | ((unit >> 12) & 0x0F)) as u8);
                out.push((0x80 | ((unit >> 6) & 0x3F)) as u8);
                out.push((0x80 | (unit & 0x3F)) as u8);
            }
        }
    }
}

pub fn encoded_len(s: &str) -> usize {
    if s.bytes().all(|b| b != 0 && b < 0x80) {
        return s.len();
    }
    s.encode_utf16()
        .map(|unit| match unit {
            0x0001..=0x007F => 1,
            0x0000 | 0x0080..=0x07FF => 2,
            _ => 3,
        })
        .sum()
}

pub fn decode(bytes: &[u8]) -> Option<String> {
    if bytes.iter().all(|&b| b != 0 && b < 0x80) {
        return Some(String::from_utf8_lossy(bytes).into_owned());
    }
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let a = bytes[i] as u16;
        if a & 0x80 == 0 {
            units.push(a);
            i += 1;
        } else if a & 0xE0 == 0xC0 {
            let b = *bytes.get(i + 1)? as u16;
            units.push(((a & 0x1F) << 6) | (b & 0x3F));
            i += 2;
        } else if a & 0xF0 == 0xE0 {
            let b = *bytes.get(i + 1)? as u16;
            let c = *bytes.get(i + 2)? as u16;
            units.push(((a & 0x0F) << 12) | ((b & 0x3F) << 6) | (c & 0x3F));
            i += 3;
        } else {
            return None;
        }
    }
    Some(String::from_utf16_lossy(&units))
}
