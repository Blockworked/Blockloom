//! Shared render assets and measurements for the game world.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use blockloom_core::scene::Visual;
use std::collections::HashMap;

/// Minimum pixel bytes for the Game view's scratch target and image ring.
#[derive(Resource, Default)]
pub struct GameViewTargetBytes(pub u64);

#[derive(SystemParam)]
pub struct PerformanceStores<'w> {
    pub cache: ResMut<'w, RenderCache>,
    pub cells: ResMut<'w, crate::streaming::StreamingCells>,
    pub warmup: Option<ResMut<'w, crate::streaming::Warmup>>,
}

/// Shared handles let Bevy batch repeated opaque meshes into instanced draws.
#[derive(Resource, Default)]
pub struct RenderCache {
    meshes: HashMap<MeshKey, Handle<Mesh>>,
    scaled_meshes: HashMap<MeshKey, Handle<Mesh>>,
    lod_meshes: HashMap<(MeshKey, u8), Handle<Mesh>>,
    materials: HashMap<String, Handle<StandardMaterial>>,
    box_materials: HashMap<String, Handle<crate::materials::BoxMaterial>>,
    instanced: HashMap<String, Handle<crate::batching::InstancedMaterial>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MeshKey {
    Cuboid([u32; 3]),
    Sphere(u32),
    Capsule(u32, u32),
    Plane([u32; 2]),
    Model([u32; 3]),
}

impl MeshKey {
    fn of(visual: &Visual) -> Option<Self> {
        match visual {
            Visual::Cuboid { size, .. } => Some(Self::Cuboid(size.map(f32::to_bits))),
            Visual::Sphere { radius, .. } => Some(Self::Sphere(radius.to_bits())),
            Visual::Capsule { radius, height, .. } => {
                Some(Self::Capsule(radius.to_bits(), height.to_bits()))
            }
            Visual::Plane { size, .. } => Some(Self::Plane(size.map(f32::to_bits))),
            Visual::Model { scale, .. } => Some(Self::Model(scale.map(f32::to_bits))),
            _ => None,
        }
    }
}

impl RenderCache {
    pub fn scaled_mesh(
        &mut self,
        visual: &Visual,
        make: impl FnOnce() -> Option<Mesh>,
        meshes: &mut Assets<Mesh>,
    ) -> Option<Handle<Mesh>> {
        let key = MeshKey::of(visual)?;
        if let Some(handle) = self.scaled_meshes.get(&key) {
            return Some(handle.clone());
        }
        let handle = meshes.add(make()?);
        self.scaled_meshes.insert(key, handle.clone());
        Some(handle)
    }

    pub fn mesh(&mut self, visual: &Visual, meshes: &mut Assets<Mesh>) -> Option<Handle<Mesh>> {
        let key = MeshKey::of(visual);
        if let Some(handle) = key.and_then(|key| self.meshes.get(&key)) {
            return Some(handle.clone());
        }
        let handle = meshes.add(super::dim3::mesh_for(visual)?);
        if let Some(key) = key {
            self.meshes.insert(key, handle.clone());
        }
        Some(handle)
    }

    /// Coarser levels for the round primitives; boxes and planes have none.
    pub fn lod(
        &mut self,
        visual: &Visual,
        high: &Handle<Mesh>,
        meshes: &mut Assets<Mesh>,
    ) -> Option<crate::culling::LodGroup> {
        let key = MeshKey::of(visual)?;
        let (radius, make): (f32, fn(&Visual, u8) -> Mesh) = match visual {
            Visual::Sphere { radius, .. } => (*radius, |visual, level| {
                let Visual::Sphere { radius, .. } = visual else {
                    unreachable!()
                };
                let (sectors, stacks) = if level == 1 { (24, 16) } else { (12, 8) };
                Sphere::new(*radius).mesh().uv(sectors, stacks)
            }),
            Visual::Capsule { radius, height, .. } => (radius + height * 0.5, |visual, level| {
                let Visual::Capsule { radius, height, .. } = visual else {
                    unreachable!()
                };
                let (longitudes, latitudes) = if level == 1 { (16, 8) } else { (8, 4) };
                Capsule3d::new(*radius, *height)
                    .mesh()
                    .longitudes(longitudes)
                    .latitudes(latitudes)
                    .build()
            }),
            _ => return None,
        };
        let mut level = |level: u8| {
            self.lod_meshes
                .entry((key, level))
                .or_insert_with(|| meshes.add(make(visual, level)))
                .clone()
        };
        let (medium, low) = (level(1), level(2));
        Some(
            crate::culling::LodGroup::new(radius)
                .level(0.25, Some(high.clone()))
                .level(0.05, Some(medium))
                .level(0.0, Some(low)),
        )
    }

    pub fn material(
        &mut self,
        key: String,
        make: impl FnOnce() -> StandardMaterial,
        materials: &mut Assets<StandardMaterial>,
    ) -> Handle<StandardMaterial> {
        self.materials
            .entry(key)
            .or_insert_with(|| materials.add(make()))
            .clone()
    }

    pub fn box_material(
        &mut self,
        key: String,
        make: impl FnOnce() -> crate::materials::BoxMaterial,
        materials: &mut Assets<crate::materials::BoxMaterial>,
    ) -> Handle<crate::materials::BoxMaterial> {
        self.box_materials
            .entry(key)
            .or_insert_with(|| materials.add(make()))
            .clone()
    }

    pub fn has_instanced(&self, key: &str) -> bool {
        self.instanced.contains_key(key)
    }

    pub fn instanced_material(
        &mut self,
        key: String,
        make: impl FnOnce() -> crate::batching::InstancedMaterial,
        materials: &mut Assets<crate::batching::InstancedMaterial>,
    ) -> Handle<crate::batching::InstancedMaterial> {
        self.instanced
            .entry(key)
            .or_insert_with(|| materials.add(make()))
            .clone()
    }

    /// A rebuild starts a new authored world. Old handles remain alive only
    /// while old entities still reference them.
    pub fn clear(&mut self) {
        self.meshes.clear();
        self.scaled_meshes.clear();
        self.lod_meshes.clear();
        self.materials.clear();
        self.box_materials.clear();
        self.instanced.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differently_colored_shapes_share_geometry() {
        let mut cache = RenderCache::default();
        let mut meshes = Assets::<Mesh>::default();
        let one = Visual::Cuboid {
            color: "#ffffff".into(),
            size: [1.0, 2.0, 3.0],
        };
        let two = Visual::Cuboid {
            color: "#ff0000".into(),
            size: [1.0, 2.0, 3.0],
        };
        let first = cache.mesh(&one, &mut meshes).unwrap();
        let second = cache.mesh(&two, &mut meshes).unwrap();
        assert_eq!(first, second);
        assert_eq!(meshes.len(), 1);
    }

    #[test]
    fn repeated_surfaces_share_one_material_asset() {
        let mut cache = RenderCache::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let first = cache.material("stone".into(), StandardMaterial::default, &mut materials);
        let second = cache.material("stone".into(), StandardMaterial::default, &mut materials);
        assert_eq!(first, second);
        assert_eq!(materials.len(), 1);
    }
}
