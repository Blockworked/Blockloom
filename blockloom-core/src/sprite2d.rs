//! The `Sprite` component: 2D dials over an actor's look. Flips, 9-slice
//! panels, stacked 2.5D sprites, a palette swap and an outline, and where it
//! sorts: the Render component's layer, an order inside that layer, and an
//! optional Y-sort for top-down depth.
//!
//! The runtime applies the authored spec at spawn and `set sprite [dial] to`
//! changes it on the fixed tick, the same tick in the VM and compiled logic.

use serde::{Deserialize, Serialize};

/// How a stretched part of a 9-slice panel fills its space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SliceFill {
    #[default]
    Stretch,
    Tile,
}

/// Cuts the image into nine parts: the corners never stretch, the sides
/// stretch or tile along one axis, the centre along both. `border` is left,
/// right, top, bottom, in source pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NineSlice {
    #[serde(default)]
    pub border: [f32; 4],
    #[serde(default)]
    pub center: SliceFill,
    #[serde(default)]
    pub sides: SliceFill,
    /// How far a corner may scale up when the panel is drawn larger.
    #[serde(default = "one")]
    pub max_corner_scale: f32,
}

fn one() -> f32 {
    1.0
}

/// Stacked 2.5D sprites: `layers` slices, one below another down `image`
/// (the look's own image when empty), the first slice at the bottom. Each
/// slice is drawn `offset` further along in screen space, so the stack
/// turns with the actor while it stays standing up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpriteStack {
    #[serde(default)]
    pub image: String,
    #[serde(default = "default_layers")]
    pub layers: u32,
    #[serde(default = "default_offset")]
    pub offset: [f32; 2],
}

fn default_layers() -> u32 {
    8
}

fn default_offset() -> [f32; 2] {
    [0.0, 1.0]
}

/// The 2D dials over an actor's look.
///
/// The palette swap reads `palette` as rows of colors: the sprite's red
/// channel picks the column and `palette_index` the row, and the sprite's
/// alpha is kept. The outline draws `outline_width` pixels round anything
/// opaque, in `outline_color`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpriteSpec {
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
    /// Order inside the Render layer: higher draws on top.
    #[serde(default)]
    pub order: i32,
    /// Lower on screen draws in front, within the same layer and order.
    #[serde(default)]
    pub y_sort: bool,
    #[serde(default)]
    pub slice: Option<NineSlice>,
    #[serde(default)]
    pub stack: Option<SpriteStack>,
    #[serde(default)]
    pub palette: String,
    #[serde(default)]
    pub palette_index: u32,
    #[serde(default)]
    pub outline_width: f32,
    #[serde(default = "default_outline")]
    pub outline_color: String,
    /// Extra brightness on the sprite's colors, added to 1: above zero it
    /// passes 1.0 on an HDR frame and blooms, and survives a dark ambient.
    #[serde(default)]
    pub glow: f32,
    /// Whether this actor's shape blocks 2D lights that cast shadows.
    #[serde(default)]
    pub casts_shadow: bool,
}

fn default_outline() -> String {
    "#000000".to_string()
}

impl Default for SpriteSpec {
    fn default() -> Self {
        Self {
            flip_x: false,
            flip_y: false,
            order: 0,
            y_sort: false,
            slice: None,
            stack: None,
            palette: String::new(),
            palette_index: 0,
            outline_width: 0.0,
            outline_color: default_outline(),
            glow: 0.0,
            casts_shadow: false,
        }
    }
}

/// Orders in a layer run from `-ORDER_LIMIT` to `ORDER_LIMIT`.
pub const ORDER_LIMIT: i32 = 40;
/// Depth between two orders: forty either way stays inside one layer.
pub const ORDER_STEP: f32 = 0.01;
/// Most depth Y-sort may add either way. Inside half an order step, so the
/// order always wins over height.
pub const Y_SORT_LIMIT: f32 = 0.004;
/// Most pixels an outline may be.
pub const OUTLINE_LIMIT: f32 = 16.0;

impl SpriteSpec {
    pub fn normalize(&mut self) {
        self.order = self.order.clamp(-ORDER_LIMIT, ORDER_LIMIT);
        if let Some(slice) = &mut self.slice {
            for inset in &mut slice.border {
                if !inset.is_finite() || *inset < 0.0 {
                    *inset = 0.0;
                }
            }
            if !slice.max_corner_scale.is_finite() || slice.max_corner_scale <= 0.0 {
                slice.max_corner_scale = 1.0;
            }
        }
        if let Some(stack) = &mut self.stack {
            stack.image = stack.image.trim().to_string();
            stack.layers = stack.layers.clamp(1, 128);
            for value in &mut stack.offset {
                if !value.is_finite() {
                    *value = 0.0;
                }
            }
        }
        self.palette = self.palette.trim().to_string();
        self.palette_index = self.palette_index.min(255);
        self.outline_width = clamp_outline(self.outline_width);
        self.glow = if self.glow.is_finite() {
            self.glow.clamp(0.0, 64.0)
        } else {
            0.0
        };
        self.outline_color = self.outline_color.trim().to_string();
        if self.outline_color.is_empty() {
            self.outline_color = default_outline();
        }
    }

    /// True when the sprite needs the effect shader: a palette, an outline
    /// or a glow.
    pub fn needs_effect(&self) -> bool {
        !self.palette.is_empty() || self.outline_width > 0.0 || self.glow > 0.0
    }
}

pub fn clamp_outline(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(0.0, OUTLINE_LIMIT)
    } else {
        0.0
    }
}

/// Depth an order adds on top of placement z plus layer.
pub fn order_depth(order: i32) -> f32 {
    order.clamp(-ORDER_LIMIT, ORDER_LIMIT) as f32 * ORDER_STEP
}

/// Y-sort depth for sprites sharing one layer and order, by rank rather than
/// raw height, so any spread of heights spans the whole band. Lower on
/// screen is nearer; equal heights tie.
pub fn y_sort_depths(heights: &[f32]) -> Vec<f32> {
    // `+ 0.0` folds -0 into 0 for `total_cmp`.
    let mut sorted: Vec<f32> = heights
        .iter()
        .filter(|h| h.is_finite())
        .map(|h| h + 0.0)
        .collect();
    sorted.sort_by(|a, b| b.total_cmp(a));
    sorted.dedup();
    let step = if sorted.len() > 1 {
        2.0 * Y_SORT_LIMIT / (sorted.len() - 1) as f32
    } else {
        0.0
    };
    let base = if sorted.len() > 1 { -Y_SORT_LIMIT } else { 0.0 };
    heights
        .iter()
        .map(
            |h| match sorted.binary_search_by(|probe| (h + 0.0).total_cmp(probe)) {
                Ok(rank) if h.is_finite() => base + rank as f32 * step,
                _ => 0.0,
            },
        )
        .collect()
}

/// Where stack slice `index` sits relative to the actor, given the actor's
/// rotation in radians: its offset is screen-space, so it is turned back.
pub fn stack_slice_offset(stack: &SpriteStack, index: u32, rotation: f32) -> [f32; 2] {
    let [x, y] = [
        stack.offset[0] * index as f32,
        stack.offset[1] * index as f32,
    ];
    let (sin, cos) = (-rotation).sin_cos();
    [x * cos - y * sin, x * sin + y * cos]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_outranks_height_and_both_stay_in_a_layer() {
        let depths = y_sort_depths(&[5000.0, -5000.0]);
        assert!(order_depth(1) + depths[0] > order_depth(0) + depths[1]);
        assert!(order_depth(ORDER_LIMIT + 10) + Y_SORT_LIMIT < 0.5);
        assert!(order_depth(-ORDER_LIMIT - 10) - Y_SORT_LIMIT > -0.5);
    }

    #[test]
    fn y_sort_ranks_any_spread_of_heights() {
        // Far apart and close together sort the same way.
        let depths = y_sort_depths(&[1e6, -1e6, 1e6 + 1.0, 0.5, 0.5, f32::NAN]);
        assert!(depths[1] > depths[3] && depths[3] > depths[0] && depths[0] > depths[2]);
        assert_eq!(depths[3], depths[4]);
        assert_eq!(depths[5], 0.0);
        assert_eq!(depths[1], Y_SORT_LIMIT);
        assert_eq!(depths[2], -Y_SORT_LIMIT);
        assert_eq!(y_sort_depths(&[42.0]), vec![0.0]);
        // Adjacent ranks stay apart at a distant layer's z.
        let many: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let depths = y_sort_depths(&many);
        let z = 50.0f32;
        assert!((1..1000).all(|i| z + depths[i] < z + depths[i - 1]));
    }

    #[test]
    fn stacked_slices_stay_up_the_screen_when_turned() {
        let stack = SpriteStack {
            image: String::new(),
            layers: 4,
            offset: [0.0, 1.0],
        };
        let turned = stack_slice_offset(&stack, 3, std::f32::consts::FRAC_PI_2);
        // A quarter turn left: screen-up is the actor's +x.
        assert!(
            (turned[0] - 3.0).abs() < 1e-5 && turned[1].abs() < 1e-5,
            "{turned:?}"
        );
    }

    #[test]
    fn normalize_clamps_and_old_documents_load() {
        let mut spec: SpriteSpec =
            serde_json::from_str(r#"{"order": 500, "outline_width": 99}"#).unwrap();
        spec.normalize();
        assert_eq!(spec.order, ORDER_LIMIT);
        assert_eq!(spec.outline_width, OUTLINE_LIMIT);
        assert!(spec.needs_effect());
        assert!(!SpriteSpec::default().needs_effect());
    }
}
