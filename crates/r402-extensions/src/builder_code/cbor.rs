//! ERC-8021 Schema 2 CBOR suffix encode/parse (official byte layout).

use compact_str::CompactString;

use super::{BuilderCodeData, ERC_8021_MARKER, SCHEMA_2_ID};

/// Encodes `[cbor][len_be_u16][schema_id][marker]`.
#[must_use]
pub fn encode_builder_code_suffix(data: &BuilderCodeData) -> Vec<u8> {
    let cbor = encode_cbor_map(data);
    let cbor_len = u16::try_from(cbor.len()).unwrap_or(u16::MAX);
    let mut suffix = Vec::with_capacity(cbor.len() + 2 + 1 + ERC_8021_MARKER.len());
    suffix.extend_from_slice(&cbor);
    suffix.extend_from_slice(&cbor_len.to_be_bytes());
    suffix.push(SCHEMA_2_ID);
    suffix.extend_from_slice(&ERC_8021_MARKER);
    suffix
}

/// Parses an ERC-8021 Schema 2 suffix from full transaction input.
#[must_use]
pub fn parse_builder_code_suffix_from_calldata(calldata: &[u8]) -> Option<BuilderCodeData> {
    const TAIL: usize = 16 + 1 + 2;
    if calldata.len() < TAIL {
        return None;
    }
    let marker_at = calldata.len() - 16;
    if calldata.get(marker_at..) != Some(ERC_8021_MARKER.as_slice()) {
        return None;
    }
    if calldata.get(marker_at - 1).copied() != Some(SCHEMA_2_ID) {
        return None;
    }
    let len_at = marker_at - 3;
    let cbor_len =
        u16::from_be_bytes([*calldata.get(len_at)?, *calldata.get(len_at + 1)?]) as usize;
    let suffix_start = marker_at.checked_sub(3 + cbor_len)?;
    if suffix_start + cbor_len + TAIL != calldata.len() {
        return None;
    }
    let cbor = calldata.get(suffix_start..suffix_start + cbor_len)?;
    decode_cbor_map(cbor)
}

fn encode_cbor_map(data: &BuilderCodeData) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut map_size = 0u64;
    if let Some(a) = data.a.as_ref() {
        map_size += 1;
        entries.extend_from_slice(&encode_cbor_string("a"));
        entries.extend_from_slice(&encode_cbor_string(a));
    }
    if let Some(w) = data.w.as_ref() {
        map_size += 1;
        entries.extend_from_slice(&encode_cbor_string("w"));
        entries.extend_from_slice(&encode_cbor_string(w));
    }
    if !data.s.is_empty() {
        map_size += 1;
        entries.extend_from_slice(&encode_cbor_string("s"));
        let items: Vec<&str> = data.s.iter().map(CompactString::as_str).collect();
        entries.extend_from_slice(&encode_cbor_array(&items));
    }
    let mut out = encode_cbor_major(5, map_size);
    out.extend_from_slice(&entries);
    out
}

fn encode_cbor_string(value: &str) -> Vec<u8> {
    let encoded = value.as_bytes();
    let mut out = encode_cbor_major(3, encoded.len() as u64);
    out.extend_from_slice(encoded);
    out
}

fn encode_cbor_array(values: &[&str]) -> Vec<u8> {
    let mut out = encode_cbor_major(4, values.len() as u64);
    for value in values {
        out.extend_from_slice(&encode_cbor_string(value));
    }
    out
}

fn encode_cbor_major(major: u8, value: u64) -> Vec<u8> {
    let mt = major << 5;
    if value <= 23 {
        vec![mt | u8::try_from(value).unwrap_or(0)]
    } else if value <= 0xff {
        vec![mt | 0x18, u8::try_from(value).unwrap_or(0)]
    } else {
        let hi = u8::try_from((value >> 8) & 0xff).unwrap_or(0);
        let lo = u8::try_from(value & 0xff).unwrap_or(0);
        vec![mt | 0x19, hi, lo]
    }
}

fn decode_cbor_map(bytes: &[u8]) -> Option<BuilderCodeData> {
    let mut r = CborReader { bytes, offset: 0 };
    if r.peek_major()? != 5 {
        return None;
    }
    let map_size = r.read_len()?;
    let mut result = BuilderCodeData::default();
    for _ in 0..map_size {
        if r.peek_major()? != 3 {
            return None;
        }
        let key = r.read_text()?;
        match key.as_str() {
            "a" | "w" => {
                if r.peek_major()? != 3 {
                    return None;
                }
                let value = CompactString::from(r.read_text()?);
                if key == "a" {
                    result.a = Some(value);
                } else {
                    result.w = Some(value);
                }
            }
            "s" => {
                result.s = r.read_text_array()?;
            }
            _ => return None,
        }
    }
    Some(result)
}

struct CborReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl CborReader<'_> {
    fn peek_major(&self) -> Option<u8> {
        Some(*self.bytes.get(self.offset)? >> 5)
    }

    fn read_len(&mut self) -> Option<usize> {
        let head = *self.bytes.get(self.offset)?;
        self.offset += 1;
        let info = head & 0x1f;
        if info <= 23 {
            Some(usize::from(info))
        } else if info == 24 {
            let v = *self.bytes.get(self.offset)?;
            self.offset += 1;
            Some(usize::from(v))
        } else {
            None
        }
    }

    fn read_text(&mut self) -> Option<String> {
        let len = self.read_len()?;
        let end = self.offset.checked_add(len)?;
        let slice = self.bytes.get(self.offset..end)?;
        self.offset = end;
        String::from_utf8(slice.to_vec()).ok()
    }

    fn read_text_array(&mut self) -> Option<Vec<CompactString>> {
        if self.peek_major()? != 4 {
            return None;
        }
        let array_size = self.read_len()?;
        let mut codes = Vec::with_capacity(array_size);
        for _ in 0..array_size {
            if self.peek_major()? != 3 {
                return None;
            }
            codes.push(CompactString::from(self.read_text()?));
        }
        Some(codes)
    }
}
