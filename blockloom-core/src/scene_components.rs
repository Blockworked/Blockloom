//! A scene's settings, as components.
//!
//! Unity edits a scene's lighting, sky, fog and physics in the inspector the
//! way it edits an actor: each setting is a component on the scene itself
//! rather than a fixed field. Blockloom's scenes work the same way on disk -
//! a `.blockscene` asset file carries its settings as a component list - while
//! in memory a [`crate::scene::World`] stays the form the runtime reads.
//! [`SceneComponents`] converts both ways, so the editor, the inspector and
//! the file format speak components and nothing else has to move.

use crate::cloud_layers::CloudLayer;
use crate::clouds::Clouds;
use crate::director::Director;
use crate::fog::Fog;
use crate::input::InputConfig;
use crate::lightning::Lightning;
use crate::material::SurfaceWeather;
use crate::nav::NavSettings;
use crate::scene::{Camera, DisplayOutput, Lighting, Mode, PostProcess, SpeechBubbleStyle, World};
use crate::sky::Sky;
use crate::sound::SoundMixer;
use crate::ui::UiDocument;
use crate::vfx::VfxSettings;
use crate::wind::Wind;
use blockloom_schema_derive::Schema;
use serde::{Deserialize, Serialize};

/// The scene components every project knows about by name.
pub const BUILT_IN_SCENE_NAMES: &[&str] = &[
    "Dimension",
    "Background",
    "Physics",
    "Camera",
    "SpeechBubble",
    "Lighting",
    "Soundscape",
    "Post",
    "Navigation",
    "Sky",
    "Fog",
    "Clouds",
    "CloudLayers",
    "Lightning",
    "Wind",
    "Director",
    "Surface",
    "Vfx",
    "Cutscenes",
    "Interface",
    "Lighting2d",
    "Post2d",
    "Camera2d",
];

/// One setting on a scene. Serialized internally-tagged, so a component reads
/// as `{"component": "Lighting", "lighting": {...}}` - the same shape actor
/// components use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
#[serde(tag = "component")]
#[allow(clippy::large_enum_variant)]
pub enum SceneComponent {
    /// Quality, Sound, Input and Display are project settings now
    /// ([`crate::settings::GameSettings`]); these four are only read, from
    /// older scene files, and moved there on load.
    Quality { settings: crate::quality::Settings },
    /// Which dimension the scene runs in.
    Dimension { mode: Mode },
    /// The clear color behind everything.
    Background { color: String },
    /// Gravity and the fixed tick rate physics and blocks advance on.
    Physics { gravity: [f32; 3], fixed_rate: f32 },
    /// Where the camera stands when nothing holds it.
    Camera { camera: Camera },
    /// The project-wide presentation of `say` bubbles.
    SpeechBubble { style: SpeechBubbleStyle },
    /// Sun, ambient, shadows and ray tracing.
    Lighting { lighting: Lighting },
    /// One gain per sound bus.
    Sound { mixer: SoundMixer },
    /// Weather loops, ducking, reverb, occlusion and the underwater muffle.
    Soundscape {
        soundscape: crate::soundscape::Soundscape,
    },
    /// Named input actions and their bindings.
    Input { config: InputConfig },
    /// The post chain, exposure to grain.
    Post { post: PostProcess },
    /// The output signal, peak brightness and paper white.
    Display { display: DisplayOutput },
    /// Cost regions and explicit links in the navigation plane.
    Navigation { settings: NavSettings },
    /// Background, ambient light and reflections, 3D only.
    Sky { sky: Sky },
    /// Height fog, volumetric fog and aerial perspective, 3D only.
    Fog { fog: Fog },
    /// Volumetric clouds.
    Clouds { clouds: Clouds },
    /// Planar cloud layers, 3D only.
    CloudLayers { layers: Vec<CloudLayer> },
    /// Strikes and the storm that throws them.
    Lightning { lightning: Lightning },
    /// The wind everything that moves with the air reads.
    Wind { wind: Wind },
    /// The 24h clock, curve tracks and weather presets.
    Director { director: Director },
    /// Snow cover and wetness, which surface masks scale by.
    Surface { surface: SurfaceWeather },
    /// The particle budget and where emitters simulate.
    Vfx { settings: VfxSettings },
    /// Named camera reels that `play cutscene` runs.
    Cutscenes {
        cutscenes: Vec<crate::cinematic::Cutscene>,
    },
    /// The screen-space interface document.
    Interface { document: UiDocument },
    /// The 2D lit look: ambient, its ramp and where lighting stops.
    Lighting2d {
        lighting2d: crate::light2d::Lighting2d,
    },
    /// 2D post: pixelation, palette, outline and CRT looks.
    Post2d { post2d: crate::post2d::Post2d },
    /// 2D camera finishing: locks, bounds, zoom, pixel snap, shake.
    Camera2d {
        camera2d: crate::camera2d::Camera2dSettings,
    },
    /// The `.blockenv` this scene's look follows, and what it overrides.
    /// Only carried while there is one.
    Environment {
        environment: crate::linked::AssetLink,
    },
}

impl SceneComponent {
    /// What the inspector and the blocks call this component.
    /// Quality, input, sound and display: project settings that older
    /// documents kept on each scene.
    pub fn is_game_setting(&self) -> bool {
        matches!(
            self,
            SceneComponent::Quality { .. }
                | SceneComponent::Input { .. }
                | SceneComponent::Sound { .. }
                | SceneComponent::Display { .. }
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            SceneComponent::Quality { .. } => "Quality",
            SceneComponent::Dimension { .. } => "Dimension",
            SceneComponent::Background { .. } => "Background",
            SceneComponent::Physics { .. } => "Physics",
            SceneComponent::Camera { .. } => "Camera",
            SceneComponent::SpeechBubble { .. } => "SpeechBubble",
            SceneComponent::Lighting { .. } => "Lighting",
            SceneComponent::Sound { .. } => "Sound",
            SceneComponent::Soundscape { .. } => "Soundscape",
            SceneComponent::Input { .. } => "Input",
            SceneComponent::Post { .. } => "Post",
            SceneComponent::Display { .. } => "Display",
            SceneComponent::Navigation { .. } => "Navigation",
            SceneComponent::Sky { .. } => "Sky",
            SceneComponent::Fog { .. } => "Fog",
            SceneComponent::Clouds { .. } => "Clouds",
            SceneComponent::CloudLayers { .. } => "CloudLayers",
            SceneComponent::Lightning { .. } => "Lightning",
            SceneComponent::Wind { .. } => "Wind",
            SceneComponent::Director { .. } => "Director",
            SceneComponent::Surface { .. } => "Surface",
            SceneComponent::Vfx { .. } => "Vfx",
            SceneComponent::Cutscenes { .. } => "Cutscenes",
            SceneComponent::Interface { .. } => "Interface",
            SceneComponent::Lighting2d { .. } => "Lighting2d",
            SceneComponent::Post2d { .. } => "Post2d",
            SceneComponent::Camera2d { .. } => "Camera2d",
            SceneComponent::Environment { .. } => "Environment",
        }
    }
}

/// A scene's settings as the inspector shows them: one entry per component,
/// in a stable order. Kept as a list rather than a map so the file has a
/// stable order to read and the inspector has one to draw.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Schema)]
#[serde(transparent)]
pub struct SceneComponents(pub Vec<SceneComponent>);

impl SceneComponents {
    /// Every setting in `world`, in inspector order. A scene file written
    /// from this always carries the whole list, so an old file missing one
    /// simply reads as its default.
    pub fn from_world(world: &World) -> Self {
        let mut list = vec![
            SceneComponent::Dimension { mode: world.mode },
            SceneComponent::Background {
                color: world.background.clone(),
            },
            SceneComponent::Physics {
                gravity: world.gravity,
                fixed_rate: world.fixed_rate,
            },
            SceneComponent::Camera {
                camera: world.camera.clone(),
            },
            SceneComponent::SpeechBubble {
                style: world.speech_bubble.clone(),
            },
            SceneComponent::Lighting {
                lighting: world.lighting.clone(),
            },
            SceneComponent::Soundscape {
                soundscape: world.soundscape.clone(),
            },
            SceneComponent::Post {
                post: world.post.clone(),
            },
            SceneComponent::Navigation {
                settings: world.navigation.clone(),
            },
            SceneComponent::Sky {
                sky: world.sky.clone(),
            },
            SceneComponent::Fog {
                fog: world.fog.clone(),
            },
            SceneComponent::Clouds {
                clouds: world.clouds.clone(),
            },
            SceneComponent::CloudLayers {
                layers: world.cloud_layers.clone(),
            },
            SceneComponent::Lightning {
                lightning: world.lightning.clone(),
            },
            SceneComponent::Wind {
                wind: world.wind.clone(),
            },
            SceneComponent::Director {
                director: world.director.clone(),
            },
            SceneComponent::Surface {
                surface: world.surface,
            },
            SceneComponent::Vfx {
                settings: world.vfx.clone(),
            },
            SceneComponent::Cutscenes {
                cutscenes: world.cutscenes.clone(),
            },
            SceneComponent::Interface {
                document: world.interface.clone(),
            },
            SceneComponent::Lighting2d {
                lighting2d: world.lighting2d.clone(),
            },
            SceneComponent::Post2d {
                post2d: world.post2d.clone(),
            },
            SceneComponent::Camera2d {
                camera2d: world.camera2d.clone(),
            },
        ];
        if let Some(environment) = &world.environment {
            list.push(SceneComponent::Environment {
                environment: environment.clone(),
            });
        }
        Self(list)
    }

    /// A `World` with every carried component laid over the default. A
    /// component the list doesn't name reads as its default - which is what
    /// removing one in the inspector means.
    pub fn to_world(&self) -> World {
        let mode = self
            .0
            .iter()
            .find_map(|component| match component {
                SceneComponent::Dimension { mode } => Some(*mode),
                _ => None,
            })
            .unwrap_or_default();
        let mut world = World {
            mode,
            gravity: World::default_gravity(mode),
            ..World::default()
        };
        self.apply_to_world(&mut world);
        world
    }

    /// Lays every carried component over `world`, leaving anything unmentioned
    /// alone. Loading a scene file starts from the default (see `to_world`);
    /// setting one component in the editor starts from the scene's own world.
    pub fn apply_to_world(&self, world: &mut World) {
        for component in &self.0 {
            match component {
                SceneComponent::Quality { settings } => {
                    world.legacy_quality = Some(settings.clone())
                }
                SceneComponent::Dimension { mode } => world.mode = *mode,
                SceneComponent::Background { color } => world.background.clone_from(color),
                SceneComponent::Physics {
                    gravity,
                    fixed_rate,
                } => {
                    world.gravity = *gravity;
                    world.fixed_rate = *fixed_rate;
                }
                SceneComponent::Camera { camera } => world.camera.clone_from(camera),
                SceneComponent::SpeechBubble { style } => {
                    world.speech_bubble.clone_from(style);
                }
                SceneComponent::Lighting { lighting } => world.lighting.clone_from(lighting),
                SceneComponent::Sound { mixer } => world.legacy_sound = Some(*mixer),
                SceneComponent::Soundscape { soundscape } => {
                    world.soundscape.clone_from(soundscape);
                }
                SceneComponent::Input { config } => world.legacy_input = Some(config.clone()),
                SceneComponent::Post { post } => world.post.clone_from(post),
                SceneComponent::Display { display } => world.legacy_display = Some(*display),
                SceneComponent::Navigation { settings } => {
                    world.navigation.clone_from(settings);
                }
                SceneComponent::Sky { sky } => world.sky.clone_from(sky),
                SceneComponent::Fog { fog } => world.fog.clone_from(fog),
                SceneComponent::Clouds { clouds } => world.clouds.clone_from(clouds),
                SceneComponent::CloudLayers { layers } => world.cloud_layers.clone_from(layers),
                SceneComponent::Lightning { lightning } => {
                    world.lightning.clone_from(lightning);
                }
                SceneComponent::Wind { wind } => world.wind.clone_from(wind),
                SceneComponent::Director { director } => world.director.clone_from(director),
                SceneComponent::Surface { surface } => world.surface.clone_from(surface),
                SceneComponent::Vfx { settings } => world.vfx.clone_from(settings),
                SceneComponent::Cutscenes { cutscenes } => world.cutscenes.clone_from(cutscenes),
                SceneComponent::Interface { document } => world.interface.clone_from(document),
                SceneComponent::Lighting2d { lighting2d } => {
                    world.lighting2d.clone_from(lighting2d)
                }
                SceneComponent::Post2d { post2d } => world.post2d.clone_from(post2d),
                SceneComponent::Camera2d { camera2d } => world.camera2d.clone_from(camera2d),
                SceneComponent::Environment { environment } => {
                    world.environment = Some(environment.clone());
                }
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&SceneComponent> {
        self.0.iter().find(|c| c.name() == name)
    }

    /// Adds `component`, replacing one of the same name. Answers whether it
    /// was new.
    pub fn insert(&mut self, component: SceneComponent) -> bool {
        match self.0.iter_mut().find(|c| c.name() == component.name()) {
            Some(slot) => {
                *slot = component;
                false
            }
            None => {
                self.0.push(component);
                true
            }
        }
    }

    /// Drops the named component. Reads of it fall back to its default.
    pub fn remove(&mut self, name: &str) -> bool {
        let Some(index) = self.0.iter().position(|c| c.name() == name) else {
            return false;
        };
        self.0.remove(index);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_round_trips_through_components() {
        let mut world = World {
            background: "#102030".to_string(),
            fixed_rate: 120.0,
            ..World::default()
        };
        // The 2D look survives a save too.
        world.camera2d.lock_x = true;
        let components = SceneComponents::from_world(&world);
        assert_eq!(components.0.len(), BUILT_IN_SCENE_NAMES.len());
        assert_eq!(components.to_world(), world);
        world.environment = Some(Default::default());
        let linked = SceneComponents::from_world(&world);
        assert_eq!(linked.0.len(), BUILT_IN_SCENE_NAMES.len() + 1);
        assert_eq!(linked.to_world(), world);
    }

    #[test]
    fn a_missing_component_reads_as_its_default() {
        let mut components = SceneComponents::from_world(&World::default());
        assert!(components.remove("Lighting"));
        assert!(!components.remove("Lighting"));
        assert_eq!(components.to_world(), World::default());
    }

    #[test]
    fn resetting_physics_keeps_the_scenes_dimension_defaults() {
        let world = World {
            mode: Mode::ThreeD,
            gravity: [1.0, -50.0, 2.0],
            ..World::default()
        };
        let mut components = SceneComponents::from_world(&world);
        assert!(components.remove("Physics"));
        let reset = components.to_world();
        assert_eq!(reset.mode, Mode::ThreeD);
        assert_eq!(reset.gravity, World::default_gravity(Mode::ThreeD));
        assert_eq!(reset.fixed_rate, crate::scene::DEFAULT_FIXED_RATE);
    }

    #[test]
    fn inserting_replaces_a_component_of_the_same_name() {
        let mut components = SceneComponents::from_world(&World::default());
        assert!(!components.insert(SceneComponent::Background {
            color: "#FF0000".to_string(),
        }));
        assert_eq!(components.to_world().background, "#FF0000".to_string());
    }

    #[test]
    fn a_component_reads_as_its_tagged_json_shape() {
        let json = serde_json::to_value(SceneComponent::Dimension { mode: Mode::ThreeD }).unwrap();
        assert_eq!(json["component"], "Dimension");
        assert_eq!(json["mode"], "ThreeD");
    }
}
