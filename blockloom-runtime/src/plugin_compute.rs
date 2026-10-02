//! The render-world half of plugin GPU compute.
//!
//! Plugin modules run on the main thread and the device lives in the render
//! world, so the two meet at a [`ComputeLink`]: the main world files the
//! commands plugins asked for, the render world runs them once a frame on its
//! own device through [`ComputeEngine`], and what comes back (errors, finished
//! reads) waits there for the main world to hand to the plugins.

use crate::bridge;
use crate::engine::Engine;
use bevy::prelude::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Render, RenderApp, RenderSystems};
use blockloom_plugin_api::compute::{GpuCommand, LoadoutKernel};
use blockloom_plugin_gpu::engine::{ComputeEngine, Report};
use blockloom_protocol::RuntimeMessage;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Link {
    /// A new kernel set for the engine to build.
    kernels: Option<Vec<LoadoutKernel>>,
    commands: Vec<(String, GpuCommand)>,
    /// The run ended: buffers and reads go, kernels stay.
    clear: bool,
    reports: Vec<Report>,
    /// Whether a device is there to compute on, once known.
    available: Option<bool>,
}

/// What the two worlds share.
#[derive(Resource, Clone, Default)]
pub struct ComputeLink(Arc<Mutex<Link>>);

impl ComputeLink {
    pub fn set_kernels(&self, kernels: Vec<LoadoutKernel>) {
        self.0.lock().unwrap().kernels = Some(kernels);
    }

    pub fn push(&self, commands: Vec<(String, GpuCommand)>) {
        self.0.lock().unwrap().commands.extend(commands);
    }

    pub fn clear(&self) {
        let mut link = self.0.lock().unwrap();
        link.clear = true;
        link.commands.clear();
    }

    pub fn take_reports(&self) -> Vec<Report> {
        std::mem::take(&mut self.0.lock().unwrap().reports)
    }

    /// `Some(false)` when there is no device to compute on.
    pub fn available(&self) -> Option<bool> {
        self.0.lock().unwrap().available
    }

    fn set_available(&self, available: bool) {
        self.0.lock().unwrap().available = Some(available);
    }
}

pub fn register(app: &mut App) {
    let link = ComputeLink::default();
    app.insert_resource(link.clone()).add_systems(Update, feed);
    match app.get_sub_app_mut(RenderApp) {
        Some(render) => {
            render
                .insert_resource(link)
                .add_systems(Render, run.in_set(RenderSystems::Cleanup));
        }
        // A world without a renderer has nothing to compute on.
        None => link.set_available(false),
    }
}

fn run(
    link: Res<ComputeLink>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut engine: Local<Option<ComputeEngine>>,
) {
    let (kernels, commands, clear) = {
        let mut shared = link.0.lock().unwrap();
        (
            shared.kernels.take(),
            std::mem::take(&mut shared.commands),
            std::mem::take(&mut shared.clear),
        )
    };
    if engine.is_none() {
        // Nothing has asked yet: do not build a pipeline cache for no one.
        if kernels.as_ref().is_none_or(Vec::is_empty) && commands.is_empty() {
            return;
        }
        *engine = Some(ComputeEngine::new(
            device.wgpu_device().clone(),
            (**queue).clone(),
        ));
        link.set_available(true);
    }
    let engine = engine.as_mut().unwrap();
    let mut reports = Vec::new();
    if let Some(kernels) = kernels {
        for message in engine.set_kernels(&kernels) {
            reports.push(Report::Error {
                plugin: String::new(),
                message,
            });
        }
    }
    if clear {
        engine.clear();
    }
    for (plugin, command) in commands {
        if let Err(message) = engine.submit(&plugin, command) {
            reports.push(Report::Error { plugin, message });
        }
    }
    reports.extend(engine.run());
    if !reports.is_empty() {
        link.0.lock().unwrap().reports.extend(reports);
    }
}

/// Main world: files the plugins' commands and the kernel set with the
/// render world, and hands back what it reports.
fn feed(
    mut engine: NonSendMut<Engine>,
    link: Res<ComputeLink>,
    mut seen: Local<u64>,
    mut warned: Local<bool>,
) {
    if engine.plugins.kernels_serial != *seen {
        *seen = engine.plugins.kernels_serial;
        link.set_kernels(engine.plugins.loadout.kernels.clone());
    }
    if std::mem::take(&mut engine.plugins.gpu_clear) {
        link.clear();
    }
    let commands = std::mem::take(&mut engine.plugins.gpu);
    if !commands.is_empty() {
        if link.available() == Some(false) {
            if !std::mem::replace(&mut *warned, true) {
                bridge::send(&RuntimeMessage::Error {
                    actor: String::new(),
                    message: "plugin GPU compute needs a rendering world, and this one has none"
                        .to_string(),
                });
            }
        } else {
            link.push(commands);
        }
    }
    let reports = link.take_reports();
    if !reports.is_empty() {
        crate::plugins::gpu_reports(&mut engine, reports);
    }
}
