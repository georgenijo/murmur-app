//! Read-only bounded ZIP subset. Never extracts paths or executes archive data.
use crate::section::Problem;
use std::{collections::BTreeMap, io::Read};
const MAX_ARCHIVE: usize = 20 * 1024 * 1024;
const MAX_EXPANDED: usize = 50 * 1024 * 1024;
fn u16_at(b: &[u8], n: usize) -> Result<usize, Problem> {
    Ok(u16::from_le_bytes(b.get(n..n + 2).ok_or(Problem::Invalid)?.try_into().unwrap()) as usize)
}
fn u32_at(b: &[u8], n: usize) -> Result<usize, Problem> {
    Ok(u32::from_le_bytes(b.get(n..n + 4).ok_or(Problem::Invalid)?.try_into().unwrap()) as usize)
}
#[derive(Clone)]
struct Entry {
    offset: usize,
    compressed: usize,
    expanded: usize,
    method: usize,
    crc: u32,
}
pub struct Archive<'a> {
    bytes: &'a [u8],
    entries: BTreeMap<String, Entry>,
}
pub fn safe_path(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && !name.starts_with('/')
        && !name.contains(['\\', ':', '\0'])
        && name
            .trim_end_matches('/')
            .split('/')
            .all(|p| !p.is_empty() && !matches!(p, "." | ".."))
}
impl<'a> Archive<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, Problem> {
        if bytes.len() > MAX_ARCHIVE || bytes.len() < 22 {
            return Err(Problem::Invalid);
        }
        let end = (bytes.len().saturating_sub(65557)..=bytes.len() - 22)
            .rev()
            .find(|i| {
                bytes[*i..].starts_with(b"PK\x05\x06")
                    && u16_at(bytes, *i + 20).is_ok_and(|n| *i + 22 + n == bytes.len())
            })
            .ok_or(Problem::Invalid)?;
        if u16_at(bytes, end + 4)? != 0
            || u16_at(bytes, end + 6)? != 0
            || u16_at(bytes, end + 8)? != u16_at(bytes, end + 10)?
        {
            return Err(Problem::Invalid);
        }
        let count = u16_at(bytes, end + 10)?;
        let size = u32_at(bytes, end + 12)?;
        let start = u32_at(bytes, end + 16)?;
        if count > 2000
            || count == 65535
            || size == u32::MAX as usize
            || start == u32::MAX as usize
            || start.checked_add(size) != Some(end)
        {
            return Err(Problem::Invalid);
        }
        let mut cursor = start;
        let mut expanded = 0usize;
        let mut entries = BTreeMap::new();
        let mut ranges = Vec::new();
        for _ in 0..count {
            if !bytes
                .get(cursor..cursor + 4)
                .is_some_and(|b| b == b"PK\x01\x02")
            {
                return Err(Problem::Invalid);
            }
            let flags = u16_at(bytes, cursor + 8)?;
            let method = u16_at(bytes, cursor + 10)?;
            let compressed = u32_at(bytes, cursor + 20)?;
            let plain = u32_at(bytes, cursor + 24)?;
            let name_len = u16_at(bytes, cursor + 28)?;
            let extra_len = u16_at(bytes, cursor + 30)?;
            let comment_len = u16_at(bytes, cursor + 32)?;
            let offset = u32_at(bytes, cursor + 42)?;
            if flags & !0x808 != 0
                || ![0, 8].contains(&method)
                || compressed == u32::MAX as usize
                || plain == u32::MAX as usize
                || offset == u32::MAX as usize
                || u16_at(bytes, cursor + 34)? != 0
            {
                return Err(Problem::Invalid);
            }
            let name = std::str::from_utf8(
                bytes
                    .get(cursor + 46..cursor + 46 + name_len)
                    .ok_or(Problem::Invalid)?,
            )
            .map_err(|_| Problem::Invalid)?;
            if !safe_path(name) || entries.contains_key(name) {
                return Err(Problem::Invalid);
            } // Reject symlinks/devices and every ZIP64 extra, including zero-size sentinels.
            let mode = (u32_at(bytes, cursor + 38)? >> 16) & 0xf000;
            if ![0, 0x8000, 0x4000].contains(&mode) {
                return Err(Problem::Invalid);
            }
            let extra_start = cursor + 46 + name_len;
            let mut e = extra_start;
            while e < extra_start + extra_len {
                let kind = u16_at(bytes, e)?;
                let len = u16_at(bytes, e + 2)?;
                if kind == 1 || e + 4 + len > extra_start + extra_len {
                    return Err(Problem::Invalid);
                }
                e += 4 + len;
            }
            expanded = expanded.checked_add(plain).ok_or(Problem::Invalid)?;
            if expanded > MAX_EXPANDED
                || plain > 0 && (compressed == 0 || plain > compressed.saturating_mul(200))
            {
                return Err(Problem::Invalid);
            }
            if !bytes
                .get(offset..offset + 4)
                .is_some_and(|b| b == b"PK\x03\x04")
                || u16_at(bytes, offset + 6)? != flags
                || u16_at(bytes, offset + 8)? != method
            {
                return Err(Problem::Invalid);
            }
            let local_name = u16_at(bytes, offset + 26)?;
            let local_extra = u16_at(bytes, offset + 28)?;
            if bytes.get(offset + 30..offset + 30 + local_name) != Some(name.as_bytes()) {
                return Err(Problem::Invalid);
            }
            let local_extra_start = offset + 30 + local_name;
            let mut e = local_extra_start;
            while e < local_extra_start + local_extra {
                let kind = u16_at(bytes, e)?;
                let len = u16_at(bytes, e + 2)?;
                if kind == 1 || e + 4 + len > local_extra_start + local_extra {
                    return Err(Problem::Invalid);
                }
                e += 4 + len;
            }
            if flags & 8 == 0
                && (u32_at(bytes, offset + 18)? != compressed
                    || u32_at(bytes, offset + 22)? != plain
                    || u32_at(bytes, offset + 14)? != u32_at(bytes, cursor + 16)?)
            {
                return Err(Problem::Invalid);
            }
            let data = offset + 30 + local_name + local_extra;
            let data_end = data.checked_add(compressed).ok_or(Problem::Invalid)?;
            if data_end > start || data < offset {
                return Err(Problem::Invalid);
            }
            ranges.push((offset, data_end));
            entries.insert(
                name.into(),
                Entry {
                    offset: data,
                    compressed,
                    expanded: plain,
                    method,
                    crc: u32_at(bytes, cursor + 16)? as u32,
                },
            );
            cursor += 46 + name_len + extra_len + comment_len;
            if cursor > end {
                return Err(Problem::Invalid);
            }
        }
        if cursor != end {
            return Err(Problem::Invalid);
        }
        ranges.sort_unstable();
        if ranges.windows(2).any(|r| r[0].1 > r[1].0) {
            return Err(Problem::Invalid);
        }
        Ok(Self { bytes, entries })
    }
    pub fn read(&self, name: &str, max: usize) -> Result<Vec<u8>, Problem> {
        let e = self.entries.get(name).ok_or(Problem::Invalid)?;
        if e.expanded > max {
            return Err(Problem::Invalid);
        }
        let source = &self.bytes[e.offset..e.offset + e.compressed];
        let out = if e.method == 0 {
            source.to_vec()
        } else {
            let mut out = Vec::new();
            let mut decoder = flate2::read::DeflateDecoder::new(source);
            decoder
                .by_ref()
                .take(max as u64 + 1)
                .read_to_end(&mut out)
                .map_err(|_| Problem::Invalid)?;
            if decoder.total_in() != e.compressed as u64 {
                return Err(Problem::Invalid);
            }
            out
        };
        if out.len() != e.expanded || out.len() > max || crc(&out) != e.crc {
            return Err(Problem::Invalid);
        }
        Ok(out)
    }
}
// IEEE CRC-32, no executable/archive dependency and bounded by selected JSON.
pub fn crc(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 }
        }
    }
    !crc
}
