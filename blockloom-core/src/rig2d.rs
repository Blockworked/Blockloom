//! 2D skeletal rigs: bones, slots that hold swappable sprite attachments,
//! two-bone IK and keyed animations, imported from Spine or DragonBones JSON.
//!
//! The runtime loads the file an `Animation` component names
//! ([`crate::animation::AnimationSpec::rig`]), samples a [`LocalPose`] per
//! fixed tick (blending two for a crossfade), and [`Rig::solve`]s it into
//! world transforms for every slot's sprite. Everything is in the rig's own
//! space: y up, degrees counter-clockwise, the same as a 2D actor.
//!
//! Only region/image attachments draw; mesh attachments and bezier curves
//! are read as nothing and as linear respectively.

use serde_json::{Map, Value as Json};

/// A 2D affine transform: `p -> (a*x + c*y + tx, b*x + d*y + ty)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine2 {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Affine2 {
    pub const IDENTITY: Affine2 = Affine2 {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// Translate, then rotate (degrees), then scale.
    pub fn from_parts(x: f32, y: f32, rotation: f32, sx: f32, sy: f32) -> Affine2 {
        let (sin, cos) = rotation.to_radians().sin_cos();
        Affine2 {
            a: cos * sx,
            b: sin * sx,
            c: -sin * sy,
            d: cos * sy,
            tx: x,
            ty: y,
        }
    }

    pub fn apply(self, p: [f32; 2]) -> [f32; 2] {
        [
            self.a * p[0] + self.c * p[1] + self.tx,
            self.b * p[0] + self.d * p[1] + self.ty,
        ]
    }

    /// The x axis's angle, in degrees.
    pub fn rotation(self) -> f32 {
        self.b.atan2(self.a).to_degrees()
    }

    /// Scale along the rotated axes; a mirrored transform has a negative y.
    pub fn scale(self) -> [f32; 2] {
        let sx = self.a.hypot(self.b);
        let det = self.a * self.d - self.b * self.c;
        [sx, if sx > 0.0 { det / sx } else { 0.0 }]
    }
}

impl std::ops::Mul for Affine2 {
    type Output = Affine2;

    /// `self` applied after `inner`.
    fn mul(self, inner: Affine2) -> Affine2 {
        Affine2 {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            tx: self.a * inner.tx + self.c * inner.ty + self.tx,
            ty: self.b * inner.tx + self.d * inner.ty + self.ty,
        }
    }
}

/// One bone's setup pose, relative to its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub x: f32,
    pub y: f32,
    pub rotation: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub length: f32,
}

/// A draw slot on a bone. `attachment` is what it shows at setup; empty
/// shows nothing. `color` is RGBA 0-1.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub name: String,
    pub bone: usize,
    pub attachment: String,
    pub color: [f32; 4],
}

/// A sprite a slot can show, placed in its bone's frame. A zero size means
/// the image's own size.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub name: String,
    /// Project-relative image path.
    pub image: String,
    pub x: f32,
    pub y: f32,
    pub rotation: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    pub name: String,
    /// `(slot, attachment)` pairs.
    pub attachments: Vec<(usize, Attachment)>,
}

/// Points a chain of one or two bones at a target bone. Two bones bend at
/// the joint between them, towards positive rotation when `bend_positive`.
#[derive(Debug, Clone, PartialEq)]
pub struct IkConstraint {
    pub name: String,
    pub bones: Vec<usize>,
    pub target: usize,
    pub bend_positive: bool,
    pub mix: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Linear,
    Stepped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Key<T> {
    pub time: f32,
    pub value: T,
    pub curve: Curve,
}

/// Keys over one bone, as offsets from its setup pose: degrees added,
/// translation added, scale multiplied.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoneTimeline {
    pub bone: usize,
    pub rotate: Vec<Key<f32>>,
    pub translate: Vec<Key<[f32; 2]>>,
    pub scale: Vec<Key<[f32; 2]>>,
}

/// Keys over one slot: attachment swaps (empty hides) and colors.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SlotTimeline {
    pub slot: usize,
    pub attachment: Vec<(f32, String)>,
    pub color: Vec<Key<[f32; 4]>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RigAnimation {
    pub name: String,
    pub duration: f32,
    pub bones: Vec<BoneTimeline>,
    pub slots: Vec<SlotTimeline>,
    /// Named events by time; they fire as clip markers.
    pub events: Vec<(f32, String)>,
}

/// One bone's local transform in a pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneLocal {
    pub x: f32,
    pub y: f32,
    pub rotation: f32,
    pub scale_x: f32,
    pub scale_y: f32,
}

/// A sampled pose before the hierarchy and IK: every bone's local transform
/// and every slot's attachment and color.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalPose {
    pub bones: Vec<BoneLocal>,
    pub attachments: Vec<String>,
    pub colors: Vec<[f32; 4]>,
}

impl LocalPose {
    /// `self` towards `other` by `w` (0 is self). Angles take the short way
    /// round; attachments switch halfway.
    pub fn blend(&self, other: &LocalPose, w: f32) -> LocalPose {
        let w = w.clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * w;
        let bones = self
            .bones
            .iter()
            .zip(&other.bones)
            .map(|(a, b)| BoneLocal {
                x: lerp(a.x, b.x),
                y: lerp(a.y, b.y),
                rotation: a.rotation + wrap_degrees(b.rotation - a.rotation) * w,
                scale_x: lerp(a.scale_x, b.scale_x),
                scale_y: lerp(a.scale_y, b.scale_y),
            })
            .collect();
        let attachments = if w < 0.5 {
            self.attachments.clone()
        } else {
            other.attachments.clone()
        };
        let colors = self
            .colors
            .iter()
            .zip(&other.colors)
            .map(|(a, b)| std::array::from_fn(|i| lerp(a[i], b[i])))
            .collect();
        LocalPose {
            bones,
            attachments,
            colors,
        }
    }
}

/// One sprite to draw, in draw order, in rig space.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotDraw {
    pub slot: usize,
    pub image: String,
    pub transform: Affine2,
    pub size: [f32; 2],
    pub color: [f32; 4],
}

/// A solved pose: every bone's world transform and what each slot draws.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldPose {
    pub bones: Vec<Affine2>,
    pub draws: Vec<SlotDraw>,
}

/// A whole skeleton.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rig {
    pub bones: Vec<Bone>,
    pub slots: Vec<Slot>,
    pub skins: Vec<Skin>,
    pub ik: Vec<IkConstraint>,
    pub animations: Vec<RigAnimation>,
}

fn wrap_degrees(mut degrees: f32) -> f32 {
    degrees %= 360.0;
    if degrees > 180.0 {
        degrees -= 360.0;
    } else if degrees < -180.0 {
        degrees += 360.0;
    }
    degrees
}

fn sample_keys<T: Copy>(keys: &[Key<T>], time: f32, lerp: impl Fn(T, T, f32) -> T) -> Option<T> {
    let first = keys.first()?;
    if time <= first.time {
        return Some(first.value);
    }
    for pair in keys.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if time < b.time {
            if a.curve == Curve::Stepped || b.time <= a.time {
                return Some(a.value);
            }
            return Some(lerp(a.value, b.value, (time - a.time) / (b.time - a.time)));
        }
    }
    keys.last().map(|key| key.value)
}

impl Rig {
    /// Reads Spine (`bones`/`slots`/`skins`) or DragonBones (`armature`)
    /// JSON. `folder` is the project-relative folder the file sits in, which
    /// attachment images are found under.
    pub fn parse(text: &str, folder: &str) -> Result<Rig, String> {
        let json: Json =
            serde_json::from_str(text).map_err(|error| format!("not a rig file: {error}"))?;
        let rig = if json.get("armature").is_some() {
            parse_dragonbones(&json, folder)?
        } else if json.get("bones").is_some() {
            parse_spine(&json, folder)?
        } else {
            return Err("not a Spine or DragonBones skeleton".to_string());
        };
        if rig.bones.is_empty() {
            return Err("the skeleton has no bones".to_string());
        }
        Ok(rig)
    }

    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones
            .iter()
            .position(|bone| bone.name.eq_ignore_ascii_case(name.trim()))
    }

    pub fn slot_index(&self, name: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.name.eq_ignore_ascii_case(name.trim()))
    }

    pub fn ik_index(&self, name: &str) -> Option<usize> {
        self.ik
            .iter()
            .position(|ik| ik.name.eq_ignore_ascii_case(name.trim()))
    }

    pub fn animation(&self, name: &str) -> Option<&RigAnimation> {
        let wanted = name.trim();
        if wanted.is_empty() {
            return None;
        }
        self.animations
            .iter()
            .find(|animation| animation.name.eq_ignore_ascii_case(wanted))
    }

    pub fn setup_pose(&self) -> LocalPose {
        LocalPose {
            bones: self
                .bones
                .iter()
                .map(|bone| BoneLocal {
                    x: bone.x,
                    y: bone.y,
                    rotation: bone.rotation,
                    scale_x: bone.scale_x,
                    scale_y: bone.scale_y,
                })
                .collect(),
            attachments: self
                .slots
                .iter()
                .map(|slot| slot.attachment.clone())
                .collect(),
            colors: self.slots.iter().map(|slot| slot.color).collect(),
        }
    }

    /// The pose `animation` holds at `time` seconds (already wrapped into
    /// its length). `None` is the setup pose. With `pin_root`, the root
    /// bone's keyed translation is left out: root motion moves the actor.
    pub fn sample(&self, animation: Option<&RigAnimation>, time: f32, pin_root: bool) -> LocalPose {
        let mut pose = self.setup_pose();
        let Some(animation) = animation else {
            return pose;
        };
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        for timeline in &animation.bones {
            let Some(local) = pose.bones.get_mut(timeline.bone) else {
                continue;
            };
            if let Some(angle) = sample_keys(&timeline.rotate, time, |a, b, t| {
                a + wrap_degrees(b - a) * t
            }) {
                local.rotation += angle;
            }
            if !(pin_root && timeline.bone == 0)
                && let Some(offset) = sample_keys(&timeline.translate, time, |a, b, t| {
                    [lerp(a[0], b[0], t), lerp(a[1], b[1], t)]
                })
            {
                local.x += offset[0];
                local.y += offset[1];
            }
            if let Some(scale) = sample_keys(&timeline.scale, time, |a, b, t| {
                [lerp(a[0], b[0], t), lerp(a[1], b[1], t)]
            }) {
                local.scale_x *= scale[0];
                local.scale_y *= scale[1];
            }
        }
        for timeline in &animation.slots {
            if timeline.slot >= self.slots.len() {
                continue;
            }
            let shown = timeline
                .attachment
                .iter()
                .take_while(|(at, _)| *at <= time)
                .last()
                .or(timeline.attachment.first());
            if let Some((_, name)) = shown {
                pose.attachments[timeline.slot] = name.clone();
            }
            if let Some(color) = sample_keys(&timeline.color, time, |a, b, t| {
                std::array::from_fn(|i| lerp(a[i], b[i], t))
            }) {
                pose.colors[timeline.slot] = color;
            }
        }
        pose
    }

    fn world_bones(&self, pose: &LocalPose) -> Vec<Affine2> {
        let mut world: Vec<Affine2> = Vec::with_capacity(self.bones.len());
        for (index, bone) in self.bones.iter().enumerate() {
            let local = pose.bones.get(index).copied().unwrap_or(BoneLocal {
                x: bone.x,
                y: bone.y,
                rotation: bone.rotation,
                scale_x: bone.scale_x,
                scale_y: bone.scale_y,
            });
            let own = Affine2::from_parts(
                local.x,
                local.y,
                local.rotation,
                local.scale_x,
                local.scale_y,
            );
            // Parents come first in both formats, so the parent is solved.
            let parent = bone
                .parent
                .filter(|&parent| parent < index)
                .map(|parent| world[parent])
                .unwrap_or(Affine2::IDENTITY);
            world.push(parent * own);
        }
        world
    }

    /// Composes the hierarchy, runs IK, and lists what each slot draws.
    /// `targets` overrides an IK constraint's target with a point in rig
    /// space, by constraint index.
    pub fn solve(&self, pose: &LocalPose, skin: &str, targets: &[(usize, [f32; 2])]) -> WorldPose {
        let mut local = pose.clone();
        let mut world = self.world_bones(&local);
        for (index, ik) in self.ik.iter().enumerate() {
            let target = targets
                .iter()
                .find(|(which, _)| *which == index)
                .map(|(_, point)| *point)
                .or_else(|| world.get(ik.target).map(|t| [t.tx, t.ty]));
            let Some(target) = target else {
                continue;
            };
            if solve_ik(self, ik, target, &mut local, &world) {
                world = self.world_bones(&local);
            }
        }
        let draws = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let name = local.attachments.get(index)?;
                if name.is_empty() {
                    return None;
                }
                let attachment = self.attachment(skin, index, name)?;
                let bone = *world.get(slot.bone)?;
                Some(SlotDraw {
                    slot: index,
                    image: attachment.image.clone(),
                    transform: bone
                        * Affine2::from_parts(
                            attachment.x,
                            attachment.y,
                            attachment.rotation,
                            attachment.scale_x,
                            attachment.scale_y,
                        ),
                    size: [attachment.width, attachment.height],
                    color: local.colors.get(index).copied().unwrap_or([1.0; 4]),
                })
            })
            .collect();
        WorldPose {
            bones: world,
            draws,
        }
    }

    /// The attachment `name` for `slot` in `skin`, falling back to the
    /// default skin (named "default", or the first).
    pub fn attachment(&self, skin: &str, slot: usize, name: &str) -> Option<&Attachment> {
        fn find<'s>(skin: &'s Skin, slot: usize, name: &str) -> Option<&'s Attachment> {
            skin.attachments
                .iter()
                .find(|(at, attachment)| *at == slot && attachment.name.eq_ignore_ascii_case(name))
                .map(|(_, attachment)| attachment)
        }
        let chosen = self
            .skins
            .iter()
            .find(|candidate| !skin.is_empty() && candidate.name.eq_ignore_ascii_case(skin));
        let fallback = self
            .skins
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case("default"))
            .or(self.skins.first());
        chosen
            .and_then(|skin| find(skin, slot, name))
            .or_else(|| fallback.and_then(|skin| find(skin, slot, name)))
    }

    /// Every image any skin names, for preloading.
    pub fn images(&self) -> Vec<String> {
        let mut images: Vec<String> = self
            .skins
            .iter()
            .flat_map(|skin| skin.attachments.iter().map(|(_, a)| a.image.clone()))
            .filter(|image| !image.is_empty())
            .collect();
        images.sort();
        images.dedup();
        images
    }
}

impl RigAnimation {
    /// Where the root bone's keyed translation stands at `time`.
    fn root_offset(&self, time: f32) -> [f32; 2] {
        self.bones
            .iter()
            .find(|timeline| timeline.bone == 0)
            .and_then(|timeline| {
                sample_keys(&timeline.translate, time, |a, b, t| {
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                })
            })
            .unwrap_or([0.0, 0.0])
    }

    /// How far the root bone travels between two unwrapped times, counting
    /// a full pass for each loop in between.
    pub fn root_travel(&self, from: f32, to: f32, looping: bool) -> [f32; 2] {
        let length = self.duration;
        if length <= 0.0 || to <= from {
            return [0.0, 0.0];
        }
        if !looping {
            let (a, b) = (
                self.root_offset(from.min(length)),
                self.root_offset(to.min(length)),
            );
            return [b[0] - a[0], b[1] - a[1]];
        }
        let pass = {
            let (start, end) = (self.root_offset(0.0), self.root_offset(length));
            [end[0] - start[0], end[1] - start[1]]
        };
        let (a, b) = (
            self.root_offset(from.rem_euclid(length)),
            self.root_offset(to.rem_euclid(length)),
        );
        let loops = (to / length).floor() - (from / length).floor();
        [b[0] - a[0] + pass[0] * loops, b[1] - a[1] + pass[1] * loops]
    }

    /// Events whose time falls in `(from, to]` of unwrapped time, in order.
    /// `from` below zero means the animation just started, so time zero
    /// counts.
    pub fn events_between(&self, from: f32, to: f32, looping: bool) -> Vec<&str> {
        let length = self.duration;
        if self.events.is_empty() || to < from {
            return Vec::new();
        }
        if !looping || length <= 0.0 {
            return self
                .events
                .iter()
                .filter(|(at, _)| *at > from && *at <= to)
                .map(|(_, name)| name.as_str())
                .collect();
        }
        let mut out = Vec::new();
        let first = (from.max(0.0) / length).floor() as i64;
        let last = (to / length).floor() as i64;
        for pass in first..=last.min(first + 64) {
            let base = pass as f32 * length;
            for (at, name) in &self.events {
                let when = base + at;
                if when > from && when <= to {
                    out.push(name.as_str());
                }
            }
        }
        out
    }
}

/// Bends one or two bones so the chain's end reaches `target`. Returns
/// whether it changed anything.
fn solve_ik(
    rig: &Rig,
    ik: &IkConstraint,
    target: [f32; 2],
    local: &mut LocalPose,
    world: &[Affine2],
) -> bool {
    let mix = ik.mix.clamp(0.0, 1.0);
    if mix <= 0.0 {
        return false;
    }
    match ik.bones.as_slice() {
        [only] => {
            let Some(bone) = world.get(*only) else {
                return false;
            };
            let aim = (target[1] - bone.ty)
                .atan2(target[0] - bone.tx)
                .to_degrees();
            local.bones[*only].rotation += wrap_degrees(aim - bone.rotation()) * mix;
            true
        }
        [upper, lower] => {
            let (Some(first), Some(second)) = (world.get(*upper), world.get(*lower)) else {
                return false;
            };
            let length_a = (second.tx - first.tx).hypot(second.ty - first.ty);
            let length_b = rig.bones[*lower].length * second.scale()[0].abs();
            if length_a <= f32::EPSILON || length_b <= f32::EPSILON {
                return false;
            }
            let to_target = [target[0] - first.tx, target[1] - first.ty];
            let reach = to_target[0].hypot(to_target[1]).clamp(
                (length_a - length_b).abs() + 1e-4,
                length_a + length_b - 1e-4,
            );
            let cos_inner = ((length_a * length_a + reach * reach - length_b * length_b)
                / (2.0 * length_a * reach))
                .clamp(-1.0, 1.0);
            let aim = to_target[1].atan2(to_target[0]);
            let bend = if ik.bend_positive { 1.0 } else { -1.0 };
            let upper_angle = aim + bend * cos_inner.acos();
            let elbow = [
                first.tx + length_a * upper_angle.cos(),
                first.ty + length_a * upper_angle.sin(),
            ];
            let lower_angle = (target[1] - elbow[1]).atan2(target[0] - elbow[0]);
            // The upper bone turns so its child sits on the solved elbow.
            let current_upper = (second.ty - first.ty).atan2(second.tx - first.tx);
            let turn_upper = wrap_degrees((upper_angle - current_upper).to_degrees()) * mix;
            local.bones[*upper].rotation += turn_upper;
            // Turning the upper bone turned the lower one with it.
            let lower_now = second.rotation() + turn_upper;
            local.bones[*lower].rotation +=
                wrap_degrees(lower_angle.to_degrees() - lower_now) * mix;
            true
        }
        _ => false,
    }
}

// ─── Importers ───────────────────────────────────────────────────────────

fn num(value: Option<&Json>, fallback: f32) -> f32 {
    value
        .and_then(Json::as_f64)
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .unwrap_or(fallback)
}

fn text(value: Option<&Json>) -> &str {
    value.and_then(Json::as_str).unwrap_or("")
}

fn empty_map() -> &'static Map<String, Json> {
    static EMPTY: std::sync::OnceLock<Map<String, Json>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Map::new)
}

fn object(value: Option<&Json>) -> &Map<String, Json> {
    value
        .and_then(Json::as_object)
        .unwrap_or_else(|| empty_map())
}

fn array(value: Option<&Json>) -> &[Json] {
    value.and_then(Json::as_array).map_or(&[], Vec::as_slice)
}

/// `folder/sub/name`, with `./` and empty segments dropped.
fn join_path(parts: &[&str]) -> String {
    parts
        .iter()
        .flat_map(|part| part.split(['/', '\\']))
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn image_path(folder: &str, images: &str, name: &str) -> String {
    let mut path = join_path(&[folder, images, name]);
    if std::path::Path::new(&path).extension().is_none() {
        path.push_str(".png");
    }
    path
}

/// `RRGGBBAA` or `RRGGBB` hex to RGBA 0-1.
fn hex_color(hex: &str) -> [f32; 4] {
    let hex = hex.trim().trim_start_matches('#');
    let channel = |i: usize| {
        hex.get(i * 2..i * 2 + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map(|v| v as f32 / 255.0)
    };
    [
        channel(0).unwrap_or(1.0),
        channel(1).unwrap_or(1.0),
        channel(2).unwrap_or(1.0),
        channel(3).unwrap_or(1.0),
    ]
}

fn curve_of(key: &Json) -> Curve {
    if key.get("curve").and_then(Json::as_str) == Some("stepped") {
        Curve::Stepped
    } else {
        Curve::Linear
    }
}

fn parse_spine(json: &Json, folder: &str) -> Result<Rig, String> {
    let images = json
        .get("skeleton")
        .and_then(|skeleton| skeleton.get("images"))
        .and_then(Json::as_str)
        .filter(|images| !images.trim().is_empty())
        .unwrap_or("images");
    let mut rig = Rig::default();
    for bone in array(json.get("bones")) {
        let parent_name = text(bone.get("parent"));
        let parent =
            if parent_name.is_empty() {
                None
            } else {
                Some(rig.bone_index(parent_name).ok_or_else(|| {
                    format!("bone parent \"{parent_name}\" comes after its child")
                })?)
            };
        rig.bones.push(Bone {
            name: text(bone.get("name")).to_string(),
            parent,
            x: num(bone.get("x"), 0.0),
            y: num(bone.get("y"), 0.0),
            rotation: num(bone.get("rotation"), 0.0),
            scale_x: num(bone.get("scaleX"), 1.0),
            scale_y: num(bone.get("scaleY"), 1.0),
            length: num(bone.get("length"), 0.0),
        });
    }
    for slot in array(json.get("slots")) {
        let bone = text(slot.get("bone"));
        let Some(bone) = rig.bone_index(bone) else {
            return Err(format!("slot on a missing bone \"{bone}\""));
        };
        rig.slots.push(Slot {
            name: text(slot.get("name")).to_string(),
            bone,
            attachment: text(slot.get("attachment")).to_string(),
            color: slot
                .get("color")
                .and_then(Json::as_str)
                .map(hex_color)
                .unwrap_or([1.0; 4]),
        });
    }
    // Spine 3.8+ writes skins as a list; older files as a map by name.
    let skins: Vec<(String, &Map<String, Json>)> = match json.get("skins") {
        Some(Json::Array(list)) => list
            .iter()
            .map(|skin| {
                (
                    text(skin.get("name")).to_string(),
                    object(skin.get("attachments")),
                )
            })
            .collect(),
        Some(Json::Object(map)) => map
            .iter()
            .map(|(name, slots)| (name.clone(), object(Some(slots))))
            .collect(),
        _ => Vec::new(),
    };
    for (name, slots) in skins {
        let mut skin = Skin {
            name,
            attachments: Vec::new(),
        };
        for (slot_name, attachments) in slots {
            let Some(slot) = rig.slot_index(slot_name) else {
                continue;
            };
            for (key, attachment) in object(Some(attachments)) {
                let kind = text(attachment.get("type"));
                if !kind.is_empty() && kind != "region" {
                    continue;
                }
                let file = attachment
                    .get("path")
                    .or(attachment.get("name"))
                    .and_then(Json::as_str)
                    .unwrap_or(key);
                skin.attachments.push((
                    slot,
                    Attachment {
                        name: key.clone(),
                        image: image_path(folder, images, file),
                        x: num(attachment.get("x"), 0.0),
                        y: num(attachment.get("y"), 0.0),
                        rotation: num(attachment.get("rotation"), 0.0),
                        scale_x: num(attachment.get("scaleX"), 1.0),
                        scale_y: num(attachment.get("scaleY"), 1.0),
                        width: num(attachment.get("width"), 0.0),
                        height: num(attachment.get("height"), 0.0),
                    },
                ));
            }
        }
        rig.skins.push(skin);
    }
    for ik in array(json.get("ik")) {
        let bones: Vec<usize> = array(ik.get("bones"))
            .iter()
            .filter_map(|bone| bone.as_str().and_then(|name| rig.bone_index(name)))
            .collect();
        let Some(target) = rig.bone_index(text(ik.get("target"))) else {
            continue;
        };
        if bones.is_empty() || bones.len() > 2 {
            continue;
        }
        rig.ik.push(IkConstraint {
            name: text(ik.get("name")).to_string(),
            bones,
            target,
            bend_positive: ik
                .get("bendPositive")
                .and_then(Json::as_bool)
                .unwrap_or(true),
            mix: num(ik.get("mix"), 1.0),
        });
    }
    for (name, animation) in object(json.get("animations")) {
        let mut out = RigAnimation {
            name: name.clone(),
            ..RigAnimation::default()
        };
        let mut end: f32 = 0.0;
        let mut time_of = |key: &Json| {
            let time = num(key.get("time"), 0.0);
            end = end.max(time);
            time
        };
        for (bone_name, timelines) in object(animation.get("bones")) {
            let Some(bone) = rig.bone_index(bone_name) else {
                continue;
            };
            let mut timeline = BoneTimeline {
                bone,
                ..BoneTimeline::default()
            };
            for key in array(timelines.get("rotate")) {
                timeline.rotate.push(Key {
                    time: time_of(key),
                    value: num(key.get("angle").or(key.get("value")), 0.0),
                    curve: curve_of(key),
                });
            }
            for key in array(timelines.get("translate")) {
                timeline.translate.push(Key {
                    time: time_of(key),
                    value: [num(key.get("x"), 0.0), num(key.get("y"), 0.0)],
                    curve: curve_of(key),
                });
            }
            for key in array(timelines.get("scale")) {
                timeline.scale.push(Key {
                    time: time_of(key),
                    value: [num(key.get("x"), 1.0), num(key.get("y"), 1.0)],
                    curve: curve_of(key),
                });
            }
            out.bones.push(timeline);
        }
        for (slot_name, timelines) in object(animation.get("slots")) {
            let Some(slot) = rig.slot_index(slot_name) else {
                continue;
            };
            let mut timeline = SlotTimeline {
                slot,
                ..SlotTimeline::default()
            };
            for key in array(timelines.get("attachment")) {
                timeline
                    .attachment
                    .push((time_of(key), text(key.get("name")).to_string()));
            }
            for key in array(timelines.get("color").or(timelines.get("rgba"))) {
                timeline.color.push(Key {
                    time: time_of(key),
                    value: hex_color(text(key.get("color"))),
                    curve: curve_of(key),
                });
            }
            out.slots.push(timeline);
        }
        for key in array(animation.get("events")) {
            let at = time_of(key);
            out.events.push((at, text(key.get("name")).to_string()));
        }
        out.duration = end;
        rig.animations.push(out);
    }
    Ok(rig)
}

/// DragonBones color multipliers are percentages.
fn db_color(value: Option<&Json>) -> [f32; 4] {
    let color = object(value);
    let channel = |key: &str| num(color.get(key), 100.0) / 100.0;
    [channel("rM"), channel("gM"), channel("bM"), channel("aM")]
}

/// DragonBones is y-down with clockwise angles; flip both into y-up.
fn db_transform(value: Option<&Json>) -> (f32, f32, f32, f32, f32) {
    let t = object(value);
    (
        num(t.get("x"), 0.0),
        -num(t.get("y"), 0.0),
        -num(t.get("skX").or(t.get("skY")), 0.0),
        num(t.get("scX"), 1.0),
        num(t.get("scY"), 1.0),
    )
}

fn parse_dragonbones(json: &Json, folder: &str) -> Result<Rig, String> {
    let armature = array(json.get("armature"))
        .first()
        .ok_or("the file has no armature")?;
    let rate = num(armature.get("frameRate").or(json.get("frameRate")), 24.0).max(1.0);
    let mut rig = Rig::default();
    for bone in array(armature.get("bone")) {
        let parent_name = text(bone.get("parent"));
        let parent =
            if parent_name.is_empty() {
                None
            } else {
                Some(rig.bone_index(parent_name).ok_or_else(|| {
                    format!("bone parent \"{parent_name}\" comes after its child")
                })?)
            };
        let (x, y, rotation, scale_x, scale_y) = db_transform(bone.get("transform"));
        rig.bones.push(Bone {
            name: text(bone.get("name")).to_string(),
            parent,
            x,
            y,
            rotation,
            scale_x,
            scale_y,
            length: num(bone.get("length"), 0.0),
        });
    }
    // A slot shows its display by index; names come from the skin.
    let mut display_names: Vec<Vec<String>> = Vec::new();
    let mut skin = Skin {
        name: "default".to_string(),
        attachments: Vec::new(),
    };
    let skin_slots = array(armature.get("skin"))
        .first()
        .map(|skin| array(skin.get("slot")))
        .unwrap_or(&[]);
    let mut shown: Vec<i64> = Vec::new();
    for slot in array(armature.get("slot")) {
        let bone = text(slot.get("parent"));
        let Some(bone) = rig.bone_index(bone) else {
            return Err(format!("slot on a missing bone \"{bone}\""));
        };
        rig.slots.push(Slot {
            name: text(slot.get("name")).to_string(),
            bone,
            attachment: String::new(),
            color: db_color(slot.get("color")),
        });
        shown.push(slot.get("displayIndex").and_then(Json::as_i64).unwrap_or(0));
        display_names.push(Vec::new());
    }
    for entry in skin_slots {
        let Some(slot) = rig.slot_index(text(entry.get("name"))) else {
            continue;
        };
        for display in array(entry.get("display")) {
            let name = text(display.get("name")).to_string();
            display_names[slot].push(name.clone());
            let kind = text(display.get("type"));
            if !kind.is_empty() && kind != "image" {
                continue;
            }
            let file = display.get("path").and_then(Json::as_str).unwrap_or(&name);
            let (x, y, rotation, scale_x, scale_y) = db_transform(display.get("transform"));
            skin.attachments.push((
                slot,
                Attachment {
                    name: name.clone(),
                    image: image_path(folder, "", file),
                    x,
                    y,
                    rotation,
                    scale_x,
                    scale_y,
                    width: 0.0,
                    height: 0.0,
                },
            ));
        }
    }
    for (slot, index) in shown.iter().enumerate() {
        if let Some(name) = usize::try_from(*index)
            .ok()
            .and_then(|i| display_names[slot].get(i))
        {
            rig.slots[slot].attachment = name.clone();
        }
    }
    rig.skins.push(skin);
    for ik in array(armature.get("ik")) {
        let Some(end) = rig.bone_index(text(ik.get("bone"))) else {
            continue;
        };
        let Some(target) = rig.bone_index(text(ik.get("target"))) else {
            continue;
        };
        let chain = ik.get("chain").and_then(Json::as_u64).unwrap_or(0);
        let bones = match (chain, rig.bones[end].parent) {
            (1.., Some(parent)) => vec![parent, end],
            _ => vec![end],
        };
        rig.ik.push(IkConstraint {
            name: text(ik.get("name")).to_string(),
            bones,
            target,
            bend_positive: ik
                .get("bendPositive")
                .and_then(Json::as_bool)
                .unwrap_or(true),
            mix: num(ik.get("weight"), 1.0),
        });
    }
    // Frames carry durations in frames; a null tweenEasing holds.
    fn frames<T>(
        list: &[Json],
        rate: f32,
        value: impl Fn(&Json) -> T,
        end: &mut f32,
    ) -> Vec<Key<T>> {
        let mut at = 0.0;
        list.iter()
            .map(|frame| {
                let curve = match frame.get("tweenEasing") {
                    Some(Json::Null) => Curve::Stepped,
                    _ => Curve::Linear,
                };
                let key = Key {
                    time: at / rate,
                    value: value(frame),
                    curve,
                };
                at += num(frame.get("duration"), 1.0);
                *end = end.max(key.time);
                key
            })
            .collect()
    }
    for animation in array(armature.get("animation")) {
        let mut out = RigAnimation {
            name: text(animation.get("name")).to_string(),
            ..RigAnimation::default()
        };
        let mut end = num(animation.get("duration"), 0.0) / rate;
        for timelines in array(animation.get("bone")) {
            let Some(bone) = rig.bone_index(text(timelines.get("name"))) else {
                continue;
            };
            out.bones.push(BoneTimeline {
                bone,
                rotate: frames(
                    array(timelines.get("rotateFrame")),
                    rate,
                    |f| -num(f.get("rotate"), 0.0),
                    &mut end,
                ),
                translate: frames(
                    array(timelines.get("translateFrame")),
                    rate,
                    |f| [num(f.get("x"), 0.0), -num(f.get("y"), 0.0)],
                    &mut end,
                ),
                scale: frames(
                    array(timelines.get("scaleFrame")),
                    rate,
                    |f| [num(f.get("x"), 1.0), num(f.get("y"), 1.0)],
                    &mut end,
                ),
            });
        }
        for timelines in array(animation.get("slot")) {
            let Some(slot) = rig.slot_index(text(timelines.get("name"))) else {
                continue;
            };
            let names = display_names[slot].clone();
            let swaps = frames(
                array(timelines.get("displayFrame")),
                rate,
                |f| {
                    let index = f.get("value").and_then(Json::as_i64).unwrap_or(0);
                    usize::try_from(index)
                        .ok()
                        .and_then(|i| names.get(i).cloned())
                        .unwrap_or_default()
                },
                &mut end,
            );
            out.slots.push(SlotTimeline {
                slot,
                attachment: swaps.into_iter().map(|key| (key.time, key.value)).collect(),
                color: frames(
                    array(timelines.get("colorFrame")),
                    rate,
                    |f| db_color(f.get("value")),
                    &mut end,
                ),
            });
        }
        let mut at = 0.0;
        for frame in array(animation.get("frame")) {
            let time = at / rate;
            let named = array(frame.get("events"))
                .iter()
                .map(|event| text(event.get("name")).to_string())
                .chain(
                    frame
                        .get("event")
                        .and_then(Json::as_str)
                        .map(str::to_string),
                );
            for name in named.filter(|name| !name.is_empty()) {
                out.events.push((time, name));
            }
            at += num(frame.get("duration"), 1.0);
        }
        out.duration = end;
        rig.animations.push(out);
    }
    Ok(rig)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPINE: &str = r#"{
        "skeleton": { "spine": "4.1.0", "images": "./parts/" },
        "bones": [
            { "name": "root" },
            { "name": "upper", "parent": "root", "length": 10, "rotation": 0 },
            { "name": "lower", "parent": "upper", "length": 10, "x": 10 },
            { "name": "target", "parent": "root", "x": 20 }
        ],
        "slots": [
            { "name": "arm", "bone": "upper", "attachment": "arm", "color": "ff000080" },
            { "name": "hand", "bone": "lower", "attachment": "hand" }
        ],
        "ik": [ { "name": "reach", "bones": ["upper", "lower"], "target": "target" } ],
        "skins": [
            { "name": "default", "attachments": {
                "arm": { "arm": { "x": 5, "width": 10, "height": 4 } },
                "hand": { "hand": { "width": 4, "height": 4 }, "fist": { "path": "fist-2", "width": 4, "height": 4 } }
            } },
            { "name": "gold", "attachments": { "hand": { "hand": { "path": "gold-hand", "width": 5, "height": 5 } } } }
        ],
        "animations": {
            "wave": {
                "bones": {
                    "root": { "translate": [ { "time": 0, "x": 0 }, { "time": 1, "x": 30 } ] },
                    "upper": { "rotate": [ { "time": 0, "value": 0 }, { "time": 1, "value": 90 } ] }
                },
                "slots": { "hand": { "attachment": [ { "time": 0.5, "name": "fist" } ] } },
                "events": [ { "time": 0.25, "name": "step" } ]
            }
        }
    }"#;

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn spine_imports_bones_slots_skins_and_keys() {
        let rig = Rig::parse(SPINE, "assets/rigs").unwrap();
        assert_eq!(rig.bones.len(), 4);
        assert_eq!(rig.bones[2].parent, Some(1));
        assert_eq!(rig.slots[0].color, [1.0, 0.0, 0.0, 128.0 / 255.0]);
        let fist = rig.attachment("", 1, "fist").unwrap();
        assert_eq!(fist.image, "assets/rigs/parts/fist-2.png");
        // A skin without the attachment falls back to the default one.
        assert_eq!(rig.attachment("gold", 0, "arm").unwrap().name, "arm");
        assert_eq!(
            rig.attachment("gold", 1, "hand").unwrap().image,
            "assets/rigs/parts/gold-hand.png"
        );
        let wave = rig.animation("WAVE").unwrap();
        assert_eq!(wave.duration, 1.0);
        let pose = rig.sample(Some(wave), 0.5, false);
        assert!((pose.bones[1].rotation - 45.0).abs() < 1e-4);
        assert_eq!(pose.attachments[1], "fist");
        assert_eq!(wave.events_between(-1.0, 0.3, false), vec!["step"]);
        // Two loops of a one-second walk carry the root 60 along.
        assert!(near(wave.root_travel(0.0, 2.0, true), [60.0, 0.0]));
        let pinned = rig.sample(Some(wave), 0.5, true);
        assert_eq!(pinned.bones[0].x, 0.0);
    }

    #[test]
    fn two_bone_ik_reaches_a_target_in_range() {
        let rig = Rig::parse(SPINE, "").unwrap();
        let setup = rig.setup_pose();
        // Pull the target in to (10, 10): both bones are 10 long.
        let solved = rig.solve(&setup, "", &[(0, [10.0, 10.0])]);
        let lower = solved.bones[2];
        let tip = lower.apply([rig.bones[2].length, 0.0]);
        assert!(near(tip, [10.0, 10.0]), "tip at {tip:?}");
        // Bending positive puts the elbow on the counter-clockwise side.
        let elbow = [lower.tx, lower.ty];
        assert!(near(elbow, [0.0, 10.0]) || near(elbow, [10.0, 0.0]));
        // Out of reach, the chain points straight at it.
        let far = rig.solve(&setup, "", &[(0, [0.0, 50.0])]);
        let tip = far.bones[2].apply([10.0, 0.0]);
        assert!(
            (tip[0]).abs() < 0.05 && (tip[1] - 20.0).abs() < 1e-3,
            "tip at {tip:?}"
        );
    }

    #[test]
    fn solved_slots_carry_their_attachment_frame() {
        let mut rig = Rig::parse(SPINE, "").unwrap();
        rig.ik.clear();
        let world = rig.solve(&rig.setup_pose(), "", &[]);
        assert_eq!(world.draws.len(), 2);
        let arm = &world.draws[0];
        assert!(near([arm.transform.tx, arm.transform.ty], [5.0, 0.0]));
        assert_eq!(arm.size, [10.0, 4.0]);
    }

    #[test]
    fn blends_take_the_short_way_round() {
        let rig = Rig::parse(SPINE, "").unwrap();
        let mut a = rig.setup_pose();
        let mut b = rig.setup_pose();
        a.bones[1].rotation = 170.0;
        b.bones[1].rotation = -170.0;
        let half = a.blend(&b, 0.5);
        assert!(
            (wrap_degrees(half.bones[1].rotation) - 180.0).abs() < 1e-3
                || (wrap_degrees(half.bones[1].rotation) + 180.0).abs() < 1e-3
        );
    }

    #[test]
    fn dragonbones_imports_flipped_into_y_up() {
        let text = r#"{
            "frameRate": 10,
            "armature": [{
                "name": "hero",
                "bone": [
                    { "name": "root" },
                    { "name": "arm", "parent": "root", "length": 8, "transform": { "y": 4, "skX": 30 } }
                ],
                "slot": [ { "name": "arm", "parent": "arm", "displayIndex": 1 } ],
                "skin": [ { "slot": [ { "name": "arm", "display": [
                    { "name": "open" }, { "name": "closed", "path": "hands/closed" }
                ] } ] } ],
                "ik": [ { "name": "aim", "bone": "arm", "target": "root" } ],
                "animation": [ {
                    "name": "swing", "duration": 10,
                    "bone": [ { "name": "arm", "rotateFrame": [
                        { "duration": 5, "tweenEasing": 0, "rotate": 0 },
                        { "duration": 5, "tweenEasing": null, "rotate": 20 },
                        { "duration": 0, "rotate": 40 }
                    ] } ],
                    "frame": [ { "duration": 5 }, { "duration": 5, "events": [ { "name": "hit" } ] } ]
                } ]
            }]
        }"#;
        let rig = Rig::parse(text, "assets/hero").unwrap();
        assert_eq!(rig.bones[1].y, -4.0);
        assert_eq!(rig.bones[1].rotation, -30.0);
        assert_eq!(rig.slots[0].attachment, "closed");
        assert_eq!(
            rig.attachment("", 0, "closed").unwrap().image,
            "assets/hero/hands/closed.png"
        );
        assert_eq!(rig.ik[0].bones, vec![1]);
        let swing = rig.animation("swing").unwrap();
        assert_eq!(swing.duration, 1.0);
        assert_eq!(swing.events, vec![(0.5, "hit".to_string())]);
        // The second key holds (a null easing) until the third.
        let held = rig.sample(Some(swing), 0.75, false);
        assert!((held.bones[1].rotation - (-30.0 - 20.0)).abs() < 1e-4);
        let mid = rig.sample(Some(swing), 0.25, false);
        assert!((mid.bones[1].rotation - (-30.0 - 10.0)).abs() < 1e-4);
    }

    #[test]
    fn junk_is_refused() {
        assert!(Rig::parse("{}", "").is_err());
        assert!(Rig::parse("not json", "").is_err());
        assert!(Rig::parse(r#"{"bones":[]}"#, "").is_err());
    }
}
