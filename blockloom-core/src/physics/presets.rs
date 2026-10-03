//! Player presets: one transaction that gives an actor everything a playable
//! character needs - a visual, a controller, a motor, a camera and the
//! standard input actions - and a preview of what that will change.

use super::BodyType;
use super::controller::CharacterControllerSpec;
use super::material::MaterialLibrary;
use super::motor::{CharacterMotorSpec, MotorOwner, MoveSpace};
use crate::components::{ActorComponent, CameraAttach, CameraView};
use crate::input::InputAction;
use crate::player_camera::PlayerCameraSpec;
use crate::project::Scene;
use crate::scene::{Mode, Visual};
use serde::{Deserialize, Serialize};

/// A ready-made kind of player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlayerPreset {
    FirstPerson3d,
    ThirdPerson3d,
    TopDown3d,
    Platformer2d,
    TopDown2d,
}

impl PlayerPreset {
    pub const ALL: [PlayerPreset; 5] = [
        Self::FirstPerson3d,
        Self::ThirdPerson3d,
        Self::TopDown3d,
        Self::Platformer2d,
        Self::TopDown2d,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::FirstPerson3d => "first-person-3d",
            Self::ThirdPerson3d => "third-person-3d",
            Self::TopDown3d => "top-down-3d",
            Self::Platformer2d => "platformer-2d",
            Self::TopDown2d => "top-down-2d",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::FirstPerson3d => "First person (3D)",
            Self::ThirdPerson3d => "Third person (3D)",
            Self::TopDown3d => "Top down (3D)",
            Self::Platformer2d => "Platformer (2D)",
            Self::TopDown2d => "Top down (2D)",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        let flat = |text: &str| {
            text.trim()
                .to_ascii_lowercase()
                .replace([' ', '_', '(', ')'], "-")
                .replace("--", "-")
                .trim_matches('-')
                .to_string()
        };
        let word = flat(word);
        Self::ALL
            .into_iter()
            .find(|p| p.name() == word || flat(p.label()) == word)
    }

    /// The dimension the preset is made for.
    pub fn mode(self) -> Mode {
        match self {
            Self::FirstPerson3d | Self::ThirdPerson3d | Self::TopDown3d => Mode::ThreeD,
            Self::Platformer2d | Self::TopDown2d => Mode::TwoD,
        }
    }

    /// The components the preset installs.
    pub fn parts(self) -> PresetParts {
        let mode = self.mode();
        let mut controller = CharacterControllerSpec::for_mode(mode);
        let mut motor = CharacterMotorSpec::for_mode(mode);
        motor.owner = MotorOwner::Player;
        let mut camera = CameraAttach::default();
        let mut player_camera = PlayerCameraSpec::default();
        let mut boom = None;
        match self {
            Self::FirstPerson3d => {
                motor.space = MoveSpace::Camera;
                motor.turn_speed = 0.0;
                camera.view = CameraView::FirstPerson;
                camera.offset = [0.0, 0.7, 0.0];
                camera.pitch = 0.0;
                player_camera = PlayerCameraSpec::first_person();
            }
            Self::ThirdPerson3d => {
                motor.space = MoveSpace::Camera;
                camera.view = CameraView::ThirdPerson;
                camera.offset = [0.0, 0.6, 0.0];
                camera.distance = 5.0;
                camera.pitch = 15.0;
                player_camera.zoom = true;
            }
            Self::TopDown3d => {
                motor.space = MoveSpace::World;
                camera.view = CameraView::Follow;
                player_camera.look = false;
                player_camera.collision = false;
                player_camera.smoothing = 0.2;
                boom = Some(([0.0, 14.0, 8.0], [0.0, 0.0, 0.0]));
            }
            Self::Platformer2d => {
                motor.space = MoveSpace::World;
                camera.view = CameraView::Follow;
                player_camera = PlayerCameraSpec::for_2d();
            }
            Self::TopDown2d => {
                motor.space = MoveSpace::World;
                motor.top_down = true;
                motor.jump_height = 0.0;
                motor.max_jumps = 0;
                motor.crouch_height = 0.0;
                camera.view = CameraView::Follow;
                player_camera = PlayerCameraSpec::for_2d();
                player_camera.dead_zone = [24.0, 24.0];
                player_camera.look_ahead = 0.0;
            }
        }
        controller.layer = 1;
        let visual = match mode {
            Mode::ThreeD => Visual::Capsule {
                color: "#4C97FF".to_string(),
                radius: controller.radius,
                height: (controller.height - 2.0 * controller.radius).max(0.0),
            },
            Mode::TwoD => Visual::Rect {
                color: "#4C97FF".to_string(),
                size: [controller.radius * 2.0, controller.height],
            },
        };
        PresetParts {
            visual,
            controller,
            motor: Some(motor),
            camera: Some(camera),
            player_camera: Some(player_camera),
            boom,
            actions: Vec::new(),
        }
    }
}

/// What a preset installs.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetParts {
    pub visual: Visual,
    pub controller: CharacterControllerSpec,
    pub motor: Option<CharacterMotorSpec>,
    pub camera: Option<CameraAttach>,
    pub player_camera: Option<PlayerCameraSpec>,
    /// A top-down 3D view's camera offset and aim point, if it sets them.
    pub boom: Option<([f32; 3], [f32; 3])>,
    /// Input actions to add when the project has none of that name; empty
    /// means the standard player actions.
    pub actions: Vec<InputAction>,
}

/// What happens to one component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StepKind {
    /// The actor has none; the preset adds one.
    Add,
    /// The actor has one; the preset replaces its settings.
    Replace,
    /// Already as the preset wants it.
    Keep,
    /// The preset takes it away (only with the conversion).
    Remove,
    /// The preset changes it in place (only with the conversion).
    Convert,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PresetStep {
    pub component: String,
    pub kind: StepKind,
    pub detail: String,
}

/// Something already on the actor that cannot sit beside a player.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PresetConflict {
    pub component: String,
    pub reason: String,
    /// What converting does about it.
    pub conversion: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PresetPreview {
    pub preset: PlayerPreset,
    pub actor: String,
    pub steps: Vec<PresetStep>,
    pub conflicts: Vec<PresetConflict>,
    /// Why the preset cannot be applied at all (wrong dimension, no actor).
    pub blocked: Option<String>,
}

impl PresetPreview {
    /// Whether applying needs the creator to accept a conversion.
    pub fn needs_conversion(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

impl Scene {
    /// What `apply_player_preset` would do to `actor_id`, without doing it.
    pub fn preview_player_preset(&self, actor_id: &str, preset: PlayerPreset) -> PresetPreview {
        let mut preview = PresetPreview {
            preset,
            actor: actor_id.to_string(),
            steps: Vec::new(),
            conflicts: Vec::new(),
            blocked: None,
        };
        let Some(actor) = self.actors.iter().find(|a| a.id == actor_id) else {
            preview.blocked = Some("Actor not found".to_string());
            return preview;
        };
        if self.world.mode != preset.mode() {
            preview.blocked = Some(format!(
                "{} needs a {} scene; this one is {}",
                preset.label(),
                dimension(preset.mode()),
                dimension(self.world.mode)
            ));
            return preview;
        }
        let components = &actor.components;
        let mut push = |component: &str, kind: StepKind, detail: &str| {
            preview.steps.push(PresetStep {
                component: component.to_string(),
                kind,
                detail: detail.to_string(),
            });
        };
        if components.visual().is_none() {
            push(
                "Look",
                StepKind::Add,
                "a plain capsule or rectangle so it shows before any art",
            );
        } else {
            push("Look", StepKind::Keep, "your visual stays");
        }
        let kind = |present: bool| {
            if present {
                StepKind::Replace
            } else {
                StepKind::Add
            }
        };
        push(
            "CharacterController",
            kind(components.character_controller().is_some()),
            "a capsule that slides along the world",
        );
        push(
            "CharacterMotor",
            kind(components.character_motor().is_some()),
            "walking, sprinting, jumping and crouching",
        );
        push(
            "Camera",
            kind(components.contains("Camera")),
            match preset {
                PlayerPreset::FirstPerson3d => "eye-height first person view",
                PlayerPreset::ThirdPerson3d => "an orbiting view behind the player",
                _ => "a following view",
            },
        );
        push(
            "PlayerCamera",
            kind(components.player_camera().is_some()),
            "look input, wall avoidance and follow policy",
        );
        let missing: Vec<String> = ["Move", "Look", "Jump", "Sprint", "Crouch", "Interact"]
            .iter()
            .filter(|n| self.world.input.find(n).is_none())
            .map(|n| n.to_string())
            .collect();
        if missing.is_empty() {
            push(
                "Input",
                StepKind::Keep,
                "the standard actions already exist",
            );
        } else {
            push(
                "Input",
                StepKind::Add,
                &format!("actions {}", missing.join(", ")),
            );
        }
        if self
            .actors
            .iter()
            .any(|a| a.id != actor_id && a.components.contains("Camera"))
        {
            push(
                "Camera",
                StepKind::Remove,
                "the world camera moves from another actor to this one",
            );
        }
        if components
            .rigidbody()
            .is_some_and(|b| b.body_type == BodyType::Dynamic)
        {
            preview.conflicts.push(PresetConflict {
                component: "Rigidbody".to_string(),
                reason: "a dynamic Rigidbody is moved by physics, a CharacterController by its own sweeps".to_string(),
                conversion: "the Rigidbody becomes kinematic".to_string(),
            });
        }
        if components.contains("Body") {
            preview.conflicts.push(PresetConflict {
                component: "Body".to_string(),
                reason: "a legacy Body and a controller would both own the actor's physics"
                    .to_string(),
                conversion: "the legacy Body is removed".to_string(),
            });
        }
        preview
    }

    /// Installs the preset on `actor_id` as one step: either everything lands
    /// or the actor is left as it was. Conflicts need `convert`.
    pub fn apply_player_preset(
        &mut self,
        actor_id: &str,
        preset: PlayerPreset,
        convert: bool,
        library: &MaterialLibrary,
    ) -> Result<PresetPreview, String> {
        let preview = self.preview_player_preset(actor_id, preset);
        if let Some(why) = &preview.blocked {
            return Err(why.clone());
        }
        self.install_player(actor_id, preset.parts(), &preview, convert, library)?;
        Ok(preview)
    }

    /// Puts `parts` on the actor; shared by presets and saved profiles.
    pub(crate) fn install_player(
        &mut self,
        actor_id: &str,
        parts: PresetParts,
        preview: &PresetPreview,
        convert: bool,
        library: &MaterialLibrary,
    ) -> Result<(), String> {
        if preview.needs_conversion() && !convert {
            let what: Vec<String> = preview
                .conflicts
                .iter()
                .map(|c| format!("{}: {}", c.component, c.reason))
                .collect();
            return Err(format!(
                "This actor already moves some other way ({}). Apply with conversion to switch it.",
                what.join("; ")
            ));
        }
        let before_actors = self.actors.clone();
        let before_input = self.world.input.clone();
        let before_camera = self.world.camera.clone();
        let result = (|| {
            self.world.input.add_player_actions();
            for action in &parts.actions {
                if self.world.input.find(&action.name).is_none() {
                    self.world.input.actions.push(action.clone());
                }
            }
            if let Some((position, look_at)) = parts.boom {
                self.world.camera.position = position;
                self.world.camera.look_at = look_at;
            }
            if parts.camera.is_some() {
                for other in &mut self.actors {
                    if other.id != actor_id {
                        other.components.remove("Camera");
                    }
                }
            }
            let Some(actor) = self.actors.iter_mut().find(|a| a.id == actor_id) else {
                return Err("Actor not found".to_string());
            };
            if convert {
                actor.components.remove("Body");
                for component in &mut actor.components.0 {
                    if let ActorComponent::Rigidbody { rigidbody } = component
                        && rigidbody.body_type == BodyType::Dynamic
                    {
                        rigidbody.body_type = BodyType::Kinematic;
                    }
                }
            }
            if actor.components.visual().is_none() {
                actor.components.set_visual(parts.visual);
            }
            // Keep identities across a re-apply.
            let mut controller = parts.controller;
            if let Some(old) = actor.components.character_controller() {
                controller.id = old.id.clone();
            }
            actor
                .components
                .insert(ActorComponent::CharacterController { controller });
            if let Some(mut motor) = parts.motor {
                if let Some(old) = actor.components.character_motor() {
                    motor.id = old.id.clone();
                }
                actor
                    .components
                    .insert(ActorComponent::CharacterMotor { motor });
            }
            if let Some(camera) = parts.camera {
                actor.components.insert(ActorComponent::Camera { camera });
            }
            if let Some(player_camera) = parts.player_camera {
                actor
                    .components
                    .insert(ActorComponent::PlayerCamera { player_camera });
            }
            Ok(())
        })();
        let verdict = result.and_then(|()| {
            let errors: Vec<_> = self
                .physics_issues(library)
                .into_iter()
                .filter(|i| i.is_error() && i.actor.as_deref() == Some(actor_id))
                .collect();
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors
                    .iter()
                    .map(|i| i.message.clone())
                    .collect::<Vec<_>>()
                    .join("; "))
            }
        });
        if let Err(why) = verdict {
            self.actors = before_actors;
            self.world.input = before_input;
            self.world.camera = before_camera;
            return Err(why);
        }
        Ok(())
    }
}

/// A player setup saved for reuse: the controller, motor, camera and the
/// input actions they read, in one file that can move between projects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerProfile {
    pub version: u32,
    pub name: String,
    pub mode: Mode,
    pub controller: CharacterControllerSpec,
    #[serde(default)]
    pub motor: Option<CharacterMotorSpec>,
    #[serde(default)]
    pub camera: Option<CameraAttach>,
    #[serde(default)]
    pub player_camera: Option<PlayerCameraSpec>,
    /// The input actions the motor and camera read, bindings included.
    #[serde(default)]
    pub actions: Vec<InputAction>,
}

impl PlayerProfile {
    pub const VERSION: u32 = 1;

    /// Reads the player setup off an actor.
    pub fn capture(scene: &Scene, actor_id: &str, name: &str) -> Result<Self, String> {
        let actor = scene
            .actors
            .iter()
            .find(|a| a.id == actor_id)
            .ok_or("Actor not found")?;
        let c = &actor.components;
        let controller = c
            .character_controller()
            .cloned()
            .ok_or("This actor has no CharacterController to save")?;
        let camera = match c.get("Camera") {
            Some(ActorComponent::Camera { camera }) => Some(*camera),
            _ => None,
        };
        let actions = ["Move", "Look", "Jump", "Sprint", "Crouch", "Interact"]
            .iter()
            .filter_map(|n| scene.world.input.find(n).cloned())
            .collect();
        Ok(Self {
            version: Self::VERSION,
            name: name.to_string(),
            mode: scene.world.mode,
            controller,
            motor: c.character_motor().cloned(),
            camera,
            player_camera: c.player_camera().copied(),
            actions,
        })
    }

    /// What applying would do, using the same report a preset gives.
    pub fn preview(&self, scene: &Scene, actor_id: &str) -> PresetPreview {
        let base = match (self.mode, &self.motor) {
            (Mode::TwoD, Some(m)) if m.top_down => PlayerPreset::TopDown2d,
            (Mode::TwoD, _) => PlayerPreset::Platformer2d,
            (Mode::ThreeD, _) => PlayerPreset::ThirdPerson3d,
        };
        let mut preview = scene.preview_player_preset(actor_id, base);
        if self.version > Self::VERSION {
            preview.blocked = Some(format!(
                "\"{}\" was saved by a newer Blockloom (profile version {})",
                self.name, self.version
            ));
        }
        preview
    }

    /// Installs the profile on an actor as one undoable step. Input actions
    /// the project already has keep their own bindings.
    pub fn apply(
        &self,
        scene: &mut Scene,
        actor_id: &str,
        convert: bool,
        library: &MaterialLibrary,
    ) -> Result<PresetPreview, String> {
        let preview = self.preview(scene, actor_id);
        if let Some(why) = &preview.blocked {
            return Err(why.clone());
        }
        let visual = PlayerPreset::ThirdPerson3d.parts().visual;
        let visual = if self.mode == Mode::TwoD {
            PlayerPreset::Platformer2d.parts().visual
        } else {
            visual
        };
        let parts = PresetParts {
            visual,
            controller: self.controller.clone(),
            motor: self.motor.clone(),
            camera: self.camera,
            player_camera: self.player_camera,
            boom: None,
            actions: self.actions.clone(),
        };
        scene.install_player(actor_id, parts, &preview, convert, library)?;
        Ok(preview)
    }
}

fn dimension(mode: Mode) -> &'static str {
    match mode {
        Mode::ThreeD => "3D",
        Mode::TwoD => "2D",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Actor;

    fn scene(mode: Mode) -> (Scene, String) {
        let mut scene = Scene::new("Test", mode);
        let mut ground = Actor::new(
            "Ground",
            Visual::Cuboid {
                color: "#888".into(),
                size: [20.0, 1.0, 20.0],
            },
        );
        ground.id = "ground".into();
        let mut player = Actor::new(
            "Player",
            Visual::Cuboid {
                color: "#fff".into(),
                size: [1.0; 3],
            },
        );
        player.id = "player".into();
        scene.actors.push(ground);
        scene.actors.push(player);
        (scene, "player".to_string())
    }

    #[test]
    fn every_preset_installs_a_valid_player_from_nothing() {
        for preset in PlayerPreset::ALL {
            let (mut scene, id) = scene(preset.mode());
            scene.actors[1].components.remove("Look");
            let preview = scene
                .apply_player_preset(&id, preset, false, &MaterialLibrary::default())
                .unwrap_or_else(|e| panic!("{}: {e}", preset.name()));
            assert!(preview.conflicts.is_empty());
            let c = &scene.actors[1].components;
            assert!(
                c.visual().is_some(),
                "{} draws before any art",
                preset.name()
            );
            assert!(c.character_controller().is_some());
            assert!(c.character_motor().is_some());
            assert!(c.contains("Camera"));
            assert!(c.player_camera().is_some());
            for name in ["Move", "Look", "Jump", "Sprint", "Crouch", "Interact"] {
                assert!(scene.world.input.find(name).is_some(), "{name}");
            }
            assert!(
                scene
                    .physics_issues(&MaterialLibrary::default())
                    .iter()
                    .all(|i| !i.is_error()),
                "{}",
                preset.name()
            );
        }
    }

    #[test]
    fn a_preset_of_the_wrong_dimension_is_refused_untouched() {
        let (mut scene, id) = scene(Mode::ThreeD);
        let before = scene.clone();
        let err = scene
            .apply_player_preset(
                &id,
                PlayerPreset::Platformer2d,
                false,
                &MaterialLibrary::default(),
            )
            .unwrap_err();
        assert!(err.contains("2D"), "{err}");
        assert_eq!(scene, before);
    }

    #[test]
    fn a_dynamic_body_asks_for_conversion_and_gets_it_when_allowed() {
        let (mut scene, id) = scene(Mode::ThreeD);
        let lib = MaterialLibrary::default();
        let mut body = super::super::spec::RigidbodySpec::default();
        body.body_type = BodyType::Dynamic;
        scene.set_rigidbody(&id, body, &lib).unwrap();
        let preview = scene.preview_player_preset(&id, PlayerPreset::ThirdPerson3d);
        assert_eq!(preview.conflicts.len(), 1);
        assert_eq!(preview.conflicts[0].component, "Rigidbody");
        let before = scene.clone();
        assert!(
            scene
                .apply_player_preset(&id, PlayerPreset::ThirdPerson3d, false, &lib)
                .is_err()
        );
        assert_eq!(scene, before, "a refused apply leaves no trace");
        scene
            .apply_player_preset(&id, PlayerPreset::ThirdPerson3d, true, &lib)
            .unwrap();
        assert_eq!(
            scene.actors[1].components.rigidbody().unwrap().body_type,
            BodyType::Kinematic
        );
    }

    #[test]
    fn applying_again_keeps_identities_and_the_camera_moves_over() {
        let (mut scene, id) = scene(Mode::ThreeD);
        let lib = MaterialLibrary::default();
        scene.actors[0].components.insert(ActorComponent::Camera {
            camera: CameraAttach::default(),
        });
        scene
            .apply_player_preset(&id, PlayerPreset::FirstPerson3d, false, &lib)
            .unwrap();
        assert!(!scene.actors[0].components.contains("Camera"), "one camera");
        let controller = scene.actors[1]
            .components
            .character_controller()
            .unwrap()
            .id
            .clone();
        scene
            .apply_player_preset(&id, PlayerPreset::ThirdPerson3d, false, &lib)
            .unwrap();
        assert_eq!(
            scene.actors[1]
                .components
                .character_controller()
                .unwrap()
                .id,
            controller
        );
        let camera = scene.actors[1].components.get("Camera").unwrap();
        assert!(matches!(
            camera,
            ActorComponent::Camera { camera } if camera.view == CameraView::ThirdPerson
        ));
    }

    #[test]
    fn the_preview_says_what_is_added_and_what_is_kept() {
        let (scene, id) = scene(Mode::TwoD);
        let preview = scene.preview_player_preset(&id, PlayerPreset::Platformer2d);
        let kinds: Vec<_> = preview
            .steps
            .iter()
            .map(|s| (s.component.as_str(), s.kind))
            .collect();
        assert!(kinds.contains(&("Look", StepKind::Keep)));
        assert!(kinds.contains(&("CharacterController", StepKind::Add)));
        assert!(kinds.contains(&("Input", StepKind::Add)));
        assert!(preview.blocked.is_none());
        assert!(
            scene
                .preview_player_preset("nobody", PlayerPreset::Platformer2d)
                .blocked
                .is_some()
        );
    }

    #[test]
    fn preset_names_round_trip() {
        for preset in PlayerPreset::ALL {
            assert_eq!(PlayerPreset::parse(preset.name()), Some(preset));
            assert_eq!(PlayerPreset::parse(preset.label()), Some(preset));
        }
        assert_eq!(PlayerPreset::parse("nope"), None);
    }

    #[test]
    fn a_profile_carries_a_setup_to_another_actor_through_json() {
        let library = MaterialLibrary::default();
        let (mut from, id) = scene(Mode::ThreeD);
        from.apply_player_preset(&id, PlayerPreset::ThirdPerson3d, false, &library)
            .unwrap();
        let profile = PlayerProfile::capture(&from, &id, "Hero").unwrap();
        let text = serde_json::to_string(&profile).unwrap();
        let loaded: PlayerProfile = serde_json::from_str(&text).unwrap();
        assert_eq!(loaded.name, "Hero");

        let (mut to, other) = scene(Mode::ThreeD);
        assert!(loaded.preview(&to, &other).blocked.is_none());
        loaded.apply(&mut to, &other, false, &library).unwrap();
        let c = &to.actors[1].components;
        assert!(c.character_controller().is_some());
        assert!(c.character_motor().is_some());
        assert!(c.player_camera().is_some());
        assert!(to.world.input.find("Move").is_some());
    }

    #[test]
    fn a_profile_from_a_newer_version_is_blocked_not_applied() {
        let library = MaterialLibrary::default();
        let (mut scene, id) = scene(Mode::ThreeD);
        scene
            .apply_player_preset(&id, PlayerPreset::FirstPerson3d, false, &library)
            .unwrap();
        let mut profile = PlayerProfile::capture(&scene, &id, "Future").unwrap();
        profile.version = PlayerProfile::VERSION + 1;
        assert!(profile.preview(&scene, &id).blocked.is_some());
    }
}
