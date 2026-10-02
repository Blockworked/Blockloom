//! Running plugin kernels on a device the caller owns.
//!
//! A plugin never sees any of this. It names buffers and kernels in
//! [`GpuCommand`]s, which wait in its own queue; [`ComputeEngine::run`]
//! carries them out in order inside one command encoder under a per-frame
//! invocation budget, and a read comes back a few frames later as a
//! [`Report::Read`]. The engine owns every buffer, enforces the limits in
//! [`blockloom_plugin_api::compute`] and keeps one plugin's work from
//! starving another's: queues are served round-robin, each stopping at its
//! first dispatch that would overrun the frame.

use crate::check::{KernelInfo, check_kernel};
use blockloom_plugin_api::compute::{
    BindingKind, FRAME_INVOCATIONS, GpuCommand, KernelSchema, LoadoutKernel, MAX_BUFFERS,
    MAX_DISPATCH_INVOCATIONS, MAX_PENDING_READS, MAX_PLUGIN_WORDS, MAX_QUEUED, ReadAs,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

/// What running a frame's commands, or a read finishing, tells the caller.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    /// A command was refused or failed; the commands after it still run.
    Error { plugin: String, message: String },
    /// A read finished.
    Read {
        plugin: String,
        tag: String,
        buffer: String,
        offset: u32,
        as_type: ReadAs,
        words: Vec<u32>,
    },
}

struct Kernel {
    schema: KernelSchema,
    info: KernelInfo,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

struct Buffer {
    buffer: wgpu::Buffer,
    /// What the plugin asked for; the allocation is rounded up to 16 bytes.
    words: u32,
}

#[derive(Default)]
struct PluginState {
    buffers: BTreeMap<String, Buffer>,
    queue: VecDeque<GpuCommand>,
    held_words: u64,
}

type Landed = Arc<Mutex<Option<Result<(), String>>>>;

struct InFlight {
    plugin: String,
    tag: String,
    buffer: String,
    offset: u32,
    as_type: ReadAs,
    words: u32,
    staging: wgpu::Buffer,
    landed: Landed,
}

pub struct ComputeEngine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    kernels: BTreeMap<(String, String), Kernel>,
    plugins: BTreeMap<String, PluginState>,
    reading: Vec<InFlight>,
    /// Where the round-robin starts next frame.
    cursor: usize,
    /// Invocations dispatched over the engine's life, for diagnostics.
    dispatched: u64,
}

fn round_up(words: u32) -> u64 {
    (u64::from(words) * 4).div_ceil(16) * 16
}

impl ComputeEngine {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> ComputeEngine {
        ComputeEngine {
            device,
            queue,
            kernels: BTreeMap::new(),
            plugins: BTreeMap::new(),
            reading: Vec::new(),
            cursor: 0,
            dispatched: 0,
        }
    }

    /// Replaces the kernels the engine knows. A kernel that fails the check
    /// is left out and reported; the rest are ready.
    pub fn set_kernels(&mut self, kernels: &[LoadoutKernel]) -> Vec<String> {
        let mut errors = Vec::new();
        let mut next = BTreeMap::new();
        for loaded in kernels {
            let key = (loaded.plugin.clone(), loaded.schema.name.clone());
            match self.build(loaded) {
                Ok(kernel) => {
                    next.insert(key, kernel);
                }
                Err(e) => errors.push(format!("{}: {e}", loaded.plugin)),
            }
        }
        self.kernels = next;
        errors
    }

    fn build(&self, loaded: &LoadoutKernel) -> Result<Kernel, String> {
        let schema = &loaded.schema;
        let info = check_kernel(schema, &loaded.source)?;
        let entries: Vec<wgpu::BindGroupLayoutEntry> = schema
            .bindings
            .iter()
            .map(|b| wgpu::BindGroupLayoutEntry {
                binding: b.binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: match b.kind {
                        BindingKind::Uniform => wgpu::BufferBindingType::Uniform,
                        BindingKind::Read => wgpu::BufferBindingType::Storage { read_only: true },
                        BindingKind::ReadWrite => {
                            wgpu::BufferBindingType::Storage { read_only: false }
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let label = format!("plugin kernel {}/{}", loaded.plugin, schema.name);
        let layout = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(&label),
                entries: &entries,
            });
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&label),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&label),
                source: wgpu::ShaderSource::Wgsl(loaded.source.as_str().into()),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(&label),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(&schema.entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        Ok(Kernel {
            schema: schema.clone(),
            info,
            pipeline,
            layout,
        })
    }

    pub fn has_kernel(&self, plugin: &str, name: &str) -> bool {
        self.kernels
            .contains_key(&(plugin.to_string(), name.to_string()))
    }

    /// Queues a command for the next [`run`](ComputeEngine::run).
    pub fn submit(&mut self, plugin: &str, command: GpuCommand) -> Result<(), String> {
        command.check()?;
        let reads = self.reading.iter().filter(|r| r.plugin == plugin).count();
        let state = self.plugins.entry(plugin.to_string()).or_default();
        if state.queue.len() >= MAX_QUEUED {
            return Err(format!("{MAX_QUEUED} GPU commands are already waiting"));
        }
        if matches!(command, GpuCommand::Read { .. }) {
            let waiting = state
                .queue
                .iter()
                .filter(|c| matches!(c, GpuCommand::Read { .. }))
                .count();
            if reads + waiting >= MAX_PENDING_READS {
                return Err(format!("{MAX_PENDING_READS} reads are already in flight"));
            }
        }
        state.queue.push_back(command);
        Ok(())
    }

    /// Commands waiting, over every plugin.
    pub fn queued(&self) -> usize {
        self.plugins.values().map(|p| p.queue.len()).sum()
    }

    /// Words a plugin's buffers hold.
    pub fn held_words(&self, plugin: &str) -> u64 {
        self.plugins.get(plugin).map_or(0, |p| p.held_words)
    }

    /// Invocations dispatched so far.
    pub fn dispatched(&self) -> u64 {
        self.dispatched
    }

    /// Forgets a plugin's buffers, queue and reads in flight.
    pub fn drop_plugin(&mut self, plugin: &str) {
        self.plugins.remove(plugin);
        self.reading.retain(|r| r.plugin != plugin);
    }

    /// Forgets everything but the kernels: a run ended.
    pub fn clear(&mut self) {
        self.plugins.clear();
        self.reading.clear();
    }

    /// Runs what the frame's budget allows, then collects finished reads.
    pub fn run(&mut self) -> Vec<Report> {
        let mut reports = Vec::new();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("plugin compute"),
            });
        let mut budget = FRAME_INVOCATIONS;
        let mut ran = false;
        let mut started = Vec::new();
        let names: Vec<String> = self.plugins.keys().cloned().collect();
        let count = names.len();
        for step in 0..count {
            let name = &names[(self.cursor + step) % count];
            loop {
                let Some(command) = self
                    .plugins
                    .get_mut(name)
                    .and_then(|p| p.queue.front().cloned())
                else {
                    break;
                };
                let cost = match &command {
                    GpuCommand::Dispatch { kernel, .. } => command.invocations(
                        self.kernels
                            .get(&(name.clone(), kernel.clone()))
                            .map(|k| &k.schema),
                    ),
                    _ => 0,
                };
                // A dispatch that doesn't fit waits, unless nothing has run
                // yet this frame (the per-dispatch cap fits the frame).
                if cost > budget && ran {
                    break;
                }
                self.plugins.get_mut(name).unwrap().queue.pop_front();
                budget = budget.saturating_sub(cost);
                if cost > 0 {
                    ran = true;
                }
                match self.execute(name, command, &mut encoder, &mut started) {
                    Ok(()) => {}
                    Err(message) => reports.push(Report::Error {
                        plugin: name.clone(),
                        message,
                    }),
                }
            }
        }
        if count > 0 {
            self.cursor = (self.cursor + 1) % count;
        }
        self.queue.submit([encoder.finish()]);
        for read in started {
            let landed = read.landed.clone();
            read.staging
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    *landed.lock().unwrap() = Some(result.map_err(|e| e.to_string()));
                });
            self.reading.push(read);
        }
        reports.extend(self.collect());
        reports
    }

    /// Reads that have landed since the last call.
    pub fn collect(&mut self) -> Vec<Report> {
        let _ = self.device.poll(wgpu::PollType::Poll);
        let mut reports = Vec::new();
        let mut waiting = Vec::new();
        for read in std::mem::take(&mut self.reading) {
            let outcome = read.landed.lock().unwrap().take();
            match outcome {
                None => waiting.push(read),
                Some(Err(e)) => reports.push(Report::Error {
                    plugin: read.plugin.clone(),
                    message: format!("read of {}: {e}", read.buffer),
                }),
                Some(Ok(())) => {
                    let words: Result<Vec<u32>, String> =
                        match read.staging.slice(..).get_mapped_range() {
                            Ok(view) => Ok(view
                                .chunks_exact(4)
                                .take(read.words as usize)
                                .map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
                                .collect()),
                            Err(e) => Err(e.to_string()),
                        };
                    read.staging.unmap();
                    reports.push(match words {
                        Ok(words) => Report::Read {
                            plugin: read.plugin,
                            tag: read.tag,
                            buffer: read.buffer,
                            offset: read.offset,
                            as_type: read.as_type,
                            words,
                        },
                        Err(e) => Report::Error {
                            plugin: read.plugin,
                            message: format!("read of {}: {e}", read.buffer),
                        },
                    });
                }
            }
        }
        self.reading = waiting;
        reports
    }

    fn execute(
        &mut self,
        plugin: &str,
        command: GpuCommand,
        encoder: &mut wgpu::CommandEncoder,
        started: &mut Vec<InFlight>,
    ) -> Result<(), String> {
        match command {
            GpuCommand::Buffer { name, words } => {
                let state = self.plugins.get_mut(plugin).unwrap();
                let replaced = state.buffers.get(&name).map_or(0, |b| u64::from(b.words));
                if replaced == 0 && state.buffers.len() >= MAX_BUFFERS {
                    return Err(format!("buffer {name}: {MAX_BUFFERS} buffers already"));
                }
                if state.held_words - replaced + u64::from(words) > MAX_PLUGIN_WORDS {
                    return Err(format!(
                        "buffer {name}: the plugin's buffers would pass {MAX_PLUGIN_WORDS} words"
                    ));
                }
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("plugin {plugin}/{name}")),
                    size: round_up(words),
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::UNIFORM
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                state.held_words = state.held_words - replaced + u64::from(words);
                state.buffers.insert(name, Buffer { buffer, words });
                Ok(())
            }
            GpuCommand::Write {
                buffer,
                offset,
                data,
            } => {
                let target = self.buffer(plugin, &buffer)?;
                let end = u64::from(offset) + data.len() as u64;
                if end > u64::from(target.words) {
                    return Err(format!(
                        "write to {buffer}: words {offset}..{end} are past its {} words",
                        target.words
                    ));
                }
                // Through a staging copy so the write lands in order with
                // the dispatches around it, not at the start of the frame.
                let bytes = data.len() as u64 * 4;
                let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("plugin write"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: true,
                });
                let packed: Vec<u8> = data.iter().flat_map(|w| w.to_ne_bytes()).collect();
                staging
                    .slice(..)
                    .get_mapped_range_mut()
                    .map_err(|e| format!("write to {buffer}: {e}"))?
                    .copy_from_slice(&packed);
                staging.unmap();
                encoder.copy_buffer_to_buffer(
                    &staging,
                    0,
                    &target.buffer,
                    u64::from(offset) * 4,
                    bytes,
                );
                Ok(())
            }
            GpuCommand::Dispatch {
                kernel,
                bindings,
                groups,
            } => {
                let loaded = self
                    .kernels
                    .get(&(plugin.to_string(), kernel.clone()))
                    .ok_or_else(|| format!("dispatch {kernel}: no such kernel"))?;
                let total: u64 = groups.iter().map(|g| u64::from(*g)).product::<u64>()
                    * u64::from(loaded.schema.workgroup_size.iter().product::<u32>());
                if total > MAX_DISPATCH_INVOCATIONS {
                    return Err(format!(
                        "dispatch {kernel}: {total} invocations, at most {MAX_DISPATCH_INVOCATIONS}"
                    ));
                }
                let given: BTreeMap<&str, &str> = bindings
                    .iter()
                    .map(|(binding, buffer)| (binding.as_str(), buffer.as_str()))
                    .collect();
                if given.len() != bindings.len() {
                    return Err(format!("dispatch {kernel}: a binding is named twice"));
                }
                if let Some(unknown) = given.keys().find(|b| loaded.schema.binding(b).is_none()) {
                    return Err(format!("dispatch {kernel}: no binding named {unknown}"));
                }
                let mut entries = Vec::new();
                let mut writers = BTreeSet::new();
                let mut users = BTreeMap::<&str, usize>::new();
                for declared in &loaded.schema.bindings {
                    let buffer_name = given.get(declared.name.as_str()).ok_or_else(|| {
                        format!("dispatch {kernel}: {} is not bound", declared.name)
                    })?;
                    let buffer = self.buffer(plugin, buffer_name)?;
                    let needed = loaded
                        .info
                        .min_sizes
                        .get(&declared.binding)
                        .copied()
                        .unwrap_or(0);
                    if round_up(buffer.words) < needed {
                        return Err(format!(
                            "dispatch {kernel}: {} needs a buffer of {needed} bytes, {buffer_name} has {}",
                            declared.name,
                            u64::from(buffer.words) * 4
                        ));
                    }
                    if declared.kind == BindingKind::Uniform && round_up(buffer.words) > 64 * 1024 {
                        return Err(format!(
                            "dispatch {kernel}: {} is a uniform, and {buffer_name} is over 64 KiB",
                            declared.name
                        ));
                    }
                    if declared.kind == BindingKind::ReadWrite {
                        writers.insert(*buffer_name);
                    }
                    *users.entry(buffer_name).or_default() += 1;
                    entries.push(wgpu::BindGroupEntry {
                        binding: declared.binding,
                        resource: buffer.buffer.as_entire_binding(),
                    });
                }
                if let Some(shared) = writers.iter().find(|b| users[**b] > 1) {
                    return Err(format!(
                        "dispatch {kernel}: {shared} is written and bound twice"
                    ));
                }
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("plugin kernel"),
                    layout: &loaded.layout,
                    entries: &entries,
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(&kernel),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&loaded.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
                drop(pass);
                self.dispatched += total;
                Ok(())
            }
            GpuCommand::Read {
                buffer,
                offset,
                words,
                tag,
                as_type,
            } => {
                let source = self.buffer(plugin, &buffer)?;
                let end = u64::from(offset) + u64::from(words);
                if end > u64::from(source.words) {
                    return Err(format!(
                        "read of {buffer}: words {offset}..{end} are past its {} words",
                        source.words
                    ));
                }
                let bytes = u64::from(words) * 4;
                let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("plugin read"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                encoder.copy_buffer_to_buffer(
                    &source.buffer,
                    u64::from(offset) * 4,
                    &staging,
                    0,
                    bytes,
                );
                started.push(InFlight {
                    plugin: plugin.to_string(),
                    tag,
                    buffer,
                    offset,
                    as_type,
                    words,
                    staging,
                    landed: Arc::new(Mutex::new(None)),
                });
                Ok(())
            }
            GpuCommand::Free { buffer } => {
                let state = self.plugins.get_mut(plugin).unwrap();
                let freed = state
                    .buffers
                    .remove(&buffer)
                    .ok_or_else(|| format!("free {buffer}: no such buffer"))?;
                state.held_words -= u64::from(freed.words);
                Ok(())
            }
        }
    }

    fn buffer(&self, plugin: &str, name: &str) -> Result<&Buffer, String> {
        self.plugins
            .get(plugin)
            .and_then(|p| p.buffers.get(name))
            .ok_or_else(|| format!("no buffer named {name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_plugin_api::compute::KernelBinding;

    /// A device, or `None` where there is no adapter (CI runs these with
    /// lavapipe: `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`).
    fn engine() -> Option<ComputeEngine> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(ComputeEngine::new(device, queue))
    }

    const DOUBLE: &str = "
        @group(0) @binding(0) var<storage, read> input: array<f32>;
        @group(0) @binding(1) var<storage, read_write> output: array<f32>;
        @compute @workgroup_size(64)
        fn main(@builtin(global_invocation_id) id: vec3<u32>) {
            if (id.x < arrayLength(&input)) { output[id.x] = input[id.x] * 2.0; }
        }";

    fn double() -> LoadoutKernel {
        LoadoutKernel {
            plugin: "p".to_string(),
            schema: KernelSchema {
                name: "double".to_string(),
                file: "double.wgsl".to_string(),
                entry: "main".to_string(),
                workgroup_size: [64, 1, 1],
                bindings: vec![
                    KernelBinding {
                        name: "input".to_string(),
                        binding: 0,
                        kind: BindingKind::Read,
                    },
                    KernelBinding {
                        name: "output".to_string(),
                        binding: 1,
                        kind: BindingKind::ReadWrite,
                    },
                ],
                description: String::new(),
            },
            source: DOUBLE.to_string(),
        }
    }

    fn buffer(name: &str, words: u32) -> GpuCommand {
        GpuCommand::Buffer {
            name: name.to_string(),
            words,
        }
    }

    fn dispatch(bindings: &[(&str, &str)], groups: u32) -> GpuCommand {
        GpuCommand::Dispatch {
            kernel: "double".to_string(),
            bindings: bindings
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            groups: [groups, 1, 1],
        }
    }

    fn read(buffer: &str, words: u32, tag: &str) -> GpuCommand {
        GpuCommand::Read {
            buffer: buffer.to_string(),
            offset: 0,
            words,
            tag: tag.to_string(),
            as_type: ReadAs::F32,
        }
    }

    fn finish(engine: &mut ComputeEngine, mut reports: Vec<Report>) -> Vec<Report> {
        for _ in 0..200 {
            if engine.reading.is_empty() {
                break;
            }
            let _ = engine.device.poll(wgpu::PollType::wait_indefinitely());
            reports.extend(engine.collect());
        }
        reports
    }

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn a_kernel_runs_over_host_buffers_and_a_read_returns_the_words() {
        let Some(mut engine) = engine() else { return };
        assert!(engine.set_kernels(&[double()]).is_empty());
        let data: Vec<f32> = (0..100).map(|i| i as f32).collect();
        for command in [
            buffer("a", 100),
            buffer("b", 100),
            GpuCommand::Write {
                buffer: "a".to_string(),
                offset: 0,
                data: data.iter().map(|v| v.to_bits()).collect(),
            },
            dispatch(&[("input", "a"), ("output", "b")], 2),
            read("b", 100, "doubled"),
        ] {
            engine.submit("p", command).unwrap();
        }
        let reports = engine.run();
        let reports = finish(&mut engine, reports);
        let [Report::Read { tag, words, .. }] = &reports[..] else {
            panic!("{reports:?}");
        };
        assert_eq!(tag, "doubled");
        let doubled: Vec<f32> = words.iter().map(|w| f32::from_bits(*w)).collect();
        let want: Vec<f32> = data.iter().map(|v| v * 2.0).collect();
        assert_eq!(doubled, want);
        assert_eq!(engine.held_words("p"), 200);
        assert_eq!(engine.dispatched(), 128);
    }

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn commands_run_in_order_so_a_write_after_a_dispatch_is_not_seen_by_it() {
        let Some(mut engine) = engine() else { return };
        assert!(engine.set_kernels(&[double()]).is_empty());
        let bits = |v: f32| vec![v.to_bits()];
        for command in [
            buffer("a", 1),
            buffer("b", 1),
            GpuCommand::Write {
                buffer: "a".to_string(),
                offset: 0,
                data: bits(1.0),
            },
            dispatch(&[("input", "a"), ("output", "b")], 1),
            GpuCommand::Write {
                buffer: "a".to_string(),
                offset: 0,
                data: bits(5.0),
            },
            dispatch(&[("input", "a"), ("output", "b")], 1),
            read("b", 1, "last"),
        ] {
            engine.submit("p", command).unwrap();
        }
        let reports = engine.run();
        let reports = finish(&mut engine, reports);
        let [Report::Read { words, .. }] = &reports[..] else {
            panic!("{reports:?}");
        };
        assert_eq!(f32::from_bits(words[0]), 10.0);
    }

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn bad_commands_are_reported_and_the_rest_still_run() {
        let Some(mut engine) = engine() else { return };
        assert!(engine.set_kernels(&[double()]).is_empty());
        for command in [
            buffer("a", 4),
            buffer("b", 4),
            dispatch(&[("input", "a")], 1), // output unbound
            dispatch(&[("input", "a"), ("output", "a")], 1), // written and bound twice
            dispatch(&[("input", "a"), ("output", "ghost")], 1), // no such buffer
            GpuCommand::Write {
                buffer: "a".to_string(),
                offset: 3,
                data: vec![0, 0],
            }, // past the end
            read("b", 4, "ok"),
        ] {
            engine.submit("p", command).unwrap();
        }
        let reports = engine.run();
        let reports = finish(&mut engine, reports);
        let errors: Vec<&str> = reports
            .iter()
            .filter_map(|r| match r {
                Report::Error { message, .. } => Some(message.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert!(errors[0].contains("not bound"), "{errors:?}");
        assert!(errors[1].contains("bound twice"), "{errors:?}");
        assert!(errors[2].contains("ghost"), "{errors:?}");
        assert!(errors[3].contains("past its"), "{errors:?}");
        assert!(
            reports
                .iter()
                .any(|r| matches!(r, Report::Read { tag, .. } if tag == "ok"))
        );
    }

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn memory_and_count_limits_hold_and_free_gives_words_back() {
        let Some(mut engine) = engine() else { return };
        let words = u32::try_from(MAX_PLUGIN_WORDS / 2).unwrap();
        for command in [
            buffer("a", words),
            buffer("b", words),
            buffer("c", 16), // over the plugin's cap
            GpuCommand::Free {
                buffer: "b".to_string(),
            },
            buffer("c", 16),
            GpuCommand::Free {
                buffer: "ghost".to_string(),
            },
        ] {
            engine.submit("p", command).unwrap();
        }
        let reports = engine.run();
        let errors: Vec<_> = reports
            .iter()
            .filter_map(|r| match r {
                Report::Error { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains("would pass"));
        assert!(errors[1].contains("no such buffer"));
        assert_eq!(engine.held_words("p"), u64::from(words) + 16);
    }

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn work_over_the_frame_budget_waits_for_the_next_frame_in_order() {
        let Some(mut engine) = engine() else { return };
        assert!(engine.set_kernels(&[double()]).is_empty());
        engine.submit("p", buffer("a", 4)).unwrap();
        engine.submit("p", buffer("b", 4)).unwrap();
        // Each of these is 64 * 65535 * 1 invocations: a frame holds 32 of them.
        for _ in 0..40 {
            engine
                .submit("p", dispatch(&[("input", "a"), ("output", "b")], 65_535))
                .unwrap();
        }
        let reports = engine.run();
        assert!(reports.is_empty(), "{reports:?}");
        let left = engine.queued();
        assert!(left > 0 && left < 40, "{left} left after one frame");
        while engine.queued() > 0 {
            engine.run();
        }
        assert_eq!(engine.dispatched(), 40 * 64 * 65_535);
    }

    #[test]
    #[ignore = "needs a GPU or lavapipe"]
    fn a_kernel_that_fails_the_check_is_left_out() {
        let Some(mut engine) = engine() else { return };
        let mut bad = double();
        bad.source = "@compute @workgroup_size(64) fn main() { loop { } }".to_string();
        let errors = engine.set_kernels(&[bad]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(!engine.has_kernel("p", "double"));
    }

    #[test]
    fn a_queue_and_its_reads_are_capped_without_a_device() {
        // Limits are enforced at submit, so no device is needed to see them.
        let Some(mut engine) = engine() else { return };
        for _ in 0..MAX_PENDING_READS {
            engine.submit("p", read("a", 1, "t")).unwrap();
        }
        assert!(engine.submit("p", read("a", 1, "t")).is_err());
        for _ in 0..(MAX_QUEUED - MAX_PENDING_READS) {
            engine.submit("p", buffer("x", 1)).unwrap();
        }
        assert!(
            engine
                .submit("p", buffer("x", 1))
                .unwrap_err()
                .contains("waiting")
        );
        assert!(engine.submit("p", buffer("", 1)).is_err());
    }
}
