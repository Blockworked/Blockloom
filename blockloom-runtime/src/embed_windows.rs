//! Vulkan images shared with Qt through dedicated Win32 memory handles.

use super::*;
use ash::{khr, vk};
use bevy::camera::{ManualTextureViewHandle, NormalizedRenderTarget};
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::TextureView;
use bevy::render::renderer::{RenderDevice, RenderQueue, raw_vulkan_init::RawVulkanInitSettings};
use bevy::render::texture::{ManualTextureView, ManualTextureViews, OutputColorAttachment};
use bevy::render::view::{ViewTargetAttachments, clear_view_attachments, prepare_view_attachments};
use bevy::render::{Render, RenderApp, RenderSystems};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use wgpu::hal::api::Vulkan;

pub const VIEW: ManualTextureViewHandle = ManualTextureViewHandle(0xB10C);
const HANDLE: vk::ExternalMemoryHandleTypeFlags = vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32;
const USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING);

pub fn vulkan_settings() -> RawVulkanInitSettings {
    let mut settings = RawVulkanInitSettings::default();
    // SAFETY: only enables an extension the selected adapter supports.
    unsafe {
        settings.add_create_device_callback(|args, adapter, _| {
            let name = khr::external_memory_win32::NAME;
            if adapter
                .physical_device_capabilities()
                .supports_extension(name)
                && !args.extensions.contains(&name)
            {
                args.extensions.push(name);
            }
        });
    }
    settings
}

#[derive(Resource, Clone, ExtractResource, Default)]
#[extract_app(RenderApp)]
pub struct WindowsSurface {
    pub generation: u64,
    exchange: Option<Arc<FrameExchange>>,
    size: UVec2,
    textures: Vec<wgpu::Texture>,
    views: Vec<TextureView>,
    _scratch: Option<wgpu::Texture>,
    released: Vec<Arc<AtomicBool>>,
}

#[derive(Resource, Default)]
struct Drawing(Option<usize>);

pub fn add(app: &mut App) {
    app.init_resource::<WindowsSurface>()
        .add_plugins(ExtractResourcePlugin::<WindowsSurface>::default())
        .add_systems(Last, build.before(super::build_shm_target));
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<Drawing>().add_systems(
            Render,
            (
                aim.in_set(RenderSystems::PrepareViews)
                    .after(clear_view_attachments)
                    .before(prepare_view_attachments),
                finish.in_set(RenderSystems::Cleanup),
            ),
        );
    }
}

fn build(
    mut surface: ResMut<GameSurface>,
    mut shared: ResMut<WindowsSurface>,
    device: Res<RenderDevice>,
    mut views: ResMut<ManualTextureViews>,
    mut target_bytes: ResMut<crate::performance::GameViewTargetBytes>,
) {
    if !surface.exchange.sharing.load(Ordering::Acquire) {
        if shared.generation != 0 {
            surface.exchange.clear();
            *shared = WindowsSurface::default();
            views.remove(&VIEW);
        }
        return;
    }
    let wanted = surface.exchange.wanted();
    surface.viewport = wanted;
    if shared.generation != 0 && shared.size == wanted.size {
        return;
    }
    let gpu = device.wgpu_device();
    let allocated = (0..3)
        .map(|_| allocate(gpu, wanted.size))
        .collect::<Result<Vec<_>, _>>();
    let slots = match allocated {
        Ok(slots) => slots,
        Err(error) => {
            tracing::warn!("Game view: Vulkan sharing unavailable, using readback: {error}");
            surface.exchange.enable_sharing(false);
            return;
        }
    };
    // Finish the previous ring's submissions before replacing its resources.
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let scratch = gpu.create_texture(&descriptor(wanted.size));
    views.insert(
        VIEW,
        ManualTextureView {
            texture_view: scratch.create_view(&default()).into(),
            size: wanted.size,
            view_format: TARGET_FORMAT,
        },
    );
    let (textures, images): (Vec<_>, Vec<_>) = slots.into_iter().unzip();
    let texture_views = textures
        .iter()
        .map(|t| t.create_view(&default()).into())
        .collect();
    // Discard readbacks queued before Vulkan sharing became available.
    if let Ok(mut shm) = surface.exchange.shm.lock() {
        shm.pixels = None;
    }
    let generation = surface
        .exchange
        .install(wanted.size.x, wanted.size.y, 1, images);
    *shared = WindowsSurface {
        generation,
        exchange: Some(surface.exchange.clone()),
        size: wanted.size,
        released: textures
            .iter()
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect(),
        textures,
        views: texture_views,
        _scratch: Some(scratch),
    };
    target_bytes.0 = wanted.size.x as u64 * wanted.size.y as u64 * 4 * 4;
}

fn aim(
    shared: Res<WindowsSurface>,
    cameras: Query<&ExtractedCamera>,
    mut attachments: ResMut<ViewTargetAttachments>,
    mut drawing: ResMut<Drawing>,
    device: Res<RenderDevice>,
) {
    drawing.0 = None;
    let Some(exchange) = &shared.exchange else {
        return;
    };
    let target = NormalizedRenderTarget::TextureView(VIEW);
    if !cameras
        .iter()
        .any(|camera| camera.target.as_ref() == Some(&target))
    {
        return;
    }
    let Some(index) = exchange.claim(shared.generation) else {
        return;
    };
    // A new image starts on the producer; a recycled one comes from EXTERNAL.
    // wgpu's tracker retains RESOURCE after the previous frame's release.
    if shared.released[index].load(Ordering::Acquire)
        && let Err(error) = ownership(device.wgpu_device(), &shared.textures[index], true)
    {
        tracing::error!("Game view acquire: {error}");
        exchange.enable_sharing(false);
        return;
    }
    attachments.insert(
        target,
        OutputColorAttachment::new(shared.views[index].clone(), TARGET_FORMAT),
    );
    drawing.0 = Some(index);
}

fn finish(
    shared: Res<WindowsSurface>,
    mut drawing: ResMut<Drawing>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(index) = drawing.0.take() else {
        return;
    };
    let Some(exchange) = &shared.exchange else {
        return;
    };
    let texture = &shared.textures[index];
    let mut encoder =
        device
            .wgpu_device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("game view release"),
            });
    encoder.transition_resources(
        std::iter::empty(),
        std::iter::once(wgpu::TextureTransition {
            texture,
            selector: None,
            state: wgpu::TextureUses::RESOURCE,
        }),
    );
    queue.submit([encoder.finish()]);
    // Completion protects both the raw queue operation and Qt's first sample.
    if device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
        exchange.enable_sharing(false);
        return;
    }
    if let Err(error) = ownership(device.wgpu_device(), texture, false) {
        tracing::error!("Game view release: {error}");
        exchange.enable_sharing(false);
        return;
    }
    shared.released[index].store(true, Ordering::Release);
    exchange.finished(shared.generation, index);
}

fn descriptor(size: UVec2) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("shared game view"),
        size: wgpu::Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: USAGE,
        view_formats: &[],
    }
}

fn allocate(device: &wgpu::Device, size: UVec2) -> Result<(wgpu::Texture, SharedImage), String> {
    // SAFETY: images and dedicated memory are created on wgpu's device, then
    // transferred to its texture owner. Every failed allocation is freed here.
    unsafe {
        let hal = device
            .as_hal::<Vulkan>()
            .ok_or("the world isn't using Vulkan")?;
        if !hal
            .enabled_device_extensions()
            .contains(&khr::external_memory_win32::NAME)
        {
            return Err("VK_KHR_external_memory_win32 unavailable".into());
        }
        let raw = hal.raw_device();
        let instance = hal.shared_instance().raw_instance();
        let physical = hal.raw_physical_device();
        let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(HANDLE);
        let mut external_format =
            vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(HANDLE);
        let format_info = vk::PhysicalDeviceImageFormatInfo2::default()
            .format(vk::Format::R8G8B8A8_SRGB)
            .ty(vk::ImageType::TYPE_2D)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
            .push_next(&mut external_format);
        let mut external_properties = vk::ExternalImageFormatProperties::default();
        let mut format_properties =
            vk::ImageFormatProperties2::default().push_next(&mut external_properties);
        instance
            .get_physical_device_image_format_properties2(
                physical,
                &format_info,
                &mut format_properties,
            )
            .map_err(|e| format!("Win32 image sharing unsupported: {e}"))?;
        if !external_properties
            .external_memory_properties
            .external_memory_features
            .contains(
                vk::ExternalMemoryFeatureFlags::EXPORTABLE
                    | vk::ExternalMemoryFeatureFlags::IMPORTABLE,
            )
        {
            return Err("the RGBA image format cannot be shared through Win32 handles".into());
        }
        let info = vk::ImageCreateInfo::default()
            .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_SRGB)
            .extent(vk::Extent3D {
                width: size.x,
                height: size.y,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .push_next(&mut external);
        let image = raw.create_image(&info, None).map_err(|e| e.to_string())?;
        let needs = raw.get_image_memory_requirements(image);
        let properties = instance.get_physical_device_memory_properties(physical);
        let Some(memory_type) = (0..properties.memory_type_count)
            .filter(|&i| needs.memory_type_bits & (1 << i) != 0)
            .max_by_key(|&i| {
                properties.memory_types[i as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
        else {
            raw.destroy_image(image, None);
            return Err("no compatible memory type".into());
        };
        let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(HANDLE);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let info = vk::MemoryAllocateInfo::default()
            .allocation_size(needs.size)
            .memory_type_index(memory_type)
            .push_next(&mut export)
            .push_next(&mut dedicated);
        let memory = match raw.allocate_memory(&info, None) {
            Ok(memory) => memory,
            Err(e) => {
                raw.destroy_image(image, None);
                return Err(e.to_string());
            }
        };
        let handle = match raw.bind_image_memory(image, memory, 0).and_then(|()| {
            khr::external_memory_win32::Device::new(instance, raw).get_memory_win32_handle(
                &vk::MemoryGetWin32HandleInfoKHR::default()
                    .memory(memory)
                    .handle_type(HANDLE),
            )
        }) {
            Ok(handle) => OwnedHandle::from_raw_handle(handle as _),
            Err(e) => {
                raw.destroy_image(image, None);
                raw.free_memory(memory, None);
                return Err(e.to_string());
            }
        };
        let mut id = vk::PhysicalDeviceIDProperties::default();
        let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
        instance.get_physical_device_properties2(physical, &mut properties);
        let shared = SharedImage {
            handle,
            allocation_size: needs.size,
            memory_type,
            device_uuid: id.device_uuid,
            offset: 0,
            stride: size.x * 4,
        };
        let texture = hal.texture_from_raw(
            image,
            &wgpu::hal::TextureDescriptor {
                label: Some("shared game view"),
                size: descriptor(size).size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TARGET_FORMAT,
                usage: wgpu::TextureUses::COLOR_TARGET | wgpu::TextureUses::RESOURCE,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: vec![],
            },
            None,
            wgpu::hal::vulkan::TextureMemory::Dedicated(memory),
        );
        drop(hal);
        let texture = device.create_texture_from_hal::<Vulkan>(
            texture,
            &descriptor(size),
            wgpu::TextureUses::UNINITIALIZED,
        );
        Ok((texture, shared))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn images() -> Vec<SharedImage> {
        (0..3)
            .map(|_| SharedImage {
                handle: std::fs::File::open(std::env::current_exe().unwrap())
                    .unwrap()
                    .into(),
                allocation_size: 4096,
                memory_type: 0,
                device_uuid: [0; 16],
                offset: 0,
                stride: 64,
            })
            .collect()
    }

    #[test]
    fn a_reserved_frame_cannot_be_recycled_when_a_newer_frame_finishes() {
        let exchange = FrameExchange::new(|| {});
        let generation = exchange.install(16, 16, 1, images());
        let first = exchange.claim(generation).unwrap();
        exchange.finished(generation, first);
        assert!(exchange.reserve(generation, first));
        let second = exchange.claim(generation).unwrap();
        exchange.finished(generation, second);
        let third = exchange.claim(generation).unwrap();
        assert_ne!(third, first);
        assert!(exchange.claim(generation).is_none());
        exchange.hold(generation, first);
        assert!(exchange.claim(generation).is_none());
        assert!(exchange.reserve(generation, second));
        exchange.hold(generation, second);
        assert_eq!(exchange.claim(generation), Some(first));
    }

    #[test]
    fn stale_and_busy_frames_are_never_reserved() {
        let exchange = FrameExchange::new(|| {});
        let old = exchange.install(16, 16, 1, images());
        let slot = exchange.claim(old).unwrap();
        assert!(!exchange.reserve(old, slot));
        exchange.finished(old, slot);
        let new = exchange.install(32, 16, 1, images());
        assert!(!exchange.reserve(old, slot));
        assert!(!exchange.reserve(new, usize::MAX));
        exchange.hold(new, usize::MAX);
        assert!(exchange.claim(new).is_some());
        exchange.clear();
        assert!(!exchange.reserve(new, slot));
    }
}

fn ownership(device: &wgpu::Device, texture: &wgpu::Texture, acquire: bool) -> Result<(), String> {
    // SAFETY: called on the render thread after wgpu's submission completes.
    // The temporary command pool is destroyed only after its fence completes.
    unsafe {
        let hal = device.as_hal::<Vulkan>().ok_or("not Vulkan")?;
        let image = texture
            .as_hal::<Vulkan>()
            .ok_or("not a Vulkan texture")?
            .raw_handle();
        let raw = hal.raw_device();
        let pool = raw
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(hal.queue_family_index()),
                None,
            )
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let command = raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            raw.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            let barrier = vk::ImageMemoryBarrier::default()
                .image(image)
                .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(if acquire {
                    vk::QUEUE_FAMILY_EXTERNAL
                } else {
                    hal.queue_family_index()
                })
                .dst_queue_family_index(if acquire {
                    hal.queue_family_index()
                } else {
                    vk::QUEUE_FAMILY_EXTERNAL
                })
                .src_access_mask(if acquire {
                    vk::AccessFlags::empty()
                } else {
                    vk::AccessFlags::MEMORY_WRITE
                })
                .dst_access_mask(if acquire {
                    vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE
                } else {
                    vk::AccessFlags::empty()
                })
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .level_count(1)
                        .layer_count(1),
                );
            raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
            raw.end_command_buffer(command)?;
            let commands = [command];
            raw.queue_submit(
                hal.raw_queue(),
                &[vk::SubmitInfo::default().command_buffers(&commands)],
                vk::Fence::null(),
            )?;
            raw.queue_wait_idle(hal.raw_queue())
        })();
        raw.destroy_command_pool(pool, None);
        result.map_err(|e| e.to_string())?;
        Ok(())
    }
}
