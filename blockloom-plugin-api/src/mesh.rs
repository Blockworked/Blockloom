//! A mesh a plugin asks the world to draw: the mesh submission service.
//!
//! Plain flat arrays, so a module of any tier can build one without a Bevy
//! type in sight. The world owns what the mesh becomes (an entity, a GPU
//! buffer, an optional collider); the plugin names it and can replace or
//! remove it by that name.

use serde::{Deserialize, Serialize};

/// The most vertices one submitted mesh may hold.
pub const MAX_VERTICES: usize = 262_144;

/// What a solid mesh collides as. A trimesh is exact and for fixed things; a
/// convex hull or a box is cheaper and what a moving thing wants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColliderKind {
    #[default]
    Trimesh,
    ConvexHull,
    /// The mesh's axis-aligned bounding box.
    Aabb,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshData {
    /// Names the mesh within its plugin; a later mesh with the same name
    /// replaces this one.
    pub name: String,
    /// x, y, z per vertex, in the mesh's own frame.
    pub positions: Vec<f32>,
    /// x, y, z per vertex, unit length.
    pub normals: Vec<f32>,
    /// Linear r, g, b, a per vertex, multiplied into the surface color.
    pub colors: Vec<f32>,
    /// Three per triangle, counter-clockwise from the front.
    pub indices: Vec<u32>,
    /// Where the mesh's origin stands in the world.
    #[serde(default)]
    pub origin: [f32; 3],
    /// Scene-linear HDR emission of the whole mesh; `None` for a lit surface
    /// that glows with nothing.
    #[serde(default)]
    pub emission: Option<[f32; 3]>,
    /// The surface's roughness.
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    /// Dither crossfade duration for replacing or retiring visual-only meshes.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub transition_ms: u16,
    /// Whether the mesh is solid: a fixed collider over exactly these
    /// triangles.
    #[serde(default)]
    pub collider: bool,
    /// The shape of that collider; a trimesh unless the plugin says otherwise.
    #[serde(default)]
    pub collider_kind: ColliderKind,
    /// x, y per vertex, into `texture` when one is set. Empty when untextured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uvs: Vec<f32>,
    /// Project asset path of the texture the uvs read (for example
    /// `assets/textures/grass_top.png`). The world loads it as the
    /// surface's base color map; without it the vertex colors draw alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    /// Compute triangle output or persistent compact quads for direct rendering.
    /// CPU arrays remain the collision mesh and the rendering fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuVertices>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<MeshBody>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuVertices {
    /// Compute buffer name; ignored when `quads` supplies persistent records.
    pub buffer: String,
    pub vertices: u32,
    /// Persistent quad records for direct raster decoding; CPU arrays still collide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quads: Option<CompactQuads>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Two words per axis-aligned quad in a tile of at most 128 cells. Word zero
/// packs plane/u/v in three bytes, then axis*2+positive in three bits. Word one
/// packs width/height/palette index in three bytes; coordinates scale by `voxel`.
pub struct CompactQuads {
    pub records: Vec<[u32; 2]>,
    pub palette: Vec<[f32; 4]>,
    pub voxel: f32,
}

/// A movable mesh. Fixed meshes omit this field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshBody {
    pub mass: f32,
    #[serde(default)]
    pub velocity: [f32; 3],
    #[serde(default)]
    pub angular_velocity: [f32; 3],
    #[serde(default = "identity_rotation")]
    pub rotation: [f32; 4],
}

fn is_zero(value: &u16) -> bool {
    *value == 0
}

fn identity_rotation() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

fn default_roughness() -> f32 {
    0.9
}

impl MeshData {
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Whether the arrays agree with each other and the world can draw them.
    pub fn check(&self) -> Result<(), String> {
        let name = &self.name;
        if self.transition_ms > 500
            || (self.transition_ms > 0 && (self.collider || self.body.is_some()))
        {
            return Err(format!(
                "mesh {name}: transitions require a visual-only mesh and at most 500 ms"
            ));
        }
        if name.is_empty() {
            return Err("a mesh needs a name".to_string());
        }
        if let Some(gpu) = &self.gpu {
            crate::id::validate_type_id(&gpu.buffer)?;
            if let Some(quads) = &gpu.quads {
                if quads.records.len() * 6 != gpu.vertices as usize
                    || quads.palette.is_empty()
                    || quads.palette.len() > 256
                    || !quads.voxel.is_finite()
                    || quads.voxel <= 0.0
                    || quads.palette.iter().flatten().any(|v| !v.is_finite())
                    || quads.records.iter().any(|[r, m]| {
                        r >> 27 != 0
                            || m >> 24 != 0
                            || (r >> 24) & 7 > 5
                            || [0, 8, 16].iter().any(|s| (r >> s) & 255 > 128)
                            || [0, 8]
                                .iter()
                                .any(|s| !(1..=128).contains(&((m >> s) & 255)))
                            || ((r >> 8) & 255) + (m & 255) > 128
                            || ((r >> 16) & 255) + ((m >> 8) & 255) > 128
                            || ((m >> 16) & 255) as usize >= quads.palette.len()
                    })
                {
                    return Err(format!("mesh {name}: invalid compact quads"));
                }
            }
            if gpu.vertices as usize > MAX_VERTICES
                || (gpu.vertices as usize) < self.indices.len()
                || gpu.vertices == 0
                || gpu.vertices % 3 != 0
            {
                return Err(format!("mesh {name}: invalid GPU vertex capacity"));
            }
        }
        if let Some(body) = &self.body
            && (!body.mass.is_finite()
                || body.mass <= 0.0
                || body
                    .velocity
                    .iter()
                    .chain(&body.angular_velocity)
                    .any(|v| !v.is_finite())
                || body.rotation.iter().any(|v| !v.is_finite())
                || (body.rotation.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() > 1e-3
                || self.collider_kind == ColliderKind::Trimesh
                || !self.collider)
        {
            return Err(format!(
                "mesh {name}: a moving mesh needs positive mass and a convex collider"
            ));
        }
        let count = self.vertex_count();
        if !self.positions.len().is_multiple_of(3) {
            return Err(format!("mesh {name}: positions are not a multiple of 3"));
        }
        if count > MAX_VERTICES {
            return Err(format!(
                "mesh {name}: {count} vertices, at most {MAX_VERTICES}"
            ));
        }
        if self.normals.len() != count * 3 {
            return Err(format!("mesh {name}: one normal per vertex"));
        }
        if self.colors.len() != count * 4 {
            return Err(format!("mesh {name}: one rgba color per vertex"));
        }
        if !self.uvs.is_empty() && self.uvs.len() != count * 2 {
            return Err(format!("mesh {name}: one uv per vertex"));
        }
        if self.texture.is_some() && self.uvs.is_empty() {
            return Err(format!("mesh {name}: a texture needs uvs"));
        }
        if let Some(texture) = &self.texture
            && (texture.is_empty() || texture.len() > 256)
        {
            return Err(format!(
                "mesh {name}: a texture path needs 1 to 256 characters"
            ));
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(format!("mesh {name}: indices are not a multiple of 3"));
        }
        if let Some(bad) = self.indices.iter().find(|&&i| i as usize >= count) {
            return Err(format!("mesh {name}: index {bad} is past {count} vertices"));
        }
        let finite = self
            .positions
            .iter()
            .chain(&self.normals)
            .chain(&self.colors)
            .chain(&self.uvs)
            .chain(&self.origin)
            .chain(self.emission.iter().flatten())
            .chain([&self.roughness])
            .all(|v| v.is_finite());
        if !finite {
            return Err(format!("mesh {name}: a value is not finite"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> MeshData {
        MeshData {
            name: "t".into(),
            positions: vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            normals: vec![0., 0., 1., 0., 0., 1., 0., 0., 1.],
            colors: vec![1.; 12],
            indices: vec![0, 1, 2],
            origin: [0.; 3],
            emission: None,
            roughness: 0.9,
            transition_ms: 0,
            collider: false,
            collider_kind: ColliderKind::Trimesh,
            uvs: Vec::new(),
            texture: None,
            gpu: None,
            body: None,
        }
    }

    #[test]
    fn textured_meshes_carry_uvs_and_a_path() {
        let mut mesh = triangle();
        mesh.uvs = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        mesh.texture = Some("assets/textures/grass.png".into());
        assert!(mesh.check().is_ok());
        mesh.uvs.pop();
        assert!(mesh.check().is_err());
    }

    #[test]
    fn a_consistent_mesh_passes() {
        assert_eq!(triangle().check(), Ok(()));
    }

    #[test]
    fn quad_packets_and_visual_transition_limits_are_checked() {
        let mut mesh = triangle();
        mesh.gpu = Some(GpuVertices {
            buffer: "quads".into(),
            vertices: 6,
            quads: Some(CompactQuads {
                records: vec![[0, 1 | 1 << 8]],
                palette: vec![[1.0; 4]],
                voxel: 1.0,
            }),
        });
        assert!(mesh.check().is_ok());
        let mut bad = mesh.clone();
        bad.gpu.as_mut().unwrap().quads.as_mut().unwrap().records[0][0] |= 128 << 8;
        assert!(bad.check().is_err());
        let mut bad = mesh.clone();
        bad.gpu.as_mut().unwrap().quads.as_mut().unwrap().records[0][1] |= 1 << 16;
        assert!(bad.check().is_err());
        mesh.transition_ms = 150;
        assert!(mesh.check().is_ok());
        mesh.collider = true;
        assert!(mesh.check().is_err());
        mesh.collider = false;
        mesh.transition_ms = 501;
        assert!(mesh.check().is_err());
    }

    #[test]
    fn mismatched_arrays_are_named() {
        let mut mesh = triangle();
        mesh.colors.pop();
        assert!(mesh.check().unwrap_err().contains("rgba"));
        let mut mesh = triangle();
        mesh.indices[2] = 3;
        assert!(mesh.check().unwrap_err().contains("past 3"));
        let mut mesh = triangle();
        mesh.positions[1] = f32::NAN;
        assert!(mesh.check().unwrap_err().contains("finite"));
        let mut mesh = triangle();
        mesh.name.clear();
        assert!(mesh.check().is_err());
    }

    #[test]
    fn compute_capacity_and_moving_body_fields_are_checked() {
        let mut mesh = triangle();
        mesh.gpu = Some(GpuVertices {
            buffer: "vertices".into(),
            vertices: 2,
            quads: None,
        });
        assert!(mesh.check().is_err());
        mesh.gpu.as_mut().unwrap().vertices = 3;
        assert!(mesh.check().is_ok());
        mesh.body = Some(MeshBody {
            mass: 1.0,
            velocity: [0.; 3],
            angular_velocity: [0.; 3],
            rotation: [0., 0., 0., 1.],
        });
        assert!(mesh.check().is_err());
        mesh.collider = true;
        mesh.collider_kind = ColliderKind::ConvexHull;
        assert!(mesh.check().is_ok());
        mesh.body.as_mut().unwrap().rotation = [0.; 4];
        assert!(mesh.check().is_err());
    }

    #[test]
    fn it_reads_from_json_with_defaults() {
        let mesh: MeshData = serde_json::from_value(serde_json::json!({
            "name": "t",
            "positions": [0, 0, 0, 1, 0, 0, 0, 1, 0],
            "normals": [0, 0, 1, 0, 0, 1, 0, 0, 1],
            "colors": [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            "indices": [0, 1, 2],
        }))
        .unwrap();
        assert_eq!(mesh.roughness, 0.9);
        assert!(!mesh.collider && mesh.emission.is_none());
        assert_eq!(mesh.check(), Ok(()));
    }
}
