//! Palette: GIMP palettes (`.gpl`) as palette images.
//!
//! The importer `gpl` answers `palette.png` (one pixel per colour in a single
//! row, the shape a palette swap reads) and `palette.json` (the palette's name
//! and each colour's name and hex). The build hook `cook` fails the build when
//! a `.gpl` has no imported `palette.png` and ships `palettes.json`, the list
//! of palettes the game has. Neither touches the disk: the host hands over the
//! bytes and writes what comes back.

use blockloom_plugin_sdk::{
    Error, Host, Plugin, Produced, Request, Value, build_op, export_plugin, importer_op, json,
};

/// The most colours a palette may have; a row of 4096 is already wide.
const MAX_COLORS: usize = 4096;

struct Palette;

struct Parsed {
    name: String,
    colors: Vec<([u8; 3], String)>,
    warnings: Vec<String>,
}

fn parse(text: &str) -> Result<Parsed, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("GIMP Palette") {
        return Err("not a GIMP palette (the first line must be \"GIMP Palette\")".to_string());
    }
    let mut parsed = Parsed {
        name: String::new(),
        colors: Vec::new(),
        warnings: Vec::new(),
    };
    for (number, line) in lines.enumerate().map(|(i, l)| (i + 2, l.trim())) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix("Name:") {
            parsed.name = name.trim().to_string();
            continue;
        }
        if line.starts_with("Columns:") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let channels: Vec<Option<u8>> = (0..3)
            .map(|_| parts.next().and_then(|p| p.parse::<u8>().ok()))
            .collect();
        let [Some(r), Some(g), Some(b)] = channels[..] else {
            parsed
                .warnings
                .push(format!("line {number} isn't a colour, so it was skipped"));
            continue;
        };
        if parsed.colors.len() == MAX_COLORS {
            return Err(format!("more than {MAX_COLORS} colours"));
        }
        let rest: Vec<&str> = parts.collect();
        parsed.colors.push(([r, g, b], rest.join(" ")));
    }
    if parsed.colors.is_empty() {
        return Err("the palette has no colours".to_string());
    }
    Ok(parsed)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// An 8-bit RGBA PNG one pixel tall, its data in stored (uncompressed)
/// deflate blocks, so no compressor is needed.
fn png(colors: &[[u8; 3]]) -> Vec<u8> {
    let mut row = vec![0u8]; // filter: none
    for [r, g, b] in colors {
        row.extend_from_slice(&[*r, *g, *b, 255]);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = row.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&row).to_be_bytes());

    let mut header = Vec::new();
    header.extend_from_slice(&(colors.len() as u32).to_be_bytes());
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn import(request: &Request, data: &[u8]) -> Produced {
    let text = String::from_utf8_lossy(data);
    let parsed = match parse(&text) {
        Ok(parsed) => parsed,
        Err(why) => return Produced::default().error(format!("{}: {why}", request.path)),
    };
    let rgb: Vec<[u8; 3]> = parsed.colors.iter().map(|(c, _)| *c).collect();
    let listing = json!({
        "name": parsed.name,
        "colors": parsed.colors.iter().map(|([r, g, b], name)| {
            json!({"name": name, "hex": format!("#{r:02x}{g:02x}{b:02x}")})
        }).collect::<Vec<_>>(),
    });
    let mut made = Produced::default()
        .file("palette.png", png(&rgb))
        .file("palette.json", serde_json_bytes(&listing));
    for warning in parsed.warnings {
        made = made.warn(warning);
    }
    made
}

fn serde_json_bytes(value: &Value) -> Vec<u8> {
    value.to_string().into_bytes()
}

/// Every `.gpl` in the project must have been imported.
fn cook(request: &Request) -> Produced {
    let mut names: Vec<&str> = request
        .assets
        .iter()
        .map(String::as_str)
        .filter(|p| p.ends_with(".gpl"))
        .collect();
    names.sort_unstable();
    let mut made = Produced::default();
    for name in &names {
        let output = format!("{name}.imported/palette.png");
        if !request.assets.contains(&output) {
            made = made.error(format!("{name} was never imported (run plugin-import)"));
        }
    }
    made.file(
        "palettes.json",
        serde_json_bytes(&json!({"palettes": names})),
    )
}

impl Plugin for Palette {
    fn start(_host: &Host) -> Result<Self, Error> {
        Ok(Palette)
    }

    fn call(&mut self, _host: &Host, op: &str, input: &[u8]) -> Result<Vec<u8>, Error> {
        if op != importer_op("gpl") && op != build_op("cook") {
            return Err(Error::unsupported(op));
        }
        let (request, data) = Request::decode(input).map_err(Error::bad_argument)?;
        if op == importer_op("gpl") {
            Ok(import(&request, data).encode())
        } else {
            Ok(cook(&request).encode())
        }
    }
}

export_plugin!(Palette);

#[cfg(test)]
mod tests {
    use super::*;

    const GPL: &str = "GIMP Palette\nName: Sunset\nColumns: 4\n# a comment\n255 0 0 Red\n  0 128 255   Sky blue\nnonsense\n";

    #[test]
    fn a_palette_is_parsed_with_names_and_warnings() {
        let parsed = parse(GPL).unwrap();
        assert_eq!(parsed.name, "Sunset");
        assert_eq!(parsed.colors[1], ([0, 128, 255], "Sky blue".to_string()));
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parse("P3\n").is_err());
        assert!(parse("GIMP Palette\n").is_err());
        assert!(parse("GIMP Palette\n300 0 0 x\n").is_err());
    }

    #[test]
    fn the_png_has_the_signature_and_checksums_that_hold() {
        let bytes = png(&[[1, 2, 3], [4, 5, 6]]);
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        // IHDR: width 2, height 1.
        assert_eq!(&bytes[16..24], &[0, 0, 0, 2, 0, 0, 0, 1]);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
