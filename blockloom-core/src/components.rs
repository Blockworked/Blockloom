//! An actor's properties, as components.
//!
//! Everything an actor *is* - where it stands, what it looks like, whether
//! physics owns it, whether it is drawn - is one entry in its
//! [`Components`] list rather than a fixed field, and the runtime turns each
//! entry into the Bevy component it names. Removing `Body` really does leave
//! the actor without a rigid body; removing `Look` leaves a positioned,
//! scriptable actor with nothing to draw.
//!
//! Four components have no fixed-field ancestor. [`ActorComponent::Camera`]
//! attaches the world camera to the actor, in first person, third person or
//! plain follow. [`ActorComponent::Script`] names a Rust file in the project's
//! `assets/scripts`, compiled and loaded by the runtime (see
//! [`crate::script`]). [`ActorComponent::Custom`] is a named bag of values the
//! project invents - `Health { hp, armour }` - which blocks, scripts and the
//! inspector all read and write by name.

use crate::scene::{Physics, Placement, Visual};
use crate::value::Evaluated;
use serde::{Deserialize, Serialize};

/// The components every project knows about by name. A custom component
/// can't take one of these names.
pub const BUILT_IN_NAMES: &[&str] = &[
    "Place", "Look", "Render", "Body", "Camera", "Script", "Parent",
];

/// One component on an actor. Serialized internally-tagged, so a component
/// reads as `{"component": "Body", "physics": {...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "component")]
pub enum ActorComponent {
    /// Where the actor stands - its Bevy `Transform`. Every actor has one:
    /// there is nowhere for an entity without a transform to be drawn.
    Place { placement: Placement },
    /// What the actor looks like, and so what shape it collides with.
    Look { visual: Visual },
    /// Whether the actor is drawn. Absent means visible.
    Render { visible: bool },
    /// A rigid body and collider. Absent means blocks move the actor and
    /// nothing else does.
    Body { physics: Physics },
    /// Puts the world camera on this actor.
    Camera { camera: CameraAttach },
    /// A Rust file under the project's `assets/scripts`, compiled to a shared
    /// library the runtime loads and ticks. `path` is relative to the project
    /// folder - see [`crate::script`].
    Script { path: String },
    /// Hangs this actor off another one: every move the parent makes is
    /// made to it too. `parent` is the other actor's id.
    ///
    /// `offset` is where the child stands in its parent's frame. `None` -
    /// which is what a document written before offsets says, and what `set
    /// my parent to` leaves - means the child keeps the world position its
    /// `Place` gives it.
    Parent {
        parent: String,
        #[serde(default)]
        offset: Option<[f32; 3]>,
    },
    /// A named set of values this project invented, readable and writable
    /// from blocks.
    Custom {
        name: String,
        #[serde(default)]
        fields: Vec<ComponentField>,
    },
}

impl ActorComponent {
    /// What the editor and the blocks call this component.
    pub fn name(&self) -> &str {
        match self {
            ActorComponent::Place { .. } => "Place",
            ActorComponent::Look { .. } => "Look",
            ActorComponent::Render { .. } => "Render",
            ActorComponent::Body { .. } => "Body",
            ActorComponent::Camera { .. } => "Camera",
            ActorComponent::Script { .. } => "Script",
            ActorComponent::Parent { .. } => "Parent",
            ActorComponent::Custom { name, .. } => name,
        }
    }

    /// True for the one component an actor can't be without.
    pub fn is_required(&self) -> bool {
        matches!(self, ActorComponent::Place { .. })
    }
}

/// How the camera sits on the actor it is attached to. In a 2D project all
/// three views centre the camera on the actor: there is no depth to stand in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CameraView {
    /// Keeps the world camera's own offset, centred on the actor.
    #[default]
    Follow,
    /// Sits at the actor's eye: yaw from the actor, `pitch` from this rig.
    /// `set camera pitch` drives that angle mid-run.
    FirstPerson,
    /// Sits `distance` behind and `pitch` degrees above the actor, looking
    /// back at it.
    ThirdPerson,
}

/// The camera component's settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraAttach {
    #[serde(default)]
    pub view: CameraView,
    /// Offset from the actor's origin, in the actor's own frame: the eye in
    /// first person, the point the boom looks at in third.
    #[serde(default = "default_eye")]
    pub offset: [f32; 3],
    /// How far behind the actor a third-person camera sits.
    #[serde(default = "default_distance")]
    pub distance: f32,
    /// How far above the actor a third-person camera looks down from, in
    /// degrees; a first-person camera's look-up angle, also in degrees.
    #[serde(default = "default_pitch")]
    pub pitch: f32,
}

fn default_eye() -> [f32; 3] {
    [0.0, 0.6, 0.0]
}

fn default_distance() -> f32 {
    6.0
}

fn default_pitch() -> f32 {
    15.0
}

impl Default for CameraAttach {
    fn default() -> Self {
        Self {
            view: CameraView::default(),
            offset: default_eye(),
            distance: default_distance(),
            pitch: default_pitch(),
        }
    }
}

/// One value on a custom component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentField {
    pub name: String,
    pub value: Evaluated,
}

impl ComponentField {
    pub fn number(name: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            value: Evaluated::Number(value),
        }
    }
}

/// An actor's components, in the order the editor shows them. Kept as a list
/// rather than a map so the inspector has a stable order to draw, and so a
/// component's name can live on the component itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Components(pub Vec<ActorComponent>);

impl Components {
    /// The components a freshly made actor has: somewhere to stand, something
    /// to draw, and drawn. A body is added when the actor needs one.
    pub fn new(visual: Visual) -> Self {
        Self(vec![
            ActorComponent::Place {
                placement: Placement::default(),
            },
            ActorComponent::Look { visual },
            ActorComponent::Render { visible: true },
        ])
    }

    pub fn iter(&self) -> impl Iterator<Item = &ActorComponent> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ActorComponent> {
        self.0.iter_mut()
    }

    pub fn get(&self, name: &str) -> Option<&ActorComponent> {
        self.0.iter().find(|component| component.name() == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut ActorComponent> {
        self.0.iter_mut().find(|component| component.name() == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Adds `component`, replacing one of the same name. Returns whether it
    /// was new.
    pub fn insert(&mut self, component: ActorComponent) -> bool {
        match self.get_mut(component.name()) {
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

    /// Drops the named component. The required one stays; everything else
    /// goes, and reads of it fall back to its default.
    pub fn remove(&mut self, name: &str) -> bool {
        let Some(index) = self
            .0
            .iter()
            .position(|component| component.name() == name && !component.is_required())
        else {
            return false;
        };
        self.0.remove(index);
        true
    }

    // ─── The built-ins, as the rest of the engine reads them ───────────────

    /// What the actor looks like, or `None` when it has no `Look` - a
    /// positioned actor its blocks can still drive, with nothing to draw.
    pub fn visual(&self) -> Option<&Visual> {
        match self.get("Look") {
            Some(ActorComponent::Look { visual }) => Some(visual),
            _ => None,
        }
    }

    pub fn set_visual(&mut self, visual: Visual) {
        self.insert(ActorComponent::Look { visual });
    }

    pub fn placement(&self) -> Placement {
        match self.get("Place") {
            Some(ActorComponent::Place { placement }) => *placement,
            _ => Placement::default(),
        }
    }

    /// The actor's placement, adding a default `Place` first if it somehow
    /// has none.
    pub fn placement_mut(&mut self) -> &mut Placement {
        if !self.contains("Place") {
            self.0.insert(
                0,
                ActorComponent::Place {
                    placement: Placement::default(),
                },
            );
        }
        match self.get_mut("Place") {
            Some(ActorComponent::Place { placement }) => placement,
            _ => unreachable!("just inserted"),
        }
    }

    pub fn set_placement(&mut self, placement: Placement) {
        *self.placement_mut() = placement;
    }

    /// The actor's physics, or the inert default when it has no `Body`.
    pub fn physics(&self) -> Physics {
        match self.get("Body") {
            Some(ActorComponent::Body { physics }) => *physics,
            _ => Physics::default(),
        }
    }

    pub fn set_physics(&mut self, physics: Physics) {
        self.insert(ActorComponent::Body { physics });
    }

    /// Whether the actor is drawn. An actor with no `Render` is visible.
    pub fn visible(&self) -> bool {
        match self.get("Render") {
            Some(ActorComponent::Render { visible }) => *visible,
            _ => true,
        }
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.insert(ActorComponent::Render { visible });
    }

    pub fn camera(&self) -> Option<&CameraAttach> {
        match self.get("Camera") {
            Some(ActorComponent::Camera { camera }) => Some(camera),
            _ => None,
        }
    }

    /// The script file this actor runs, if any.
    pub fn script(&self) -> Option<&str> {
        match self.get("Script") {
            Some(ActorComponent::Script { path }) => Some(path),
            _ => None,
        }
    }

    /// The actor this one hangs off, by id, if any.
    pub fn parent(&self) -> Option<&str> {
        match self.get("Parent") {
            Some(ActorComponent::Parent { parent, .. }) if !parent.is_empty() => Some(parent),
            _ => None,
        }
    }

    /// Where this actor stands in its parent's frame, if it was authored
    /// that way. Nothing reads it without a parent to read it against.
    pub fn parent_offset(&self) -> Option<[f32; 3]> {
        match self.get("Parent") {
            Some(ActorComponent::Parent { parent, offset }) if !parent.is_empty() => *offset,
            _ => None,
        }
    }

    /// Hangs the actor off `parent`, or takes it off whatever it was on when
    /// `parent` is empty. An offset already authored stays: it is about where
    /// the child stands, not about which actor it hangs off.
    pub fn set_parent(&mut self, parent: &str) {
        if parent.is_empty() {
            self.remove("Parent");
            return;
        }
        let offset = self.parent_offset();
        self.insert(ActorComponent::Parent {
            parent: parent.to_string(),
            offset,
        });
    }

    /// Places the actor in its parent's frame, or puts it back in world
    /// coordinates with `None`. Silent without a parent to stand against.
    pub fn set_parent_offset(&mut self, offset: Option<[f32; 3]>) {
        let Some(parent) = self.parent().map(str::to_string) else {
            return;
        };
        self.insert(ActorComponent::Parent { parent, offset });
    }

    // ─── Custom components ─────────────────────────────────────────────────

    /// Every custom component, by name, with its fields - what the runtime
    /// puts on the entity and the blocks read back.
    pub fn custom(&self) -> impl Iterator<Item = (&str, &[ComponentField])> {
        self.0.iter().filter_map(|component| match component {
            ActorComponent::Custom { name, fields } => Some((name.as_str(), fields.as_slice())),
            _ => None,
        })
    }

    /// One field of one custom component.
    pub fn field(&self, component: &str, field: &str) -> Option<&Evaluated> {
        match self.get(component) {
            Some(ActorComponent::Custom { fields, .. }) => fields
                .iter()
                .find(|candidate| candidate.name == field)
                .map(|candidate| &candidate.value),
            _ => None,
        }
    }

    /// Writes a field of an existing custom component, adding the field if it
    /// isn't declared yet. Returns false when there's no such component -
    /// only the editor creates components, so a block can't conjure one.
    pub fn set_field(&mut self, component: &str, field: &str, value: Evaluated) -> bool {
        let Some(ActorComponent::Custom { fields, .. }) = self.get_mut(component) else {
            return false;
        };
        match fields.iter_mut().find(|candidate| candidate.name == field) {
            Some(slot) => slot.value = value,
            None => fields.push(ComponentField {
                name: field.to_string(),
                value,
            }),
        }
        true
    }

    /// An unused custom-component name based on `base`, never colliding with
    /// a built-in - `"Health"`, then `"Health 2"`.
    pub fn unique_custom_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "Component" } else { base };
        let taken =
            |name: &str| BUILT_IN_NAMES.contains(&name) || self.0.iter().any(|c| c.name() == name);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|candidate| !taken(candidate))
            .expect("an unused name always exists")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::BodyKind;

    fn rect() -> Visual {
        Visual::Rect {
            color: "#4C97FF".to_string(),
            size: [10.0, 10.0],
        }
    }

    #[test]
    fn a_missing_component_reads_as_its_default() {
        let mut components = Components::new(rect());
        assert_eq!(components.physics().body, BodyKind::None);
        assert!(components.visible());
        assert!(components.visual().is_some());

        components.remove("Look");
        components.remove("Render");
        assert!(components.visual().is_none());
        // Nothing to draw isn't the same as hidden: `Render` is about a
        // visual that exists.
        assert!(components.visible());
    }

    #[test]
    fn the_place_component_cant_be_removed() {
        let mut components = Components::new(rect());
        assert!(!components.remove("Place"));
        assert!(components.contains("Place"));
    }

    #[test]
    fn inserting_replaces_a_component_of_the_same_name() {
        let mut components = Components::new(rect());
        assert!(components.insert(ActorComponent::Body {
            physics: Physics {
                body: BodyKind::Dynamic,
                ..Physics::default()
            },
        }));
        assert!(!components.insert(ActorComponent::Body {
            physics: Physics {
                body: BodyKind::Static,
                ..Physics::default()
            },
        }));
        assert_eq!(components.physics().body, BodyKind::Static);
        assert_eq!(components.0.len(), 4);
    }

    #[test]
    fn a_block_writes_an_existing_custom_component_and_no_other() {
        let mut components = Components::new(rect());
        components.insert(ActorComponent::Custom {
            name: "Health".to_string(),
            fields: vec![ComponentField::number("hp", 3.0)],
        });

        assert!(components.set_field("Health", "hp", Evaluated::Number(1.0)));
        assert_eq!(
            components.field("Health", "hp"),
            Some(&Evaluated::Number(1.0))
        );
        // A field nobody declared is added; a component nobody declared isn't.
        assert!(components.set_field("Health", "armour", Evaluated::Number(2.0)));
        assert!(!components.set_field("Mana", "amount", Evaluated::Number(2.0)));
        assert!(components.field("Mana", "amount").is_none());
    }

    #[test]
    fn a_custom_name_never_collides_with_a_built_in_or_another_component() {
        let mut components = Components::new(rect());
        assert_eq!(components.unique_custom_name("Body"), "Body 2");
        components.insert(ActorComponent::Custom {
            name: "Health".to_string(),
            fields: Vec::new(),
        });
        assert_eq!(components.unique_custom_name("Health"), "Health 2");
        assert_eq!(components.unique_custom_name("  "), "Component");
    }

    #[test]
    fn a_component_reads_as_its_tagged_json_shape() {
        let json = serde_json::to_value(ActorComponent::Camera {
            camera: CameraAttach {
                view: CameraView::ThirdPerson,
                ..CameraAttach::default()
            },
        })
        .unwrap();
        assert_eq!(json["component"], "Camera");
        assert_eq!(json["camera"]["view"], "ThirdPerson");
    }
}
