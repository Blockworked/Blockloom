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
//! it) draws Bevy UI nodes over everything, including the interface root.
//! `fade` is a fullscreen wash, `wipe` sweeps a panel left to right, and
//! `circle` is an iris: a centered black disc growing to cover and shrinking
//! to reveal.

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
        matches!(self.phase, VeilPhase::Idle | VeilPhase::Covered)
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

/// Marker on the fullscreen veil node the driver owns. For `circle` this is
/// the transparent fullscreen root; the black disc itself is [`VeilCircle`].
#[derive(Component)]
pub(crate) struct VeilNode;

/// Marker on the black iris disc a `circle` transition draws.
#[derive(Component)]
pub(crate) struct VeilCircle;

/// How big the iris gets when fully covered, in vmin (percent of the smaller
/// viewport side). 400 covers even a 32:9 screen's corners from the center.
pub const CIRCLE_COVER_VMIN: f32 = 400.0;

/// Draws [`SceneVeil`] over everything (the interface root sits at z 40; the
/// veil takes 50). Runs on the real clock so a transition outlives pause, and
/// holds the reveal while the new scene's warmup window is still open so the
/// cut lands on settled frames. Fade and wipe are one fullscreen node;
/// circle is a transparent root with a centered black disc child.
pub fn drive_veil(
    mut commands: Commands,
    mut engine: NonSendMut<crate::engine::Engine>,
    real: Res<Time<Real>>,
    warmup: Res<crate::streaming::Warmup>,
    mut nodes: Query<
        (Entity, &mut Node, &mut BackgroundColor),
        (With<VeilNode>, Without<VeilCircle>),
    >,
    mut discs: Query<
        (Entity, &mut Node, &mut BackgroundColor),
        (With<VeilCircle>, Without<VeilNode>),
    >,
) {
    if !engine.veil.running() {
        for (entity, _, _) in &nodes {
            commands.entity(entity).despawn();
        }
        for (entity, _, _) in &discs {
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
        for (entity, _, _) in &discs {
            commands.entity(entity).despawn();
        }
        return;
    }
    let (kind, phase, t) = (engine.veil.kind, engine.veil.phase, engine.veil.t);
    if kind == VeilKind::Circle {
        drive_circle(&mut commands, &mut nodes, &mut discs, t);
        // A stale wash from a previous fade/wipe must not linger under the
        // disc; the disc path owns no wash, so drop extras.
        return;
    }
    // Fade/wipe own no disc; a stale one from an interrupted circle goes.
    for (entity, _, _) in &discs {
        commands.entity(entity).despawn();
    }
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

/// Draws the circle iris: a transparent fullscreen root centering a black
/// disc whose diameter is the cover. Out grows it to cover, in shrinks it to
/// reveal from the edges inward.
fn drive_circle(
    commands: &mut Commands,
    nodes: &mut Query<
        (Entity, &mut Node, &mut BackgroundColor),
        (With<VeilNode>, Without<VeilCircle>),
    >,
    discs: &mut Query<
        (Entity, &mut Node, &mut BackgroundColor),
        (With<VeilCircle>, Without<VeilNode>),
    >,
    t: f32,
) {
    let t = t.clamp(0.0, 1.0);
    let root = if let Some((_, mut node, mut color)) = nodes.iter_mut().next() {
        node.position_type = PositionType::Absolute;
        node.left = Val::Percent(0.0);
        node.right = Val::Percent(0.0);
        node.top = Val::Percent(0.0);
        node.bottom = Val::Percent(0.0);
        node.width = Val::Percent(100.0);
        node.height = Val::Percent(100.0);
        node.display = Display::Flex;
        node.justify_content = JustifyContent::Center;
        node.align_items = AlignItems::Center;
        color.0 = Color::BLACK.with_alpha(0.0);
        for (entity, _, _) in nodes.iter().skip(1) {
            commands.entity(entity).despawn();
        }
        None
    } else {
        let node = Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(0.0),
            right: Val::Percent(0.0),
            top: Val::Percent(0.0),
            bottom: Val::Percent(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            display: Display::Flex,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        };
        Some(
            commands
                .spawn((
                    Name::new("scene-veil"),
                    VeilNode,
                    node,
                    BackgroundColor(Color::BLACK.with_alpha(0.0)),
                    GlobalZIndex(50),
                ))
                .id(),
        )
    };
    let root_entity = root.or_else(|| nodes.iter().next().map(|(entity, _, _)| entity));
    let diameter = Val::VMin(t * CIRCLE_COVER_VMIN);
    if let Some((_, mut node, mut color)) = discs.iter_mut().next() {
        node.width = diameter;
        node.height = diameter;
        node.border_radius = BorderRadius::all(Val::Percent(50.0));
        color.0 = Color::BLACK;
        for (entity, _, _) in discs.iter().skip(1) {
            commands.entity(entity).despawn();
        }
    } else {
        let node = Node {
            width: diameter,
            height: diameter,
            border_radius: BorderRadius::all(Val::Percent(50.0)),
            ..default()
        };
        let disc = commands
            .spawn((
                Name::new("scene-veil-disc"),
                VeilCircle,
                node,
                BackgroundColor(Color::BLACK),
                GlobalZIndex(50),
            ))
            .id();
        // Flex centering needs the disc under the root; without a root yet
        // (first frame race) it draws top-left once, then centers.
        if let Some(root) = root_entity {
            commands.entity(root).add_child(disc);
        }
    }
}

fn veil_bundle(kind: VeilKind, phase: VeilPhase, t: f32) -> (Node, BackgroundColor) {
    let mut node = Node::default();
    let mut color = BackgroundColor(Color::BLACK);
    style_veil(&mut node, &mut color, kind, phase, t);
    (node, color)
}

/// A fade is a fullscreen wash whose alpha is the cover. A wipe is an
/// opaque panel sweeping left to right: it grows from the left edge while
/// covering, then shrinks toward the right edge while revealing. A circle is
/// an iris handled by `style_circle` (a centered disc, not this wash).
fn style_veil(
    node: &mut Node,
    color: &mut BackgroundColor,
    kind: VeilKind,
    phase: VeilPhase,
    t: f32,
) {
    let t = t.clamp(0.0, 1.0);
    match kind {
        // Circle never reaches here: `drive_veil` draws it as a disc, not a
        // wash. Keep it opaque-black fullscreen if it ever does, so a bug
        // covers rather than flashes.
        VeilKind::Circle => {
            node.position_type = PositionType::Absolute;
            node.left = Val::Percent(0.0);
            node.right = Val::Percent(0.0);
            node.top = Val::Percent(0.0);
            node.height = Val::Percent(100.0);
            node.width = Val::Percent(100.0);
            color.0 = Color::BLACK;
        }
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
        VeilKind::Fade | VeilKind::None => {
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
    fn a_circle_covers_like_a_fade_with_its_own_kind() {
        let mut veil = SceneVeil::default();
        assert!(veil.start("circle"));
        assert_eq!(veil.kind, VeilKind::Circle);
        assert!(!veil.ready_to_switch());
        assert!(veil.advance_out(OUT_SECS));
        assert!(veil.ready_to_switch());
        veil.begin_reveal();
        assert!(veil.advance_in(IN_SECS));
        assert_eq!(veil.phase, VeilPhase::Idle);
        // The disc covers the diagonal: even ultrawide corners hide.
        const { assert!(CIRCLE_COVER_VMIN >= 300.0) };
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
