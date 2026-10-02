//! Collision layers: 32 named slots, a symmetric matrix per dimension and the
//! per-collider overrides laid over it.
//!
//! A pair of colliders collides when [`pair_collides`] says so. Rapier can only test
//! two 32-bit group masks, so [`ColliderFilter::groups`] encodes the common case
//! (the matrix plus excludes and legacy masks) exactly into masks, and a project
//! that uses an include override or a priority gets the exact test from a physics
//! hook instead (see [`needs_exact`]).

use serde::{Deserialize, Serialize};

use super::spec::{LAYER_SLOTS, LayerOverrides};
use crate::scene::Mode;

/// The bit a layer (1 to 32) occupies in a mask.
pub fn layer_bit(layer: u8) -> u32 {
    1u32 << (layer.clamp(1, LAYER_SLOTS) - 1)
}

/// Layer names and the pairs that do not collide, per dimension. Everything
/// collides with everything until a pair is switched off, and a project that never
/// touches layers writes nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LayerSettings {
    /// Names by slot (index 0 is layer 1); missing or empty names read as "Layer N".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// 3D layer pairs that do not collide, each `[a, b]` with `a <= b`, 1 based.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_3d: Vec<[u8; 2]>,
    /// The same for 2D.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_2d: Vec<[u8; 2]>,
}

impl LayerSettings {
    fn disabled(&self, mode: Mode) -> &Vec<[u8; 2]> {
        match mode {
            Mode::ThreeD => &self.disabled_3d,
            Mode::TwoD => &self.disabled_2d,
        }
    }

    fn disabled_mut(&mut self, mode: Mode) -> &mut Vec<[u8; 2]> {
        match mode {
            Mode::ThreeD => &mut self.disabled_3d,
            Mode::TwoD => &mut self.disabled_2d,
        }
    }

    /// The display name of a layer.
    pub fn name(&self, layer: u8) -> String {
        self.names
            .get(usize::from(layer.clamp(1, LAYER_SLOTS)) - 1)
            .filter(|name| !name.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Layer {layer}"))
    }

    /// Names a layer; an empty name goes back to the default.
    pub fn set_name(&mut self, layer: u8, name: &str) -> Result<(), String> {
        check_layer(layer)?;
        let index = usize::from(layer) - 1;
        if self.names.len() <= index {
            self.names.resize(index + 1, String::new());
        }
        self.names[index] = name.trim().to_string();
        while self.names.last().is_some_and(String::is_empty) {
            self.names.pop();
        }
        Ok(())
    }

    /// Whether layers `a` and `b` collide in `mode` before any override.
    pub fn collides(&self, mode: Mode, a: u8, b: u8) -> bool {
        let pair = ordered(a, b);
        !self.disabled(mode).contains(&pair)
    }

    /// Switches a pair on or off. The matrix is symmetric, so one call covers both
    /// orders.
    pub fn set_collides(&mut self, mode: Mode, a: u8, b: u8, on: bool) -> Result<(), String> {
        check_layer(a)?;
        check_layer(b)?;
        let pair = ordered(a, b);
        let list = self.disabled_mut(mode);
        list.retain(|existing| *existing != pair);
        if !on {
            list.push(pair);
            list.sort_unstable();
        }
        Ok(())
    }

    /// The layers `layer` collides with, as a mask.
    pub fn row_mask(&self, mode: Mode, layer: u8) -> u32 {
        let mut mask = u32::MAX;
        for [a, b] in self.disabled(mode) {
            if *a == layer {
                mask &= !layer_bit(*b);
            }
            if *b == layer {
                mask &= !layer_bit(*a);
            }
        }
        mask
    }

    /// Every listed pair names real layers.
    pub fn validate(&self) -> Result<(), String> {
        if self.names.len() > usize::from(LAYER_SLOTS) {
            return Err(format!("A project has {LAYER_SLOTS} layers at most"));
        }
        for [a, b] in self.disabled_3d.iter().chain(&self.disabled_2d) {
            check_layer(*a)?;
            check_layer(*b)?;
            if a > b {
                return Err(format!(
                    "Layer pair [{a}, {b}] must list the lower layer first"
                ));
            }
        }
        Ok(())
    }
}

fn ordered(a: u8, b: u8) -> [u8; 2] {
    if a <= b { [a, b] } else { [b, a] }
}

fn check_layer(layer: u8) -> Result<(), String> {
    if (1..=LAYER_SLOTS).contains(&layer) {
        Ok(())
    } else {
        Err(format!(
            "Layer {layer} doesn't exist; layers are 1 to {LAYER_SLOTS}"
        ))
    }
}

/// What a collider contributes to pair filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ColliderFilter {
    pub layer: u8,
    pub overrides: LayerOverrides,
    /// A migrated body's old per-object mask: the only layers it accepts.
    pub legacy_mask: Option<u32>,
}

/// Rapier's membership and filter masks for one collider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GroupBits {
    pub memberships: u32,
    pub filter: u32,
}

impl ColliderFilter {
    /// What this collider's own overrides say about colliding with `other`, if
    /// anything: `Some(true)` for an include, `Some(false)` for an exclude. An
    /// exclude beats an include on the same collider.
    fn verdict(&self, other: u8) -> Option<bool> {
        let bit = layer_bit(other);
        if self.overrides.exclude & bit != 0 {
            Some(false)
        } else if self.overrides.include & bit != 0 {
            Some(true)
        } else {
            None
        }
    }

    /// The masks that reproduce [`pair_collides`] when no include or priority is in
    /// play. With `exact` the filter accepts everything and a hook decides.
    pub fn groups(&self, layers: &LayerSettings, mode: Mode, exact: bool) -> GroupBits {
        let memberships = layer_bit(self.layer);
        let filter = if exact {
            u32::MAX
        } else {
            let accepted = (layers.row_mask(mode, self.layer) | self.overrides.include)
                & !self.overrides.exclude;
            accepted & self.legacy_mask.unwrap_or(u32::MAX)
        };
        GroupBits {
            memberships,
            filter,
        }
    }
}

/// Whether two colliders collide. Their overrides decide first (an exclude beats an
/// include on one collider; between the two colliders the higher priority wins and
/// a tie excludes), then the project matrix, and finally each side's legacy mask
/// must accept the other's layer.
pub fn pair_collides(
    a: &ColliderFilter,
    b: &ColliderFilter,
    layers: &LayerSettings,
    mode: Mode,
) -> bool {
    let decided = match (a.verdict(b.layer), b.verdict(a.layer)) {
        (Some(x), Some(y)) if x == y => Some(x),
        (Some(x), Some(y)) => {
            use std::cmp::Ordering::*;
            match a.overrides.priority.cmp(&b.overrides.priority) {
                Greater => Some(x),
                Less => Some(y),
                Equal => Some(false),
            }
        }
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    };
    let by_layers = decided.unwrap_or_else(|| layers.collides(mode, a.layer, b.layer));
    by_layers
        && a.legacy_mask
            .is_none_or(|mask| mask & layer_bit(b.layer) != 0)
        && b.legacy_mask
            .is_none_or(|mask| mask & layer_bit(a.layer) != 0)
}

/// Whether any collider needs the exact test: an include widens past the matrix and a
/// priority picks between disagreeing sides, neither of which two masks can say.
pub fn needs_exact<'a>(filters: impl IntoIterator<Item = &'a ColliderFilter>) -> bool {
    filters.into_iter().any(|f| {
        f.overrides.include != 0 || (f.overrides.exclude != 0 && f.overrides.priority != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(layer: u8) -> ColliderFilter {
        ColliderFilter {
            layer,
            overrides: LayerOverrides::default(),
            legacy_mask: None,
        }
    }

    /// Rapier's own test: both sides must accept the other's membership.
    fn by_groups(
        a: &ColliderFilter,
        b: &ColliderFilter,
        layers: &LayerSettings,
        mode: Mode,
    ) -> bool {
        let (ga, gb) = (a.groups(layers, mode, false), b.groups(layers, mode, false));
        ga.memberships & gb.filter != 0 && gb.memberships & ga.filter != 0
    }

    #[test]
    fn everything_collides_until_a_pair_is_switched_off() {
        let mut layers = LayerSettings::default();
        assert!(layers.collides(Mode::ThreeD, 3, 7));
        layers.set_collides(Mode::ThreeD, 7, 3, false).unwrap();
        assert!(!layers.collides(Mode::ThreeD, 3, 7));
        assert!(
            !layers.collides(Mode::ThreeD, 7, 3),
            "the matrix is symmetric"
        );
        assert!(layers.collides(Mode::TwoD, 3, 7), "2D has its own matrix");
        assert_eq!(layers.disabled_3d, vec![[3, 7]]);
        layers.set_collides(Mode::ThreeD, 3, 7, true).unwrap();
        assert_eq!(layers, LayerSettings::default());
    }

    #[test]
    fn layers_outside_the_slots_are_refused() {
        let mut layers = LayerSettings::default();
        assert!(layers.set_collides(Mode::ThreeD, 0, 1, false).is_err());
        assert!(layers.set_collides(Mode::ThreeD, 1, 33, false).is_err());
        assert!(layers.set_name(40, "x").is_err());
        let mut broken = LayerSettings::default();
        broken.disabled_3d.push([5, 2]);
        assert!(broken.validate().is_err());
    }

    #[test]
    fn names_fall_back_and_trailing_blanks_are_trimmed() {
        let mut layers = LayerSettings::default();
        assert_eq!(layers.name(4), "Layer 4");
        layers.set_name(4, " Enemies ").unwrap();
        assert_eq!(layers.name(4), "Enemies");
        layers.set_name(4, "").unwrap();
        assert!(layers.names.is_empty());
    }

    #[test]
    fn the_masks_agree_with_the_pair_rule_without_includes() {
        let mut layers = LayerSettings::default();
        layers.set_collides(Mode::ThreeD, 2, 3, false).unwrap();
        let mut excluding = filter(5);
        excluding.overrides.exclude = layer_bit(1);
        let mut legacy = filter(1);
        legacy.legacy_mask = Some(layer_bit(2) | layer_bit(5));
        let colliders = [filter(1), filter(2), filter(3), excluding, legacy];
        assert!(!needs_exact(&colliders));
        for a in &colliders {
            for b in &colliders {
                assert_eq!(
                    pair_collides(a, b, &layers, Mode::ThreeD),
                    by_groups(a, b, &layers, Mode::ThreeD),
                    "{a:?} vs {b:?}"
                );
            }
        }
        assert!(!pair_collides(
            &filter(2),
            &filter(3),
            &layers,
            Mode::ThreeD
        ));
        assert!(!pair_collides(
            &excluding,
            &filter(1),
            &layers,
            Mode::ThreeD
        ));
    }

    #[test]
    fn an_include_beats_the_matrix_from_either_side() {
        let mut layers = LayerSettings::default();
        layers.set_collides(Mode::ThreeD, 2, 3, false).unwrap();
        let mut includer = filter(2);
        includer.overrides.include = layer_bit(3);
        assert!(needs_exact([&includer]));
        assert!(pair_collides(&includer, &filter(3), &layers, Mode::ThreeD));
        assert!(pair_collides(&filter(3), &includer, &layers, Mode::ThreeD));
        // Exact mode lets everything through the masks and leaves the decision to the hook.
        assert_eq!(
            includer.groups(&layers, Mode::ThreeD, true).filter,
            u32::MAX
        );
    }

    #[test]
    fn an_exclude_on_either_side_wins_a_tie_and_priority_breaks_it() {
        let layers = LayerSettings::default();
        let mut excluder = filter(1);
        excluder.overrides.exclude = layer_bit(2);
        let mut includer = filter(2);
        includer.overrides.include = layer_bit(1);
        assert!(!pair_collides(&excluder, &includer, &layers, Mode::ThreeD));
        includer.overrides.priority = 5;
        assert!(pair_collides(&excluder, &includer, &layers, Mode::ThreeD));
        excluder.overrides.priority = 9;
        assert!(!pair_collides(&excluder, &includer, &layers, Mode::ThreeD));
        // The same collider excluding and including a layer excludes it.
        let mut both = filter(4);
        both.overrides.exclude = layer_bit(6);
        both.overrides.include = layer_bit(6);
        assert!(!pair_collides(&both, &filter(6), &layers, Mode::ThreeD));
    }

    #[test]
    fn a_legacy_mask_limits_both_directions() {
        let layers = LayerSettings::default();
        let mut picky = filter(1);
        picky.legacy_mask = Some(layer_bit(2));
        assert!(pair_collides(&picky, &filter(2), &layers, Mode::TwoD));
        assert!(!pair_collides(&picky, &filter(3), &layers, Mode::TwoD));
        assert!(!pair_collides(&filter(3), &picky, &layers, Mode::TwoD));
    }
}
