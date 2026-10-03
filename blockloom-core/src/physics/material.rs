//! Physics materials: how a surface grips and bounces.
//!
//! 3D follows Unity's Physics Material (separate static and dynamic friction,
//! bounciness, and a combine mode for each of friction and bounce). 2D follows
//! Unity's Physics Material 2D (one friction, one bounciness, with fixed combine
//! rules) and is specified separately: the 3D options are not 2D equivalents.

use serde::{Deserialize, Serialize};

use crate::scene::Mode;

/// How two surfaces' values are combined into the one the contact uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CombineMode {
    #[default]
    Average,
    Minimum,
    Multiply,
    Maximum,
}

impl CombineMode {
    /// Priority when the two sides disagree: Maximum > Multiply > Minimum > Average.
    fn priority(self) -> u8 {
        match self {
            CombineMode::Average => 0,
            CombineMode::Minimum => 1,
            CombineMode::Multiply => 2,
            CombineMode::Maximum => 3,
        }
    }

    /// The mode a contact uses for two surfaces that asked for `self` and `other`.
    pub fn resolve(self, other: CombineMode) -> CombineMode {
        if other.priority() > self.priority() {
            other
        } else {
            self
        }
    }

    /// Combines two values under this mode.
    pub fn apply(self, a: f32, b: f32) -> f32 {
        match self {
            CombineMode::Average => (a + b) * 0.5,
            CombineMode::Minimum => a.min(b),
            CombineMode::Multiply => a * b,
            CombineMode::Maximum => a.max(b),
        }
    }
}

/// A 3D surface. Friction and bounciness are unitless coefficients.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsMaterial {
    #[serde(default = "default_friction")]
    pub static_friction: f32,
    #[serde(default = "default_friction")]
    pub dynamic_friction: f32,
    #[serde(default)]
    pub bounciness: f32,
    #[serde(default)]
    pub friction_combine: CombineMode,
    #[serde(default)]
    pub bounce_combine: CombineMode,
}

fn default_friction() -> f32 {
    0.6
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            static_friction: 0.6,
            dynamic_friction: 0.6,
            bounciness: 0.0,
            friction_combine: CombineMode::Average,
            bounce_combine: CombineMode::Average,
        }
    }
}

impl PhysicsMaterial {
    /// The friction and bounce two touching 3D surfaces produce, as
    /// `(static, dynamic, bounce)`. Both modes resolve by priority first, then each
    /// coefficient combines under the winning mode.
    pub fn contact(a: &PhysicsMaterial, b: &PhysicsMaterial) -> (f32, f32, f32) {
        let friction = a.friction_combine.resolve(b.friction_combine);
        let bounce = a.bounce_combine.resolve(b.bounce_combine);
        (
            friction.apply(a.static_friction, b.static_friction),
            friction.apply(a.dynamic_friction, b.dynamic_friction),
            bounce.apply(a.bounciness, b.bounciness),
        )
    }
}

/// A 2D surface. The contact uses the geometric mean of the two frictions and the
/// larger bounciness (Unity 2D's fixed rules).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsMaterial2D {
    #[serde(default = "default_friction_2d")]
    pub friction: f32,
    #[serde(default)]
    pub bounciness: f32,
}

fn default_friction_2d() -> f32 {
    0.4
}

impl Default for PhysicsMaterial2D {
    fn default() -> Self {
        Self {
            friction: 0.4,
            bounciness: 0.0,
        }
    }
}

impl PhysicsMaterial2D {
    /// `(friction, bounce)` for two touching 2D surfaces.
    pub fn contact(a: &PhysicsMaterial2D, b: &PhysicsMaterial2D) -> (f32, f32) {
        (
            (a.friction * b.friction).sqrt(),
            a.bounciness.max(b.bounciness),
        )
    }
}

/// The values a material asset holds, in one dimension.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "dimension")]
pub enum MaterialBody {
    Three { material: PhysicsMaterial },
    Two { material: PhysicsMaterial2D },
}

impl MaterialBody {
    /// Why these values are out of bounds, if they are.
    pub fn validate(&self) -> Result<(), String> {
        use super::meta::{self, MATERIAL};
        let numbers: Vec<(&str, f32)> = match self {
            MaterialBody::Three { material: m } => vec![
                ("static_friction", m.static_friction),
                ("dynamic_friction", m.dynamic_friction),
                ("bounciness", m.bounciness),
            ],
            MaterialBody::Two { material: m } => {
                vec![("friction", m.friction), ("bounciness", m.bounciness)]
            }
        };
        for (name, value) in numbers {
            meta::check(MATERIAL, name, value)?;
        }
        Ok(())
    }

    pub fn mode(&self) -> Mode {
        match self {
            MaterialBody::Three { .. } => Mode::ThreeD,
            MaterialBody::Two { .. } => Mode::TwoD,
        }
    }
}

/// A reusable material in the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialDef {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub body: MaterialBody,
}

/// The materials every project has without storing them.
pub const BUILT_IN_MATERIALS: &[&str] = &["Default", "Ice", "Rubber", "No Bounce"];

/// A built-in material by name. Default follows Unity's defaults; the others are
/// Blockloom presets.
pub fn built_in(name: &str, mode: Mode) -> Option<MaterialBody> {
    use CombineMode::*;
    let three = |sf: f32, df: f32, b: f32, fc: CombineMode, bc: CombineMode| MaterialBody::Three {
        material: PhysicsMaterial {
            static_friction: sf,
            dynamic_friction: df,
            bounciness: b,
            friction_combine: fc,
            bounce_combine: bc,
        },
    };
    let two = |f: f32, b: f32| MaterialBody::Two {
        material: PhysicsMaterial2D {
            friction: f,
            bounciness: b,
        },
    };
    Some(match (name, mode) {
        ("Default", Mode::ThreeD) => three(0.6, 0.6, 0.0, Average, Average),
        ("Ice", Mode::ThreeD) => three(0.05, 0.03, 0.0, Minimum, Average),
        ("Rubber", Mode::ThreeD) => three(1.0, 0.9, 0.7, Maximum, Maximum),
        ("No Bounce", Mode::ThreeD) => three(0.6, 0.6, 0.0, Average, Minimum),
        ("Default", Mode::TwoD) => two(0.4, 0.0),
        ("Ice", Mode::TwoD) => two(0.02, 0.0),
        ("Rubber", Mode::TwoD) => two(1.0, 0.7),
        ("No Bounce", Mode::TwoD) => two(0.4, 0.0),
        _ => return None,
    })
}

/// Where a collider gets its surface from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum MaterialRef {
    /// The built-in Default material.
    #[default]
    Default,
    /// A built-in preset by name (`Ice`, `Rubber`, `No Bounce`).
    BuiltIn { name: String },
    /// A material the project stores, by id.
    Asset { id: String },
}

/// Per-collider overrides laid over the referenced material. `None` keeps the
/// material's value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct MaterialOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_friction: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_friction: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounciness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friction_combine: Option<CombineMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounce_combine: Option<CombineMode>,
}

impl MaterialOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The numeric overrides, by field name, for validation.
    pub fn numbers(&self) -> Vec<(&'static str, f32)> {
        [
            ("static_friction", self.static_friction),
            ("dynamic_friction", self.dynamic_friction),
            ("bounciness", self.bounciness),
        ]
        .into_iter()
        .filter_map(|(name, value)| Some((name, value?)))
        .collect()
    }

    /// `base` with the overrides applied.
    pub fn over(&self, base: PhysicsMaterial) -> PhysicsMaterial {
        PhysicsMaterial {
            static_friction: self.static_friction.unwrap_or(base.static_friction),
            dynamic_friction: self.dynamic_friction.unwrap_or(base.dynamic_friction),
            bounciness: self.bounciness.unwrap_or(base.bounciness),
            friction_combine: self.friction_combine.unwrap_or(base.friction_combine),
            bounce_combine: self.bounce_combine.unwrap_or(base.bounce_combine),
        }
    }
}

/// The project's stored materials plus the lookups that resolve a [`MaterialRef`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MaterialLibrary {
    #[serde(default)]
    pub materials: Vec<MaterialDef>,
}

impl MaterialLibrary {
    pub fn get(&self, id: &str) -> Option<&MaterialDef> {
        self.materials.iter().find(|m| m.id == id)
    }

    /// The material `reference` names in a world of `mode`, or why it can't be found.
    pub fn resolve(&self, reference: &MaterialRef, mode: Mode) -> Result<MaterialBody, String> {
        match reference {
            MaterialRef::Default => Ok(built_in("Default", mode).expect("Default exists")),
            MaterialRef::BuiltIn { name } => built_in(name, mode)
                .ok_or_else(|| format!("no built-in material called \"{name}\"")),
            MaterialRef::Asset { id } => {
                let def = self
                    .get(id)
                    .ok_or_else(|| format!("no material with id \"{id}\""))?;
                if def.body.mode() != mode {
                    return Err(format!(
                        "material \"{}\" is a {} material and this world is {}",
                        def.name,
                        dimension_word(def.body.mode()),
                        dimension_word(mode)
                    ));
                }
                Ok(def.body)
            }
        }
    }

    /// Stores a new material and returns its id.
    pub fn add(&mut self, name: &str, body: MaterialBody) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A material needs a name".to_string());
        }
        if self.materials.iter().any(|m| m.name == name) || BUILT_IN_MATERIALS.contains(&name) {
            return Err(format!("A material called \"{name}\" already exists"));
        }
        let mut n = self.materials.len() + 1;
        let mut id = format!("material-{n}");
        while self.get(&id).is_some() {
            n += 1;
            id = format!("material-{n}");
        }
        self.materials.push(MaterialDef {
            id: id.clone(),
            name: name.to_string(),
            body,
        });
        Ok(id)
    }

    /// Replaces a stored material's name and values; its id stays.
    pub fn update(
        &mut self,
        id: &str,
        name: Option<&str>,
        body: MaterialBody,
    ) -> Result<(), String> {
        if let Some(name) = name {
            let name = name.trim();
            if name.is_empty() {
                return Err("A material needs a name".to_string());
            }
            if self.materials.iter().any(|m| m.name == name && m.id != id)
                || BUILT_IN_MATERIALS.contains(&name)
            {
                return Err(format!("A material called \"{name}\" already exists"));
            }
        }
        let Some(def) = self.materials.iter_mut().find(|m| m.id == id) else {
            return Err(format!("No material with id \"{id}\""));
        };
        if let Some(name) = name {
            def.name = name.trim().to_string();
        }
        def.body = body;
        Ok(())
    }

    /// Forgets a stored material. Callers check that nothing refers to it.
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let Some(index) = self.materials.iter().position(|m| m.id == id) else {
            return Err(format!("No material with id \"{id}\""));
        };
        self.materials.remove(index);
        Ok(())
    }

    /// The id of a stored material with exactly these values, adding one named
    /// `name` when none matches. Migration uses this to keep a body's old
    /// coefficients without one asset per actor.
    pub fn find_or_add(&mut self, name: &str, body: MaterialBody) -> String {
        if let Some(existing) = self.materials.iter().find(|m| m.body == body) {
            return existing.id.clone();
        }
        let mut unique = name.to_string();
        let mut n = 2;
        while self.materials.iter().any(|m| m.name == unique) {
            unique = format!("{name} {n}");
            n += 1;
        }
        let id = format!("material-{}", self.materials.len() + 1);
        let mut id = id;
        while self.get(&id).is_some() {
            id.push('x');
        }
        self.materials.push(MaterialDef {
            id: id.clone(),
            name: unique,
            body,
        });
        id
    }
}

pub(crate) fn dimension_word(mode: Mode) -> &'static str {
    match mode {
        Mode::TwoD => "2D",
        Mode::ThreeD => "3D",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_priority_is_max_multiply_min_average() {
        use CombineMode::*;
        for (a, b, wins) in [
            (Average, Minimum, Minimum),
            (Minimum, Multiply, Multiply),
            (Multiply, Maximum, Maximum),
            (Average, Maximum, Maximum),
            (Maximum, Average, Maximum),
            (Minimum, Minimum, Minimum),
        ] {
            assert_eq!(a.resolve(b), wins, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn contact_combines_each_coefficient_under_the_winning_mode() {
        let rubber = built_in("Rubber", Mode::ThreeD).unwrap();
        let ice = built_in("Ice", Mode::ThreeD).unwrap();
        let (MaterialBody::Three { material: r }, MaterialBody::Three { material: i }) =
            (rubber, ice)
        else {
            panic!("3D presets");
        };
        // Rubber asks for Maximum friction, Ice for Minimum: Maximum wins.
        let (s, d, b) = PhysicsMaterial::contact(&r, &i);
        assert_eq!((s, d), (1.0, 0.9));
        // Rubber's Maximum bounce beats Ice's Average.
        assert_eq!(b, 0.7);
    }

    #[test]
    fn two_d_contact_uses_fixed_rules() {
        let a = PhysicsMaterial2D {
            friction: 0.25,
            bounciness: 0.2,
        };
        let b = PhysicsMaterial2D {
            friction: 1.0,
            bounciness: 0.6,
        };
        assert_eq!(PhysicsMaterial2D::contact(&a, &b), (0.5, 0.6));
    }

    #[test]
    fn references_resolve_by_dimension() {
        let mut lib = MaterialLibrary::default();
        let id = lib.find_or_add(
            "Grippy",
            MaterialBody::Two {
                material: PhysicsMaterial2D {
                    friction: 0.9,
                    bounciness: 0.1,
                },
            },
        );
        assert!(
            lib.resolve(&MaterialRef::Asset { id: id.clone() }, Mode::TwoD)
                .is_ok()
        );
        assert!(
            lib.resolve(&MaterialRef::Asset { id }, Mode::ThreeD)
                .unwrap_err()
                .contains("2D material")
        );
        assert!(
            lib.resolve(&MaterialRef::BuiltIn { name: "Mud".into() }, Mode::ThreeD)
                .is_err()
        );
    }

    #[test]
    fn find_or_add_reuses_equal_materials() {
        let mut lib = MaterialLibrary::default();
        let body = MaterialBody::Three {
            material: PhysicsMaterial::default(),
        };
        let a = lib.find_or_add("Legacy", body);
        let b = lib.find_or_add("Other name", body);
        assert_eq!(a, b);
        assert_eq!(lib.materials.len(), 1);
    }
}
