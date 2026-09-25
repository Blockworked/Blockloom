//! Punctual lights: an actor's `Light` component as a Bevy point or spot
//! light, in lumens with a range in metres. 3D only.
//!
//! The light rides a child entity rather than the actor itself, since
//! batching hides a merged actor through an empty `RenderLayers`, which
//! would switch its light off too.

use crate::engine::{ActorId, Engine, PendingEffects};
use crate::world::parse_color;
use bevy::prelude::*;
use blockloom_core::components::{LightKind, LightSpec};
use blockloom_core::vm::Effect;

/// What an actor's light child was built from, so a quiet frame changes
/// nothing.
#[derive(Component)]
pub struct Lit {
    spec: LightSpec,
    child: Entity,
}

/// The child entity carrying an actor's light.
#[derive(Component)]
pub struct ActorLight;

/// `set my light to` for the rest of the run, by actor id. Cleared with
/// everything else live on a rebuild.
pub fn apply_light_effects(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        if let Effect::SetLightIntensity { actor, intensity } = effect {
            engine
                .light_intensity
                .insert(actor.clone(), intensity.max(0.0));
        }
    }
}

/// The light each actor should be carrying right now, from what it holds
/// (`engine.attached`, so attach and detach work mid-run), what the editor
/// authored and what the blocks have set it to since.
fn wanted(engine: &Engine, actor: &str) -> Option<LightSpec> {
    if !engine.has_component(actor, "Light") {
        return None;
    }
    let mut spec = engine.actor(actor)?.components.light()?.clone();
    if let Some(intensity) = engine.light_intensity.get(actor) {
        spec.intensity = *intensity;
    }
    Some(spec)
}

/// Builds, rebuilds or takes away each actor's light child to match.
pub fn sync_lights(
    mut commands: Commands,
    engine: NonSend<Engine>,
    actors: Query<(Entity, &ActorId, Option<&Lit>)>,
) {
    for (entity, id, lit) in &actors {
        let spec = wanted(&engine, &id.0);
        if spec.as_ref() == lit.map(|lit| &lit.spec) {
            continue;
        }
        if let Some(lit) = lit {
            commands.entity(lit.child).despawn();
            commands.entity(entity).remove::<Lit>();
        }
        let Some(spec) = spec else {
            continue;
        };
        let mut child = commands.spawn((ActorLight, Transform::default(), ChildOf(entity)));
        insert_light(&mut child, &spec);
        let child = child.id();
        commands.entity(entity).insert(Lit { spec, child });
    }
}

fn insert_light(entity: &mut EntityCommands, spec: &LightSpec) {
    let color = parse_color(&spec.color);
    let intensity = spec.intensity.max(0.0);
    let range = spec.range.max(0.01);
    let radius = spec.radius.max(0.0);
    match spec.kind {
        LightKind::Point => {
            entity.insert(PointLight {
                color,
                intensity,
                range,
                radius,
                shadow_maps_enabled: spec.shadows,
                ..default()
            });
        }
        LightKind::Spot => {
            let (inner_angle, outer_angle) = spec.cone();
            entity.insert(SpotLight {
                color,
                intensity,
                range,
                radius,
                shadow_maps_enabled: spec.shadows,
                inner_angle,
                outer_angle,
                ..default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::components::ActorComponent;
    use blockloom_core::scene::Mode;

    fn app_with_lamp(kind: LightKind) -> (App, Entity) {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        let lamp = &mut engine.project.actors[0];
        lamp.components.insert(ActorComponent::Light {
            light: LightSpec {
                kind,
                ..LightSpec::default()
            },
        });
        let id = lamp.id.clone();
        engine
            .attached
            .entry(id.clone())
            .or_default()
            .insert("Light".to_string());
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<PendingEffects>()
            .add_systems(Update, (apply_light_effects, sync_lights).chain());
        let actor = app.world_mut().spawn(ActorId(id)).id();
        (app, actor)
    }

    fn light_child(app: &mut App) -> Option<Entity> {
        app.world_mut()
            .query_filtered::<Entity, With<ActorLight>>()
            .iter(app.world())
            .next()
    }

    #[test]
    fn a_light_component_hangs_a_light_off_its_actor() {
        let (mut app, actor) = app_with_lamp(LightKind::Spot);
        app.update();
        let child = light_child(&mut app).expect("a light child");
        assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), actor);
        assert_eq!(
            app.world().get::<SpotLight>(child).unwrap().intensity,
            800.0
        );

        // A quiet frame leaves the same child in place.
        app.update();
        assert_eq!(light_child(&mut app), Some(child));
    }

    #[test]
    fn a_block_brightens_the_light_and_detaching_takes_it_away() {
        let (mut app, actor) = app_with_lamp(LightKind::Point);
        let id = app.world().get::<ActorId>(actor).unwrap().0.clone();
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.update();
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::SetLightIntensity {
                actor: id.clone(),
                intensity: 2400.0,
            });
        app.update();
        let child = light_child(&mut app).unwrap();
        assert_eq!(
            app.world().get::<PointLight>(child).unwrap().intensity,
            2400.0
        );

        app.world_mut()
            .non_send_mut::<Engine>()
            .attached
            .get_mut(&id)
            .unwrap()
            .remove("Light");
        app.update();
        assert!(light_child(&mut app).is_none());
        assert!(app.world().get::<Lit>(actor).is_none());
    }
}
