//! The type registry: what every built-in component and world setting looks
//! like, with its defaults, described by `#[derive(Schema)]` in the same
//! `FieldType` model plugin schemas use. Inspectors, settings pages, the
//! shell and MCP read it instead of keeping their own copies of field lists
//! and defaults; `set_property` checks a write against it.

use crate::components::ActorComponent;
use crate::physics::controller::CharacterControllerSpec;
use crate::physics::motor::CharacterMotorSpec;
use crate::scene::{Mode, World};
use crate::settings::GameSettings;
use blockloom_plugin_api::schema::{FieldSchema, FieldType, Schema, fill_field_defaults};
use serde_json::{Value, json};

/// One kind of actor component.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ComponentType {
    /// The `component` tag, `Light`.
    pub name: String,
    /// What the Add component menu calls it.
    pub label: String,
    /// `2d` or `3d` when the kind only works in one dimension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<&'static str>,
    /// More than one may sit on an actor.
    pub repeatable: bool,
    /// Made by its own command (it needs a file or a shape), not from defaults.
    pub special: bool,
    /// The component's fields beside the tag, defaults filled in.
    pub fields: Vec<FieldSchema>,
    /// A fresh component, as Add component makes it.
    pub default: Value,
}

/// Every component kind but `Place` (always there) and `Plugin` (described
/// by its plugin's own schema), in Add component order.
pub const COMPONENTS: &[&str] = &[
    "Look",
    "Render",
    "Body",
    "Rigidbody",
    "Collider",
    "Constraint",
    "CharacterController",
    "CharacterMotor",
    "PlayerCamera",
    "Joint",
    "Brain",
    "Camera",
    "Script",
    "Parent",
    "Material",
    "Emitter",
    "Trail",
    "Light",
    "Light2d",
    "Conveyor",
    "Hazard",
    "Animation",
    "Sprite",
    "Volume",
    "SoundZone",
    "Probe",
    "Terrain",
    "Fracture",
    "Water",
    "Buoyancy",
    "Parallax",
    "Room",
    "Persist",
    "Prefab",
    "Custom",
];

fn label(name: &str) -> &str {
    match name {
        "Custom" => "Custom…",
        "Light2d" => "2D light",
        "Conveyor" => "Conveyor belt",
        "Hazard" => "Hurt volume",
        "SoundZone" => "Sound zone",
        other => other,
    }
}

fn dimension(name: &str) -> Option<&'static str> {
    match name {
        "Sprite" | "Light2d" | "Conveyor" | "Hazard" => Some("2d"),
        "Fracture" => Some("3d"),
        _ => None,
    }
}

/// The key a component keeps its spec under, beside its tag.
fn spec_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "Look" => "visual",
        "Body" => "physics",
        "Rigidbody" => "rigidbody",
        "Collider" => "collider",
        "Constraint" => "constraint",
        "CharacterController" => "controller",
        "CharacterMotor" => "motor",
        "PlayerCamera" => "player_camera",
        "Joint" => "joint",
        "Brain" => "brain",
        "Camera" => "camera",
        "Material" => "material",
        "Fracture" => "fracture",
        "Emitter" => "emitter",
        "Trail" => "trail",
        "Light" => "light",
        "Light2d" => "light2d",
        "Conveyor" => "conveyor",
        "Hazard" => "hazard",
        "Animation" => "animation",
        "Volume" => "volume",
        "Probe" => "probe",
        "Terrain" => "terrain",
        "Sprite" => "sprite",
        "Water" => "water",
        "Buoyancy" => "buoyancy",
        "Parallax" => "parallax",
        "Room" => "room",
        "SoundZone" => "zone",
        "Prefab" => "prefab",
        _ => return None,
    })
}

/// A fresh component of kind `name` for a `mode` world, as Add component
/// makes it. `None` for a kind made by its own command (Collider, Script) or
/// one that isn't a component.
pub fn default_component(name: &str, mode: Mode) -> Option<ActorComponent> {
    let three = mode.is_3d();
    let tag = |extra: Value| -> Value {
        let mut object = extra.as_object().cloned().unwrap_or_default();
        object.insert("component".into(), json!(name));
        Value::Object(object)
    };
    let value = match name {
        "Collider" | "Script" | "Prefab" => return None,
        "Look" if three => {
            tag(json!({"visual": {"shape": "Cuboid", "color": "#4C97FF", "size": [1, 1, 1]}}))
        }
        "Look" => tag(json!({"visual": {"shape": "Rect", "color": "#4C97FF", "size": [60, 60]}})),
        "Render" => tag(json!({"visible": true, "layer": 0})),
        "Body" => tag(json!({"physics": {
            "body": "Dynamic", "gravity_scale": 1, "lock_rotation": false, "restitution": 0,
            "friction": 0.5, "density": 1, "mass": null, "trigger": false,
            "collision_layer": 1, "collision_mask": 255
        }})),
        "Joint" => {
            tag(json!({"joint": {"target": "", "kind": "Fixed", "anchor": [0, 0, 0], "length": 2}}))
        }
        "PlayerCamera" if !three => tag(json!({"player_camera": {
            "look": false, "collision": false, "smoothing": 0.15, "dead_zone": [48, 32], "look_ahead": 64
        }})),
        // Each dimension's own sizes: metres in 3D, pixels in 2D.
        "CharacterController" => tag(json!({"controller":
            serde_json::to_value(CharacterControllerSpec::for_mode(mode)).ok()?})),
        "CharacterMotor" => tag(json!({"motor":
            serde_json::to_value(CharacterMotorSpec::for_mode(mode)).ok()?})),
        "Camera" => tag(json!({"camera": {
            "view": "ThirdPerson", "offset": [0, 0.6, 0], "distance": 6, "pitch": 15, "fov": 75
        }})),
        "Parent" => tag(json!({"parent": "", "offset": null})),
        "Animation" => tag(json!({"animation": {"clips": [], "states": []}})),
        "Fracture" => tag(json!({"fracture": {
            "cells": 16, "seed": 1, "interior": {}, "impulse_threshold": 8, "lifetime": 20,
            "sleep_seconds": 2, "pool_cap": 256, "bounce_sound": ""
        }})),
        "Water" if !three => tag(json!({"water": {
            "size": [800, 300], "depth": 300,
            "waves": {"amplitude": 6, "wavelength": 220, "direction": 90, "spread": 20},
            "detail": {"scale": 60}, "look": {"absorption": 220},
            "foam": {"shore": 12, "scale": 40}, "underwater": {"distance": 600, "caustics_scale": 60},
            "splash": {"min_speed": 60}, "ripples": {"speed": 120, "extent": 1600}
        }})),
        "Persist" => tag(json!({})),
        "Custom" => tag(json!({"name": "Component", "fields": [
            {"name": "value", "value": {"kind": "Number", "value": 0}}
        ]})),
        other => tag(json!({ spec_key(other)?: {} })),
    };
    serde_json::from_value(value).ok()
}

/// The fields of component `name` beside its tag, straight from the
/// derive.
pub(crate) fn component_fields(name: &str) -> Vec<FieldSchema> {
    match ActorComponent::field_type() {
        FieldType::Enum { variants, .. } => variants
            .into_iter()
            .find(|v| v.name == name)
            .map(|v| v.fields)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Every built-in component kind for a `mode` world.
pub fn component_types(mode: Mode) -> Vec<ComponentType> {
    COMPONENTS
        .iter()
        .map(|name| {
            let default = default_component(name, mode)
                .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            let mut fields = component_fields(name);
            fill_field_defaults(&mut fields, &default);
            for_mode(&mut fields, mode);
            ComponentType {
                name: name.to_string(),
                label: label(name).to_string(),
                dimension: dimension(name),
                repeatable: matches!(*name, "Custom" | "Collider" | "Constraint" | "Script"),
                special: matches!(*name, "Collider" | "Script" | "Prefab"),
                fields,
                default,
            }
        })
        .collect()
}

/// Drops fields marked for the other dimension (`only_2d`/`only_3d`), at
/// any depth, and the marks themselves.
fn for_mode(fields: &mut Vec<FieldSchema>, mode: Mode) {
    let other = if mode == Mode::TwoD { "3d" } else { "2d" };
    fields.retain(|f| f.ui.as_ref().is_none_or(|ui| ui.dimension != other));
    for field in fields.iter_mut() {
        if let Some(ui) = field.ui.as_mut() {
            ui.dimension.clear();
            if *ui == Default::default() {
                field.ui = None;
            }
        }
        type_for_mode(&mut field.ty, mode);
    }
}

fn type_for_mode(ty: &mut FieldType, mode: Mode) {
    match ty {
        FieldType::Struct { fields } => for_mode(fields, mode),
        FieldType::List { item: inner, .. }
        | FieldType::Map { value: inner }
        | FieldType::Optional { inner } => type_for_mode(inner, mode),
        FieldType::Enum { variants, .. } => {
            for v in variants {
                for_mode(&mut v.fields, mode);
                if let Some(inner) = v.inner.as_mut() {
                    type_for_mode(inner, mode);
                }
            }
        }
        _ => {}
    }
}

/// A scene's world settings (sky, fog, post, quality, ...) with defaults
/// filled in from `World::default()`.
pub fn world_fields() -> Vec<FieldSchema> {
    let FieldType::Struct { mut fields } = World::field_type() else {
        return Vec::new();
    };
    let default = serde_json::to_value(World::default()).unwrap_or(Value::Null);
    fill_field_defaults(&mut fields, &default);
    fields
}

/// The project's game settings (quality, input, sound, display), with
/// defaults from `GameSettings::default()`.
pub fn settings_fields() -> Vec<FieldSchema> {
    let FieldType::Struct { mut fields } = GameSettings::field_type() else {
        return Vec::new();
    };
    let default = serde_json::to_value(GameSettings::default()).unwrap_or(Value::Null);
    fill_field_defaults(&mut fields, &default);
    fields
}

/// The whole registry as the frontend, shell and MCP read it.
pub fn to_json(mode: Mode) -> Value {
    let mut out = json!({
        "components": component_types(mode),
        "world": world_fields(),
        "settings": settings_fields(),
    });
    blank_ids(&mut out);
    out
}

/// A default's freshly minted ids read as blank, so the listing is the same
/// every time; `add-component` mints its own.
fn blank_ids(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let is_id_field = map.get("name").and_then(Value::as_str) == Some("id");
            for (key, v) in map.iter_mut() {
                if (key == "id" || (is_id_field && key == "default")) && v.is_string() {
                    *v = Value::String(String::new());
                } else {
                    blank_ids(v);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(blank_ids),
        _ => {}
    }
}

/// The type at `path` inside a value of type `root`, given the value now.
pub fn type_at<'a>(root: &'a FieldType, path: &[&str], current: &Value) -> Option<&'a FieldType> {
    root.at_path(path, current)
}

/// Splits `a.b.0.c` into its steps. Empty means the whole value.
pub fn split_path(path: &str) -> Vec<&str> {
    path.split('.').filter(|s| !s.is_empty()).collect()
}

/// Checks `value` as the type at `path` in `root`, then writes it into
/// `target` there (making missing objects on the way).
pub fn write_at(
    root: &FieldType,
    target: &mut Value,
    path: &[&str],
    value: Value,
) -> Result<(), String> {
    let joined = path.join(".");
    let ty =
        type_at(root, path, target).ok_or_else(|| format!("nothing called \"{joined}\" to set"))?;
    let errors = ty.check_value(&value);
    if let Some(first) = errors.first() {
        let at = if first.at.is_empty() {
            joined.clone()
        } else {
            format!("{joined}.{}", first.at)
        };
        return Err(format!("{at}: {}", first.message));
    }
    let slot = slot_at(target, path)?;
    // Colors are stored upper case, as every color picker writes them.
    *slot = match (ty, value) {
        (FieldType::Color, Value::String(c)) => Value::String(c.to_uppercase()),
        (_, v) => v,
    };
    Ok(())
}

/// The slot at `path` in `target`, making missing objects on the way. No
/// type check: the caller validates the whole value afterwards.
pub fn slot_at<'a>(target: &'a mut Value, path: &[&str]) -> Result<&'a mut Value, String> {
    let joined = path.join(".");
    let mut slot = target;
    for step in path {
        slot = match slot {
            Value::Array(items) => {
                let i: usize = step
                    .parse()
                    .map_err(|_| format!("\"{step}\" isn't a list index in {joined}"))?;
                items
                    .get_mut(i)
                    .ok_or_else(|| format!("{joined}: no item {i}"))?
            }
            Value::Object(map) => map.entry(step.to_string()).or_insert(Value::Null),
            other => {
                *other = json!({});
                other
                    .as_object_mut()
                    .unwrap()
                    .entry(step.to_string())
                    .or_insert(Value::Null)
            }
        };
    }
    Ok(slot)
}

/// Sets `path` on a component, checked against its schema. The result is
/// the component as it would deserialize, so serde has the last word.
pub fn set_component_property(
    component: &ActorComponent,
    path: &str,
    value: Value,
) -> Result<ActorComponent, String> {
    let name = component.name().to_string();
    let fields = component_fields(&name);
    if fields.is_empty() && !matches!(component, ActorComponent::Persist) {
        return Err(format!("{name} has no settable fields"));
    }
    let root = FieldType::Struct { fields };
    let mut current = serde_json::to_value(component).map_err(|e| e.to_string())?;
    let steps = split_path(path);
    if steps.first() == Some(&"component") {
        return Err("a component's kind can't be changed".into());
    }
    write_at(&root, &mut current, &steps, value)?;
    serde_json::from_value(current).map_err(|e| format!("{name}.{path}: {e}"))
}

/// A path as blocks and scripts spell it, as the component serializes:
/// most components wrap one spec (`{"light": {...}}`), so `intensity`
/// means `light.intensity` there.
pub fn resolve_component_path(component: &ActorComponent, path: &str) -> String {
    match serde_json::to_value(component) {
        Ok(json) => resolve_path_in(&json, path),
        Err(_) => path.to_string(),
    }
}

/// [`resolve_component_path`] over a component already serialized.
pub fn resolve_path_in(component: &Value, path: &str) -> String {
    let Value::Object(map) = component else {
        return path.to_string();
    };
    let mut keys = map.keys().filter(|key| key.as_str() != "component");
    let first = split_path(path).first().copied().unwrap_or_default();
    match (keys.next(), keys.next()) {
        (Some(only), None) if only != first && map[only].is_object() => {
            if path.is_empty() {
                only.clone()
            } else {
                format!("{only}.{path}")
            }
        }
        _ => path.to_string(),
    }
}

/// The value at `path` inside a component, as it serializes. An empty path
/// is the whole component.
pub fn component_property(component: &ActorComponent, path: &str) -> Result<Value, String> {
    let json = serde_json::to_value(component).map_err(|e| e.to_string())?;
    value_at(&json, path).cloned()
}

/// The value at `path` inside serialized JSON, borrowed.
pub fn value_at<'a>(mut current: &'a Value, path: &str) -> Result<&'a Value, String> {
    for step in split_path(path) {
        current = match current {
            Value::Object(map) => map
                .get(step)
                .ok_or_else(|| format!("there is no \"{path}\""))?,
            Value::Array(items) => {
                let i: usize = step
                    .parse()
                    .map_err(|_| format!("\"{step}\" isn't a list index in {path}"))?;
                items.get(i).ok_or_else(|| format!("{path}: no item {i}"))?
            }
            _ => return Err(format!("there is no \"{path}\"")),
        };
    }
    Ok(current)
}

/// A block's value as JSON of the type at `path` in a component: text that
/// reads as a number becomes one for a number field, and so on, so a block
/// can type `0.5` into a text slot.
pub fn coerce_component_value(
    component: &ActorComponent,
    path: &str,
    value: &crate::value::Evaluated,
) -> Value {
    use crate::value::Evaluated;
    let fields = component_fields(component.name());
    let root = FieldType::Struct { fields };
    let current = serde_json::to_value(component).unwrap_or(Value::Null);
    let ty = type_at(&root, &split_path(path), &current);
    match (ty, value) {
        (Some(FieldType::Int { .. }), v) => v
            .as_number()
            .map(|n| Value::from(n.round() as i64))
            .unwrap_or_else(|_| crate::value::json_of(v)),
        (Some(FieldType::Number { .. }), v) => v
            .as_number()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| crate::value::json_of(v)),
        (Some(FieldType::Bool), v) => Value::Bool(v.as_bool()),
        (Some(FieldType::Text { .. } | FieldType::Color | FieldType::Choice { .. }), v) => {
            Value::String(v.as_text())
        }
        // Lists, vectors and structs are written as JSON text.
        (_, Evaluated::Text(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        (_, v) => crate::value::json_of(v),
    }
}

/// Sets `path` on a scene's world settings, checked against their schema.
pub fn set_world_property(world: &World, path: &str, value: Value) -> Result<World, String> {
    let root = FieldType::Struct {
        fields: world_fields(),
    };
    let mut current = serde_json::to_value(world).map_err(|e| e.to_string())?;
    write_at(&root, &mut current, &split_path(path), value)?;
    let mut world: World =
        serde_json::from_value(current).map_err(|e| format!("world.{path}: {e}"))?;
    world.normalize_settings();
    Ok(world)
}

/// Sets `path` on the project's game settings, checked against their schema.
pub fn set_settings_property(
    settings: &GameSettings,
    path: &str,
    value: Value,
) -> Result<GameSettings, String> {
    let root = FieldType::Struct {
        fields: settings_fields(),
    };
    let mut current = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    write_at(&root, &mut current, &split_path(path), value)?;
    let mut settings: GameSettings =
        serde_json::from_value(current).map_err(|e| format!("settings.{path}: {e}"))?;
    settings.normalize();
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque_paths(t: &FieldType, at: String, out: &mut Vec<String>) {
        match t {
            FieldType::Opaque => out.push(at),
            FieldType::Struct { fields } => fields
                .iter()
                .for_each(|f| opaque_paths(&f.ty, format!("{at}.{}", f.name), out)),
            FieldType::List { item, .. }
            | FieldType::Map { value: item }
            | FieldType::Optional { inner: item } => opaque_paths(item, format!("{at}[]"), out),
            FieldType::Enum { variants, .. } => {
                for v in variants {
                    for f in &v.fields {
                        opaque_paths(&f.ty, format!("{at}<{}>.{}", v.name, f.name), out);
                    }
                    if let Some(i) = &v.inner {
                        opaque_paths(i, format!("{at}<{}>", v.name), out);
                    }
                }
            }
            _ => {}
        }
    }

    #[test]
    fn every_component_kind_has_a_default_that_matches_its_schema() {
        for mode in [Mode::TwoD, Mode::ThreeD] {
            for ty in component_types(mode) {
                if ty.special {
                    assert!(ty.default.is_null(), "{}", ty.name);
                    continue;
                }
                assert!(
                    !ty.default.is_null(),
                    "{} has no default in {mode:?}",
                    ty.name
                );
                let schema = ActorComponent::field_type();
                assert!(
                    schema.check_value(&ty.default).is_empty(),
                    "{}: {:?}",
                    ty.name,
                    schema.check_value(&ty.default)
                );
            }
        }
    }

    #[test]
    fn only_known_spots_are_left_undescribed() {
        let mut out = Vec::new();
        opaque_paths(&ActorComponent::field_type(), String::new(), &mut out);
        opaque_paths(&World::field_type(), "world".into(), &mut out);
        // Ids with custom serde, behaviour-tree children (recursive), custom
        // and UI values (any kind), and the plugin record (its plugin's
        // schema describes it).
        let allowed = [
            ".id",
            ".children[]",
            ".fields[].value",
            "<Plugin>.record",
            ".element.value",
            ".keys[].value",
        ];
        let unexpected: Vec<_> = out
            .iter()
            .filter(|p| !allowed.iter().any(|a| p.ends_with(a)))
            .collect();
        assert!(unexpected.is_empty(), "undescribed: {unexpected:?}");
    }

    #[test]
    fn setting_a_light_field_checks_its_type() {
        let light = default_component("Light", Mode::ThreeD).unwrap();
        let brighter = set_component_property(&light, "light.intensity", json!(1600)).unwrap();
        let ActorComponent::Light { light } = &brighter else {
            panic!()
        };
        assert_eq!(light.intensity, 1600.0);
        assert!(set_component_property(&brighter, "light.intensity", json!("bright")).is_err());
        assert!(set_component_property(&brighter, "light.kind", json!("Laser")).is_err());
        assert!(set_component_property(&brighter, "light.nope", json!(1)).is_err());
        assert!(set_component_property(&brighter, "component", json!("Body")).is_err());
    }

    #[test]
    fn setting_a_fog_field_reaches_the_world() {
        let world = World::default();
        let foggy = set_world_property(&world, "fog.height.distance", json!(120)).unwrap();
        assert_eq!(foggy.fog.height.distance, 120.0);
        assert!(set_world_property(&world, "fog.height.day_color", json!("grey")).is_err());
        let fields = world_fields();
        let fog = fields.iter().find(|f| f.name == "fog").unwrap();
        assert!(fog.default.is_some(), "defaults come from Rust");
    }
}
