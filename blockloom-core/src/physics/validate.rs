//! Validation: what a saved physics document may say.
//!
//! Runs before a spec is saved or applied and again before Play. Mass must be above
//! zero, shapes valid, materials resolvable, ids unique, and an actor may not own
//! two motion components or a legacy `Body` beside the new ones. Every issue names
//! the actor, the component id and the field, so an editor can point at it.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::ids::ColliderId;
use super::material::MaterialLibrary;
use super::meta;
use super::migrate::shape_from_look;
use super::ownership::PhysicsOwnership;
use super::spec::{
    BodyType, ColliderGeometry, ColliderShape, ColliderSpec, CollisionDetection, MassSource,
    RigidbodySpec,
};
use crate::components::ActorComponent;
use crate::project::Actor;
use crate::scene::{Mode, Visual};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Severity {
    /// Play and Build refuse.
    Error,
    /// Worth knowing, does not stop a run.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhysicsIssue {
    pub severity: Severity,
    pub actor: Option<String>,
    /// The collider or rigidbody id, when the issue is about one.
    pub component: Option<String>,
    pub field: Option<String>,
    pub message: String,
}

impl PhysicsIssue {
    pub(super) fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            actor: None,
            component: None,
            field: None,
            message: message.into(),
        }
    }

    pub(super) fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(message)
        }
    }

    pub(super) fn on(mut self, actor: &str) -> Self {
        self.actor = Some(actor.to_string());
        self
    }

    pub(super) fn of(mut self, component: &str) -> Self {
        self.component = Some(component.to_string());
        self
    }

    pub(super) fn field(mut self, field: &str) -> Self {
        self.field = Some(field.to_string());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl std::fmt::Display for PhysicsIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// A problem with one field of one spec.
type FieldIssue = (String, String);

fn finite3(name: &str, values: &[f32], out: &mut Vec<FieldIssue>) {
    if values.iter().any(|v| !v.is_finite()) {
        out.push((name.into(), format!("{name} must be finite numbers")));
    }
}

fn number(table: &'static [meta::Property], name: &str, value: f32, out: &mut Vec<FieldIssue>) {
    if let Err(message) = meta::check(table, name, value) {
        out.push((name.into(), message));
    }
}

impl RigidbodySpec {
    /// Problems with this spec in a world of `mode`.
    pub fn validate(&self, mode: Mode) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let bad = |field: &str, message: &str| (field.to_string(), message.to_string());
        if self.id.is_empty() {
            out.push(bad("id", "a Rigidbody needs an id"));
        }
        match self.mass {
            MassSource::Explicit { mass } => number(meta::RIGIDBODY, "mass.mass", mass, &mut out),
            MassSource::Density { density } => {
                number(meta::RIGIDBODY, "mass.density", density, &mut out)
            }
        }
        number(
            meta::RIGIDBODY,
            "linear_damping",
            self.linear_damping,
            &mut out,
        );
        number(
            meta::RIGIDBODY,
            "angular_damping",
            self.angular_damping,
            &mut out,
        );
        number(
            meta::RIGIDBODY,
            "gravity_scale",
            self.gravity_scale,
            &mut out,
        );
        number(
            meta::RIGIDBODY,
            "max_linear_velocity",
            self.max_linear_velocity,
            &mut out,
        );
        number(
            meta::RIGIDBODY,
            "max_angular_velocity",
            self.max_angular_velocity,
            &mut out,
        );
        number(
            meta::RIGIDBODY,
            "sleep_threshold",
            self.sleep_threshold,
            &mut out,
        );
        number(
            meta::RIGIDBODY,
            "max_depenetration_velocity",
            self.max_depenetration_velocity,
            &mut out,
        );
        if let Some(centre) = self.center_of_mass {
            finite3("center_of_mass", &centre, &mut out);
        }
        if let Some(inertia) = self.inertia {
            for value in inertia {
                number(meta::RIGIDBODY, "inertia", value, &mut out);
            }
        }
        if let Some(iterations) = self.solver_iterations {
            number(
                meta::RIGIDBODY,
                "solver_iterations",
                iterations as f32,
                &mut out,
            );
        }
        match mode {
            Mode::ThreeD => {
                if self.body_type == BodyType::Static {
                    out.push(bad(
                        "body_type",
                        "A 3D static collider has no Rigidbody; remove the Rigidbody instead",
                    ));
                }
                if !self.simulated {
                    out.push(bad("simulated", "Simulated applies to 2D bodies only"));
                }
            }
            Mode::TwoD => {
                if matches!(
                    self.collision_detection,
                    CollisionDetection::ContinuousDynamic
                        | CollisionDetection::ContinuousSpeculative
                ) {
                    out.push(bad(
                        "collision_detection",
                        "2D bodies offer Discrete and Continuous only",
                    ));
                }
                let c = self.constraints;
                if c.freeze_position[2] || c.freeze_rotation[0] || c.freeze_rotation[1] {
                    out.push(bad(
                        "constraints",
                        "A 2D body can freeze X and Y position and Z rotation only",
                    ));
                }
            }
        }
        out
    }
}

/// A mesh asset path is project-relative with forward slashes and cannot climb out.
fn bad_asset_path(path: &str) -> Option<&'static str> {
    if path.trim().is_empty() {
        return Some("choose a mesh asset");
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Some("a mesh path is relative to the project, with forward slashes");
    }
    if path.split('/').any(|part| part == "..") {
        return Some("a mesh path cannot leave the project folder");
    }
    None
}

impl ColliderSpec {
    /// Problems with this spec in a world of `mode`, with materials looked up in
    /// `library`. Whether a derived shape has a Look to follow is the scene's
    /// check, since it needs the actor.
    pub fn validate(&self, mode: Mode, library: &MaterialLibrary) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let bad = |field: &str, message: String| (field.to_string(), message);
        if self.id.is_empty() {
            out.push(bad("id", "a collider needs an id".into()));
        }
        finite3("center", &self.center, &mut out);
        finite3("rotation", &self.rotation, &mut out);
        number(meta::COLLIDER, "layer", f32::from(self.layer), &mut out);
        if let Some(offset) = self.contact_offset {
            number(meta::COLLIDER, "contact_offset", offset, &mut out);
        }
        if self.trigger && self.one_way {
            out.push(bad("one_way", "A trigger cannot be one way".into()));
        }
        if self.one_way && mode == Mode::ThreeD {
            out.push(bad("one_way", "One-way surfaces are 2D only".into()));
        }
        if mode == Mode::TwoD
            && (self.center[2] != 0.0 || self.rotation[0] != 0.0 || self.rotation[1] != 0.0)
        {
            out.push(bad(
                "center",
                "A 2D collider has an X/Y center and a Z rotation only".into(),
            ));
        }
        for (name, value) in self.material_overrides.numbers() {
            number(meta::MATERIAL, name, value, &mut out);
        }
        match library.resolve(&self.material, mode) {
            Ok(_) => {}
            Err(message) => out.push(bad("material", message)),
        }
        if !self.material_overrides.is_empty() && mode == Mode::TwoD {
            out.push(bad(
                "material_overrides",
                "2D colliders take their friction and bounciness from the material".into(),
            ));
        }
        if let ColliderGeometry::Shape { shape } = &self.geometry {
            for (field, message) in validate_shape(shape, mode) {
                out.push((format!("geometry.{field}"), message));
            }
        }
        out
    }
}

fn positive(name: &str, value: f32, out: &mut Vec<FieldIssue>) {
    if !(value.is_finite() && value > 0.0) {
        out.push((name.into(), format!("{name} must be above zero")));
    }
}

fn validate_shape(shape: &ColliderShape, mode: Mode) -> Vec<FieldIssue> {
    let mut out = Vec::new();
    if !shape.fits(mode) {
        out.push((
            "shape".to_string(),
            format!(
                "{} is a {} shape and this world is {}",
                shape.label(),
                shape.world_word(),
                super::material::dimension_word(mode)
            ),
        ));
        return out;
    }
    match shape {
        ColliderShape::Box { size } => size.iter().for_each(|v| positive("size", *v, &mut out)),
        ColliderShape::Sphere { radius } | ColliderShape::Circle { radius } => {
            positive("radius", *radius, &mut out)
        }
        ColliderShape::Capsule { radius, height, .. } => {
            positive("radius", *radius, &mut out);
            positive("height", *height, &mut out);
            if height.is_finite() && radius.is_finite() && *height < 2.0 * radius {
                out.push((
                    "height".into(),
                    "A capsule's height is end to end, so it must be at least twice the radius"
                        .into(),
                ));
            }
        }
        ColliderShape::ConvexHull { mesh } | ColliderShape::TriangleMesh { mesh } => {
            if let Some(why) = bad_asset_path(mesh) {
                out.push(("mesh".into(), why.into()));
            }
        }
        ColliderShape::Terrain | ColliderShape::Tilemap => {}
        ColliderShape::Rect { size } => size.iter().for_each(|v| positive("size", *v, &mut out)),
        ColliderShape::Capsule2d { size, .. } => {
            size.iter().for_each(|v| positive("size", *v, &mut out))
        }
        ColliderShape::Polygon { points } => {
            if points.len() < 3 {
                out.push(("points".into(), "A polygon needs at least 3 points".into()));
            } else if points.iter().flatten().any(|v| !v.is_finite()) {
                out.push(("points".into(), "points must be finite numbers".into()));
            } else if area(points).abs() < 1e-9 {
                out.push(("points".into(), "A polygon needs some area".into()));
            }
        }
        ColliderShape::Edge { a, b } => {
            if a.iter().chain(b).any(|v| !v.is_finite()) {
                out.push(("points".into(), "points must be finite numbers".into()));
            } else if a == b {
                out.push(("points".into(), "An edge needs two different points".into()));
            }
        }
        ColliderShape::Chain { points, .. } => {
            if points.len() < 2 {
                out.push(("points".into(), "A chain needs at least 2 points".into()));
            } else if points.iter().flatten().any(|v| !v.is_finite()) {
                out.push(("points".into(), "points must be finite numbers".into()));
            }
        }
    }
    out
}

/// Twice the signed area of a polygon.
fn area(points: &[[f32; 2]]) -> f32 {
    let mut sum = 0.0;
    for i in 0..points.len() {
        let [x1, y1] = points[i];
        let [x2, y2] = points[(i + 1) % points.len()];
        sum += x1 * y2 - x2 * y1;
    }
    sum
}

/// Validates one scene's physics: every spec, the ids across actors, ownership
/// rules and the legacy/new split.
pub fn validate_scene(
    actors: &[Actor],
    mode: Mode,
    library: &MaterialLibrary,
) -> Vec<PhysicsIssue> {
    validate_scene_with(actors, mode, library, None)
}

/// [`validate_scene`] knowing which meshes cook to convex pieces, so a triangle
/// mesh among them may sit on a dynamic body. With `None` the question is left
/// to the plan, which has the cooking settings (an edit can't know them yet).
pub fn validate_scene_with(
    actors: &[Actor],
    mode: Mode,
    library: &MaterialLibrary,
    decomposed: Option<&dyn Fn(&str) -> bool>,
) -> Vec<PhysicsIssue> {
    let mut issues = Vec::new();
    super::joints::plan_constraints(actors, mode, &mut issues);
    let mut seen_colliders: HashMap<&ColliderId, &str> = HashMap::new();
    let mut seen_bodies: HashMap<&str, &str> = HashMap::new();

    for actor in actors {
        let id = actor.id.as_str();
        let bodies = actor
            .components
            .iter()
            .filter(|c| matches!(c, ActorComponent::Rigidbody { .. }))
            .count();
        if bodies > 1 {
            issues.push(
                PhysicsIssue::error(format!(
                    "\"{}\" has {bodies} Rigidbodies; an actor has at most one",
                    actor.name
                ))
                .on(id),
            );
        }
        let has_new = bodies > 0 || actor.components.colliders().next().is_some();
        let controllers = actor
            .components
            .iter()
            .filter(|c| matches!(c, ActorComponent::CharacterController { .. }))
            .count();
        let motors = actor
            .components
            .iter()
            .filter(|c| matches!(c, ActorComponent::CharacterMotor { .. }))
            .count();
        if motors > 1 {
            issues.push(
                PhysicsIssue::error(format!(
                    "\"{}\" has {motors} CharacterMotors; an actor has at most one",
                    actor.name
                ))
                .on(id),
            );
        }
        if controllers > 1 {
            issues.push(
                PhysicsIssue::error(format!(
                    "\"{}\" has {controllers} CharacterControllers; an actor has at most one",
                    actor.name
                ))
                .on(id),
            );
        }
        if has_new && actor.components.contains("Body") {
            issues.push(
                PhysicsIssue::error(format!(
                    "\"{}\" has a legacy Body beside Rigidbody/Collider components; migrate it so one system owns its physics",
                    actor.name
                ))
                .on(id),
            );
        }
        for component in actor.components.iter() {
            match component {
                ActorComponent::Rigidbody { rigidbody } => {
                    if let Some(other) = seen_bodies.insert(rigidbody.id.as_str(), id) {
                        issues.push(
                            PhysicsIssue::error(format!(
                                "Rigidbody id \"{}\" is used on \"{other}\" and \"{id}\"",
                                rigidbody.id
                            ))
                            .on(id)
                            .of(rigidbody.id.as_str()),
                        );
                    }
                    for (field, message) in rigidbody.validate(mode) {
                        issues.push(
                            PhysicsIssue::error(message)
                                .on(id)
                                .of(rigidbody.id.as_str())
                                .field(&field),
                        );
                    }
                }
                ActorComponent::Collider { collider } => {
                    if let Some(other) = seen_colliders.insert(&collider.id, id) {
                        issues.push(
                            PhysicsIssue::error(format!(
                                "Collider id \"{}\" is used on \"{other}\" and \"{id}\"",
                                collider.id
                            ))
                            .on(id)
                            .of(collider.id.as_str()),
                        );
                    }
                    for (field, message) in collider.validate(mode, library) {
                        issues.push(
                            PhysicsIssue::error(message)
                                .on(id)
                                .of(collider.id.as_str())
                                .field(&field),
                        );
                    }
                    check_sources(actor, collider, mode, &mut issues);
                }
                ActorComponent::CharacterController { controller } => {
                    for (field, message) in controller.validate(mode) {
                        issues.push(
                            PhysicsIssue::error(message)
                                .on(id)
                                .of(controller.id.as_str())
                                .field(&field),
                        );
                    }
                    if rigidbody_of(actor).is_some_and(|b| b.body_type == BodyType::Dynamic) {
                        issues.push(
                            PhysicsIssue::error(format!(
                                "\"{}\" has a CharacterController and a dynamic Rigidbody; the controller moves the actor, so make the body kinematic or remove it",
                                actor.name
                            ))
                            .on(id)
                            .of(controller.id.as_str()),
                        );
                    }
                    if actor.components.contains("Body") {
                        issues.push(
                            PhysicsIssue::warning(format!(
                                "\"{}\" has a CharacterController beside a legacy Body; migrate the Body so one system owns its physics",
                                actor.name
                            ))
                            .on(id)
                            .of(controller.id.as_str()),
                        );
                    }
                }
                ActorComponent::CharacterMotor { motor } => {
                    for (field, message) in motor.validate() {
                        issues.push(
                            PhysicsIssue::error(message)
                                .on(id)
                                .of(motor.id.as_str())
                                .field(&field),
                        );
                    }
                    if actor.components.character_controller().is_none() {
                        issues.push(
                            PhysicsIssue::error(format!(
                                "\"{}\" has a CharacterMotor but no CharacterController to move; add one",
                                actor.name
                            ))
                            .on(id)
                            .of(motor.id.as_str()),
                        );
                    }
                    if motor.top_down && mode == Mode::ThreeD {
                        issues.push(
                            PhysicsIssue::warning(format!(
                                "\"{}\" has a top-down motor in a 3D project; top down only applies in 2D",
                                actor.name
                            ))
                            .on(id)
                            .of(motor.id.as_str())
                            .field("top_down"),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // Ownership rules.
    let table = PhysicsOwnership::resolve(actors);
    let bodies: HashMap<&str, &RigidbodySpec> = actors
        .iter()
        .filter_map(|a| Some((a.id.as_str(), a.components.rigidbody()?)))
        .collect();
    for owned in &table.colliders {
        let (Some(body_actor), Some(spec)) = (
            owned.body_actor.as_deref(),
            owned.body_actor.as_deref().and_then(|a| bodies.get(a)),
        ) else {
            continue;
        };
        let actor = actors.iter().find(|a| a.id == owned.collider_actor);
        let Some(collider) = actor.and_then(|a| a.components.collider(&owned.collider)) else {
            continue;
        };
        if spec.body_type == BodyType::Dynamic
            && !collider.trigger
            && collider.shape().is_some_and(|shape| {
                shape.is_concave()
                    && match shape {
                        ColliderShape::TriangleMesh { mesh } => {
                            decomposed.is_some_and(|decomposed| !decomposed(mesh))
                        }
                        _ => true,
                    }
            })
        {
            issues.push(
                PhysicsIssue::error(format!(
                    "{} cannot be a solid shape on the dynamic body of \"{body_actor}\"; use a convex hull, a compound of primitives, decompose the mesh in the cooking settings, or make the body kinematic",
                    collider.shape().map(ColliderShape::label).unwrap_or("This shape")
                ))
                .on(&owned.collider_actor)
                .of(collider.id.as_str())
                .field("geometry"),
            );
        }
    }
    let mut seen = HashSet::new();
    for body in table.bodies_without_shape() {
        if seen.insert(&body.actor) {
            issues.push(
                PhysicsIssue::warning(
                    "This Rigidbody has no collider, so it falls and takes forces but touches nothing",
                )
                .on(&body.actor)
                .of(body.body.as_str()),
            );
        }
    }
    issues
}

fn rigidbody_of(actor: &Actor) -> Option<&RigidbodySpec> {
    actor.components.rigidbody()
}

/// A collider whose shape comes from a sibling component needs that component.
fn check_sources(
    actor: &Actor,
    collider: &ColliderSpec,
    mode: Mode,
    issues: &mut Vec<PhysicsIssue>,
) {
    let id = actor.id.as_str();
    let missing = |why: String| {
        PhysicsIssue::error(why)
            .on(id)
            .of(collider.id.as_str())
            .field("geometry")
    };
    match &collider.geometry {
        ColliderGeometry::FromLook => match actor.components.visual() {
            None => issues.push(missing(format!(
                "\"{}\" has no Look to take a shape from; give the collider a saved shape",
                actor.name
            ))),
            Some(visual) => {
                if shape_from_look(visual, mode).is_none() {
                    issues.push(
                        PhysicsIssue::warning(format!(
                            "This Look has no collision shape in a {} world, so the collider is empty",
                            super::material::dimension_word(mode)
                        ))
                        .on(id)
                        .of(collider.id.as_str())
                        .field("geometry"),
                    );
                }
            }
        },
        ColliderGeometry::Shape {
            shape: ColliderShape::Terrain,
        } => {
            if !actor.components.contains("Terrain") {
                issues.push(missing(
                    "A Terrain collider needs the actor's Terrain component".into(),
                ));
            }
        }
        ColliderGeometry::Shape {
            shape: ColliderShape::Tilemap,
        } if !matches!(actor.components.visual(), Some(Visual::Tilemap { .. })) => {
            issues.push(missing("A Tilemap collider needs a tilemap Look".into()));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::spec::Constraints;
    use super::*;

    fn lib() -> MaterialLibrary {
        MaterialLibrary::default()
    }

    #[test]
    fn defaults_are_valid_in_their_world() {
        assert!(RigidbodySpec::default().validate(Mode::ThreeD).is_empty());
        assert!(RigidbodySpec::default().validate(Mode::TwoD).is_empty());
        let boxed = ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] });
        assert!(boxed.validate(Mode::ThreeD, &lib()).is_empty());
    }

    #[test]
    fn rigidbody_rejects_bad_numbers() {
        let body = RigidbodySpec {
            mass: MassSource::Explicit { mass: 0.0 },
            linear_damping: -1.0,
            gravity_scale: f32::NAN,
            ..Default::default()
        };
        let fields: Vec<_> = body
            .validate(Mode::ThreeD)
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        assert!(fields.contains(&"mass.mass".to_string()));
        assert!(fields.contains(&"linear_damping".to_string()));
        assert!(fields.contains(&"gravity_scale".to_string()));
    }

    #[test]
    fn dimension_specific_body_rules() {
        let body = RigidbodySpec {
            body_type: BodyType::Static,
            ..Default::default()
        };
        assert!(!body.validate(Mode::ThreeD).is_empty());
        assert!(body.validate(Mode::TwoD).is_empty());
        let body = RigidbodySpec {
            collision_detection: CollisionDetection::ContinuousDynamic,
            ..Default::default()
        };
        assert!(body.validate(Mode::ThreeD).is_empty());
        assert!(!body.validate(Mode::TwoD).is_empty());
        let body = RigidbodySpec {
            constraints: Constraints {
                freeze_position: [false, false, true],
                freeze_rotation: [false; 3],
            },
            ..Default::default()
        };
        assert!(!body.validate(Mode::TwoD).is_empty());
        assert!(body.validate(Mode::ThreeD).is_empty());
    }

    #[test]
    fn shapes_are_checked() {
        let check =
            |shape: ColliderShape, mode| ColliderSpec::new(shape).validate(mode, &lib()).len();
        assert_eq!(
            check(ColliderShape::Sphere { radius: 0.0 }, Mode::ThreeD),
            1
        );
        assert_eq!(
            check(
                ColliderShape::Box {
                    size: [1.0, -1.0, 1.0]
                },
                Mode::ThreeD
            ),
            1
        );
        // 0.4 tall with radius 0.5 would be shorter than its own caps.
        assert_eq!(
            check(
                ColliderShape::Capsule {
                    radius: 0.5,
                    height: 0.4,
                    axis: Default::default()
                },
                Mode::ThreeD
            ),
            1
        );
        assert_eq!(
            check(ColliderShape::Circle { radius: 1.0 }, Mode::ThreeD),
            1
        );
        assert_eq!(
            check(
                ColliderShape::Polygon {
                    points: vec![[0.0, 0.0], [1.0, 0.0]]
                },
                Mode::TwoD
            ),
            1
        );
        assert_eq!(
            check(
                ColliderShape::Polygon {
                    points: vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]
                },
                Mode::TwoD
            ),
            1,
            "collinear points have no area"
        );
        assert_eq!(
            check(
                ColliderShape::ConvexHull {
                    mesh: "../x.glb".into()
                },
                Mode::ThreeD
            ),
            1
        );
        assert_eq!(
            check(
                ColliderShape::ConvexHull {
                    mesh: "assets/x.glb".into()
                },
                Mode::ThreeD
            ),
            0
        );
    }

    #[test]
    fn material_reference_must_resolve_in_this_world() {
        let mut spec = ColliderSpec::new(ColliderShape::Sphere { radius: 1.0 });
        spec.material = super::super::material::MaterialRef::BuiltIn { name: "Ice".into() };
        assert!(spec.validate(Mode::ThreeD, &lib()).is_empty());
        spec.material = super::super::material::MaterialRef::BuiltIn { name: "Mud".into() };
        assert_eq!(spec.validate(Mode::ThreeD, &lib())[0].0, "material");
    }

    #[test]
    fn trigger_and_one_way_conflict() {
        let mut spec = ColliderSpec::new(ColliderShape::Rect { size: [1.0, 1.0] });
        spec.one_way = true;
        assert!(spec.validate(Mode::TwoD, &lib()).is_empty());
        spec.trigger = true;
        assert!(!spec.validate(Mode::TwoD, &lib()).is_empty());
        spec.trigger = false;
        assert!(
            spec.validate(Mode::ThreeD, &lib())
                .iter()
                .any(|(f, _)| f == "one_way")
        );
    }
}
