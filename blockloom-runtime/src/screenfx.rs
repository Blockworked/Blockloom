//! Screen feedback blocks: a colored flash that fades out and a cover that
//! fades, wipes or irises in and out. Both run on the real clock, over the
//! interface, and are separate from the scene-switch veil.

use crate::engine::Engine;
use crate::transition::{CIRCLE_COVER_VMIN, VeilKind, normalize_kind};
use bevy::prelude::*;
use blockloom_core::blocks::Look2dDial;
use blockloom_core::post2d::parse_srgb;
use blockloom_core::sense::{self};

const FLASH_Z: i32 = 54;
const COVER_Z: i32 = 52;

/// How a cover hides the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoverKind {
    #[default]
    Fade,
    Wipe,
    /// A disc grows from the centre.
    Circle,
    /// The picture shrinks to a hole in the centre and closes.
    Iris,
}

impl CoverKind {
    fn parse(name: &str) -> Self {
        if name.trim().eq_ignore_ascii_case("iris") {
            return Self::Iris;
        }
        match normalize_kind(name) {
            VeilKind::Wipe => Self::Wipe,
            VeilKind::Circle => Self::Circle,
            _ => Self::Fade,
        }
    }
}

/// What the flash and the cover are doing.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenFx {
    pub kind: CoverKind,
    pub color: [f32; 3],
    /// Seconds from clear to opaque.
    pub time: f32,
    pub cover: f32,
    pub target: f32,
    pub flash_color: [f32; 3],
    pub flash: f32,
    flash_len: f32,
}

impl Default for ScreenFx {
    fn default() -> Self {
        Self {
            kind: CoverKind::Fade,
            color: [0.0; 3],
            time: 0.5,
            cover: 0.0,
            target: 0.0,
            flash_color: [1.0; 3],
            flash: 0.0,
            flash_len: 0.0,
        }
    }
}

impl ScreenFx {
    /// Applies one dial. A value that doesn't read is ignored.
    pub fn set(&mut self, dial: Look2dDial, value: &str) {
        let number = value.trim().parse::<f32>().ok().filter(|n| n.is_finite());
        match dial {
            Look2dDial::Flash => {
                if let Some(n) = number.filter(|n| *n > 0.0) {
                    self.flash_len = n.min(10.0);
                    self.flash = self.flash_len;
                }
            }
            Look2dDial::FlashColor => {
                if let Some(c) = parse_srgb(value) {
                    self.flash_color = c;
                }
            }
            Look2dDial::Cover => {
                if let Some(n) = number {
                    self.target = n.clamp(0.0, 1.0);
                    if self.time <= 0.0 {
                        self.cover = self.target;
                    }
                }
            }
            Look2dDial::CoverKind => {
                self.kind = CoverKind::parse(value);
            }
            Look2dDial::CoverColor => {
                if let Some(c) = parse_srgb(value) {
                    self.color = c;
                }
            }
            Look2dDial::CoverTime => {
                if let Some(n) = number {
                    self.time = n.clamp(0.0, 30.0);
                }
            }
            _ => {}
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.flash = (self.flash - dt).max(0.0);
        if self.cover != self.target {
            let rate = if self.time > 0.0 { dt / self.time } else { 1.0 };
            self.cover = if self.cover < self.target {
                (self.cover + rate).min(self.target)
            } else {
                (self.cover - rate).max(self.target)
            };
        }
    }

    /// The flash's opacity, 1 when it starts and 0 when it is over.
    pub fn flash_alpha(&self) -> f32 {
        if self.flash_len > 0.0 {
            (self.flash / self.flash_len).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

#[derive(Resource, Default)]
pub struct ScreenFxState(pub ScreenFx);

#[derive(Component)]
struct FlashNode;

/// The cover's node, with the kind it was built for.
#[derive(Component)]
struct CoverNode(CoverKind);

#[derive(Component)]
struct CoverDisc;

pub fn register(app: &mut App) {
    app.init_resource::<ScreenFxState>()
        .add_systems(Update, drive_screenfx.after(crate::camera2d::apply_camera));
}

fn rgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

fn full(mut node: Node) -> Node {
    node.position_type = PositionType::Absolute;
    node.left = Val::Percent(0.0);
    node.top = Val::Percent(0.0);
    node.width = Val::Percent(100.0);
    node.height = Val::Percent(100.0);
    node
}

#[allow(clippy::too_many_arguments)]
fn drive_screenfx(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    mut state: ResMut<ScreenFxState>,
    real: Res<Time<Real>>,
    mut flashes: Query<(Entity, &mut BackgroundColor), (With<FlashNode>, Without<CoverNode>)>,
    mut covers: Query<(Entity, &CoverNode, &mut Node, &mut BackgroundColor), Without<FlashNode>>,
    mut discs: Query<
        (&mut Node, &mut BackgroundColor),
        (With<CoverDisc>, Without<CoverNode>, Without<FlashNode>),
    >,
) {
    if !engine.running {
        state.0 = ScreenFx::default();
        engine.look2d.fx.clear();
    }
    for (dial, value) in std::mem::take(&mut engine.look2d.fx) {
        state.0.set(dial, &value);
    }
    state.0.step(real.delta_secs().max(0.0));
    let fx = &state.0;

    let alpha = fx.flash_alpha();
    if alpha > 0.0 {
        let color = rgb(fx.flash_color).with_alpha(alpha);
        if let Some((_, mut bg)) = flashes.iter_mut().next() {
            bg.0 = color;
        } else {
            commands.spawn((
                Name::new("screen-flash"),
                FlashNode,
                full(Node::default()),
                BackgroundColor(color),
                GlobalZIndex(FLASH_Z),
            ));
        }
    } else {
        for (entity, _) in &flashes {
            commands.entity(entity).despawn();
        }
    }

    let cover = fx.cover.clamp(0.0, 1.0);
    sense_cover(cover);
    let stale = covers.iter().any(|(_, built, _, _)| built.0 != fx.kind);
    if cover <= 0.0 || stale {
        for (entity, _, _, _) in &covers {
            commands.entity(entity).despawn();
        }
        // A new kind is built next frame, once the old node is gone.
        return;
    }
    let color = rgb(fx.color);
    if let Some((_, _, mut node, mut bg)) = covers.iter_mut().next() {
        match fx.kind {
            CoverKind::Wipe => {
                node.width = Val::Percent(cover * 100.0);
                bg.0 = color;
            }
            CoverKind::Iris => {
                node.border = UiRect::all(iris_border(cover));
            }
            CoverKind::Circle => {
                bg.0 = color.with_alpha(0.0);
                let diameter = Val::VMin(cover * CIRCLE_COVER_VMIN);
                for (mut disc, mut disc_bg) in &mut discs {
                    disc.width = diameter;
                    disc.height = diameter;
                    disc_bg.0 = color;
                }
            }
            _ => bg.0 = color.with_alpha(cover),
        }
        return;
    }
    let mut root = full(Node::default());
    let kind = fx.kind;
    match kind {
        CoverKind::Wipe => {
            root.width = Val::Percent(cover * 100.0);
            commands.spawn((
                Name::new("screen-cover"),
                CoverNode(kind),
                root,
                BackgroundColor(color),
                GlobalZIndex(COVER_Z),
            ));
        }
        CoverKind::Iris => {
            // A huge ring node: its thick border is the cover, its hole the picture.
            let size = Val::VMin(CIRCLE_COVER_VMIN);
            commands.spawn((
                Name::new("screen-cover"),
                CoverNode(kind),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(50.0),
                    top: Val::Percent(50.0),
                    width: size,
                    height: size,
                    border: UiRect::all(iris_border(cover)),
                    border_radius: BorderRadius::all(Val::Percent(50.0)),
                    ..default()
                },
                UiTransform::from_translation(Val2::percent(-50.0, -50.0)),
                BackgroundColor(color.with_alpha(0.0)),
                BorderColor::all(color),
                GlobalZIndex(COVER_Z),
            ));
        }
        CoverKind::Circle => {
            root.display = Display::Flex;
            root.justify_content = JustifyContent::Center;
            root.align_items = AlignItems::Center;
            let diameter = Val::VMin(cover * CIRCLE_COVER_VMIN);
            commands
                .spawn((
                    Name::new("screen-cover"),
                    CoverNode(kind),
                    root,
                    BackgroundColor(color.with_alpha(0.0)),
                    GlobalZIndex(COVER_Z),
                ))
                .with_children(|parent| {
                    parent.spawn((
                        CoverDisc,
                        Node {
                            width: diameter,
                            height: diameter,
                            border_radius: BorderRadius::all(Val::Percent(50.0)),
                            ..default()
                        },
                        BackgroundColor(color),
                    ));
                });
        }
        _ => {
            commands.spawn((
                Name::new("screen-cover"),
                CoverNode(kind),
                root,
                BackgroundColor(color.with_alpha(cover)),
                GlobalZIndex(COVER_Z),
            ));
        }
    }
}

/// The ring's thickness for a cover: 0 leaves the whole picture, 1 closes it.
fn iris_border(cover: f32) -> Val {
    Val::VMin(cover.clamp(0.0, 1.0) * CIRCLE_COVER_VMIN / 2.0)
}

/// Lets `screen cover` read what the screen shows.
fn sense_cover(cover: f32) {
    let mut camera = sense::read(|s| s.camera2d);
    if camera.cover != cover {
        camera.cover = cover;
        sense::publish_camera2d(camera);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_fades_out_over_its_length() {
        let mut fx = ScreenFx::default();
        fx.set(Look2dDial::FlashColor, "#FF0000");
        fx.set(Look2dDial::Flash, "0.5");
        assert_eq!(fx.flash_color, [1.0, 0.0, 0.0]);
        assert_eq!(fx.flash_alpha(), 1.0);
        fx.step(0.25);
        assert!((fx.flash_alpha() - 0.5).abs() < 1e-5);
        fx.step(1.0);
        assert_eq!(fx.flash_alpha(), 0.0);
    }

    #[test]
    fn a_cover_runs_to_its_target_in_its_time() {
        let mut fx = ScreenFx::default();
        fx.set(Look2dDial::CoverTime, "2");
        fx.set(Look2dDial::Cover, "1");
        fx.step(0.5);
        assert!((fx.cover - 0.25).abs() < 1e-5);
        fx.step(5.0);
        assert_eq!(fx.cover, 1.0);
        fx.set(Look2dDial::Cover, "0.5");
        fx.step(1.0);
        assert!((fx.cover - 0.5).abs() < 1e-5, "stops at the new target");
    }

    #[test]
    fn instant_cover_and_bad_values() {
        let mut fx = ScreenFx::default();
        fx.set(Look2dDial::CoverTime, "0");
        fx.set(Look2dDial::Cover, "3");
        assert_eq!(fx.cover, 1.0, "clamped, and instant with no time");
        fx.set(Look2dDial::Cover, "nope");
        fx.set(Look2dDial::CoverKind, "iris wipe");
        assert_eq!(fx.kind, CoverKind::Fade, "unknown kinds stay a fade");
        fx.set(Look2dDial::CoverKind, "Circle");
        assert_eq!(fx.kind, CoverKind::Circle);
        fx.set(Look2dDial::CoverKind, "Iris");
        assert_eq!(fx.kind, CoverKind::Iris);
        fx.set(Look2dDial::CoverColor, "red");
        assert_eq!(fx.color, [0.0; 3], "a bad color is ignored");
    }
}
