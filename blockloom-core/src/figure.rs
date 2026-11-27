//! A blocky rigged figure, written as a `.glb` from code: a skinned mesh of
//! boxes (each rigidly on one bone) with Idle, Walk, Run and Jump clips. The
//! third-person sample ships it, so the template needs no outside asset and
//! tests can make it again. It stands 1.8 m tall around its origin (feet at
//! -0.9) and faces -Z, the way a 3D actor does.

use serde_json::{Value, json};

/// A bone: name, parent index, translation from the parent, and its box
/// (min, max) in the bone's own frame.
struct Bone {
    name: &'static str,
    parent: Option<usize>,
    at: [f32; 3],
    min: [f32; 3],
    max: [f32; 3],
}

const BONES: &[Bone] = &[
    Bone {
        name: "Hips",
        parent: None,
        at: [0.0, 0.0, 0.0],
        min: [-0.18, -0.1, -0.1],
        max: [0.18, 0.15, 0.1],
    },
    Bone {
        name: "Spine",
        parent: Some(0),
        at: [0.0, 0.15, 0.0],
        min: [-0.2, 0.0, -0.12],
        max: [0.2, 0.5, 0.12],
    },
    Bone {
        name: "Head",
        parent: Some(1),
        at: [0.0, 0.5, 0.0],
        min: [-0.12, 0.0, -0.12],
        max: [0.12, 0.25, 0.12],
    },
    Bone {
        name: "ArmL",
        parent: Some(1),
        at: [0.26, 0.45, 0.0],
        min: [-0.06, -0.6, -0.06],
        max: [0.06, 0.0, 0.06],
    },
    Bone {
        name: "ArmR",
        parent: Some(1),
        at: [-0.26, 0.45, 0.0],
        min: [-0.06, -0.6, -0.06],
        max: [0.06, 0.0, 0.06],
    },
    Bone {
        name: "ThighL",
        parent: Some(0),
        at: [0.11, -0.05, 0.0],
        min: [-0.07, -0.4, -0.07],
        max: [0.07, 0.0, 0.07],
    },
    Bone {
        name: "ShinL",
        parent: Some(5),
        at: [0.0, -0.4, 0.0],
        min: [-0.06, -0.4, -0.06],
        max: [0.06, 0.0, 0.06],
    },
    Bone {
        name: "FootL",
        parent: Some(6),
        at: [0.0, -0.4, 0.0],
        min: [-0.06, -0.05, -0.18],
        max: [0.06, 0.03, 0.05],
    },
    Bone {
        name: "ThighR",
        parent: Some(0),
        at: [-0.11, -0.05, 0.0],
        min: [-0.07, -0.4, -0.07],
        max: [0.07, 0.0, 0.07],
    },
    Bone {
        name: "ShinR",
        parent: Some(8),
        at: [0.0, -0.4, 0.0],
        min: [-0.06, -0.4, -0.06],
        max: [0.06, 0.0, 0.06],
    },
    Bone {
        name: "FootR",
        parent: Some(9),
        at: [0.0, -0.4, 0.0],
        min: [-0.06, -0.05, -0.18],
        max: [0.06, 0.03, 0.05],
    },
];

/// The bones the figure names, for an Animation component's IK and ragdoll.
pub const FEET: [&str; 2] = ["FootL", "FootR"];
pub const HIPS: &str = "Hips";
pub const HEAD: &str = "Head";
pub const RAGDOLL: [&str; 9] = [
    "Hips", "Spine", "Head", "ArmL", "ArmR", "ThighL", "ShinL", "ThighR", "ShinR",
];
/// The bones of its upper-body layer.
pub const UPPER_BODY: [&str; 1] = ["Spine"];

/// A clip: name, length, and per bone a list of (time, degrees about X).
struct Clip {
    name: &'static str,
    seconds: f32,
    keys: Vec<(&'static str, Vec<(f32, f32)>)>,
}

/// A swing: degrees at the quarter points of a cycle `seconds` long.
fn swing(seconds: f32, degrees: f32, phase: f32) -> Vec<(f32, f32)> {
    (0..=4)
        .map(|i| {
            let t = i as f32 / 4.0;
            let angle = ((t + phase) * std::f32::consts::TAU).sin() * degrees;
            (t * seconds, angle)
        })
        .collect()
}

fn clips() -> Vec<Clip> {
    let walk = 1.0;
    let run = 0.6;
    vec![
        Clip {
            name: "Idle",
            seconds: 2.0,
            keys: vec![
                ("Spine", vec![(0.0, 0.0), (1.0, 3.0), (2.0, 0.0)]),
                ("ArmL", swing(2.0, 3.0, 0.0)),
                ("ArmR", swing(2.0, 3.0, 0.5)),
            ],
        },
        Clip {
            name: "Walk",
            seconds: walk,
            keys: vec![
                ("ThighL", swing(walk, 25.0, 0.0)),
                ("ThighR", swing(walk, 25.0, 0.5)),
                (
                    "ShinL",
                    swing(walk, 12.0, 0.25)
                        .into_iter()
                        .map(|(t, a)| (t, -a.abs()))
                        .collect(),
                ),
                (
                    "ShinR",
                    swing(walk, 12.0, 0.75)
                        .into_iter()
                        .map(|(t, a)| (t, -a.abs()))
                        .collect(),
                ),
                ("ArmL", swing(walk, 20.0, 0.5)),
                ("ArmR", swing(walk, 20.0, 0.0)),
            ],
        },
        Clip {
            name: "Run",
            seconds: run,
            keys: vec![
                ("Spine", vec![(0.0, 10.0), (run, 10.0)]),
                ("ThighL", swing(run, 45.0, 0.0)),
                ("ThighR", swing(run, 45.0, 0.5)),
                (
                    "ShinL",
                    swing(run, 40.0, 0.25)
                        .into_iter()
                        .map(|(t, a)| (t, -a.abs()))
                        .collect(),
                ),
                (
                    "ShinR",
                    swing(run, 40.0, 0.75)
                        .into_iter()
                        .map(|(t, a)| (t, -a.abs()))
                        .collect(),
                ),
                ("ArmL", swing(run, 40.0, 0.5)),
                ("ArmR", swing(run, 40.0, 0.0)),
            ],
        },
        Clip {
            name: "Jump",
            seconds: 0.8,
            keys: vec![
                ("ThighL", vec![(0.0, 0.0), (0.2, 50.0), (0.8, 30.0)]),
                ("ThighR", vec![(0.0, 0.0), (0.2, 40.0), (0.8, 20.0)]),
                ("ShinL", vec![(0.0, 0.0), (0.2, -80.0), (0.8, -50.0)]),
                ("ShinR", vec![(0.0, 0.0), (0.2, -70.0), (0.8, -40.0)]),
                ("ArmL", vec![(0.0, 0.0), (0.2, -150.0), (0.8, -120.0)]),
                ("ArmR", vec![(0.0, 0.0), (0.2, -150.0), (0.8, -120.0)]),
            ],
        },
    ]
}

/// The clips and their lengths, as `pipeline::model_clips` would read them.
pub fn clip_lengths() -> Vec<(&'static str, f32)> {
    clips()
        .iter()
        .map(|clip| (clip.name, clip.seconds))
        .collect()
}

/// Appends little-endian floats or shorts to the binary chunk, 4-aligned,
/// answering the buffer view's index.
struct Bin {
    bytes: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn view(&mut self, data: &[u8], target: Option<u32>) -> usize {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        let mut view = json!({
            "buffer": 0,
            "byteOffset": self.bytes.len(),
            "byteLength": data.len(),
        });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.bytes.extend_from_slice(data);
        self.views.push(view);
        self.views.len() - 1
    }

    fn floats(&mut self, values: &[f32], kind: &str, target: Option<u32>, bounds: bool) -> usize {
        let width = match kind {
            "SCALAR" => 1,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => 16,
        };
        let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&data, target);
        let mut accessor = json!({
            "bufferView": view,
            "componentType": 5126,
            "count": values.len() / width,
            "type": kind,
        });
        if bounds {
            let column = |c: usize| values.iter().skip(c).step_by(width).copied();
            let min: Vec<f32> = (0..width)
                .map(|c| column(c).fold(f32::MAX, f32::min))
                .collect();
            let max: Vec<f32> = (0..width)
                .map(|c| column(c).fold(f32::MIN, f32::max))
                .collect();
            accessor["min"] = json!(min);
            accessor["max"] = json!(max);
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn shorts(&mut self, values: &[u16], kind: &str, target: Option<u32>) -> usize {
        let width = if kind == "VEC4" { 4 } else { 1 };
        let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&data, target);
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": 5123,
            "count": values.len() / width,
            "type": kind,
        }));
        self.accessors.len() - 1
    }
}

/// Each bone's position in the figure's frame, at rest.
fn rest_positions() -> Vec<[f32; 3]> {
    let mut out: Vec<[f32; 3]> = Vec::with_capacity(BONES.len());
    for bone in BONES {
        let base = bone.parent.map_or([0.0; 3], |p| out[p]);
        out.push([
            base[0] + bone.at[0],
            base[1] + bone.at[1],
            base[2] + bone.at[2],
        ]);
    }
    out
}

/// Positions, normals, joints, weights and indices.
type Buffers = (Vec<f32>, Vec<f32>, Vec<u16>, Vec<f32>, Vec<u16>);
/// A face's normal and its corners, as which-extreme flags per axis.
type Face = ([f32; 3], [(bool, bool, bool); 4]);

/// A box's 24 corners (four per face, for flat normals) and 36 indices.
fn push_box(bone: usize, origin: [f32; 3], min: [f32; 3], max: [f32; 3], mesh: &mut Buffers) {
    let (positions, normals, joints, weights, indices) = mesh;
    let corner = |x: bool, y: bool, z: bool| {
        [
            origin[0] + if x { max[0] } else { min[0] },
            origin[1] + if y { max[1] } else { min[1] },
            origin[2] + if z { max[2] } else { min[2] },
        ]
    };
    // Each face: normal, then four corners counterclockwise seen from outside.
    let faces: [Face; 6] = [
        (
            [1.0, 0.0, 0.0],
            [
                (true, false, true),
                (true, false, false),
                (true, true, false),
                (true, true, true),
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                (false, false, false),
                (false, false, true),
                (false, true, true),
                (false, true, false),
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                (false, true, true),
                (true, true, true),
                (true, true, false),
                (false, true, false),
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                (false, false, false),
                (true, false, false),
                (true, false, true),
                (false, false, true),
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [
                (false, false, true),
                (true, false, true),
                (true, true, true),
                (false, true, true),
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                (true, false, false),
                (false, false, false),
                (false, true, false),
                (true, true, false),
            ],
        ),
    ];
    for (normal, corners) in faces {
        let first = (positions.len() / 3) as u16;
        for (x, y, z) in corners {
            positions.extend(corner(x, y, z));
            normals.extend(normal);
            joints.extend([bone as u16, 0, 0, 0]);
            weights.extend([1.0, 0.0, 0.0, 0.0]);
        }
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
}

/// The figure as `.glb` bytes.
pub fn glb() -> Vec<u8> {
    let mut bin = Bin {
        bytes: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let rest = rest_positions();
    let mut mesh = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, bone) in BONES.iter().enumerate() {
        push_box(i, rest[i], bone.min, bone.max, &mut mesh);
    }
    let (positions, normals, joints, weights, indices) = mesh;
    let position = bin.floats(&positions, "VEC3", Some(34962), true);
    let normal = bin.floats(&normals, "VEC3", Some(34962), false);
    let joint = bin.shorts(&joints, "VEC4", Some(34962));
    let weight = bin.floats(&weights, "VEC4", Some(34962), false);
    let index = bin.shorts(&indices, "SCALAR", Some(34963));
    // Inverse binds: each joint at rest is a plain translation.
    let ibms: Vec<f32> = rest
        .iter()
        .flat_map(|p| {
            [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -p[0], -p[1], -p[2],
                1.0,
            ]
        })
        .collect();
    let ibm = bin.floats(&ibms, "MAT4", None, false);

    // Node 0 is the mesh, 1.. the bones.
    let mut nodes = vec![json!({"name": "Figure", "mesh": 0, "skin": 0})];
    for (i, bone) in BONES.iter().enumerate() {
        let kids: Vec<usize> = BONES
            .iter()
            .enumerate()
            .filter(|(_, b)| b.parent == Some(i))
            .map(|(j, _)| j + 1)
            .collect();
        let mut node = json!({"name": bone.name, "translation": bone.at});
        if !kids.is_empty() {
            node["children"] = json!(kids);
        }
        nodes.push(node);
    }

    let mut animations = Vec::new();
    for clip in clips() {
        let mut samplers = Vec::new();
        let mut channels = Vec::new();
        for (bone, keys) in &clip.keys {
            let node = BONES
                .iter()
                .position(|b| b.name == *bone)
                .expect("a figure bone")
                + 1;
            let times: Vec<f32> = keys.iter().map(|(t, _)| *t).collect();
            let rotations: Vec<f32> = keys
                .iter()
                .flat_map(|(_, degrees)| {
                    let half = degrees.to_radians() / 2.0;
                    [half.sin(), 0.0, 0.0, half.cos()]
                })
                .collect();
            let input = bin.floats(&times, "SCALAR", None, true);
            let output = bin.floats(&rotations, "VEC4", None, false);
            channels.push(json!({
                "sampler": samplers.len(),
                "target": {"node": node, "path": "rotation"},
            }));
            samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
        }
        animations.push(json!({"name": clip.name, "samplers": samplers, "channels": channels}));
    }
    while !bin.bytes.len().is_multiple_of(4) {
        bin.bytes.push(0);
    }

    let document = json!({
        "asset": {"version": "2.0", "generator": "Blockloom figure"},
        "scene": 0,
        "scenes": [{"name": "Figure", "nodes": [0, 1]}],
        "nodes": nodes,
        "meshes": [{
            "name": "Figure",
            "primitives": [{
                "attributes": {
                    "POSITION": position,
                    "NORMAL": normal,
                    "JOINTS_0": joint,
                    "WEIGHTS_0": weight,
                },
                "indices": index,
                "material": 0,
            }],
        }],
        "materials": [{
            "name": "Figure",
            "pbrMetallicRoughness": {"baseColorFactor": [0.85, 0.55, 0.3, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.8},
        }],
        "skins": [{
            "inverseBindMatrices": ibm,
            "joints": (1..=BONES.len()).collect::<Vec<_>>(),
            "skeleton": 1,
        }],
        "animations": animations,
        "accessors": bin.accessors,
        "bufferViews": bin.views,
        "buffers": [{"byteLength": bin.bytes.len()}],
    });
    let mut text = serde_json::to_vec(&document).expect("the figure's JSON writes");
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + bin.bytes.len();
    let mut out = Vec::with_capacity(total);
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(text);
    out.extend((bin.bytes.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin.bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_figure_reads_back_as_a_skinned_model_with_its_clips() {
        let bytes = glb();
        let info = crate::pipeline::inspect_model("figure.glb", &bytes).unwrap();
        let clips: Vec<(String, f32)> = info
            .clips
            .iter()
            .map(|clip| (clip.name.clone(), clip.seconds))
            .collect();
        let wanted: Vec<(String, f32)> = clip_lengths()
            .into_iter()
            .map(|(name, seconds)| (name.to_string(), seconds))
            .collect();
        assert_eq!(clips, wanted);
        // The feet's soles sit at -0.9 and the head's top at 0.9.
        let rest = rest_positions();
        let foot = BONES.iter().position(|b| b.name == "FootL").unwrap();
        assert!((rest[foot][1] + BONES[foot].min[1] + 0.9).abs() < 1e-5);
        let head = BONES.iter().position(|b| b.name == HEAD).unwrap();
        assert!((rest[head][1] + BONES[head].max[1] - 0.9).abs() < 1e-5);
        for name in FEET
            .iter()
            .chain(&RAGDOLL)
            .chain(&UPPER_BODY)
            .chain([&HIPS])
        {
            assert!(BONES.iter().any(|b| b.name == *name), "{name}");
        }
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
    }
}
