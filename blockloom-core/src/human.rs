//! The third-person template's character: Quaternius's CC0 "Animated
//! Human" (see `res/models/README.md`), 1.8 m tall around its origin and
//! facing -Z, with Idle, Walk, Run, Jump, Punch and Death clips.

/// The model, as the template writes it into a new project.
pub const GLB: &[u8] = include_bytes!("../../res/models/human.glb");

/// The bones the template's Animation component names.
pub const FEET: [&str; 2] = ["LeftFoot", "RightFoot"];
pub const HIPS: &str = "Hips";
pub const HEAD: &str = "Head";
pub const RAGDOLL: [&str; 11] = [
    "Hips",
    "Spine1",
    "Head",
    "LeftArm",
    "LeftForeArm",
    "RightArm",
    "RightForeArm",
    "LeftUpLeg",
    "LeftLeg",
    "RightUpLeg",
    "RightLeg",
];
/// The root of its upper-body layer.
pub const UPPER_BODY: [&str; 1] = ["Spine"];

/// The clips and their lengths, read from the model the way import does.
pub fn clips() -> Vec<crate::pipeline::ModelClip> {
    json()
        .map(|value| crate::pipeline::model_clips(&value))
        .unwrap_or_default()
}

/// The glb's JSON chunk.
pub fn json() -> Option<serde_json::Value> {
    let length = u32::from_le_bytes(GLB.get(12..16)?.try_into().ok()?) as usize;
    serde_json::from_slice(GLB.get(20..20 + length)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_has_the_clips_and_bones_the_template_names() {
        assert_eq!(&GLB[..4], b"glTF");
        let names: Vec<_> = clips().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Idle", "Walk", "Run", "Jump", "Punch", "Death"]);
        let json = json().unwrap();
        let nodes: Vec<&str> = json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["name"].as_str())
            .collect();
        for bone in FEET
            .iter()
            .chain(&RAGDOLL)
            .chain(&UPPER_BODY)
            .chain([&HIPS, &HEAD])
        {
            assert!(nodes.contains(bone), "no bone {bone}");
        }
    }
}
