//! Status reports to the editor, input remaps, rumble and pointer lock.

use super::*;

/// Tells the editor where everything is, a few times a second.
pub fn report_status(
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    diagnostics: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    target_bytes: Option<Res<crate::performance::GameViewTargetBytes>>,
    pace: Option<Res<crate::performance::LoopPace>>,
    sim: Option<Res<crate::performance::SimSplit>>,
    update: Option<Res<crate::performance::UpdateSplit>>,
    batches: Option<Res<crate::batching::Batches>>,
    culling: Option<Res<crate::culling::Culling>>,
    clouds: Option<Res<crate::clouds::CloudStats>>,
    streaming: crate::streaming::StreamingReport,
    gpu: crate::gpu::GpuReport,
    volumes: Option<Res<crate::volumes::VolumeBlend>>,
    terrain: Option<Res<crate::terrain::TerrainStats>>,
    (vfx, decals, destruction, scaling, indirect): (
        Option<Res<crate::vfx::VfxStats>>,
        Option<Res<crate::decals::Decals>>,
        Option<Res<crate::destruction::Destruction>>,
        Option<Res<crate::quality::Scaling>>,
        Option<Res<crate::indirect::IndirectDrawCounters>>,
    ),
    actors: Query<(&ActorId, &Transform, &Visibility)>,
) {
    let now = time.elapsed_secs_f64();
    if now < engine.next_report {
        return;
    }
    engine.next_report = now + 0.2;
    let statuses = actors
        .iter()
        .map(|(id, transform, visibility)| {
            let (rx, ry, rz) = transform.rotation.to_euler(EulerRot::XYZ);
            ActorStatus {
                id: id.0.clone(),
                position: transform.translation.to_array(),
                rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
                visible: *visibility != Visibility::Hidden,
            }
        })
        .collect();
    let variables = engine.variables.snapshot();
    let globals = variables
        .globals
        .iter()
        .map(|(name, value)| VariableValue {
            name: name.clone(),
            value: value.clone(),
        })
        .collect();
    let mut render_metrics: Vec<RenderMetric> = diagnostics
        .iter()
        .flat_map(|store| store.iter())
        .filter_map(|diagnostic| {
            let name = diagnostic.path().as_str();
            let measured = name == crate::gpu::FRAME_GPU.as_str()
                || name.starts_with("render/")
                    && (name.ends_with("/elapsed_gpu") || name.ends_with("/elapsed_cpu"));
            if !measured && name != "mesh_allocator_slabs_size" {
                return None;
            }
            // Group mesh memory with the other `memory/*` rows so the
            // profiler can list it under Memory instead of by Bevy's raw name.
            let name = if name == "mesh_allocator_slabs_size" {
                "memory/mesh_slabs"
            } else {
                name
            };
            Some(RenderMetric {
                name: name.to_string(),
                value: diagnostic.smoothed()?,
                unit: if name == "memory/mesh_slabs" {
                    "bytes"
                } else {
                    "ms"
                }
                .into(),
            })
        })
        .collect();
    if engine.project.world.clouds.enabled
        && let Some(clouds) = clouds
    {
        for (name, value) in [
            ("clouds/steps", clouds.steps),
            ("clouds/overdraw", clouds.overdraw),
        ] {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value: value as f64,
                unit: "samples/pixel".into(),
            });
        }
    }
    if let Some(ms) = last_rebuild_ms() {
        render_metrics.push(RenderMetric {
            name: "world/rebuild".into(),
            value: ms,
            unit: "ms".into(),
        });
    }
    if let Some(ms) = crate::hdr::tonemap_ms(&render_metrics) {
        render_metrics.push(RenderMetric {
            name: "hdr/tonemap".into(),
            value: ms,
            unit: "ms".into(),
        });
    }
    if let Some(bytes) = target_bytes.filter(|bytes| bytes.0 > 0) {
        render_metrics.push(RenderMetric {
            name: "memory/targets/game_view_minimum".into(),
            value: bytes.0 as f64,
            unit: "bytes".into(),
        });
    }
    if let Some(batches) = batches {
        let stats = batches.stats;
        for (name, value) in [
            ("batching/instanced_draws", stats.instanced_draws),
            ("batching/instanced_actors", stats.instanced_actors),
            ("batching/static_batches", stats.static_batches),
            ("batching/static_members", stats.static_members),
            ("batching/dynamic_batches", stats.dynamic_batches),
            ("batching/dynamic_members", stats.dynamic_members),
        ] {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value: value as f64,
                unit: "count".into(),
            });
        }
    }
    for (name, value) in crate::physics_debug::metrics() {
        render_metrics.push(RenderMetric {
            name: name.into(),
            value,
            unit: if name.ends_with("_ms") { "ms" } else { "count" }.into(),
        });
    }
    for (name, value) in crate::plugins::report(&mut engine) {
        render_metrics.push(RenderMetric {
            name,
            value,
            unit: "plugin".into(),
        });
    }
    // One row per script file, so a heavy file stands out from a busy
    // project. Keyed by path, named by it: two folders may hold one stem.
    let mut script_rows: Vec<(&String, &f64)> = engine.script_times.iter().collect();
    script_rows.sort_by(|a, b| a.0.cmp(b.0));
    for (path, ms) in script_rows {
        render_metrics.push(RenderMetric {
            name: format!("script/{path}"),
            value: *ms,
            unit: "ms".into(),
        });
    }
    if let Some(state) = destruction {
        for (name, mut value) in state.metrics() {
            if name == "destruction/shard_budget" {
                value = scaling
                    .as_ref()
                    .map_or(blockloom_core::destruction::SHARD_CAP, |s| {
                        s.shard_budget(blockloom_core::destruction::SHARD_CAP)
                    }) as f64;
            }
            render_metrics.push(RenderMetric {
                name: name.into(),
                value,
                unit: "count".into(),
            });
        }
    }
    if let Some(decals) = decals {
        for (name, value) in [
            ("decals/active", decals.pool.marks.len() as f64),
            (
                "decals/budget",
                scaling
                    .as_ref()
                    .map_or(blockloom_core::decals::CAPACITY, |s| s.budget().decals)
                    as f64,
            ),
            ("decals/stolen", decals.pool.stolen as f64),
        ] {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value,
                unit: "count".into(),
            });
        }
    }
    if let Some(vfx) = vfx.filter(|vfx| vfx.emitters > 0) {
        for (name, value, unit) in vfx.metrics() {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value,
                unit: unit.into(),
            });
        }
    }
    if let Some(culling) = culling {
        let stats = culling.stats;
        for (name, value) in [
            ("culling/lod_groups", stats.lod_groups),
            ("culling/lod_reduced", stats.lod_reduced),
            ("culling/lod_culled", stats.lod_culled),
            ("culling/occluders", stats.occluders),
            ("culling/occlusion_tested", stats.tested),
            ("culling/occluded", stats.occluded),
        ] {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value: value as f64,
                unit: "count".into(),
            });
        }
    }
    if let Some(terrain) = terrain.filter(|t| t.terrains > 0) {
        for (name, value) in terrain.metrics() {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value: value as f64,
                unit: "count".into(),
            });
        }
    }
    for (name, value) in streaming.metrics() {
        render_metrics.push(RenderMetric {
            name: name.into(),
            value: value as f64,
            unit: "count".into(),
        });
    }
    for (name, value, unit) in gpu.metrics() {
        render_metrics.push(RenderMetric {
            name: name.into(),
            value,
            unit: unit.into(),
        });
    }
    if let Some(pace) = pace {
        for (name, value) in [
            ("loop/update", pace.update_ms),
            ("loop/present_wait", pace.wait_ms),
            ("loop/main", pace.main_ms),
            ("loop/render", pace.render_ms),
        ] {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value,
                unit: "ms".into(),
            });
        }
    }
    if let Some(sim) = sim {
        render_metrics.push(RenderMetric {
            name: "loop/fixed".into(),
            value: sim.fixed_ms,
            unit: "ms".into(),
        });
        render_metrics.push(RenderMetric {
            name: "loop/fixed_steps".into(),
            value: sim.last_steps as f64,
            unit: "count".into(),
        });
    }
    if let Some(update) = update {
        for (label, ms) in &update.segments {
            render_metrics.push(RenderMetric {
                name: format!("update/{label}"),
                value: *ms,
                unit: "ms".into(),
            });
        }
    }
    if let Some(indirect) = indirect {
        for (name, value, unit) in [
            ("indirect/api_draws", indirect.api_draws as f64, "count"),
            ("indirect/commands", indirect.commands as f64, "count"),
            ("indirect/instances", indirect.instances as f64, "count"),
            (
                "indirect/multidraw_sets",
                indirect.multidraw_sets as f64,
                "count",
            ),
            (
                "indirect/transparent_draws",
                indirect.transparent_draws as f64,
                "count",
            ),
            ("indirect/gpu", f64::from(u8::from(indirect.gpu)), "flag"),
        ] {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value,
                unit: unit.into(),
            });
        }
        for (index, system) in crate::quality::SYSTEMS.iter().enumerate() {
            render_metrics.push(RenderMetric {
                name: format!("indirect/draws/{system}"),
                value: indirect.draws_per_system[index] as f64,
                unit: "count".into(),
            });
        }
    }
    if let Some(scaling) = scaling {
        let budget = scaling.budget();
        for (index, system) in crate::quality::SYSTEMS.iter().enumerate() {
            for (metric, value) in [
                ("estimated_draws", scaling.costs[index].draws as f64),
                ("draw_budget", budget.draws[index] as f64),
                ("triangles", scaling.costs[index].triangles as f64),
                ("triangle_budget", budget.triangles[index] as f64),
            ] {
                render_metrics.push(RenderMetric {
                    name: format!("budget/{system}/{metric}"),
                    value,
                    unit: "count".into(),
                });
            }
            if blockloom_core::quality::GeometryController::LOCAL_SYSTEMS.contains(&index) {
                let metric = match index {
                    0..=2 => "distance_scale",
                    5 => "tessellation_scale",
                    _ => "density_scale",
                };
                render_metrics.push(RenderMetric {
                    name: format!("budget/{system}/{metric}"),
                    value: scaling.geometry.factors[index] as f64,
                    unit: "ratio".into(),
                });
            }
        }
        for (name, value, unit) in [
            (
                "quality/preset",
                scaling.controller.quality as u32 as f64,
                "index",
            ),
            (
                "quality/resolution_scale",
                scaling.controller.scale as f64,
                "ratio",
            ),
            ("quality/frame", scaling.controller.frame_ms as f64, "ms"),
            ("quality/target", scaling.settings.target_ms as f64, "ms"),
            ("quality/drops", scaling.controller.drops as f64, "count"),
            (
                "quality/estimated_mesh_draws",
                scaling.draw_calls as f64,
                "count",
            ),
            (
                "quality/instanced_draws",
                scaling.instanced_draws as f64,
                "count",
            ),
            (
                "quality/batched_instances",
                scaling.batched_instances as f64,
                "count",
            ),
            (
                "quality/visible_mesh_triangles",
                scaling.triangles as f64,
                "count",
            ),
            (
                "quality/dlss_available",
                f64::from(u8::from(scaling.dlss_available)),
                "bool",
            ),
        ] {
            render_metrics.push(RenderMetric {
                name: name.into(),
                value,
                unit: unit.into(),
            });
        }
    }
    render_metrics.sort_by(|a, b| a.name.cmp(&b.name));
    bridge::send(&RuntimeMessage::Status(Status {
        running: engine.running || engine.starting,
        paused: engine.paused,
        time: engine.run_time(now),
        fps: 1.0 / time.delta_secs().max(f32::EPSILON),
        render_metrics,
        actors: statuses,
        globals,
        volumes: volumes.as_ref().map(|v| v.statuses()).unwrap_or_default(),
        volume_trace: volumes.map(|v| v.trace.clone()).unwrap_or_default(),
    }));
}

/// Empties the effect list once every apply system has seen it.
pub fn clear_effects(mut effects: ResMut<PendingEffects>) {
    effects.0.clear();
}

/// Run-scoped input remaps: lowercase action name -> bindings. The `bind`
/// and `clear` blocks move it, so a settings screen remaps for this run
/// without touching the document. Forgotten on a rebuild.
#[derive(Resource, Default, Debug, Clone)]
pub struct InputRun(pub HashMap<String, Vec<blockloom_core::input::InputBinding>>);

/// Carries out run-scoped input remaps. A `bind` adds one binding to an
/// action's override, a `clear` empties it; both last exactly as long as the
/// run. Unknown actions and unparseable bindings are reported, not applied.
pub fn apply_input_effects(
    engine: NonSend<Engine>,
    mut run: ResMut<InputRun>,
    effects: Res<PendingEffects>,
) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::BindAction {
                actor,
                action,
                binding,
            } => {
                let Some(found) = engine
                    .project
                    .settings
                    .input
                    .find(action)
                    .map(|found| found.name.clone())
                else {
                    crate::bridge::send(&RuntimeMessage::Error {
                        actor: actor.clone(),
                        message: format!("there's no input action named \"{action}\""),
                    });
                    continue;
                };
                match blockloom_core::input::parse_binding(binding) {
                    Some(parsed) => {
                        let key = found.to_lowercase();
                        if !run.0.contains_key(&key) {
                            let defaults = engine
                                .project
                                .settings
                                .input
                                .find(&found)
                                .map(|found| found.bindings.clone())
                                .unwrap_or_default();
                            run.0.insert(key.clone(), defaults);
                        }
                        let entry = run.0.entry(key).or_default();
                        let text = parsed.text();
                        if !entry.iter().any(|held| held.text() == text) {
                            entry.push(parsed);
                        }
                    }
                    None => {
                        crate::bridge::send(&RuntimeMessage::Error {
                            actor: actor.clone(),
                            message: format!("\"{binding}\" isn't a binding"),
                        });
                    }
                }
            }
            Effect::ClearActionBindings { action, actor } => {
                let Some(found) = engine
                    .project
                    .settings
                    .input
                    .find(action)
                    .map(|found| found.name.clone())
                else {
                    crate::bridge::send(&RuntimeMessage::Error {
                        actor: actor.clone(),
                        message: format!("there's no input action named \"{action}\""),
                    });
                    continue;
                };
                run.0.insert(found.to_lowercase(), Vec::new());
            }
            _ => {}
        }
    }
}

/// Rumbles every connected gamepad. A zero strength or duration stops
/// instead, so `rumble 0 for 0` is a stop block.
pub fn apply_rumble(
    engine: NonSend<Engine>,
    effects: Res<PendingEffects>,
    pads: Query<Entity, With<Gamepad>>,
    mut rumbles: MessageWriter<GamepadRumbleRequest>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        if let Effect::RumbleGamepad { strength, duration } = effect {
            for entity in &pads {
                if *strength <= 0.0 || *duration <= 0.0 {
                    rumbles.write(GamepadRumbleRequest::Stop { gamepad: entity });
                } else {
                    let clamped = (strength / 100.0).clamp(0.0, 1.0);
                    rumbles.write(GamepadRumbleRequest::Add {
                        gamepad: entity,
                        duration: std::time::Duration::from_secs_f32(duration.max(0.0)),
                        intensity: GamepadRumbleIntensity {
                            strong_motor: clamped,
                            weak_motor: clamped,
                        },
                    });
                }
            }
        }
    }
}

/// Carries out pointer lock requests, and guarantees a stopped run never
/// keeps the user's mouse: locking only means anything while running, so a
/// fresh Play always starts unlocked.
pub fn apply_cursor_lock(
    mut engine: NonSendMut<Engine>,
    effects: Res<PendingEffects>,
    mut targets: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let wanted = engine.wants_cursor_locked;
    if !engine.running {
        engine.wants_cursor_locked = false;
        engine.look_lock_offered = false;
    } else {
        // A look camera takes the pointer once per run; a block can free it.
        if !engine.look_lock_offered && crate::player_camera::wants_lock(&engine) {
            engine.look_lock_offered = true;
            engine.wants_cursor_locked = true;
        }
        for effect in &effects.0 {
            if let Effect::SetMouseLocked { locked } = effect {
                engine.wants_cursor_locked = *locked;
            }
        }
    }
    let Ok((mut window, mut cursor)) = targets.single_mut() else {
        // Embedded there is no window: the editor's view holds the pointer.
        if engine.wants_cursor_locked != wanted {
            crate::bridge::send(&RuntimeMessage::PointerLock {
                locked: engine.wants_cursor_locked,
            });
        }
        return;
    };
    // A browser grants a lock only inside a click, which `web` waits for.
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (&mut window, &mut cursor);
        crate::web::want_pointer_lock(engine.running && engine.wants_cursor_locked);
    }
    #[cfg(not(target_arch = "wasm32"))]
    lock_window_cursor(&engine, &mut window, &mut cursor);
}

/// Grabs or frees a native window's pointer to match what the game wants.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn lock_window_cursor(engine: &Engine, window: &mut Window, cursor: &mut CursorOptions) {
    if !engine.running {
        set_cursor_locked(cursor, false);
        return;
    }
    // Re-asserted every tick, not just when the block runs. The first request
    // usually lands before the window is focused, and the backend answers an
    // unfocused grab with a silent no-op while the component still says
    // Locked - so without this the pointer roams free with no way back.
    // Rewriting the same values re-triggers the backend attempt, so the real
    // lock sticks as soon as the window can hold it, and re-sticks after any
    // later drop. Skipped while unfocused, where no grab can succeed and
    // every attempt only logs another failure. A tab-out releases instead,
    // browser-style, and the return trip re-grabs.
    if engine.wants_cursor_locked && engine.window_focused {
        if cursor.grab_mode != CursorGrabMode::Locked {
            // Center first: a lock pins the cursor where it stands, which is
            // usually still over the editor from the Play click - freezing it
            // there, visible, outside this window. The backend applies the
            // warp before the clip, so the pin and the hidden cursor land
            // inside the game instead.
            let center = Vec2::new(window.width(), window.height()) / 2.0;
            window.set_cursor_position(Some(center));
        }
        set_cursor_locked(cursor, true);
    } else if cursor.grab_mode != CursorGrabMode::None {
        set_cursor_locked(cursor, false);
    }
}

/// Locked is grabbed and hidden, the first-person standard; unlocked is a
/// plain visible pointer again.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn set_cursor_locked(cursor: &mut CursorOptions, locked: bool) {
    cursor.grab_mode = if locked {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    cursor.visible = !locked;
}

// ─── Small shared helpers ──────────────────────────────────────────────────
