//! Shared render assets and measurements for the game world.

// `std::time::Instant` panics on wasm (no OS clock there); `web_time` reads
// the browser's clock instead and wraps std everywhere else.
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use blockloom_core::scene::Visual;
use std::collections::HashMap;

/// Minimum pixel bytes for the Game view's scratch target and image ring.
#[derive(Resource, Default)]
pub struct GameViewTargetBytes(pub u64);

/// One paced-loop cycle split in three, smoothed: real update work versus
/// waiting for the Game view to put a frame on screen. The `fps` the status
/// reports covers all of it, so this is what says which part a slow frame is.
/// `main_ms` times the main schedule only (`First` to `Last`); whatever is
/// left of `update_ms` is the render world's extract, encode and submit.
#[derive(Resource, Default, Debug)]
pub struct LoopPace {
    pub update_ms: f64,
    pub wait_ms: f64,
    pub main_ms: f64,
    pub render_ms: f64,
    /// This frame's main window before smoothing, for the `other` bucket.
    pub frame_main_ms: f64,
    main_start: Option<web_time::Instant>,
}

impl LoopPace {
    /// Fold paced-loop timings in. Only the embedded Game view paces, so web
    /// and Android builds never call this.
    #[cfg_attr(any(target_arch = "wasm32", target_os = "android"), allow(dead_code))]
    pub fn push(&mut self, update_ms: f64, wait_ms: f64) {
        const BLEND: f64 = 0.1;
        self.update_ms += BLEND * (update_ms - self.update_ms);
        self.wait_ms += BLEND * (wait_ms - self.wait_ms);
        let render_ms = update_ms - self.main_ms;
        self.render_ms += BLEND * (render_ms.max(0.0) - self.render_ms);
    }

    pub fn begin_main(&mut self) {
        self.main_start = Some(web_time::Instant::now());
    }

    pub fn end_main(&mut self) {
        if let Some(t0) = self.main_start.take() {
            const BLEND: f64 = 0.1;
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            self.frame_main_ms = ms;
            self.main_ms += BLEND * (ms - self.main_ms);
        }
    }
}

pub fn mark_main_start(mut pace: ResMut<LoopPace>) {
    pace.begin_main();
}

pub fn mark_main_end(mut pace: ResMut<LoopPace>) {
    pace.end_main();
}

/// Fixed-step cost, summed across every step the last frame ran. Times the
/// whole step - rapier, the VM, scripts, parenting - from `FixedFirst` to
/// `FixedLast`, so one number says whether the sim is the slow half.
#[derive(Resource, Default, Debug)]
pub struct SimSplit {
    step_start: Option<web_time::Instant>,
    accum_ms: f64,
    steps: u32,
    pub fixed_ms: f64,
    pub last_steps: u32,
    /// This frame's steps before smoothing, for the `other` bucket.
    pub frame_fixed_ms: f64,
}

impl SimSplit {
    fn end_step(&mut self) {
        if let Some(t0) = self.step_start.take() {
            self.accum_ms += t0.elapsed().as_secs_f64() * 1000.0;
            self.steps += 1;
        }
    }

    fn publish(&mut self) {
        const BLEND: f64 = 0.1;
        self.frame_fixed_ms = self.accum_ms;
        self.fixed_ms += BLEND * (self.accum_ms - self.fixed_ms);
        self.last_steps = self.steps;
        self.accum_ms = 0.0;
        self.steps = 0;
    }
}

pub fn mark_step_start(mut split: ResMut<SimSplit>) {
    split.step_start = Some(web_time::Instant::now());
}

pub fn mark_step_end(mut split: ResMut<SimSplit>) {
    split.end_step();
}

/// Folds this frame's steps into the smoothed total. Runs in `Last`, after
/// every fixed step of the frame has landed.
pub fn publish_sim_split(mut split: ResMut<SimSplit>) {
    split.publish();
}

/// Per-frame `Update` segments, for the profiler. One plain marker system
/// brackets each group in the chained `Update` tuple (closures don't satisfy
/// this Bevy's `.chain()` bounds); `publish` folds each span into a smoothed
/// per-label millisecond count, labeled by position. Whatever the marks don't
/// cover - the second `Update` tuple, `PostUpdate` batching and culling -
/// lands in `other`, worked out from the main-schedule window minus fixed
/// sim minus the measured spans.
#[derive(Resource, Default, Debug)]
pub struct UpdateSplit {
    marks: Vec<web_time::Instant>,
    post_start: Option<web_time::Instant>,
    pub segments: Vec<(String, f64)>,
}

/// Labels for the spans between consecutive marks, in chain order.
const SEGMENT_LABELS: [&str; 6] = ["editor", "environment", "ui", "sensors", "camera", "tail"];

impl UpdateSplit {
    fn publish(&mut self, main_ms: f64, fixed_ms: f64) {
        const BLEND: f64 = 0.1;
        // Every frame runs every mark; a short vec means a partial frame,
        // which has no honest labels.
        if self.marks.len() == SEGMENT_LABELS.len() + 1 {
            let mut spans: Vec<(String, f64)> = self
                .marks
                .windows(2)
                .enumerate()
                .map(|(index, pair)| {
                    let ms = pair[1].duration_since(pair[0]).as_secs_f64() * 1000.0;
                    (SEGMENT_LABELS[index].to_string(), ms)
                })
                .collect();
            let mut measured: f64 = spans.iter().map(|(_, ms)| ms).sum();
            // The `PostUpdate` window runs to here, so this also covers the
            // `Last` systems ahead of it (sub-millisecond).
            if let Some(t0) = self.post_start.take() {
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                measured += ms;
                spans.push(("post".to_string(), ms));
            }
            spans.push((
                "other".to_string(),
                (main_ms - fixed_ms - measured).max(0.0),
            ));
            for (label, ms) in spans {
                match self.segments.iter_mut().find(|(name, _)| *name == label) {
                    Some(slot) => slot.1 += BLEND * (ms - slot.1),
                    None => self.segments.push((label, ms)),
                }
            }
        }
        self.marks.clear();
    }
}

pub fn mark_update_segment(mut split: ResMut<UpdateSplit>) {
    split.marks.push(web_time::Instant::now());
}

pub fn mark_post_start(mut split: ResMut<UpdateSplit>) {
    split.post_start = Some(web_time::Instant::now());
}

pub fn publish_update_split(
    mut split: ResMut<UpdateSplit>,
    sim: Res<SimSplit>,
    pace: Res<LoopPace>,
) {
    split.publish(pace.frame_main_ms, sim.frame_fixed_ms);
}

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

    /// Spheres and capsules swap to coarser meshes at range; boxes and
    /// planes and models get a cull-only level, preserving visible surfaces.
    /// Visible props can still merge; culled props stay out of batches.
    pub fn lod(
        &mut self,
        visual: &Visual,
        high: &Handle<Mesh>,
        meshes: &mut Assets<Mesh>,
    ) -> Option<crate::culling::LodGroup> {
        // Below this screen size a prop leaves the main view. A 1 m box
        // at 70 degrees FOV hits it around 70 m out; the props throttle
        // shrinks the measured size first, so pressure culls closer in.
        const PROP_CULL_SCREEN: f32 = 0.01;
        if let Visual::Model { scale, .. } = visual {
            let cull_radius = Vec3::from(*scale).length() * 0.5;
            if cull_radius > 0.0 {
                return Some(
                    crate::culling::LodGroup::new(cull_radius).level(PROP_CULL_SCREEN, None),
                );
            }
            return None;
        }
        let cull_radius = match visual {
            Visual::Cuboid { size, .. } => Vec3::from(*size).length() * 0.5,
            Visual::Plane { size, .. } => {
                Vec3::new(size[0], crate::dim3::PLANE_THICKNESS, size[1]).length() * 0.5
            }
            _ => 0.0,
        };
        if cull_radius > 0.0 {
            return Some(
                crate::culling::LodGroup::new(cull_radius)
                    .level(PROP_CULL_SCREEN, Some(high.clone())),
            );
        }
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
                .level(PROP_CULL_SCREEN, Some(low)),
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

    #[test]
    fn loop_pace_blends_toward_latest_samples() {
        let mut pace = LoopPace::default();
        for _ in 0..1000 {
            pace.push(4.0, 20.0);
        }
        assert!((pace.update_ms - 4.0).abs() < 0.01);
        assert!((pace.wait_ms - 20.0).abs() < 0.01);
    }

    #[test]
    fn sim_split_reports_steps_and_blends() {
        let mut split = SimSplit {
            accum_ms: 10.0,
            steps: 2,
            ..Default::default()
        };
        split.publish();
        assert_eq!(split.last_steps, 2);
        assert_eq!(split.steps, 0);
        assert_eq!(split.accum_ms, 0.0);
        assert_eq!(split.frame_fixed_ms, 10.0);
        assert!((split.fixed_ms - 1.0).abs() < 1e-9);
    }

    #[test]
    fn boxes_planes_and_models_get_cull_only_lods() {
        let mut cache = RenderCache::default();
        let mut meshes = Assets::<Mesh>::default();
        let high = meshes.add(Mesh::new(
            bevy::render::render_resource::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::MAIN_WORLD,
        ));
        for visual in [
            Visual::Cuboid {
                color: "#fff".into(),
                size: [2.0, 2.0, 2.0],
            },
            Visual::Plane {
                color: "#fff".into(),
                size: [4.0, 4.0],
            },
        ] {
            let group = cache.lod(&visual, &high, &mut meshes).unwrap();
            assert_eq!(group.levels().len(), 1);
            assert!(!group.swaps_meshes());
            assert_eq!(group.levels()[0].mesh, Some(high.clone()));
            assert!(group.levels()[0].min_screen > 0.0);
        }
        // Models keep their meshes until the entire scene is culled.
        let model = Visual::Model {
            path: "box.glb".into(),
            tint: "#fff".into(),
            scale: [1.0, 2.0, 3.0],
            animation: String::new(),
        };
        let group = cache.lod(&model, &high, &mut meshes).unwrap();
        assert_eq!(group.levels().len(), 1);
        assert!(!group.swaps_meshes());
        assert!(group.levels()[0].mesh.is_none());
        assert_eq!(group.levels()[0].min_screen, 0.01);
        let sphere = Visual::Sphere {
            color: "#fff".into(),
            radius: 1.0,
        };
        let group = cache.lod(&sphere, &high, &mut meshes).unwrap();
        assert_eq!(group.levels().len(), 3);
        assert!(group.swaps_meshes());
        assert!(group.levels().last().unwrap().min_screen > 0.0);
        assert!(
            cache
                .lod(
                    &Visual::Rect {
                        color: "#fff".into(),
                        size: [1.0, 1.0]
                    },
                    &high,
                    &mut meshes
                )
                .is_none()
        );
    }
}
