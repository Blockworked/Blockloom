//! Blockloom's own WESL modules: hashes, noise, FBM, scattering, the
//! standard per-view uniform and the sky. The runtime registers each as
//! `blockloom::<name>`, so the built-in passes and a project's surface files
//! import them the way they import Bevy's (`import blockloom::fbm::fbm3;`).
//! Kept here rather than in the runtime so the editor's shader check
//! resolves them too.

use std::borrow::Cow;
use wesl::syntax::{ModulePath, PathOrigin};

/// The package every module lives under.
pub const PACKAGE: &str = "blockloom";

/// Every module, by name under [`PACKAGE`].
pub const MODULES: &[(&str, &str)] = &[
    ("hash", include_str!("shaders/hash.wesl")),
    ("noise", include_str!("shaders/noise.wesl")),
    ("fbm", include_str!("shaders/fbm.wesl")),
    ("scattering", include_str!("shaders/scattering.wesl")),
    ("frame", include_str!("shaders/frame.wesl")),
    ("sky", include_str!("shaders/sky.wesl")),
    ("fog", include_str!("shaders/fog.wesl")),
    ("space", include_str!("shaders/space.wesl")),
];

pub fn module(name: &str) -> Option<&'static str> {
    MODULES
        .iter()
        .find(|(module, _)| *module == name)
        .map(|(_, source)| *source)
}

/// Link `source` against the library into one WGSL module, with `features`
/// deciding its `@if`s (anything unnamed is off). Only `blockloom::` imports
/// resolve here; Bevy's modules exist on the GPU side alone.
pub fn link(source: &str, features: &[(&str, bool)]) -> Result<String, String> {
    let mut resolver = wesl::VirtualResolver::new();
    let root = ModulePath::new(PathOrigin::Absolute, vec!["root".to_string()]);
    resolver.add_module(root.clone(), Cow::Borrowed(source));
    for (name, text) in MODULES {
        resolver.add_module(
            ModulePath::new(
                PathOrigin::Package(PACKAGE.to_string()),
                vec![name.to_string()],
            ),
            Cow::Borrowed(*text),
        );
    }
    let options = wesl::CompileOptions {
        // Keep everything, so every function gets type-checked, called or not.
        strip: false,
        lazy: false,
        validate: false,
        features: wesl::Features {
            default: wesl::Feature::Disable,
            flags: features
                .iter()
                .map(|(name, on)| (name.to_string(), wesl::Feature::from(*on)))
                .collect(),
        },
        ..Default::default()
    };
    wesl::compile(&root, &resolver, &wesl::EscapeMangler, &options)
        .map(|result| result.to_string())
        .map_err(|error| error.to_string())
}

/// [`link`], then naga's full parse and validation of the result.
pub fn validate(source: &str, features: &[(&str, bool)]) -> Result<(), String> {
    let wgsl = link(source, features)?;
    let module =
        naga::front::wgsl::parse_str(&wgsl).map_err(|error| error.emit_to_string(&wgsl))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|error| error.emit_to_string(&wgsl))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_links_and_validates() {
        // As the root, so every declaration is kept and checked.
        for (name, source) in MODULES {
            validate(source, &[]).unwrap_or_else(|error| panic!("{name}: {error}"));
        }
    }

    #[test]
    fn modules_are_usable_from_an_entry_point() {
        let root = "\
import blockloom::fbm::{fbm2, fbm3, ridged3, turbulence2};
import blockloom::noise::{value2, value3, worley2, worley3};
import blockloom::hash::{hash21, hash33};
import blockloom::scattering::{henyey_greenstein, scatter_step, beer_powder};
import blockloom::frame::{FrameUniforms, exposure_scale, luminance, full_texel};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;

@fragment
fn main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = position.xy * frame.target_texel;
    let p = vec3<f32>(uv, frame.time);
    let density = fbm3(p, 5u, 2.0, 0.5) + ridged3(p, 3u, 2.0, 0.5) + turbulence2(uv, 4u, 2.0, 0.5)
        + fbm2(uv, 4u, 2.0, 0.5) + value2(uv) + value3(p) + worley2(uv).x + worley3(p).y
        + hash21(uv) + hash33(p).z + f32(full_texel(vec2<i32>(position.xy)).x);
    let phase = henyey_greenstein(dot(normalize(p), frame.sun_direction), 0.6);
    let light = scatter_step(frame.sun_color.rgb * phase, vec3<f32>(density), 0.5);
    let shade = light * exposure_scale(frame.exposure) * beer_powder(density, 1.0);
    return vec4<f32>(shade, luminance(shade));
}
";
        validate(root, &[]).unwrap();
    }

    #[test]
    fn features_pick_a_branch() {
        let root = "\
@if(WIDE) const WIDTH: f32 = 2.0;
@else const WIDTH: f32 = 1.0;
";
        assert!(link(root, &[("WIDE", true)]).unwrap().contains("2.0"));
        assert!(link(root, &[]).unwrap().contains("1.0"));
    }

    #[test]
    fn an_unknown_module_is_an_error() {
        let root = "import blockloom::nothing::here;\nfn f() -> f32 { return here(); }\n";
        assert!(validate(root, &[]).is_err());
    }
}
