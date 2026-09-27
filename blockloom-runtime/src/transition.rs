//! Wall-clock scene transitions: the veil over a scene switch.
//!
//! A `switch scene to ... with transition ...` names `none`, `fade`, `wipe`
//! or `circle` (anything else reads as `none`, the way the VM normalizes
//! it). `none` keeps the old immediate cut. The rest cover the outgoing
//! scene on the wall clock, hold while the new one rebuilds and warms up,
//! then reveal it - so a switch never flashes half-loaded geometry.
//!
//! `SceneVeil` is plain state on [`crate::engine::Engine`]: the fixed step
//! starts it and waits for cover before swapping scenes, and `drive_veil`
//! (per-frame `Update`, on [`bevy::time::Time<Real>`] so pause never freezes
//! it) draws one fullscreen Bevy UI node over everything, including the
//! interface root. `wipe` sweeps a panel left to right; `circle` reads as a
//! fade until it gets its own mask.

use bevy::prelude::*;

/// How long the veil takes to cover the old scene.
pub const OUT_SECS: f32 = 0.25;
/// How long the new scene takes to fade back in once warm.
pub const IN_SECS: f32 = 0.3;

/// Which transition the veil is drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VeilKind {
    #[default]
    None,
    Fade,
    Wipe,
    Circle,
}

/// Where the veil is in its out-switch-in arc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VeilPhase {
    /// No transition running; the driver shows nothing.
    #[default]
    Idle,
    /// Covering the outgoing scene; `t` runs 0 to 1.
    Out,
    /// Fully covered; the fixed step swaps scenes, then reveals.
    Covered,
    /// Revealing the new scene; `t` runs 1 to 0.
    In,
}

/// The veil's state. `t` is cover: 0 clear, 1 opaque.
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneVeil {
    pub kind: VeilKind,
    pub phase: VeilPhase,
    pub t: f32,
}

/// The block's spelling into a veil kind. Unknown spellings read as `none`,
/// the same rule the VM's `normalize_transition` uses.
pub fn normalize_kind(name: &str) -> VeilKind {
    let key: String = name
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    match key.as_str() {
        "fade" => VeilKind::Fade,
        "wipe" => VeilKind::Wipe,
        "circle" => VeilKind::Circle,
        _ => VeilKind::None,
    }
}

impl SceneVeil {
    /// Starts covering for `transition`. Answers whether anything animated:
    /// `none` (and typos) leaves the veil idle and the switch cuts at once.
    pub fn start(&mut self, transition: &str) -> bool {
        match normalize_kind(transition) {
            VeilKind::None => {
                *self = SceneVeil::default();
                false
            }
            kind => {
                *self = SceneVeil {
                    kind,
                    phase: VeilPhase::Out,
                    t: 0.0,
                };
                true
            }
        }
    }

    /// Steps cover toward its target. Returns true on arrival at cover.
    /// `Covered` and `Idle` never move here; the fixed step moves the first
    /// (by swapping scenes) and nothing moves the second.
    pub fn advance_out(&mut self, dt: f32) -> bool {
        if self.phase != VeilPhase::Out {
            return self.phase == VeilPhase::Covered;
        }
        self.t = (self.t + dt / OUT_SECS).min(1.0);
        if self.t >= 1.0 {
            self.phase = VeilPhase::Covered;
            true
        } else {
            false
        }
    }

    /// Steps the reveal toward clear. Returns true when fully clear.
    pub fn advance_in(&mut self, dt: f32) -> bool {
        if self.phase != VeilPhase::In {
            return self.phase == VeilPhase::Idle;
        }
        self.t = (self.t - dt / IN_SECS).max(0.0);
        if self.t <= 0.0 {
            *self = SceneVeil::default();
            true
        } else {
            false
        }
    }

    /// The fixed step may swap scenes once cover is complete - or at once
    /// when no transition runs.
    pub fn ready_to_switch(&self) -> bool {
        match self.phase {
            VeilPhase::Idle => true,
            VeilPhase::Covered => true,
            _ => false,
        }
    }

    /// Opens the reveal after the swap. A `none` switch never covered, so
    /// there is nothing to open.
    pub fn begin_reveal(&mut self) {
        if self.phase == VeilPhase::Covered {
            self.phase = VeilPhase::In;
            self.t = 1.0;
        }
    }

    /// Drops any transition: editor loads, stops and failed switches cut
    /// straight to the world underneath.
    pub fn reset(&mut self) {
        *self = SceneVeil::default();
    }

    pub fn running(&self) -> bool {
        self.phase != VeilPhase::Idle
    }
}

/// Marker on the fullscreen veil node the driver owns.
#[derive(Component)]
pub(crate) struct VeilNode;

/// Draws [`SceneVeil`] as one fullscreen node over everything (the interface
/// root sits at z 40; the veil takes 50). Runs on the real clock so a
/// transition outlives pause, and holds the reveal while the new scene's
/// warmup window is still open so the cut lands on settled frames.
pub fn drive_veil(
    mut commands: Commands,
    mut engine: NonSendMut<crate::engine::Engine>,
    real: Res<Time<Real>>,
    warmup: Res<crate::streaming::Warmup>,
    mut nodes: Query<(Entity, &mut Node, &mut BackgroundColor), With<VeilNode>>,
) {
    if !engine.veil.running() {
        for (entity, _, _) in &nodes {
            commands.entity(entity).despawn();
        }
        return;
    }
    let dt = real.delta_secs().max(0.0);
    match engine.veil.phase {
        VeilPhase::Out => {
            engine.veil.advance_out(dt);
        }
        VeilPhase::In => {
            // The new scene rebuilds under cover; revealing early would
            // flash pipelines still compiling.
            if !warmup.active() {
                engine.veil.advance_in(dt);
            }
        }
        VeilPhase::Covered | VeilPhase::Idle => {}
    }
    if !engine.veil.running() {
        for (entity, _, _) in &nodes {
            commands.entity(entity).despawn();
        }
        return;
    }
    let (kind, phase, t) = (engine.veil.kind, engine.veil.phase, engine.veil.t);
    if let Some((_, mut node, mut color)) = nodes.iter_mut().next() {
        style_veil(&mut node, &mut color, kind, phase, t);
        // One veil only; a second node is a leftover from a reset race.
        for (entity, _, _) in nodes.iter().skip(1) {
            commands.entity(entity).despawn();
        }
    } else {
        let (node, color) = veil_bundle(kind, phase, t);
        commands.spawn((
            Name::new("scene-veil"),
            VeilNode,
            node,
            color,
            GlobalZIndex(50),
        ));
    }
}

fn veil_bundle(kind: VeilKind, phase: VeilPhase, t: f32) -> (Node, BackgroundColor) {
    let mut node = Node::default();
    let mut color = BackgroundColor(Color::BLACK);
    style_veil(&mut node, &mut color, kind, phase, t);
    (node, color)
}

/// A fade (and, until it gets its own mask, a circle) is a fullscreen wash
/// whose alpha is the cover. A wipe is an opaque panel sweeping left to
/// right: it grows from the left edge while covering, then shrinks toward
/// the right edge while revealing.
fn style_veil(
    node: &mut Node,
    color: &mut BackgroundColor,
    kind: VeilKind,
    phase: VeilPhase,
    t: f32,
) {
    let t = t.clamp(0.0, 1.0);
    match kind {
        VeilKind::Wipe => {
            node.position_type = PositionType::Absolute;
            node.top = Val::Percent(0.0);
            node.height = Val::Percent(100.0);
            node.width = Val::Percent(t * 100.0);
            if phase == VeilPhase::In {
                node.left = Val::Auto;
                node.right = Val::Percent(0.0);
            } else {
                node.left = Val::Percent(0.0);
                node.right = Val::Auto;
            }
            color.0 = Color::BLACK;
        }
        VeilKind::Fade | VeilKind::Circle | VeilKind::None => {
            node.position_type = PositionType::Absolute;
            node.left = Val::Percent(0.0);
            node.right = Val::Percent(0.0);
            node.top = Val::Percent(0.0);
            node.height = Val::Percent(100.0);
            node.width = Val::Percent(100.0);
            color.0 = Color::BLACK.with_alpha(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_transition_reads_as_none_and_cuts_at_once() {
        let mut veil = SceneVeil::default();
        assert!(!veil.start("dissolve"));
        assert_eq!(veil.phase, VeilPhase::Idle);
        assert!(veil.ready_to_switch());
    }

    #[test]
    fn a_fade_covers_then_reveals_on_the_wall_clock() {
        let mut veil = SceneVeil::default();
        assert!(veil.start("fade"));
        assert!(!veil.ready_to_switch());
        // Cover takes OUT_SECS wall seconds however the fixed tick runs.
        assert!(!veil.advance_out(OUT_SECS * 0.5));
        assert!(veil.advance_out(OUT_SECS * 0.5));
        assert_eq!(veil.phase, VeilPhase::Covered);
        assert!(veil.ready_to_switch());
        veil.begin_reveal();
        assert_eq!(veil.phase, VeilPhase::In);
        assert!(!veil.advance_in(IN_SECS * 0.9));
        assert!(veil.advance_in(IN_SECS * 0.2));
        assert_eq!(veil.phase, VeilPhase::Idle);
    }

    #[test]
    fn a_wipe_starts_and_a_reset_clears_it() {
        let mut veil = SceneVeil::default();
        assert!(veil.start("wipe"));
        assert_eq!(veil.kind, VeilKind::Wipe);
        veil.reset();
        assert!(!veil.running());
        assert!(veil.ready_to_switch());
    }
}
