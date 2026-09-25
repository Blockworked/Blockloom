//! Small behavior trees for actors that navigate without a block strand.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "node")]
pub enum BehaviorNode {
    Selector { children: Vec<BehaviorNode> },
    Sequence { children: Vec<BehaviorNode> },
    CanSeeTarget,
    TargetNear { distance: f32 },
    NavigateToTarget,
    FleeTarget,
    Idle,
}

impl Default for BehaviorNode {
    fn default() -> Self {
        Self::Selector {
            children: vec![
                Self::Sequence {
                    children: vec![Self::CanSeeTarget, Self::NavigateToTarget],
                },
                Self::Idle,
            ],
        }
    }
}

/// All distances are world units. Sight and separation are evaluated against
/// the live actor positions each fixed step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainSpec {
    /// Actor id or name to sense and follow.
    #[serde(default)]
    pub target: String,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default = "default_sight")]
    pub sight: f32,
    /// Full field of view in degrees; 360 sees in every direction.
    #[serde(default = "default_fov")]
    pub fov: f32,
    /// Radius for avoiding other moving actors.
    #[serde(default = "default_separation")]
    pub separation: f32,
    /// Bit mask used by navigation areas and links.
    #[serde(default = "default_layer")]
    pub layer: u32,
    #[serde(default)]
    pub tree: BehaviorNode,
}

fn default_speed() -> f32 {
    4.0
}
fn default_sight() -> f32 {
    12.0
}
fn default_fov() -> f32 {
    120.0
}
fn default_separation() -> f32 {
    1.0
}
fn default_layer() -> u32 {
    1
}

impl Default for BrainSpec {
    fn default() -> Self {
        Self {
            target: String::new(),
            speed: default_speed(),
            sight: default_sight(),
            fov: default_fov(),
            separation: default_separation(),
            layer: default_layer(),
            tree: BehaviorNode::default(),
        }
    }
}
