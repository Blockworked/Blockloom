//! Environment volumes: regions that lay their own look over the project's
//! settings, blended by where the camera stands.
//!
//! The project's `World` is the global default. A `Volume` component turns
//! an actor into a box, a sphere or a global layer with a priority, a blend
//! distance and a weight, and every property it carries has an override
//! checkbox, HDRP-style: a cave can take fog and exposure without touching
//! the sky. The runtime sorts the volumes by priority and blends each one's
//! checked properties in at its weight at the camera.

use crate::scene::{TonemapName, World};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VolumeShape {
    /// A box around the actor, in its own frame.
    #[default]
    Box,
    /// A ball around the actor. A disc in 2D.
    Sphere,
    /// Everywhere at once: only the weight fades it.
    Global,
}

/// One overridable property: a checkbox beside a value. The value is kept
/// while unchecked, so ticking it back on brings back what was typed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Override<T> {
    #[serde(default)]
    pub on: bool,
    pub value: T,
}

impl<T: Clone> Override<T> {
    fn off(value: T) -> Self {
        Self { on: false, value }
    }

    /// The value when checked, `None` when the layer below should show.
    pub fn get(&self) -> Option<T> {
        self.on.then(|| self.value.clone())
    }
}

/// Every property a volume can take over, named as the project settings
/// name them. Unchecked values start at the project defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumeOverrides {
    pub background: Override<String>,
    pub sun_direction: Override<[f32; 3]>,
    pub sun_color: Override<String>,
    /// Lux.
    pub illuminance: Override<f32>,
    pub ambient_color: Override<String>,
    pub ambient_brightness: Override<f32>,
    pub ao: Override<bool>,
    /// EV100.
    pub exposure: Override<f32>,
    pub tonemapping: Override<TonemapName>,
    pub bloom: Override<bool>,
    pub bloom_threshold: Override<f32>,
    pub bloom_intensity: Override<f32>,
    pub vignette: Override<f32>,
    /// Multiplies reflection probes and the sky's light, so a cave can keep
    /// the sky out of its reflections.
    pub reflections: Override<f32>,
    /// Multiplies irradiance volumes.
    pub indirect: Override<f32>,
    /// EV added to the sky's brightness, background and light alike.
    pub sky_exposure: Override<f32>,
    /// Multiplies the sky's diffuse light alone.
    pub ambient_dimmer: Override<f32>,
    /// Height fog's extinction per metre at its base; turns it on inside.
    pub fog_density: Override<f32>,
    /// Height fog's color, whatever the time of day.
    pub fog_color: Override<String>,
    pub fog_height: Override<f32>,
    /// Volumetric fog's extinction per metre; turns it on inside.
    pub volumetric_density: Override<f32>,
    pub volumetric_albedo: Override<String>,
    /// Multiplies every light's beam density.
    pub beams: Override<f32>,
    /// Metres at which aerial haze takes half a far object's light.
    pub haze_distance: Override<f32>,
}

impl Default for VolumeOverrides {
    fn default() -> Self {
        let world = World::default();
        let lighting = &world.lighting;
        let post = &world.post;
        Self {
            background: Override::off(world.background.clone()),
            sun_direction: Override::off(lighting.light_direction),
            sun_color: Override::off(lighting.light_color.clone()),
            illuminance: Override::off(lighting.illuminance),
            ambient_color: Override::off(lighting.ambient_color.clone()),
            ambient_brightness: Override::off(lighting.ambient_brightness),
            ao: Override::off(lighting.ao_enabled),
            exposure: Override::off(post.exposure_ev),
            tonemapping: Override::off(post.tonemapping),
            bloom: Override::off(post.bloom_enabled),
            bloom_threshold: Override::off(post.bloom_threshold),
            bloom_intensity: Override::off(post.bloom_intensity),
            vignette: Override::off(post.vignette_strength),
            reflections: Override::off(1.0),
            indirect: Override::off(1.0),
            sky_exposure: Override::off(world.sky.exposure),
            ambient_dimmer: Override::off(world.sky.ambient_dimmer),
            fog_density: Override::off(crate::fog::density_for_distance(world.fog.height.distance)),
            fog_color: Override::off(world.fog.height.day_color.clone()),
            fog_height: Override::off(world.fog.height.base_height),
            volumetric_density: Override::off(world.fog.volumetric.density),
            volumetric_albedo: Override::off(world.fog.volumetric.albedo.clone()),
            beams: Override::off(1.0),
            haze_distance: Override::off(world.fog.aerial.distance),
        }
    }
}

impl VolumeOverrides {
    /// Names of the checked properties, in inspector order.
    pub fn checked(&self) -> Vec<&'static str> {
        [
            ("background", self.background.on),
            ("sun_direction", self.sun_direction.on),
            ("sun_color", self.sun_color.on),
            ("illuminance", self.illuminance.on),
            ("ambient_color", self.ambient_color.on),
            ("ambient_brightness", self.ambient_brightness.on),
            ("ao", self.ao.on),
            ("exposure", self.exposure.on),
            ("tonemapping", self.tonemapping.on),
            ("bloom", self.bloom.on),
            ("bloom_threshold", self.bloom_threshold.on),
            ("bloom_intensity", self.bloom_intensity.on),
            ("vignette", self.vignette.on),
            ("reflections", self.reflections.on),
            ("indirect", self.indirect.on),
            ("sky_exposure", self.sky_exposure.on),
            ("ambient_dimmer", self.ambient_dimmer.on),
            ("fog_density", self.fog_density.on),
            ("fog_color", self.fog_color.on),
            ("fog_height", self.fog_height.on),
            ("volumetric_density", self.volumetric_density.on),
            ("volumetric_albedo", self.volumetric_albedo.on),
            ("beams", self.beams.on),
            ("haze_distance", self.haze_distance.on),
        ]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect()
    }
}

/// The `Volume` component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeSpec {
    #[serde(default)]
    pub shape: VolumeShape,
    /// Half the box on each of the actor's own axes, before its scale.
    #[serde(default = "default_half_extents")]
    pub half_extents: [f32; 3],
    /// The sphere's radius, before the actor's scale.
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// Higher blends later, so it wins where two overlap. Ties go by id.
    #[serde(default)]
    pub priority: f32,
    /// World units outside the shape over which it fades out. Zero is a
    /// hard edge.
    #[serde(default = "default_blend_distance")]
    pub blend_distance: f32,
    /// 0-1, how much of the volume shows at full coverage.
    #[serde(default = "default_weight")]
    pub weight: f32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub overrides: VolumeOverrides,
    /// Fog it adds inside its shape, whoever is looking from where.
    #[serde(default)]
    pub fog: crate::fog::LocalFog,
    /// Wind it makes inside its shape.
    #[serde(default)]
    pub wind: crate::wind::LocalWind,
}

fn default_half_extents() -> [f32; 3] {
    [5.0; 3]
}

fn default_radius() -> f32 {
    5.0
}

fn default_blend_distance() -> f32 {
    1.0
}

fn default_weight() -> f32 {
    1.0
}

fn default_enabled() -> bool {
    true
}

impl Default for VolumeSpec {
    fn default() -> Self {
        Self {
            shape: VolumeShape::Box,
            half_extents: default_half_extents(),
            radius: default_radius(),
            priority: 0.0,
            blend_distance: default_blend_distance(),
            weight: default_weight(),
            enabled: true,
            overrides: VolumeOverrides::default(),
            fog: crate::fog::LocalFog::default(),
            wind: crate::wind::LocalWind::default(),
        }
    }
}

/// Where a volume's actor stands, as plain arrays so hosts on another glam
/// can hand it over. `rotation` is a quaternion, x y z w.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumePose {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for VolumePose {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}

impl VolumeSpec {
    /// How far outside the shape `point` is, in world units: zero inside.
    /// `flat` drops depth, for a 2D world where z only sorts.
    pub fn distance_outside(&self, pose: &VolumePose, point: [f32; 3], flat: bool) -> f32 {
        let flatten = |v: Vec3| if flat { v.with_z(0.0) } else { v };
        let centre = Vec3::from(pose.position);
        let scale = Vec3::from(pose.scale).abs();
        match self.shape {
            VolumeShape::Global => 0.0,
            VolumeShape::Sphere => {
                // A stretched ball is measured by its largest axis.
                let radius = self.radius.max(0.0) * scale.max_element();
                (flatten(Vec3::from(point) - centre).length() - radius).max(0.0)
            }
            VolumeShape::Box => {
                let rotation = Quat::from_array(pose.rotation).normalize();
                let rotation = if rotation.is_finite() {
                    rotation
                } else {
                    Quat::IDENTITY
                };
                let local = rotation.inverse() * (Vec3::from(point) - centre);
                let half = Vec3::from(self.half_extents).max(Vec3::ZERO) * scale;
                // Outside distance to an oriented box, in world units since
                // the rotation keeps lengths.
                flatten((local.abs() - half).max(Vec3::ZERO)).length()
            }
        }
    }

    /// 1 inside the shape, easing to 0 across the blend distance outside it.
    /// Leaves out the weight and the switch.
    pub fn coverage(&self, pose: &VolumePose, point: [f32; 3], flat: bool) -> f32 {
        let outside = self.distance_outside(pose, point, flat);
        if outside <= 0.0 {
            return 1.0;
        }
        let blend = self.blend_distance.max(0.0);
        if blend <= 0.0 || !outside.is_finite() {
            return 0.0;
        }
        (1.0 - outside / blend).clamp(0.0, 1.0)
    }
}

/// The blend order: lowest priority first, so the highest lands last and
/// wins. Ties go by id so every run blends the same way.
pub fn blend_order<T>(volumes: &mut [T], key: impl Fn(&T) -> (f32, &str)) {
    volumes.sort_by(|a, b| {
        let (pa, ia) = key(a);
        let (pb, ib) = key(b);
        pa.total_cmp(&pb).then_with(|| ia.cmp(ib))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(position: [f32; 3]) -> VolumePose {
        VolumePose {
            position,
            ..VolumePose::default()
        }
    }

    #[test]
    fn a_box_covers_its_inside_and_fades_across_the_blend_distance() {
        let volume = VolumeSpec {
            half_extents: [2.0, 1.0, 1.0],
            blend_distance: 2.0,
            ..VolumeSpec::default()
        };
        let pose = at([10.0, 0.0, 0.0]);
        assert_eq!(volume.coverage(&pose, [11.9, 0.0, 0.0], false), 1.0);
        assert!((volume.coverage(&pose, [13.0, 0.0, 0.0], false) - 0.5).abs() < 1e-5);
        assert_eq!(volume.coverage(&pose, [15.0, 0.0, 0.0], false), 0.0);
    }

    #[test]
    fn a_box_turns_and_scales_with_its_actor() {
        let volume = VolumeSpec {
            half_extents: [4.0, 1.0, 1.0],
            blend_distance: 0.0,
            ..VolumeSpec::default()
        };
        let turned = VolumePose {
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array(),
            ..VolumePose::default()
        };
        // Long along y now, short along x.
        assert_eq!(volume.coverage(&turned, [0.0, 3.5, 0.0], false), 1.0);
        assert_eq!(volume.coverage(&turned, [3.5, 0.0, 0.0], false), 0.0);
        let doubled = VolumePose {
            scale: [2.0; 3],
            ..VolumePose::default()
        };
        assert_eq!(volume.coverage(&doubled, [7.5, 0.0, 0.0], false), 1.0);
    }

    #[test]
    fn a_sphere_fades_from_its_surface() {
        let volume = VolumeSpec {
            shape: VolumeShape::Sphere,
            radius: 3.0,
            blend_distance: 1.0,
            ..VolumeSpec::default()
        };
        let pose = VolumePose::default();
        assert_eq!(volume.coverage(&pose, [0.0, 2.9, 0.0], false), 1.0);
        assert!((volume.coverage(&pose, [0.0, 0.0, 3.25], false) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn flat_worlds_ignore_depth() {
        let volume = VolumeSpec {
            shape: VolumeShape::Sphere,
            radius: 1.0,
            blend_distance: 0.0,
            ..VolumeSpec::default()
        };
        let pose = VolumePose::default();
        assert_eq!(volume.coverage(&pose, [0.5, 0.0, 999.0], true), 1.0);
        assert_eq!(volume.coverage(&pose, [0.5, 0.0, 999.0], false), 0.0);
    }

    #[test]
    fn a_global_volume_covers_everywhere() {
        let volume = VolumeSpec {
            shape: VolumeShape::Global,
            ..VolumeSpec::default()
        };
        assert_eq!(volume.coverage(&at([1e6; 3]), [0.0; 3], false), 1.0);
    }

    #[test]
    fn blend_order_puts_the_highest_priority_last() {
        let mut volumes = vec![(2.0, "b"), (-1.0, "z"), (2.0, "a")];
        blend_order(&mut volumes, |v| (v.0, v.1));
        assert_eq!(volumes, vec![(-1.0, "z"), (2.0, "a"), (2.0, "b")]);
    }

    #[test]
    fn unchecked_overrides_keep_their_value_and_read_as_none() {
        let json = r#"{"shape":"Sphere","overrides":{"exposure":{"on":true,"value":6.0},"bloom":{"value":true}}}"#;
        let volume: VolumeSpec = serde_json::from_str(json).unwrap();
        assert_eq!(volume.shape, VolumeShape::Sphere);
        assert_eq!(volume.overrides.exposure.get(), Some(6.0));
        assert_eq!(volume.overrides.bloom.get(), None);
        assert!(volume.overrides.bloom.value);
        assert_eq!(volume.overrides.checked(), vec!["exposure"]);
        assert_eq!(volume.weight, 1.0);
        assert!(volume.enabled);
    }
}
