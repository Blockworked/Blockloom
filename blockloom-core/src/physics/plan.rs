//! The physics build plan: a scene's Rigidbody and Collider components turned into
//! exactly what a backend installs, and the errors for what it can't.
//!
//! Everything a backend needs is decided here, in core, so both dimensions and the
//! tests share one set of rules: which body carries each shape, where it stands in
//! that body's frame, the shape's concrete dimensions under the actor's scale, the
//! resolved material, the collision groups, and how mass is split between the
//! shapes and the body. The runtime only turns a plan into ECS components.
//!
//! Mass follows two rules. Shapes carry the mass by default: an explicit body mass
//! becomes a density of `mass / volume`, so the total is exact however many shapes
//! there are and the center of mass and inertia come from the shapes. Triggers carry
//! none. A body with nothing to carry it (no shape, or only triggers and shapes
//! with no volume) holds its mass itself. A custom center of mass or inertia moves
//! the whole mass onto the body, since the backend has no way to override a sum.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use serde::Serialize;

use super::cook::{CollisionLookup, MeshKind, NoCollisionData};
use super::geometry::{self, MeshShape, Solid2, Solid3};
use super::ids::ColliderId;
use super::layers::{ColliderFilter, needs_exact};
use super::material::{MaterialBody, PhysicsMaterial, PhysicsMaterial2D};
use super::migrate::shape_from_look;
use super::ownership::{actor_worlds, body_above};
use super::spec::{
    BodyType, ColliderGeometry, ColliderShape, ColliderSpec, MassSource, RigidbodySpec,
};
use super::validate::{PhysicsIssue, validate_scene_with};
use super::{PhysicsSettings, Severity};
use crate::project::Actor;
use crate::scene::Mode;

/// A body's mass when nothing says otherwise: Unity's default.
pub const DEFAULT_MASS: f32 = 1.0;

/// A collider's shape, in the dimension it lives in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum PlannedShape {
    Three(Solid3),
    Two(Solid2),
    /// A cooked mesh (3D).
    Mesh(MeshShape),
}

/// A collider's resolved surface.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum PlannedMaterial {
    Three(PhysicsMaterial),
    Two(PhysicsMaterial2D),
}

/// One shape to install.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColliderPlan {
    pub collider: ColliderId,
    pub name: String,
    /// The actor the collider component lives on.
    pub actor: String,
    /// The actor whose Rigidbody carries it; `None` for static geometry.
    pub body_actor: Option<String>,
    pub shape: PlannedShape,
    /// Where it stands in its frame, in metres (world units): the body actor's
    /// frame, or for static geometry the collider's own actor's.
    pub position: [f32; 3],
    /// The same spot as a local translation under the frame's entity, whose own
    /// scale the backend multiplies in.
    pub local: [f32; 3],
    /// Its rotation in that frame, a quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    pub material: PlannedMaterial,
    pub trigger: bool,
    pub enabled: bool,
    pub queryable: bool,
    pub one_way: bool,
    pub layer: u8,
    pub filter: ColliderFilter,
    pub memberships: u32,
    pub accepts: u32,
    pub contact_offset: Option<f32>,
    /// Cubic metres (square units in 2D); zero when the shape has no inside.
    pub volume: f32,
    /// What each unit of volume weighs; zero for a collider that carries no mass.
    pub density: f32,
}

/// Mass a body holds itself.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum ExtraMass {
    Mass(f32),
    Full {
        mass: f32,
        center: [f32; 3],
        inertia: [f32; 3],
    },
}

/// One body to install.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BodyPlan {
    pub actor: String,
    pub spec: RigidbodySpec,
    /// Indices into [`PhysicsPlan::colliders`].
    pub colliders: Vec<usize>,
    pub extra_mass: Option<ExtraMass>,
    /// Mass the body ends with, for display and tests.
    pub total_mass: f32,
}

/// A scene's physics, ready to install.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct PhysicsPlan {
    pub bodies: Vec<BodyPlan>,
    pub colliders: Vec<ColliderPlan>,
    pub issues: Vec<PhysicsIssue>,
    /// True when a pair rule needs the exact test from a hook (an include override
    /// or a priority is in play) instead of group masks alone.
    pub exact_filtering: bool,
}

impl PhysicsPlan {
    /// Plans one scene. `actors` must already have its authored child offsets
    /// resolved into placements.
    pub fn build(actors: &[Actor], mode: Mode, settings: &PhysicsSettings) -> PhysicsPlan {
        Self::build_with(actors, mode, settings, &NoCollisionData)
    }

    /// Plans one scene, asking `lookup` for the cooked data of every mesh
    /// collider.
    pub fn build_with(
        actors: &[Actor],
        mode: Mode,
        settings: &PhysicsSettings,
        lookup: &dyn CollisionLookup,
    ) -> PhysicsPlan {
        let mut issues = validate_scene_with(
            actors,
            mode,
            &settings.materials,
            Some(&|mesh| settings.cooking.decomposition_of(mesh).is_some()),
        );
        if let Err(message) = settings.layers.validate() {
            issues.push(PhysicsIssue::error(message));
        }
        let mut plan = PhysicsPlan::default();
        let by_id: HashMap<&str, &Actor> = actors.iter().map(|a| (a.id.as_str(), a)).collect();
        let worlds = actor_worlds(actors);

        // Bodies first, so a collider can find the one that carries it.
        for actor in actors {
            if let Some(spec) = actor.components.rigidbody() {
                plan.bodies.push(BodyPlan {
                    actor: actor.id.clone(),
                    spec: spec.clone(),
                    colliders: Vec::new(),
                    extra_mass: None,
                    total_mass: 0.0,
                });
            }
        }

        let mut filters = Vec::new();
        for actor in actors {
            for collider in actor.components.colliders() {
                let carrier = body_above(actor, &by_id);
                match plan_collider(actor, collider, carrier, &worlds, mode, settings, lookup) {
                    Ok(Some(mut planned)) => {
                        planned.body_actor = carrier.map(|a| a.id.clone());
                        filters.push(planned.filter);
                        plan.colliders.push(planned);
                    }
                    Ok(None) => {}
                    Err(issue) => issues.push(issue),
                }
            }
        }

        plan.exact_filtering = needs_exact(&filters);
        for planned in &mut plan.colliders {
            let bits = planned
                .filter
                .groups(&settings.layers, mode, plan.exact_filtering);
            planned.memberships = bits.memberships;
            planned.accepts = bits.filter;
        }

        for index in 0..plan.colliders.len() {
            let owner = plan.colliders[index].body_actor.clone();
            if let Some(owner) = owner
                && let Some(body) = plan.bodies.iter_mut().find(|b| b.actor == owner)
            {
                body.colliders.push(index);
            }
        }
        for body_index in 0..plan.bodies.len() {
            plan_mass(&mut plan, body_index, mode, &mut issues);
        }
        plan.issues = issues;
        plan
    }

    /// True when nothing in the plan is an error.
    pub fn is_runnable(&self) -> bool {
        self.issues.iter().all(|i| i.severity != Severity::Error)
    }

    /// The errors, for a message that refuses Play or Build.
    pub fn errors(&self) -> impl Iterator<Item = &PhysicsIssue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    /// True when the scene has anything for the new system to install.
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty() && self.colliders.is_empty()
    }

    pub fn collider(&self, id: &ColliderId) -> Option<&ColliderPlan> {
        self.colliders.iter().find(|c| &c.collider == id)
    }

    pub fn body(&self, actor: &str) -> Option<&BodyPlan> {
        self.bodies.iter().find(|b| b.actor == actor)
    }
}

impl crate::project::Scene {
    /// This scene's physics plan: what Play would install, with every problem.
    pub fn physics_plan(&self, settings: &PhysicsSettings) -> PhysicsPlan {
        PhysicsPlan::build(&self.actors, self.world.mode, settings)
    }

    /// The same plan with mesh colliders cooked through `lookup`.
    pub fn physics_plan_with(
        &self,
        settings: &PhysicsSettings,
        lookup: &dyn CollisionLookup,
    ) -> PhysicsPlan {
        PhysicsPlan::build_with(&self.actors, self.world.mode, settings, lookup)
    }
}

/// Scale of a world matrix along its own axes.
fn lossy_scale(matrix: &Mat4) -> Vec3 {
    Vec3::new(
        matrix.x_axis.truncate().length(),
        matrix.y_axis.truncate().length(),
        matrix.z_axis.truncate().length(),
    )
}

/// True when `matrix` is a translation, rotation and scale and nothing else.
fn is_trs(matrix: &Mat4) -> bool {
    let (scale, rotation, position) = matrix.to_scale_rotation_translation();
    let back = Mat4::from_scale_rotation_translation(scale, rotation, position);
    let tolerance = 1e-3 * (1.0 + scale.max_element());
    matrix
        .to_cols_array()
        .iter()
        .zip(back.to_cols_array())
        .all(|(a, b)| (a - b).abs() <= tolerance)
}

fn euler(rotation: [f32; 3]) -> Quat {
    Quat::from_euler(
        glam::EulerRot::XYZ,
        rotation[0].to_radians(),
        rotation[1].to_radians(),
        rotation[2].to_radians(),
    )
}

/// The shape a collider asks for, from its saved geometry or its actor's Look.
fn shape_of(actor: &Actor, collider: &ColliderSpec, mode: Mode) -> Option<ColliderShape> {
    match &collider.geometry {
        ColliderGeometry::Shape { shape } => Some(shape.clone()),
        ColliderGeometry::FromLook => actor
            .components
            .visual()
            .and_then(|visual| shape_from_look(visual, mode)),
    }
}

fn plan_collider(
    actor: &Actor,
    collider: &ColliderSpec,
    carrier: Option<&Actor>,
    worlds: &HashMap<String, Mat4>,
    mode: Mode,
    settings: &PhysicsSettings,
    lookup: &dyn CollisionLookup,
) -> Result<Option<ColliderPlan>, PhysicsIssue> {
    let fail = |message: String| {
        Err(PhysicsIssue::error(message)
            .on(&actor.id)
            .of(collider.id.as_str()))
    };
    // A shape no Look implies is an empty collider, which validation has warned about.
    let Some(shape) = shape_of(actor, collider, mode) else {
        return Ok(None);
    };
    if !shape.fits(mode) {
        return fail(format!(
            "{} is a {} shape and this world is {}",
            shape.label(),
            shape.world_word(),
            super::material::dimension_word(mode)
        ));
    }
    if let ColliderShape::Tilemap = shape {
        return fail("A tilemap Look collides through its legacy Body for now; tile colliders arrive with the query and mesh phase".into());
    }
    if let ColliderShape::Terrain = shape {
        return fail("A terrain collides through its own heightfield for now; terrain colliders arrive with the query pass".into());
    }

    let own = worlds.get(&actor.id).copied().unwrap_or(Mat4::IDENTITY);
    let frame_actor = carrier.unwrap_or(actor);
    let frame = worlds
        .get(&frame_actor.id)
        .copied()
        .unwrap_or(Mat4::IDENTITY);
    let frame_scale = lossy_scale(&frame);
    let planar = mode == Mode::TwoD;
    let degenerate = |s: Vec3| {
        let axes = if planar { 2 } else { 3 };
        (0..axes).any(|i| s[i].abs() < 1e-6)
    };
    let scale = lossy_scale(&own);
    if degenerate(scale) {
        return fail(format!(
            "\"{}\" has a zero scale, so its collider has no size",
            actor.name
        ));
    }
    if degenerate(frame_scale) {
        return fail(format!(
            "\"{}\" has a zero scale, so the shapes it carries have no place",
            frame_actor.name
        ));
    }

    let offset =
        Mat4::from_rotation_translation(euler(collider.rotation), Vec3::from(collider.center));
    // Centre and rotation live in the collider's own (scaled) frame.
    let pose = own * offset;
    let relative = frame.inverse() * pose;
    if !is_trs(&relative) {
        return fail(format!(
            "\"{}\" is turned inside a stretched parent, which would shear its collider; remove the stretch or the turn",
            actor.name
        ));
    }
    let (_, rotation, local) = relative.to_scale_rotation_translation();
    let (frame_signed, _, _) = frame.to_scale_rotation_translation();
    let position = local * frame_signed;

    let material = match settings.materials.resolve(&collider.material, mode) {
        Ok(MaterialBody::Three { material }) => {
            PlannedMaterial::Three(collider.material_overrides.over(material))
        }
        Ok(MaterialBody::Two { material }) => PlannedMaterial::Two(material),
        Err(why) => return fail(why),
    };

    let (planned_shape, volume) = if let ColliderShape::ConvexHull { mesh }
    | ColliderShape::TriangleMesh { mesh } = &shape
    {
        let dynamic = carrier
            .and_then(|a| a.components.rigidbody())
            .is_some_and(|spec| spec.body_type == BodyType::Dynamic);
        let decomposed = settings.cooking.decomposition_of(mesh).is_some();
        let kind = match &shape {
            ColliderShape::ConvexHull { .. } if decomposed => MeshKind::Decomposed,
            ColliderShape::ConvexHull { .. } => MeshKind::Hull,
            _ if dynamic && decomposed && !collider.trigger => MeshKind::Decomposed,
            _ => MeshKind::Triangles,
        };
        let cooked = match lookup.cooked(mesh, kind, &settings.cooking) {
            Ok(cooked) => cooked,
            Err(why) => return fail(why),
        };
        // Signed, so a mirrored actor mirrors its mesh and keeps its winding.
        let (signed, _, _) = own.to_scale_rotation_translation();
        let mesh_shape = MeshShape::from_cooked(&cooked, signed.to_array());
        let volume = mesh_shape.volume();
        (PlannedShape::Mesh(mesh_shape), volume)
    } else if planar {
        let Some(solid) = geometry::solid2(&shape, [scale.x, scale.y]) else {
            return fail(format!("{} has no 2D form", shape.label()));
        };
        if let Solid2::Convex { points } = &solid
            && !geometry::is_convex(points)
        {
            return fail(
                "A polygon collider must be convex; split a concave outline into several polygons or use a chain".into(),
            );
        }
        let area = solid.area();
        (PlannedShape::Two(solid), area)
    } else {
        let Some(solid) = geometry::solid3(&shape, scale.to_array()) else {
            return fail(format!("{} has no 3D form", shape.label()));
        };
        let volume = solid.volume();
        (PlannedShape::Three(solid), volume)
    };

    let filter = ColliderFilter {
        layer: collider.layer,
        overrides: collider.layer_overrides,
        legacy_mask: collider.legacy_mask,
    };
    Ok(Some(ColliderPlan {
        collider: collider.id.clone(),
        name: collider.name.clone(),
        actor: actor.id.clone(),
        body_actor: None,
        shape: planned_shape,
        position: position.to_array(),
        local: local.to_array(),
        rotation: rotation.to_array(),
        material,
        trigger: collider.trigger,
        enabled: collider.enabled,
        queryable: collider.queryable,
        one_way: collider.one_way,
        layer: collider.layer,
        filter,
        memberships: 0,
        accepts: 0,
        contact_offset: collider.contact_offset,
        volume,
        density: 0.0,
    }))
}

/// Splits a body's mass between its shapes and the body itself.
fn plan_mass(
    plan: &mut PhysicsPlan,
    body_index: usize,
    mode: Mode,
    issues: &mut Vec<PhysicsIssue>,
) {
    let (actor, spec, indices) = {
        let body = &plan.bodies[body_index];
        (
            body.actor.clone(),
            body.spec.clone(),
            body.colliders.clone(),
        )
    };
    let massive: Vec<usize> = indices
        .iter()
        .copied()
        .filter(|&i| {
            let c = &plan.colliders[i];
            !c.trigger && c.enabled && c.volume > 0.0
        })
        .collect();
    let volume: f32 = massive.iter().map(|&i| plan.colliders[i].volume).sum();
    let custom = spec.center_of_mass.is_some() || spec.inertia.is_some();
    let dynamic = spec.body_type == BodyType::Dynamic;

    if spec.center_of_mass.is_some() && spec.inertia.is_none() {
        issues.push(
            PhysicsIssue::error(
                "A custom center of mass needs an inertia too; automatic inertia around a moved center is not supported yet",
            )
            .on(&actor)
            .of(spec.id.as_str())
            .field("inertia"),
        );
    }

    let wanted_mass = match spec.mass {
        MassSource::Explicit { mass } => Some(mass),
        MassSource::Density { .. } => None,
    };
    let density = match spec.mass {
        MassSource::Density { density } => density,
        MassSource::Explicit { mass } if volume > 0.0 => mass / volume,
        MassSource::Explicit { .. } => 0.0,
    };

    let mut extra = None;
    let mut total = density * volume;
    if volume <= 0.0 || custom {
        // The body holds its own mass: nothing carries it, or a custom center
        // or inertia can't be laid over a sum of shapes.
        let mass = wanted_mass.unwrap_or(DEFAULT_MASS);
        total = mass;
        let inertia = spec.inertia;
        extra = Some(match inertia {
            Some(inertia) => ExtraMass::Full {
                mass,
                center: spec
                    .center_of_mass
                    .unwrap_or_else(|| shape_center(plan, &massive, mode)),
                inertia,
            },
            None => ExtraMass::Mass(mass),
        });
        for &i in &indices {
            plan.colliders[i].density = 0.0;
        }
    } else {
        for &i in &massive {
            plan.colliders[i].density = density;
        }
    }
    if !dynamic {
        // Fixed and kinematic bodies never integrate; keep the numbers for display only.
        extra = None;
        for &i in &indices {
            plan.colliders[i].density = 0.0;
        }
    }
    let body = &mut plan.bodies[body_index];
    body.extra_mass = extra;
    body.total_mass = total;
}

/// The volume-weighted middle of the shapes, in the body's frame.
fn shape_center(plan: &PhysicsPlan, massive: &[usize], mode: Mode) -> [f32; 3] {
    let total: f32 = massive.iter().map(|&i| plan.colliders[i].volume).sum();
    if total <= 0.0 {
        return [0.0; 3];
    }
    let mut sum = Vec3::ZERO;
    for &i in massive {
        let c = &plan.colliders[i];
        let mut at = Vec3::from(c.position);
        // A convex polygon's middle is not its origin.
        if let (PlannedShape::Two(Solid2::Convex { points }), Mode::TwoD) = (&c.shape, mode) {
            let n = points.len() as f32;
            let mid = points
                .iter()
                .fold([0.0f32; 2], |a, p| [a[0] + p[0], a[1] + p[1]]);
            let local = Quat::from_array(c.rotation) * Vec3::new(mid[0] / n, mid[1] / n, 0.0);
            at += local;
        }
        sum += at * c.volume;
    }
    (sum / total).to_array()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ActorComponent;
    use crate::project::Project;
    use crate::scene::Visual;

    fn project() -> Project {
        Project::starter("Plan", Mode::ThreeD)
    }

    fn add(project: &mut Project, name: &str) -> String {
        project.add_actor(Actor::new(
            name,
            Visual::Cuboid {
                color: "#ffffff".into(),
                size: [1.0; 3],
            },
        ))
    }

    fn plan_of(project: &Project) -> PhysicsPlan {
        PhysicsPlan::build(
            &project.active_scene().actors,
            Mode::ThreeD,
            &project.physics,
        )
    }

    fn give(project: &mut Project, id: &str, body: RigidbodySpec, shapes: Vec<ColliderSpec>) {
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        scene.set_rigidbody(id, body, &library).unwrap();
        for shape in shapes {
            scene.add_collider(id, shape, &library).unwrap();
        }
    }

    #[test]
    fn explicit_mass_is_exact_over_any_number_of_shapes() {
        let mut project = project();
        let id = add(&mut project, "Body");
        let body = RigidbodySpec {
            mass: MassSource::Explicit { mass: 5.0 },
            ..Default::default()
        };
        give(
            &mut project,
            &id,
            body,
            vec![
                ColliderSpec::new(ColliderShape::Sphere { radius: 0.5 }),
                ColliderSpec::new(ColliderShape::Box {
                    size: [1.0, 2.0, 3.0],
                }),
            ],
        );
        let plan = plan_of(&project);
        assert!(plan.is_runnable(), "{:?}", plan.issues);
        let body = plan.body(&id).unwrap();
        assert_eq!(body.colliders.len(), 2);
        assert!(body.extra_mass.is_none());
        let carried: f32 = body
            .colliders
            .iter()
            .map(|&i| plan.colliders[i].density * plan.colliders[i].volume)
            .sum();
        assert!((carried - 5.0).abs() < 1e-4, "{carried}");
        assert!((body.total_mass - 5.0).abs() < 1e-4);
    }

    #[test]
    fn density_mode_weighs_each_shape_by_its_volume() {
        let mut project = project();
        let id = add(&mut project, "Body");
        let body = RigidbodySpec {
            mass: MassSource::Density { density: 2.0 },
            ..Default::default()
        };
        give(
            &mut project,
            &id,
            body,
            vec![ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] })],
        );
        let plan = plan_of(&project);
        assert!((plan.body(&id).unwrap().total_mass - 2.0).abs() < 1e-5);
    }

    #[test]
    fn a_trigger_carries_no_mass() {
        let mut project = project();
        let id = add(&mut project, "Body");
        let mut sensor = ColliderSpec::new(ColliderShape::Sphere { radius: 5.0 });
        sensor.trigger = true;
        give(
            &mut project,
            &id,
            RigidbodySpec {
                mass: MassSource::Explicit { mass: 3.0 },
                ..Default::default()
            },
            vec![
                ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] }),
                sensor,
            ],
        );
        let plan = plan_of(&project);
        let body = plan.body(&id).unwrap();
        let sensor = body
            .colliders
            .iter()
            .map(|&i| &plan.colliders[i])
            .find(|c| c.trigger)
            .unwrap();
        assert_eq!(sensor.density, 0.0);
        assert!((body.total_mass - 3.0).abs() < 1e-4);
    }

    #[test]
    fn a_body_without_a_shape_holds_its_own_mass() {
        let mut project = project();
        let id = add(&mut project, "Lonely");
        give(
            &mut project,
            &id,
            RigidbodySpec {
                mass: MassSource::Explicit { mass: 2.0 },
                ..Default::default()
            },
            vec![],
        );
        let plan = plan_of(&project);
        let body = plan.body(&id).unwrap();
        assert_eq!(body.extra_mass, Some(ExtraMass::Mass(2.0)));
        assert_eq!(body.total_mass, 2.0);
        // Density has nothing to weigh, so the default stands in.
        let id = add(&mut project, "Dense");
        give(
            &mut project,
            &id,
            RigidbodySpec {
                mass: MassSource::Density { density: 9.0 },
                ..Default::default()
            },
            vec![],
        );
        let plan = plan_of(&project);
        assert_eq!(
            plan.body(&id).unwrap().extra_mass,
            Some(ExtraMass::Mass(DEFAULT_MASS))
        );
    }

    #[test]
    fn a_custom_center_of_mass_needs_an_inertia() {
        let mut project = project();
        let id = add(&mut project, "Odd");
        let spec = RigidbodySpec {
            center_of_mass: Some([0.0, -1.0, 0.0]),
            ..Default::default()
        };
        give(
            &mut project,
            &id,
            spec,
            vec![ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] })],
        );
        let plan = plan_of(&project);
        assert!(!plan.is_runnable());
        assert!(plan.errors().any(|e| e.message.contains("inertia")));
    }

    #[test]
    fn a_custom_inertia_moves_the_mass_onto_the_body() {
        let mut project = project();
        let id = add(&mut project, "Tuned");
        let spec = RigidbodySpec {
            mass: MassSource::Explicit { mass: 4.0 },
            inertia: Some([1.0, 2.0, 3.0]),
            ..Default::default()
        };
        give(
            &mut project,
            &id,
            spec,
            vec![ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] })],
        );
        let plan = plan_of(&project);
        assert!(plan.is_runnable(), "{:?}", plan.issues);
        let body = plan.body(&id).unwrap();
        assert_eq!(
            body.extra_mass,
            Some(ExtraMass::Full {
                mass: 4.0,
                center: [0.0; 3],
                inertia: [1.0, 2.0, 3.0]
            })
        );
        assert!(
            body.colliders
                .iter()
                .all(|&i| plan.colliders[i].density == 0.0)
        );
    }

    #[test]
    fn a_child_shape_stands_in_its_bodys_frame_in_metres() {
        let mut project = project();
        let car = add(&mut project, "Car");
        let wheel = add(&mut project, "Wheel");
        give(
            &mut project,
            &car,
            RigidbodySpec::default(),
            vec![ColliderSpec::new(ColliderShape::Box {
                size: [2.0, 1.0, 4.0],
            })],
        );
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        let wheel_collider = scene
            .add_collider(
                &wheel,
                ColliderSpec::new(ColliderShape::Sphere { radius: 0.5 }),
                &library,
            )
            .unwrap();
        scene
            .actors
            .iter_mut()
            .find(|a| a.id == car)
            .unwrap()
            .components
            .placement_mut()
            .scale = 2.0;
        scene
            .actors
            .iter_mut()
            .find(|a| a.id == wheel)
            .unwrap()
            .components
            .placement_mut()
            .position = [3.0, 0.0, 0.0];
        assert!(scene.move_actor(&wheel, &car, "").unwrap());

        let plan = plan_of(&project);
        assert!(plan.is_runnable(), "{:?}", plan.issues);
        let owned = plan.collider(&wheel_collider).unwrap();
        assert_eq!(owned.body_actor.as_deref(), Some(car.as_str()));
        // Three metres out in the world is three metres in the body's frame, even
        // though the body is drawn twice as big.
        assert!(
            (owned.position[0] - 3.0).abs() < 1e-4,
            "{:?}",
            owned.position
        );
        // The car's own box is twice as big as authored.
        let own = plan.colliders.iter().find(|c| c.actor == car).unwrap();
        match &own.shape {
            PlannedShape::Three(Solid3::Cuboid { half }) => {
                assert_eq!(*half, [2.0, 1.0, 4.0]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn scenery_is_planned_without_a_body() {
        let mut project = project();
        let id = add(&mut project, "Wall");
        let library = project.physics.materials.clone();
        let collider = project
            .active_scene_mut()
            .add_collider(
                &id,
                ColliderSpec::new(ColliderShape::Box {
                    size: [10.0, 4.0, 0.2],
                }),
                &library,
            )
            .unwrap();
        let plan = plan_of(&project);
        let planned = plan.collider(&collider).unwrap();
        assert_eq!(planned.body_actor, None);
        assert!(plan.bodies.is_empty());
    }

    #[test]
    fn a_mesh_collider_without_cooked_data_says_so() {
        let mut project = project();
        let id = add(&mut project, "Rock");
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        scene
            .add_collider(
                &id,
                ColliderSpec::new(ColliderShape::ConvexHull {
                    mesh: "rock.glb".into(),
                }),
                &library,
            )
            .unwrap();
        let plan = plan_of(&project);
        assert!(!plan.is_runnable());
        assert!(
            plan.errors()
                .any(|e| e.message.contains("rock.glb") && e.message.contains("not available"))
        );
    }

    mod meshes {
        use super::*;
        use crate::physics::cook::{Decompose, FolderCollision, Source};

        /// A project folder holding a unit-ish crate (2 on a side) and an L.
        fn folder(name: &str) -> std::path::PathBuf {
            let dir = std::env::temp_dir().join(format!("blockloom-plan-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("assets")).unwrap();
            let cube = crate::physics::cook::tests::cube(2.0);
            let mut crate_obj = String::new();
            for p in &cube.positions {
                crate_obj.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
            }
            for t in cube.indices.chunks(3) {
                crate_obj.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
            }
            std::fs::write(dir.join("assets/crate.obj"), crate_obj).unwrap();
            dir
        }

        fn with_mesh(
            project: &mut Project,
            name: &str,
            shape: ColliderShape,
            body: Option<RigidbodySpec>,
        ) -> String {
            let id = add(project, name);
            let library = project.physics.materials.clone();
            let scene = project.active_scene_mut();
            if let Some(body) = body {
                scene.set_rigidbody(&id, body, &library).unwrap();
            }
            scene
                .add_collider(&id, ColliderSpec::new(shape), &library)
                .unwrap();
            id
        }

        fn plan_in(project: &Project, dir: &std::path::Path) -> PhysicsPlan {
            let lookup = FolderCollision::new(dir, Source::Cook);
            project
                .active_scene()
                .physics_plan_with(&project.physics, &lookup)
        }

        #[test]
        fn a_hull_carries_mass_by_its_cooked_volume_under_scale() {
            let dir = folder("hull");
            let mut project = project();
            let id = with_mesh(
                &mut project,
                "Crate",
                ColliderShape::ConvexHull {
                    mesh: "assets/crate.obj".into(),
                },
                Some(RigidbodySpec {
                    mass: MassSource::Explicit { mass: 4.0 },
                    ..Default::default()
                }),
            );
            project
                .active_scene_mut()
                .actors
                .iter_mut()
                .find(|a| a.id == id)
                .unwrap()
                .components
                .placement_mut()
                .scale = 2.0;
            let plan = plan_in(&project, &dir);
            assert!(plan.is_runnable(), "{:?}", plan.issues);
            let planned = &plan.colliders[0];
            // A 2 m crate at scale 2 is 4 m on a side.
            assert!((planned.volume - 64.0).abs() < 1e-2, "{}", planned.volume);
            assert!((plan.bodies[0].total_mass - 4.0).abs() < 1e-3);
            assert!((planned.density - 4.0 / 64.0).abs() < 1e-4);
            let PlannedShape::Mesh(MeshShape::Hull { points, .. }) = &planned.shape else {
                panic!("{:?}", planned.shape);
            };
            assert_eq!(points.0.len(), 8);
            assert!(
                points
                    .0
                    .iter()
                    .flatten()
                    .all(|v| (v.abs() - 2.0).abs() < 1e-4)
            );
        }

        #[test]
        fn a_triangle_mesh_is_scenery_unless_its_body_is_decomposed() {
            let dir = folder("trimesh");
            let mut project = project();
            let ground = with_mesh(
                &mut project,
                "Ground",
                ColliderShape::TriangleMesh {
                    mesh: "assets/crate.obj".into(),
                },
                None,
            );
            let plan = plan_in(&project, &dir);
            assert!(plan.is_runnable(), "{:?}", plan.issues);
            assert!(matches!(
                plan.collider(&plan.colliders[0].collider).unwrap().shape,
                PlannedShape::Mesh(MeshShape::Triangles { .. })
            ));
            assert_eq!(plan.colliders[0].volume, 0.0);
            assert_eq!(plan.colliders[0].actor, ground);

            // On a dynamic body it is refused, until the mesh is set to decompose.
            let mut project = super::project();
            with_mesh(
                &mut project,
                "Barrel",
                ColliderShape::TriangleMesh {
                    mesh: "assets/crate.obj".into(),
                },
                Some(RigidbodySpec::default()),
            );
            let plan = plan_in(&project, &dir);
            assert!(plan.errors().any(|e| e.message.contains("dynamic body")));
            project
                .physics
                .cooking
                .decompose
                .insert("assets/crate.obj".into(), Decompose::default());
            let plan = plan_in(&project, &dir);
            assert!(plan.is_runnable(), "{:?}", plan.issues);
            let PlannedShape::Mesh(MeshShape::Compound { hulls, volume }) =
                &plan.colliders[0].shape
            else {
                panic!("{:?}", plan.colliders[0].shape);
            };
            assert_eq!(hulls.len(), 1, "a crate is already convex");
            assert!(*volume > 7.0 && plan.colliders[0].volume == *volume);
        }

        #[test]
        fn a_missing_model_is_one_error_naming_it() {
            let dir = folder("missing");
            let mut project = project();
            with_mesh(
                &mut project,
                "Ghost",
                ColliderShape::ConvexHull {
                    mesh: "assets/ghost.obj".into(),
                },
                None,
            );
            let plan = plan_in(&project, &dir);
            let errors: Vec<_> = plan.errors().collect();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(errors[0].message.contains("assets/ghost.obj"));
        }
    }

    #[test]
    fn a_zero_scale_has_no_collider_to_build() {
        let mut project = project();
        let id = add(&mut project, "Flat");
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        scene
            .add_collider(
                &id,
                ColliderSpec::new(ColliderShape::Sphere { radius: 1.0 }),
                &library,
            )
            .unwrap();
        scene
            .actors
            .iter_mut()
            .find(|a| a.id == id)
            .unwrap()
            .components
            .placement_mut()
            .scale = 0.0;
        let plan = plan_of(&project);
        assert!(plan.errors().any(|e| e.message.contains("zero scale")));
    }

    #[test]
    fn a_turned_child_of_a_stretched_parent_is_refused() {
        let mut project = project();
        let parent = add(&mut project, "Parent");
        let child = add(&mut project, "Child");
        give(&mut project, &parent, RigidbodySpec::default(), vec![]);
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        scene
            .add_collider(
                &child,
                ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] }),
                &library,
            )
            .unwrap();
        scene
            .actors
            .iter_mut()
            .find(|a| a.id == parent)
            .unwrap()
            .components
            .placement_mut()
            .stretch = [3.0, 1.0, 1.0];
        scene
            .actors
            .iter_mut()
            .find(|a| a.id == child)
            .unwrap()
            .components
            .placement_mut()
            .rotation = [0.0, 0.0, 33.0];
        assert!(scene.move_actor(&child, &parent, "").unwrap());
        let plan = plan_of(&project);
        assert!(
            plan.errors().any(|e| e.message.contains("shear")),
            "{:?}",
            plan.issues
        );
    }

    #[test]
    fn materials_resolve_with_their_overrides() {
        let mut project = project();
        let id = add(&mut project, "Slick");
        let mut spec = ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] });
        spec.material = super::super::MaterialRef::BuiltIn { name: "Ice".into() };
        spec.material_overrides.bounciness = Some(0.5);
        let library = project.physics.materials.clone();
        let collider = project
            .active_scene_mut()
            .add_collider(&id, spec, &library)
            .unwrap();
        let plan = plan_of(&project);
        match plan.collider(&collider).unwrap().material {
            PlannedMaterial::Three(m) => {
                assert_eq!(m.bounciness, 0.5);
                assert!(m.dynamic_friction < 0.1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn layer_overrides_with_an_include_ask_for_the_exact_test() {
        let mut project = project();
        let a = add(&mut project, "A");
        let b = add(&mut project, "B");
        let physics = project.physics.clone();
        let library = physics.materials.clone();
        let scene = project.active_scene_mut();
        scene
            .add_collider(
                &a,
                ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] }),
                &library,
            )
            .unwrap();
        let plain = PhysicsPlan::build(&scene.actors, Mode::ThreeD, &physics);
        assert!(!plain.exact_filtering);
        let mut spec = ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] });
        spec.layer_overrides.include = 0b100;
        scene.add_collider(&b, spec, &library).unwrap();
        let exact = PhysicsPlan::build(&scene.actors, Mode::ThreeD, &physics);
        assert!(exact.exact_filtering);
        assert!(exact.colliders.iter().all(|c| c.accepts == u32::MAX));
    }

    #[test]
    fn two_d_shapes_and_materials_plan_in_a_2d_world() {
        let mut project = Project::starter("Flat", Mode::TwoD);
        let id = project.add_actor(Actor::new(
            "Box",
            Visual::Rect {
                color: "#ffffff".into(),
                size: [20.0, 10.0],
            },
        ));
        let physics = project.physics.clone();
        let library = physics.materials.clone();
        let scene = project.active_scene_mut();
        scene
            .set_rigidbody(&id, RigidbodySpec::default(), &library)
            .unwrap();
        scene
            .add_collider(&id, ColliderSpec::from_look(), &library)
            .unwrap();
        let plan = PhysicsPlan::build(&scene.actors, Mode::TwoD, &physics);
        assert!(plan.is_runnable(), "{:?}", plan.issues);
        let planned = &plan.colliders[0];
        assert!(matches!(planned.material, PlannedMaterial::Two(_)));
        match &planned.shape {
            PlannedShape::Two(Solid2::Cuboid { half }) => assert_eq!(*half, [10.0, 5.0]),
            other => panic!("{other:?}"),
        }
        assert!((planned.volume - 200.0).abs() < 1e-3);
    }

    #[test]
    fn a_legacy_actor_plans_nothing() {
        let mut project = project();
        let id = add(&mut project, "Old");
        project
            .actor_mut(&id)
            .unwrap()
            .components
            .set_physics(crate::scene::Physics {
                body: crate::scene::BodyKind::Dynamic,
                ..Default::default()
            });
        assert!(plan_of(&project).is_empty());
        let _ = ActorComponent::Collider {
            collider: ColliderSpec::default(),
        };
    }
}
