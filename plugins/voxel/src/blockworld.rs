//! `.blockworld`: a world's edits as binary regions over its generated terrain.
//!
//! Layout, little endian: `BLKWORLD`, a u32 version, a u32 length and that
//! many bytes of JSON header (`page_size`, `size`, `preset`, `seed`), then
//! u32 page count and per page its section `i32 x3`, a u32 run count and
//! `(u16 count, u16 block)` runs over the section's cells, where 65535 keeps
//! the generated cell. Then u32 shape count, per shape `i32 x3`, a u8 length
//! and the shape's name, and u32 density count, per density `i32 x3, i16`.

use crate::grid::{Page, SECTION, SECTION_CELLS};
use blockloom_plugin_sdk::{Value, json};

pub const MAGIC: &[u8; 8] = b"BLKWORLD";
pub const VERSION: u32 = 1;
const MAX_PAGES: usize = 262_144;
const MAX_LIST: usize = 4_194_304;

/// The cells, shapes and densities that differ from the generated world.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Regions {
    pub pages: Vec<Page>,
    pub shapes: Vec<([i32; 3], String)>,
    pub densities: Vec<([i32; 3], i16)>,
}

/// The header a world writes: what it was generated as, to check on load.
pub fn header(size: [i32; 3], preset: &str, seed: i64) -> Value {
    json!({"page_size": SECTION, "size": size, "preset": preset, "seed": seed})
}

pub fn encode(regions: &Regions, header: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    let head = header.to_string().into_bytes();
    out.extend_from_slice(&(head.len() as u32).to_le_bytes());
    out.extend_from_slice(&head);
    out.extend_from_slice(&(regions.pages.len() as u32).to_le_bytes());
    for page in &regions.pages {
        put_cell(&mut out, page.chunk);
        out.extend_from_slice(&(page.runs.len() as u32).to_le_bytes());
        for (count, material) in &page.runs {
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&material.to_le_bytes());
        }
    }
    out.extend_from_slice(&(regions.shapes.len() as u32).to_le_bytes());
    for (cell, name) in &regions.shapes {
        put_cell(&mut out, *cell);
        out.push(name.len() as u8);
        out.extend_from_slice(name.as_bytes());
    }
    out.extend_from_slice(&(regions.densities.len() as u32).to_le_bytes());
    for (cell, density) in &regions.densities {
        put_cell(&mut out, *cell);
        out.extend_from_slice(&density.to_le_bytes());
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<(Value, Regions), String> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return Err("not a .blockworld file".into());
    }
    let version = r.u32()?;
    if version > VERSION {
        return Err(format!(
            "this .blockworld was saved by a newer voxel plugin (format {version})"
        ));
    }
    let head_len = r.u32()? as usize;
    let header: Value = blockloom_plugin_sdk::serde_json::from_slice(r.take(head_len)?)
        .map_err(|e| format!("the .blockworld header: {e}"))?;
    if header["page_size"].as_i64() != Some(i64::from(SECTION)) {
        return Err("unsupported .blockworld page size".into());
    }
    let mut regions = Regions::default();
    let pages = r.count(MAX_PAGES)?;
    for _ in 0..pages {
        let chunk = r.cell()?;
        let runs = r.count(SECTION_CELLS)?;
        let mut page = Page {
            chunk,
            runs: Vec::with_capacity(runs),
        };
        let mut total = 0usize;
        for _ in 0..runs {
            let count = r.u16()?;
            let material = r.u16()?;
            if count == 0 {
                return Err("a bad .blockworld run".into());
            }
            total += usize::from(count);
            page.runs.push((count, material));
        }
        if total != SECTION_CELLS {
            return Err("an incomplete .blockworld section".into());
        }
        regions.pages.push(page);
    }
    for _ in 0..r.count(MAX_LIST)? {
        let cell = r.cell()?;
        let len = usize::from(r.take(1)?[0]);
        let name = std::str::from_utf8(r.take(len)?).map_err(|_| "a bad shape name")?;
        regions.shapes.push((cell, name.to_string()));
    }
    for _ in 0..r.count(MAX_LIST)? {
        let cell = r.cell()?;
        let density = i16::from_le_bytes(r.take(2)?.try_into().unwrap());
        regions.densities.push((cell, density));
    }
    if r.at != bytes.len() {
        return Err("trailing bytes after the .blockworld regions".into());
    }
    Ok((header, regions))
}

/// Run-length codes a section's cells (`KEEP` for an unchanged one).
pub fn runs(cells: &[u16]) -> Vec<(u16, u16)> {
    let mut runs = Vec::<(u16, u16)>::new();
    for &m in cells {
        match runs.last_mut() {
            Some((count, value)) if *value == m && *count < u16::MAX => *count += 1,
            _ => runs.push((1, m)),
        }
    }
    runs
}

/// Standard base64, as `assets.read` answers bytes.
pub fn base64(text: &str) -> Result<Vec<u8>, String> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let text = text.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in text {
        acc = (acc << 6) | u32::from(value(c).ok_or("bad base64")?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

fn put_cell(out: &mut Vec<u8>, cell: [i32; 3]) {
    for c in cell {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.bytes.len());
        let end = end.ok_or("the .blockworld file ends early")?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self, max: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n > max {
            return Err("a .blockworld list is too long".into());
        }
        Ok(n)
    }
    fn cell(&mut self) -> Result<[i32; 3], String> {
        let mut c = [0; 3];
        for v in &mut c {
            *v = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::KEEP;

    #[test]
    fn regions_round_trip_and_damage_is_refused() {
        let mut cells = vec![KEEP; SECTION_CELLS];
        cells[5] = 3;
        cells[6] = 0;
        let regions = Regions {
            pages: vec![Page {
                chunk: [1, 0, 2],
                runs: runs(&cells),
            }],
            shapes: vec![([33, 1, 64], "stair west".into())],
            densities: vec![([40, 2, 70], -128)],
        };
        let bytes = encode(&regions, &header([64, 32, 96], "flat", 7));
        let (head, back) = decode(&bytes).unwrap();
        assert_eq!(back, regions);
        assert_eq!(head["preset"], "flat");
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode(b"NOTWORLD").is_err());
        let mut newer = bytes.clone();
        newer[8] = 9;
        assert!(decode(&newer).unwrap_err().contains("newer"));
    }

    #[test]
    fn base64_decodes_what_the_host_encodes() {
        assert_eq!(base64("aGk=").unwrap(), b"hi");
        assert_eq!(base64("AAEC/w==").unwrap(), [0, 1, 2, 255]);
    }
}
