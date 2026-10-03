//! Property metadata: the units, bounds and visibility of every physics field.
//!
//! One table per component is the single source for validation, the inspector
//! (Phase 6) and the shell's `physics-properties`. A test keeps each table in step
//! with the serialized default of its spec, so a field cannot be added without
//! saying what it is.

use serde::Serialize;

use crate::scene::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Unit {
    None,
    Metre,
    Kilogram,
    KilogramPerCubicMetre,
    MetresPerSecond,
    RadiansPerSecond,
    Degree,
    /// Unitless friction, bounciness or similar coefficient.
    Coefficient,
    /// One over seconds.
    PerSecond,
    Count,
}

/// Which worlds show and accept a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Worlds {
    Both,
    ThreeD,
    TwoD,
}

impl Worlds {
    pub fn includes(self, mode: Mode) -> bool {
        matches!(
            (self, mode),
            (Worlds::Both, _) | (Worlds::ThreeD, Mode::ThreeD) | (Worlds::TwoD, Mode::TwoD)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Property {
    /// Field path in the saved spec, dotted for nested values.
    pub name: &'static str,
    pub label: &'static str,
    pub unit: Unit,
    pub min: Option<f32>,
    /// True when `min` itself is not allowed (mass must be above zero).
    pub min_exclusive: bool,
    pub max: Option<f32>,
    pub worlds: Worlds,
    /// Hidden behind the collapsed Advanced section.
    pub advanced: bool,
    /// True when Unity has a property of this meaning; false for Blockloom's own.
    pub unity: bool,
}

const fn p(name: &'static str, label: &'static str, unit: Unit) -> Property {
    Property {
        name,
        label,
        unit,
        min: None,
        min_exclusive: false,
        max: None,
        worlds: Worlds::Both,
        advanced: false,
        unity: true,
    }
}

impl Property {
    const fn at_least(mut self, min: f32) -> Self {
        self.min = Some(min);
        self
    }

    const fn above(mut self, min: f32) -> Self {
        self.min = Some(min);
        self.min_exclusive = true;
        self
    }

    const fn at_most(mut self, max: f32) -> Self {
        self.max = Some(max);
        self
    }

    const fn only(mut self, worlds: Worlds) -> Self {
        self.worlds = worlds;
        self
    }

    const fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }

    const fn blockloom(mut self) -> Self {
        self.unity = false;
        self
    }

    /// Why `value` is out of bounds, if it is.
    pub fn check(&self, value: f32) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!("{} must be a finite number", self.label));
        }
        if let Some(min) = self.min {
            if self.min_exclusive && value <= min {
                return Err(format!("{} must be above {min}", self.label));
            }
            if !self.min_exclusive && value < min {
                return Err(format!("{} must be at least {min}", self.label));
            }
        }
        if let Some(max) = self.max
            && value > max
        {
            return Err(format!("{} must be at most {max}", self.label));
        }
        Ok(())
    }
}

pub const RIGIDBODY: &[Property] = &[
    p("body_type", "Body type", Unit::None),
    p("simulated", "Simulated", Unit::None).only(Worlds::TwoD),
    p("mass.mass", "Mass", Unit::Kilogram).above(0.0),
    p("mass.density", "Density", Unit::KilogramPerCubicMetre)
        .above(0.0)
        .blockloom(),
    p("center_of_mass", "Center of mass", Unit::Metre).advanced(),
    p("inertia", "Inertia", Unit::None).above(0.0).advanced(),
    p("linear_damping", "Linear damping", Unit::PerSecond).at_least(0.0),
    p("angular_damping", "Angular damping", Unit::PerSecond).at_least(0.0),
    p("use_gravity", "Use gravity", Unit::None),
    p("gravity_scale", "Gravity scale", Unit::None).blockloom(),
    p("interpolation", "Interpolation", Unit::None),
    p("collision_detection", "Collision detection", Unit::None),
    p("constraints", "Constraints", Unit::None),
    p(
        "max_linear_velocity",
        "Max linear velocity",
        Unit::MetresPerSecond,
    )
    .above(0.0)
    .advanced(),
    p(
        "max_angular_velocity",
        "Max angular velocity",
        Unit::RadiansPerSecond,
    )
    .above(0.0)
    .advanced(),
    p("sleep_threshold", "Sleep threshold", Unit::None)
        .at_least(0.0)
        .advanced(),
    p("solver_iterations", "Solver iterations", Unit::Count)
        .at_least(1.0)
        .at_most(255.0)
        .advanced(),
    p(
        "max_depenetration_velocity",
        "Max depenetration velocity",
        Unit::MetresPerSecond,
    )
    .at_least(0.0)
    .advanced(),
    p(
        "legacy_character_control",
        "Legacy character control",
        Unit::None,
    )
    .advanced()
    .blockloom(),
];

pub const COLLIDER: &[Property] = &[
    p("name", "Name", Unit::None).blockloom(),
    p("enabled", "Enabled", Unit::None),
    p("geometry", "Shape", Unit::None),
    p("center", "Center", Unit::Metre),
    p("rotation", "Rotation", Unit::Degree),
    p("material", "Material", Unit::None),
    p("material_overrides", "Material overrides", Unit::None)
        .advanced()
        .blockloom(),
    p("trigger", "Is trigger", Unit::None),
    p("layer", "Layer", Unit::Count).at_least(1.0).at_most(32.0),
    p("layer_overrides", "Layer overrides", Unit::None).advanced(),
    p("legacy_mask", "Legacy collision mask", Unit::None)
        .advanced()
        .blockloom(),
    p("contact_offset", "Contact offset", Unit::Metre)
        .at_least(0.0)
        .advanced(),
    p("queryable", "Visible to queries", Unit::None)
        .advanced()
        .blockloom(),
    p("one_way", "One way", Unit::None).only(Worlds::TwoD),
];

/// Bounds for a material's numbers, by field name.
pub const MATERIAL: &[Property] = &[
    p("static_friction", "Static friction", Unit::Coefficient).at_least(0.0),
    p("dynamic_friction", "Dynamic friction", Unit::Coefficient).at_least(0.0),
    p("friction", "Friction", Unit::Coefficient)
        .at_least(0.0)
        .only(Worlds::TwoD),
    p("bounciness", "Bounciness", Unit::Coefficient)
        .at_least(0.0)
        .at_most(1.0),
    p("friction_combine", "Friction combine", Unit::None).only(Worlds::ThreeD),
    p("bounce_combine", "Bounce combine", Unit::None).only(Worlds::ThreeD),
];

/// The property called `name` in `table`.
pub fn find(table: &'static [Property], name: &str) -> Option<&'static Property> {
    table.iter().find(|property| property.name == name)
}

/// Checks `value` against the bounds `table` gives `name`, or accepts it when the
/// table has none.
pub fn check(table: &'static [Property], name: &str, value: f32) -> Result<(), String> {
    match find(table, name) {
        Some(property) => property.check(value),
        None if value.is_finite() => Ok(()),
        None => Err(format!("{name} must be a finite number")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::spec::{ColliderSpec, RigidbodySpec};
    use super::*;

    fn top_level_keys(value: serde_json::Value) -> Vec<String> {
        value
            .as_object()
            .expect("an object")
            .keys()
            .filter(|key| key.as_str() != "id")
            .cloned()
            .collect()
    }

    /// Every saved field has a row, and every row names a saved field (or a
    /// nested one under a saved field), so the two cannot drift.
    fn same_fields(table: &[Property], saved: Vec<String>, optional: &[&str]) {
        for key in &saved {
            assert!(
                table
                    .iter()
                    .any(|p| p.name == key || p.name.starts_with(&format!("{key}."))),
                "{key} has no metadata row"
            );
        }
        for row in table {
            let root = row.name.split('.').next().unwrap();
            assert!(
                saved.iter().any(|key| key == root) || optional.contains(&root),
                "{} names no saved field",
                row.name
            );
        }
    }

    #[test]
    fn rigidbody_table_matches_the_spec() {
        let saved = top_level_keys(serde_json::to_value(RigidbodySpec::default()).unwrap());
        same_fields(
            RIGIDBODY,
            saved,
            &[
                "center_of_mass",
                "inertia",
                "solver_iterations",
                "legacy_character_control",
            ],
        );
    }

    #[test]
    fn collider_table_matches_the_spec() {
        let saved = top_level_keys(serde_json::to_value(ColliderSpec::default()).unwrap());
        same_fields(
            COLLIDER,
            saved,
            &[
                "material_overrides",
                "layer_overrides",
                "legacy_mask",
                "contact_offset",
            ],
        );
    }

    #[test]
    fn bounds_reject_what_they_should() {
        let mass = find(RIGIDBODY, "mass.mass").unwrap();
        assert!(mass.check(1.0).is_ok());
        assert!(mass.check(0.0).is_err());
        assert!(mass.check(f32::NAN).is_err());
        assert!(mass.check(f32::INFINITY).is_err());
        let layer = find(COLLIDER, "layer").unwrap();
        assert!(layer.check(32.0).is_ok());
        assert!(layer.check(33.0).is_err());
        assert!(layer.check(0.0).is_err());
    }
}
