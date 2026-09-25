//! Async loading and streaming. Three parts, one module:
//!
//! - Cells: the XZ grid that streams with the camera, with a hysteresis band
//!   so a border never thrashes and an entry budget so a teleport spreads its
//!   loads over frames. Content registers as a payload by reading
//!   `CellEntered`/`CellLeft` and doing its work through `CellTasks`, which
//!   keeps the cell loading until the task lands.
//! - Loads: a look whose files are still loading stands in as a flat
//!   placeholder in 3D and fades in in 2D once they arrive.
//! - Warm-up: after every rebuild the world draws everything unculled until
//!   nothing is loading and no pipeline is compiling, and Play's green flag
//!   waits for that, so first frames never hitch.

use crate::batching::InstancedMaterial;
use crate::engine::ActorId;
use crate::engine::Engine;
use crate::materials::{BoxMaterial, GraphMaterial2d, GraphMaterial3d};
use crate::model::ModelChild;
use bevy::asset::{LoadState, RecursiveDependencyLoadState, UntypedAssetId};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::tasks::{AsyncComputeTaskPool, Task, TaskPool};
use bevy::world_serialization::WorldAssetRoot;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn register(app: &mut App) {
    app.init_resource::<StreamingCells>()
        .init_resource::<Loads>()
        .init_resource::<Warmup>()
        .add_message::<CellEntered>()
        .add_message::<CellLeft>();
    // The pipeline cache lives in the render world; its backlog is shared
    // with the main world through one counter.
    let backlog = PipelineBacklog::default();
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .insert_resource(backlog.clone())
            .add_systems(Render, count_backlog.in_set(RenderSystems::Cleanup));
        app.insert_resource(backlog);
    }
    app.add_systems(
        Update,
        (watch_loads, settle_loads, fade_in, warm_up)
            .chain()
            .after(crate::world::rebuild_world),
    );
}

// ─── Cells ─────────────────────────────────────────────────────────────────

pub type Cell = (i32, i32);

/// A cell came within reach of the camera: payloads start loading it.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellEntered(pub Cell);

/// A cell fell out of the hysteresis band, or the world was rebuilt:
/// payloads drop what they hold for it.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellLeft(pub Cell);

/// Active XZ cells for terrain, props, and effects that stream with the
/// camera. `entered`/`left` collect until `update_streaming_cells` sends them
/// on as messages.
#[derive(Resource, Default)]
pub struct StreamingCells {
    pub active: HashSet<Cell>,
    pub entered: Vec<Cell>,
    pub left: Vec<Cell>,
    /// Tasks still running per cell. A cell is ready once it has none.
    pending: HashMap<Cell, u32>,
    /// In-range cells the entry budget held back last update.
    due: usize,
}

impl StreamingCells {
    pub const SIZE: f32 = 64.0;
    const ENTER: f32 = 128.0;
    const EXIT: f32 = 192.0;
    /// Cells admitted per update, nearest first.
    const ENTER_BUDGET: usize = 8;

    /// Forgets every cell, telling payloads they all left.
    pub fn clear(&mut self) {
        self.left.extend(self.active.drain());
        self.entered.clear();
        self.pending.clear();
        self.due = 0;
    }

    pub fn cell_at(position: Vec3) -> Cell {
        (
            (position.x / Self::SIZE).floor() as i32,
            (position.z / Self::SIZE).floor() as i32,
        )
    }

    pub fn update(&mut self, position: Vec3) {
        self.active.retain(|&cell| {
            let keep = cell_distance(position, cell, Self::SIZE) <= Self::EXIT;
            if !keep {
                self.left.push(cell);
                self.pending.remove(&cell);
            }
            keep
        });
        let center = Self::cell_at(position);
        let mut due: Vec<(f32, Cell)> = Vec::new();
        for x in center.0 - 4..=center.0 + 4 {
            for z in center.1 - 4..=center.1 + 4 {
                let cell = (x, z);
                let distance = cell_distance(position, cell, Self::SIZE);
                if !self.active.contains(&cell) && distance <= Self::ENTER {
                    due.push((distance, cell));
                }
            }
        }
        due.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        self.due = due.len().saturating_sub(Self::ENTER_BUDGET);
        for (_, cell) in due.into_iter().take(Self::ENTER_BUDGET) {
            self.active.insert(cell);
            self.entered.push(cell);
        }
        self.entered.sort_unstable();
        self.left.sort_unstable();
    }

    /// A payload started work on `cell`.
    pub fn begin(&mut self, cell: Cell) {
        *self.pending.entry(cell).or_default() += 1;
    }

    /// A payload finished (or dropped) work on `cell`.
    pub fn finish(&mut self, cell: Cell) {
        if let Some(count) = self.pending.get_mut(&cell) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.pending.remove(&cell);
            }
        }
    }

    /// Active, with nothing still loading in it.
    #[allow(dead_code)]
    pub fn is_ready(&self, cell: Cell) -> bool {
        self.active.contains(&cell) && !self.pending.contains_key(&cell)
    }

    /// Cells with work in flight, plus in-range cells not yet admitted.
    pub fn loading(&self) -> usize {
        self.pending.len() + self.due
    }
}

fn cell_distance(position: Vec3, cell: Cell, size: f32) -> f32 {
    let x = position
        .x
        .clamp(cell.0 as f32 * size, (cell.0 + 1) as f32 * size);
    let z = position
        .z
        .clamp(cell.1 as f32 * size, (cell.1 + 1) as f32 * size);
    Vec2::new(position.x - x, position.z - z).length()
}

pub fn update_streaming_cells(
    camera: Query<&Transform, With<crate::world::WorldCamera>>,
    mut cells: ResMut<StreamingCells>,
    mut entered: MessageWriter<CellEntered>,
    mut left: MessageWriter<CellLeft>,
) {
    if let Ok(camera) = camera.single() {
        cells.update(camera.translation);
    }
    let cells = &mut *cells;
    left.write_batch(cells.left.drain(..).map(CellLeft));
    entered.write_batch(cells.entered.drain(..).map(CellEntered));
}

/// Background work keyed by whatever a payload keys its pieces by, each
/// piece belonging to one cell. The cell counts as loading until the task
/// lands or is dropped; spawning over a running key cancels the old task.
pub struct CellTasks<K, T> {
    running: HashMap<K, (Cell, Task<T>)>,
}

impl<K, T> Default for CellTasks<K, T> {
    fn default() -> Self {
        Self {
            running: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone, T: Send + 'static> CellTasks<K, T> {
    pub fn spawn(
        &mut self,
        cells: &mut StreamingCells,
        key: K,
        cell: Cell,
        work: impl FnOnce() -> T + Send + 'static,
    ) {
        self.cancel(cells, &key);
        cells.begin(cell);
        let pool = AsyncComputeTaskPool::get_or_init(TaskPool::new);
        self.running
            .insert(key, (cell, pool.spawn(async move { work() })));
    }

    /// Drops the task under `key`, if any. Dropping a task cancels it.
    pub fn cancel(&mut self, cells: &mut StreamingCells, key: &K) {
        if let Some((cell, _)) = self.running.remove(key) {
            cells.finish(cell);
        }
    }

    /// Drops every task working on `cell`, for a payload told it left.
    #[allow(dead_code)]
    pub fn cancel_cell(&mut self, cells: &mut StreamingCells, cell: Cell) {
        self.running.retain(|_, (at, _)| {
            let keep = *at != cell;
            if !keep {
                cells.finish(cell);
            }
            keep
        });
    }

    /// Whatever finished since the last poll.
    pub fn poll(&mut self, cells: &mut StreamingCells) -> Vec<(K, T)> {
        let mut done = Vec::new();
        self.running.retain(
            |key, (cell, task)| match bevy::tasks::futures::check_ready(task) {
                Some(value) => {
                    cells.finish(*cell);
                    done.push((key.clone(), value));
                    false
                }
                None => true,
            },
        );
        done
    }
}

// ─── Loads ─────────────────────────────────────────────────────────────────

/// On an actor whose look is still waiting on files.
#[derive(Component)]
pub struct Loading {
    ids: Vec<UntypedAssetId>,
    placeholder: Option<Entity>,
    /// A sprite, which fades in once its image lands.
    fades: bool,
}

/// The flat stand-in drawn under a loading 3D look.
#[derive(Component)]
pub struct Placeholder;

/// Brings a sprite's alpha up from zero. Anything else writing the color
/// mid-fade wins, and the fade stops.
#[derive(Component)]
pub struct FadeIn {
    elapsed: f32,
    alpha: f32,
    written: f32,
}

impl FadeIn {
    const SECONDS: f32 = 0.25;
}

#[derive(Resource, Default)]
pub struct Loads {
    /// Placeholder surfaces by look color, shared like any other surface.
    materials: HashMap<String, Handle<StandardMaterial>>,
    pub pending: usize,
    pub placeholders: usize,
}

/// Whether the server is still bringing `id` or anything it depends on in.
/// Assets made in code, and files that failed, aren't waited for.
fn waiting(server: &AssetServer, id: UntypedAssetId) -> bool {
    matches!(
        server.get_load_states(id),
        Some((LoadState::Loading, ..))
            | Some((_, _, RecursiveDependencyLoadState::Loading))
            | Some((
                LoadState::Loaded,
                _,
                RecursiveDependencyLoadState::NotLoaded
            ))
    )
}

fn standard_textures(material: &StandardMaterial) -> impl Iterator<Item = UntypedAssetId> + '_ {
    [
        &material.base_color_texture,
        &material.normal_map_texture,
        &material.metallic_roughness_texture,
        &material.emissive_texture,
        &material.occlusion_texture,
    ]
    .into_iter()
    .flatten()
    .map(|handle| handle.id().untyped())
}

/// Finds looks that changed and are waiting on files. A 3D look gets a
/// placeholder child drawing its mesh flat in the look's color.
#[allow(clippy::type_complexity)]
pub fn watch_loads(
    mut commands: Commands,
    engine: NonSend<Engine>,
    server: Res<AssetServer>,
    mut loads: ResMut<Loads>,
    mut standard: Option<ResMut<Assets<StandardMaterial>>>,
    instanced: Option<Res<Assets<InstancedMaterial>>>,
    boxes: Option<Res<Assets<BoxMaterial>>>,
    graph_3d: Option<Res<Assets<GraphMaterial3d>>>,
    graph_2d: Option<Res<Assets<GraphMaterial2d>>>,
    roots: Query<&WorldAssetRoot>,
    looks: Query<
        (
            Entity,
            &ActorId,
            (Option<&Sprite>, Option<&Mesh3d>, Option<&ModelChild>),
            (
                Option<&MeshMaterial3d<StandardMaterial>>,
                Option<&MeshMaterial3d<InstancedMaterial>>,
                Option<&MeshMaterial3d<BoxMaterial>>,
                Option<&MeshMaterial3d<GraphMaterial3d>>,
                Option<&MeshMaterial2d<GraphMaterial2d>>,
            ),
            Option<&Loading>,
        ),
        Or<(
            Changed<Sprite>,
            Changed<ModelChild>,
            Changed<MeshMaterial3d<StandardMaterial>>,
            Changed<MeshMaterial3d<InstancedMaterial>>,
            Changed<MeshMaterial3d<BoxMaterial>>,
            Changed<MeshMaterial3d<GraphMaterial3d>>,
            Changed<MeshMaterial2d<GraphMaterial2d>>,
        )>,
    >,
) {
    for (entity, id, (sprite, mesh, model), (plain, inst, boxed, g3, g2), loading) in &looks {
        let mut textures: Vec<UntypedAssetId> = Vec::new();
        if let Some(sprite) = sprite {
            textures.push(sprite.image.id().untyped());
        }
        if let (Some(handle), Some(store)) = (plain, standard.as_deref()) {
            textures.extend(store.get(&handle.0).into_iter().flat_map(standard_textures));
        }
        if let (Some(handle), Some(store)) = (inst, instanced.as_deref())
            && let Some(material) = store.get(&handle.0)
        {
            textures.extend(standard_textures(&material.base));
        }
        if let (Some(handle), Some(store)) = (boxed, boxes.as_deref())
            && let Some(material) = store.get(&handle.0)
        {
            textures.extend(standard_textures(&material.base));
            let own = [
                &material.extension.albedo,
                &material.extension.normal,
                &material.extension.roughness,
            ];
            textures.extend(own.into_iter().flatten().map(|h| h.id().untyped()));
        }
        if let (Some(handle), Some(store)) = (g3, graph_3d.as_deref())
            && let Some(texture) = store.get(&handle.0).and_then(|m| m.texture.as_ref())
        {
            textures.push(texture.id().untyped());
        }
        if let (Some(handle), Some(store)) = (g2, graph_2d.as_deref())
            && let Some(texture) = store.get(&handle.0).and_then(|m| m.texture.as_ref())
        {
            textures.push(texture.id().untyped());
        }
        textures.retain(|id| waiting(&server, *id));
        // A model's box already stands in for it, so its scene only counts.
        let mut ids = textures.clone();
        if let Some(root) = model.and_then(|child| roots.get(child.0).ok())
            && waiting(&server, root.0.id().untyped())
        {
            ids.push(root.0.id().untyped());
        }
        if loading.is_some_and(|loading| loading.ids == ids) {
            continue;
        }
        if let Some(old) = loading.and_then(|loading| loading.placeholder) {
            commands.entity(old).try_despawn();
        }
        if ids.is_empty() {
            commands.entity(entity).remove::<Loading>();
            continue;
        }
        let placeholder = match (mesh, standard.as_deref_mut()) {
            (Some(mesh), Some(store)) if !textures.is_empty() => {
                let color = engine
                    .actor(&id.0)
                    .and_then(|actor| actor.visual())
                    .and_then(|visual| visual.color())
                    .unwrap_or("#9aa3ad")
                    .to_string();
                let material = loads
                    .materials
                    .entry(color.clone())
                    .or_insert_with(|| {
                        store.add(StandardMaterial {
                            base_color: crate::world::parse_color(&color),
                            perceptual_roughness: 0.8,
                            ..default()
                        })
                    })
                    .clone();
                let child = commands
                    .spawn((
                        Placeholder,
                        Mesh3d(mesh.0.clone()),
                        MeshMaterial3d(material),
                        Transform::IDENTITY,
                    ))
                    .id();
                commands.entity(entity).add_child(child);
                Some(child)
            }
            _ => None,
        };
        commands.entity(entity).insert(Loading {
            ids,
            placeholder,
            fades: sprite.is_some(),
        });
    }
}

/// Swaps placeholders out, and starts fades, once every file has landed.
pub fn settle_loads(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut loads: ResMut<Loads>,
    mut waiting_looks: Query<(Entity, &Loading, Option<&mut Sprite>)>,
) {
    let mut pending = 0;
    let mut placeholders = 0;
    for (entity, loading, sprite) in &mut waiting_looks {
        let left = loading
            .ids
            .iter()
            .filter(|id| waiting(&server, **id))
            .count();
        if left > 0 {
            pending += left;
            placeholders += usize::from(loading.placeholder.is_some());
            continue;
        }
        if let Some(placeholder) = loading.placeholder {
            commands.entity(placeholder).try_despawn();
        }
        commands.entity(entity).remove::<Loading>();
        if let (true, Some(mut sprite)) = (loading.fades, sprite) {
            let alpha = sprite.color.alpha();
            sprite.color.set_alpha(0.0);
            commands.entity(entity).insert(FadeIn {
                elapsed: 0.0,
                alpha,
                written: 0.0,
            });
        }
    }
    loads.pending = pending;
    loads.placeholders = placeholders;
}

pub fn fade_in(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut fading: Query<(Entity, &mut FadeIn, &mut Sprite)>,
) {
    for (entity, mut fade, mut sprite) in &mut fading {
        if (sprite.color.alpha() - fade.written).abs() > 1e-4 {
            commands.entity(entity).remove::<FadeIn>();
            continue;
        }
        fade.elapsed += time.delta_secs();
        let t = (fade.elapsed / FadeIn::SECONDS).min(1.0);
        let alpha = fade.alpha * t * t * (3.0 - 2.0 * t);
        sprite.color.set_alpha(alpha);
        fade.written = alpha;
        if t >= 1.0 {
            commands.entity(entity).remove::<FadeIn>();
        }
    }
}

// ─── Warm-up ───────────────────────────────────────────────────────────────

/// How many pipelines the render world is still compiling.
#[derive(Resource, Clone, Default)]
pub struct PipelineBacklog(Arc<AtomicUsize>);

impl PipelineBacklog {
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

fn count_backlog(cache: Res<PipelineCache>, backlog: Res<PipelineBacklog>) {
    backlog
        .0
        .store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

/// The window after a rebuild in which the world draws everything, so every
/// pipeline it will need compiles now rather than the first time a thing
/// turns up on screen.
#[derive(Resource)]
pub struct Warmup {
    active: bool,
    began: Option<f32>,
    frames: u32,
    quiet: u32,
    /// Longest a window stays open, however much is still loading.
    pub timeout: f32,
}

impl Default for Warmup {
    fn default() -> Self {
        Self {
            active: false,
            began: None,
            frames: 0,
            quiet: 0,
            timeout: 10.0,
        }
    }
}

impl Warmup {
    /// Frames the window stays open at least: bounds are computed a frame
    /// after spawning, and the render world runs a frame behind.
    const MIN_FRAMES: u32 = 4;
    /// Consecutive frames with nothing pending before it closes.
    const QUIET_FRAMES: u32 = 3;

    pub fn open(&mut self) {
        self.active = true;
        self.began = None;
        self.frames = 0;
        self.quiet = 0;
    }

    pub fn active(&self) -> bool {
        self.active
    }
}

/// Unculled while warming, taken off again when the window closes.
#[derive(Component)]
pub struct Warming;

#[allow(clippy::type_complexity)]
pub fn warm_up(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    mut warmup: ResMut<Warmup>,
    real: Res<Time<Real>>,
    time: Res<Time>,
    backlog: Option<Res<PipelineBacklog>>,
    loads: Res<Loads>,
    cells: Res<StreamingCells>,
    culled: Query<Entity, (With<Aabb>, Without<NoFrustumCulling>)>,
    warmed: Query<Entity, With<Warming>>,
) {
    if !warmup.active {
        // A Start with no rebuild to wait for still has to begin.
        if engine.starting && !engine.rebuild {
            crate::world::begin_run(&mut engine, time.elapsed_secs() as f64);
        }
        return;
    }
    if engine.rebuild {
        return;
    }
    let now = real.elapsed_secs();
    let began = *warmup.began.get_or_insert(now);
    warmup.frames += 1;
    for entity in &culled {
        commands.entity(entity).try_insert((NoFrustumCulling, Warming));
    }
    let pending = loads.pending + cells.loading() + backlog.map_or(0, |b| b.get());
    warmup.quiet = if pending == 0 { warmup.quiet + 1 } else { 0 };
    let settled = warmup.frames >= Warmup::MIN_FRAMES && warmup.quiet >= Warmup::QUIET_FRAMES;
    if !settled && now - began < warmup.timeout {
        return;
    }
    if !settled {
        tracing::warn!(
            "still loading after {:.0}s; starting anyway",
            warmup.timeout
        );
    }
    warmup.active = false;
    for entity in &warmed {
        commands
            .entity(entity)
            .try_remove::<(NoFrustumCulling, Warming)>();
    }
    if engine.starting {
        crate::world::begin_run(&mut engine, time.elapsed_secs() as f64);
    }
}

/// What the profiler shows for this module.
#[derive(SystemParam)]
pub struct StreamingReport<'w> {
    cells: Option<Res<'w, StreamingCells>>,
    loads: Option<Res<'w, Loads>>,
    warmup: Option<Res<'w, Warmup>>,
    backlog: Option<Res<'w, PipelineBacklog>>,
}

impl StreamingReport<'_> {
    pub fn metrics(&self) -> Vec<(&'static str, usize)> {
        let mut metrics = Vec::new();
        if let Some(cells) = &self.cells {
            metrics.push(("streaming/active_cells", cells.active.len()));
            metrics.push(("streaming/loading_cells", cells.loading()));
        }
        if let Some(loads) = &self.loads {
            metrics.push(("streaming/pending_loads", loads.pending));
            metrics.push(("streaming/placeholders", loads.placeholders));
        }
        if let Some(warmup) = &self.warmup {
            metrics.push(("streaming/warming", usize::from(warmup.active)));
        }
        if let Some(backlog) = &self.backlog {
            metrics.push(("streaming/pipelines_compiling", backlog.get()));
        }
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_cells_stay_active_in_the_hysteresis_band() {
        let mut cells = StreamingCells::default();
        assert_eq!(
            StreamingCells::cell_at(Vec3::new(-0.1, 0.0, -64.1)),
            (-1, -2)
        );
        cells.update(Vec3::ZERO);
        assert!(cells.active.contains(&(0, 0)));
        cells.update(Vec3::new(160.0, 0.0, 0.0));
        assert!(cells.active.contains(&(0, 0)));
        assert!(!cells.left.contains(&(0, 0)));
        cells.update(Vec3::new(300.0, 0.0, 0.0));
        assert!(!cells.active.contains(&(0, 0)));
        assert!(cells.left.contains(&(0, 0)));
    }

    #[test]
    fn a_jump_admits_the_nearest_cells_first_over_several_updates() {
        let mut cells = StreamingCells::default();
        let far = Vec3::new(10_000.0, 0.0, 10_000.0);
        cells.update(far);
        assert_eq!(cells.entered.len(), StreamingCells::ENTER_BUDGET);
        assert!(cells.active.contains(&StreamingCells::cell_at(far)));
        assert!(cells.loading() > 0);
        for _ in 0..8 {
            cells.update(far);
        }
        assert_eq!(cells.loading(), 0);
        let reach = cells.active.len();
        cells.entered.clear();
        cells.update(far);
        assert!(cells.entered.is_empty());
        assert_eq!(cells.active.len(), reach);
    }

    #[test]
    fn clearing_tells_payloads_every_cell_left() {
        let mut cells = StreamingCells::default();
        cells.update(Vec3::ZERO);
        let active = cells.active.len();
        cells.left.clear();
        cells.clear();
        assert_eq!(cells.left.len(), active);
        assert!(cells.active.is_empty());
    }

    #[test]
    fn a_cell_is_loading_until_its_task_lands() {
        let mut cells = StreamingCells::default();
        cells.update(Vec3::ZERO);
        let mut tasks = CellTasks::<u32, u32>::default();
        tasks.spawn(&mut cells, 7, (0, 0), || 21 * 2);
        assert!(!cells.is_ready((0, 0)));
        let mut landed = Vec::new();
        for _ in 0..1000 {
            landed = tasks.poll(&mut cells);
            if !landed.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(landed, vec![(7, 42)]);
        assert!(cells.is_ready((0, 0)));
        assert!(tasks.running.is_empty());
    }

    #[test]
    fn a_cancelled_task_stops_holding_its_cell() {
        let mut cells = StreamingCells::default();
        cells.update(Vec3::ZERO);
        let mut tasks = CellTasks::<u32, ()>::default();
        tasks.spawn(&mut cells, 1, (0, 0), || {
            std::thread::sleep(std::time::Duration::from_millis(50));
        });
        tasks.spawn(&mut cells, 2, (0, 0), || ());
        tasks.cancel_cell(&mut cells, (0, 0));
        assert!(cells.is_ready((0, 0)));
        assert!(tasks.running.is_empty());
    }
}
