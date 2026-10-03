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
    /// Whether the mesh is solid: a fixed collider over exactly these
    /// triangles.
    #[serde(default)]
    pub collider: bool,
    /// The shape of that collider; a trimesh unless the plugin says otherwise.
    #[serde(default)]
    pub collider_kind: ColliderKind,
    /// Compute output: packed position, normal, rgba (10 f32 words per vertex).
    /// CPU arrays remain the collision mesh and the rendering fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuVertices>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<MeshBody>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuVertices {
    pub buffer: String,
    pub vertices: u32,
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
        if name.is_empty() {
            return Err("a mesh needs a name".to_string());
        }
        if let Some(gpu) = &self.gpu {
            crate::id::validate_type_id(&gpu.buffer)?;
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
            collider: false,
            collider_kind: ColliderKind::Trimesh,
            gpu: None,
            body: None,
        }
    }

    #[test]
    fn a_consistent_mesh_passes() {
        assert_eq!(triangle().check(), Ok(()));
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
