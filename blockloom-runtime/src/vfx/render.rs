//! How an emitter draws: one material over the particle buffer, and a mesh
//! whose vertices only name slots. `shaders/vfx_particles.wesl` (3D) and
//! `vfx_particles_2d.wesl` read the particles the sim wrote.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline};
use blockloom_core::material::{ParticleSpec, hex_to_linear};
use blockloom_core::vfx::{LUT, ParticleBlend, ParticleFacing, RibbonSource};

pub const FLAG_LIT: u32 = 1;
pub const FLAG_SOFT: u32 = 2;
pub const FLAG_FLIPBOOK: u32 = 4;
pub const FLAG_FRAME_BLEND: u32 = 8;
pub const FLAG_PINNED_ONLY: u32 = 16;
pub const FLAG_FACE_CAMERA: u32 = 32;
pub const FLAG_ADDITIVE: u32 = 64;

/// Ribbon quads one emitter draws at most. Past it only the first slots get
/// ribbons.
pub const MAX_RIBBON_QUADS: u32 = 1 << 18;

/// `ParticleLook` in `vfx_particles.wesl`, field for field.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct ParticleLook {
    pub color_start: Vec4,
    pub color_end: Vec4,
    pub size: Vec4,
    pub flipbook: Vec4,
    pub extra: Vec4,
    pub modes: UVec4,
    pub ribbon: UVec4,
    pub curves: [Vec4; LUT],
    pub colors: [Vec4; LUT],
    pub ribbon_colors: [Vec4; LUT],
}

impl Default for ParticleLook {
    fn default() -> Self {
        ParticleLook {
            color_start: Vec4::ONE,
            color_end: Vec4::ONE,
            size: Vec4::ONE,
            flipbook: Vec4::ONE,
            extra: Vec4::ZERO,
            modes: UVec4::ZERO,
            ribbon: UVec4::ZERO,
            curves: [Vec4::ONE; LUT],
            colors: [Vec4::ONE; LUT],
            ribbon_colors: [Vec4::ONE; LUT],
        }
    }
}

impl ParticleLook {
    /// The look of `spec` drawn as billboards, or as ribbons with `ribbons`,
    /// over `capacity` slots of `points` ribbon points.
    pub fn of(
        spec: &ParticleSpec,
        capacity: u32,
        points: u32,
        ribbons: bool,
        flipbook: bool,
    ) -> Self {
        let render = &spec.render;
        let ribbon = &spec.ribbon;
        let size = render.size.bake();
        let alpha = render.alpha.bake();
        let rotation = render.rotation.bake();
        let width = ribbon.width_curve.bake();
        let colors = render.color.bake();
        let ribbon_colors = ribbon.color.bake();
        let mut flags = 0;
        if render.lit {
            flags |= FLAG_LIT;
        }
        if render.soft > 0.0 {
            flags |= FLAG_SOFT;
        }
        if flipbook {
            flags |= FLAG_FLIPBOOK;
            if render.flipbook.blend {
                flags |= FLAG_FRAME_BLEND;
            }
        }
        if ribbon.source == RibbonSource::Actor {
            flags |= FLAG_PINNED_ONLY;
        }
        if ribbon.face_camera {
            flags |= FLAG_FACE_CAMERA;
        }
        if render.blend == ParticleBlend::Additive {
            flags |= FLAG_ADDITIVE;
        }
        let facing = match render.facing {
            ParticleFacing::Camera => 0,
            ParticleFacing::Velocity => 1,
            ParticleFacing::Flat => 2,
        };
        ParticleLook {
            color_start: Vec4::from_array(hex_to_linear(&spec.color_start)),
            color_end: Vec4::from_array(hex_to_linear(&spec.color_end)),
            size: Vec4::new(
                spec.size_start,
                spec.size_end,
                render.stretch,
                render.intensity,
            ),
            flipbook: Vec4::new(
                render.flipbook.columns as f32,
                render.flipbook.rows as f32,
                render.flipbook.fps,
                render.flipbook.frames() as f32,
            ),
            extra: Vec4::new(render.soft, ribbon.width, ribbon.intensity, 0.0),
            modes: UVec4::new(ribbons as u32, facing, flags, capacity),
            ribbon: UVec4::new(points, ribbon.tessellation, 0, 0),
            curves: std::array::from_fn(|i| Vec4::new(size[i], alpha[i], rotation[i], width[i])),
            colors: colors.map(Vec4::from_array),
            ribbon_colors: ribbon_colors.map(Vec4::from_array),
        }
    }
}

/// Picks the 2D pipeline's blend.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParticleKey {
    additive: bool,
}

/// One emitter's particles (or ribbons), in either dimension.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(ParticleKey)]
pub struct ParticleMaterial {
    #[uniform(0)]
    pub look: ParticleLook,
    #[storage(1, read_only)]
    pub particles: Handle<ShaderBuffer>,
    #[storage(2, read_only)]
    pub trail: Handle<ShaderBuffer>,
    #[texture(3)]
    #[sampler(4)]
    pub sheet: Option<Handle<Image>>,
    pub additive: bool,
}

impl From<&ParticleMaterial> for ParticleKey {
    fn from(material: &ParticleMaterial) -> Self {
        ParticleKey {
            additive: material.additive,
        }
    }
}

impl Material for ParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(super::particles_3d())
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(super::particles_3d())
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.additive {
            AlphaMode::Add
        } else {
            AlphaMode::Blend
        }
    }

    // Particles read the depth prepass rather than being in it.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Both faces: a billboard or a ribbon can turn its back.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

impl Material2d for ParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(super::particles_2d())
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(super::particles_2d())
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if key.bind_group_data.additive
            && let Some(fragment) = descriptor.fragment.as_mut()
        {
            // The shader premultiplies and writes zero alpha: plain addition.
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::OneMinusSrcAlpha,
                        operation: BlendOperation::Add,
                    },
                    alpha: BlendComponent::OVER,
                });
            }
        }
        Ok(())
    }
}

/// Billboards: four vertices a slot, each naming its slot and corner.
pub fn billboard_mesh(capacity: u32) -> Mesh {
    let mut positions = Vec::with_capacity(capacity as usize * 4);
    let mut indices = Vec::with_capacity(capacity as usize * 6);
    for slot in 0..capacity {
        for corner in 0..4 {
            positions.push([slot as f32, corner as f32, 0.0]);
        }
        let base = slot * 4;
        indices.extend([base, base + 1, base + 3, base, base + 3, base + 2]);
    }
    slot_mesh(positions, indices)
}

/// Slots that get a ribbon: every one unless the quads would pass
/// [`MAX_RIBBON_QUADS`].
pub fn ribbon_slots(capacity: u32, points: u32, tessellation: u32) -> u32 {
    let segments = ribbon_segments(points, tessellation);
    capacity.min((MAX_RIBBON_QUADS / segments.max(1)).max(1))
}

pub fn ribbon_segments(points: u32, tessellation: u32) -> u32 {
    points.saturating_sub(1) * (tessellation + 1)
}

/// Ribbons: a strip of quads a slot, each vertex naming its slot, which
/// edge it is on and which end of which segment.
pub fn ribbon_mesh(slots: u32, points: u32, tessellation: u32) -> Mesh {
    let segments = ribbon_segments(points, tessellation);
    let quads = slots as usize * segments as usize;
    let mut positions = Vec::with_capacity(quads * 4);
    let mut indices = Vec::with_capacity(quads * 6);
    for slot in 0..slots {
        for segment in 0..segments {
            let base = positions.len() as u32;
            for corner in 0..4 {
                positions.push([slot as f32, corner as f32, segment as f32]);
            }
            indices.extend([base, base + 1, base + 3, base, base + 3, base + 2]);
        }
    }
    slot_mesh(positions, indices)
}

fn slot_mesh(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_indices(Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meshes_name_their_slots() {
        let mesh = billboard_mesh(3);
        assert_eq!(mesh.count_vertices(), 12);
        assert_eq!(mesh.indices().unwrap().len(), 18);
        let ribbons = ribbon_mesh(2, 4, 1);
        // Three gaps of two segments each, per slot.
        assert_eq!(ribbons.count_vertices(), 2 * 6 * 4);
        assert!(ribbon_slots(1 << 20, 64, 8) * ribbon_segments(64, 8) <= MAX_RIBBON_QUADS);
    }

    #[test]
    fn the_look_bakes_the_curves() {
        let spec = ParticleSpec::default();
        let look = ParticleLook::of(&spec, 64, 16, false, false);
        // Fades out over life by default.
        assert_eq!(look.curves[0].y, 1.0);
        assert_eq!(look.curves[LUT - 1].y, 0.0);
        assert_eq!(look.modes.w, 64);
        assert_ne!(look.modes.z & FLAG_ADDITIVE, 0);
    }
}
