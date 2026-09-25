//! IES (LM-63) photometric profiles: how bright a real fixture is in each
//! direction. Parsed whole at import, and baked by [`IesProfile::bake`] into
//! the table a light's profile texture samples.

use serde::{Deserialize, Serialize};

/// A parsed profile. Candela values are horizontal-major: all vertical angles
/// for the first horizontal angle, then the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IesProfile {
    /// Degrees from straight down (0) to straight up (180).
    pub vertical: Vec<f32>,
    /// Degrees around the fixture, 0-360.
    pub horizontal: Vec<f32>,
    /// Already scaled by the file's multiplier.
    pub candela: Vec<f32>,
    /// Rated lumens per lamp, or -1 for absolute photometry.
    pub lumens: f32,
    /// 1 = type C (the usual), 2 = B, 3 = A.
    pub photometric_type: u8,
}

/// The short of a profile for the tray.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IesInfo {
    pub vertical_angles: usize,
    pub horizontal_angles: usize,
    pub max_candela: f32,
    pub lumens: f32,
    /// `symmetric`, `quadrant`, `bilateral` or `full`.
    pub symmetry: String,
}

pub fn parse_ies(name: &str, text: &str) -> Result<IesProfile, String> {
    let mut lines = text.lines();
    // Keywords and the version line run up to TILT.
    let tilt = loop {
        let line = lines
            .next()
            .ok_or_else(|| format!("{name} has no TILT line"))?
            .trim();
        if let Some(tilt) = line.strip_prefix("TILT=") {
            break tilt.trim().to_string();
        }
    };
    let rest: Vec<&str> = lines.collect();
    let mut numbers = rest.iter().flat_map(|line| line.split([' ', '\t', ',']));
    let mut next = |what: &str| -> Result<f32, String> {
        loop {
            match numbers.next() {
                Some("") => continue,
                Some(token) => {
                    return token
                        .parse::<f32>()
                        .map_err(|_| format!("{name}: \"{token}\" isn't a number ({what})"));
                }
                None => return Err(format!("{name} ends before its {what}")),
            }
        }
    };
    if tilt == "INCLUDE" {
        // Lamp-to-luminaire geometry, then that many angle/factor pairs.
        next("tilt geometry")?;
        let pairs = next("tilt count")? as usize;
        for _ in 0..pairs * 2 {
            next("tilt data")?;
        }
    } else if tilt != "NONE" {
        return Err(format!("{name}: TILT from a separate file isn't supported"));
    }
    let _lamps = next("lamp count")?;
    let lumens = next("lumens")?;
    let multiplier = next("candela multiplier")?;
    let vertical_count = next("vertical angle count")? as usize;
    let horizontal_count = next("horizontal angle count")? as usize;
    let photometric_type = next("photometric type")? as u8;
    // Units, width, length, height, ballast factor, future use, input watts.
    for what in [
        "units",
        "width",
        "length",
        "height",
        "ballast",
        "future use",
        "watts",
    ] {
        next(what)?;
    }
    if vertical_count == 0 || horizontal_count == 0 {
        return Err(format!("{name} has no angles"));
    }
    if vertical_count * horizontal_count > 1 << 20 {
        return Err(format!("{name} has more angles than any fixture needs"));
    }
    let vertical = (0..vertical_count)
        .map(|_| next("vertical angles"))
        .collect::<Result<Vec<_>, _>>()?;
    let horizontal = (0..horizontal_count)
        .map(|_| next("horizontal angles"))
        .collect::<Result<Vec<_>, _>>()?;
    let candela = (0..vertical_count * horizontal_count)
        .map(|_| next("candela values").map(|c| (c * multiplier).max(0.0)))
        .collect::<Result<Vec<_>, _>>()?;
    let ascending = |angles: &[f32]| angles.windows(2).all(|pair| pair[1] > pair[0]);
    if !ascending(&vertical) || !ascending(&horizontal) {
        return Err(format!("{name}: angles must go up"));
    }
    if photometric_type != 1 {
        return Err(format!(
            "{name} is type {} photometry; only type C (1) is read",
            match photometric_type {
                2 => "B",
                3 => "A",
                _ => "unknown",
            }
        ));
    }
    Ok(IesProfile {
        vertical,
        horizontal,
        candela,
        lumens,
        photometric_type,
    })
}

impl IesProfile {
    pub fn info(&self) -> IesInfo {
        IesInfo {
            vertical_angles: self.vertical.len(),
            horizontal_angles: self.horizontal.len(),
            max_candela: self.max_candela(),
            lumens: self.lumens,
            symmetry: self.symmetry().to_string(),
        }
    }

    pub fn max_candela(&self) -> f32 {
        self.candela.iter().copied().fold(0.0, f32::max)
    }

    /// How much of the circle the file measured; the rest is mirrored.
    fn symmetry(&self) -> &'static str {
        match self.horizontal.last().copied().unwrap_or(0.0) {
            last if self.horizontal.len() == 1 || last <= 0.0 => "symmetric",
            last if last <= 90.0 => "quadrant",
            last if last <= 180.0 => "bilateral",
            _ => "full",
        }
    }

    /// Candela towards `vertical` degrees from down and `horizontal` around,
    /// mirrored per the file's symmetry and interpolated bilinearly.
    pub fn sample(&self, vertical: f32, horizontal: f32) -> f32 {
        let mut h = horizontal.rem_euclid(360.0);
        match self.symmetry() {
            "symmetric" => h = 0.0,
            "quadrant" => {
                h = if h > 180.0 { 360.0 - h } else { h };
                h = if h > 90.0 { 180.0 - h } else { h };
            }
            "bilateral" => h = if h > 180.0 { 360.0 - h } else { h },
            _ => {}
        }
        let (h0, h1, ht) = bracket(&self.horizontal, h);
        let (v0, v1, vt) = bracket(&self.vertical, vertical.clamp(0.0, 180.0));
        let rows = self.vertical.len();
        let at = |hi: usize, vi: usize| self.candela[hi * rows + vi];
        let top = at(h0, v0) + (at(h0, v1) - at(h0, v0)) * vt;
        let bottom = at(h1, v0) + (at(h1, v1) - at(h1, v0)) * vt;
        top + (bottom - top) * ht
    }

    /// A `width` x `height` table normalised to the brightest direction:
    /// x is vertical 0-180, y horizontal 0-360. What a profile texture holds.
    pub fn bake(&self, width: u32, height: u32) -> Vec<f32> {
        let peak = self.max_candela().max(f32::EPSILON);
        let step = |i: u32, n: u32, span: f32| span * i as f32 / (n.max(2) - 1) as f32;
        let mut table = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                table.push(self.sample(step(x, width, 180.0), step(y, height, 360.0)) / peak);
            }
        }
        table
    }
}

/// The two indices either side of `value` in ascending `angles`, and how far
/// between them it lies. Past either end clamps to it.
fn bracket(angles: &[f32], value: f32) -> (usize, usize, f32) {
    let last = angles.len() - 1;
    if value <= angles[0] {
        return (0, 0, 0.0);
    }
    if value >= angles[last] {
        return (last, last, 0.0);
    }
    let upper = angles.partition_point(|angle| *angle <= value);
    let lower = upper - 1;
    let t = (value - angles[lower]) / (angles[upper] - angles[lower]);
    (lower, upper, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A downlight: 100 cd straight down, 50 at 45°, none sideways, the
    /// same all round.
    const DOWNLIGHT: &str = "IESNA:LM-63-2002\n[MANUFAC] Test\nTILT=NONE\n\
        1 1000 1 3 1 1 2 0.1 0.1 0\n1 1 10\n0 45 90\n0\n100 50 0\n";

    #[test]
    fn a_rotationally_symmetric_profile_parses_and_samples() {
        let profile = parse_ies("down.ies", DOWNLIGHT).unwrap();
        let info = profile.info();
        assert_eq!((info.vertical_angles, info.horizontal_angles), (3, 1));
        assert_eq!(info.symmetry, "symmetric");
        assert_eq!(profile.sample(0.0, 123.0), 100.0);
        assert_eq!(profile.sample(22.5, 0.0), 75.0);
        assert_eq!(profile.sample(170.0, 0.0), 0.0);
    }

    #[test]
    fn the_multiplier_scales_every_value() {
        let doubled = DOWNLIGHT.replace("1 1000 1 3", "1 1000 2 3");
        assert_eq!(
            parse_ies("down.ies", &doubled).unwrap().max_candela(),
            200.0
        );
    }

    #[test]
    fn a_quadrant_profile_mirrors_round_the_circle() {
        let text = "TILT=NONE\n1 -1 1 2 2 1 2 0 0 0\n1 1 0\n0 90\n0 90\n10 0\n30 0\n";
        let profile = parse_ies("wall.ies", text).unwrap();
        assert_eq!(profile.info().symmetry, "quadrant");
        assert_eq!(profile.sample(0.0, 90.0), 30.0);
        assert_eq!(profile.sample(0.0, 270.0), 30.0);
        assert_eq!(profile.sample(0.0, 180.0), 10.0);
    }

    #[test]
    fn a_baked_table_peaks_at_one() {
        let profile = parse_ies("down.ies", DOWNLIGHT).unwrap();
        let table = profile.bake(3, 2);
        assert_eq!(table.len(), 6);
        assert_eq!(table[0], 1.0);
        assert_eq!(table[2], 0.0);
    }

    #[test]
    fn a_short_file_says_what_it_ran_out_of() {
        let error =
            parse_ies("cut.ies", "TILT=NONE\n1 1000 1 3 1 1 2 0 0 0\n1 1 10\n0 45").unwrap_err();
        assert!(error.contains("vertical angles"), "{error}");
    }
}
