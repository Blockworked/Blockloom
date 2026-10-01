#define VK_USE_PLATFORM_WIN32_KHR
#define NOMINMAX
#include "game_view_vulkan.h"
#include <QtQuick/QSGRendererInterface>
#include <QtQuick/qsgtexture_platform.h>
#include <cstring>

namespace {
QString failed(const char *action, VkResult result)
{
    return QStringLiteral("%1 (Vulkan %2)").arg(QLatin1String(action)).arg(int(result));
}
}

GameVulkanRing::~GameVulkanRing() { release(); }

void GameVulkanRing::release()
{
    if (functions && device) {
        functions->vkDeviceWaitIdle(device);
        if (held >= 0)
            transfer(held, false);
        for (Slot &slot : images) {
            delete slot.texture;
            if (slot.image)
                functions->vkDestroyImage(device, slot.image, nullptr);
            if (slot.memory)
                functions->vkFreeMemory(device, slot.memory, nullptr);
        }
    }
    images.clear();
    held = -1;
    size = {};
}

QString GameVulkanRing::import(const GameFrames &frames, QQuickWindow *window)
{
    release();
    // Win32 imports retain the allocation, but don't consume the handle.
    struct Handles {
        const GameFrames &frames;
        ~Handles() { for (size_t handle : frames.handles) CloseHandle(reinterpret_cast<HANDLE>(handle)); }
    } handles{frames};
    auto *ri = window->rendererInterface();
    auto *instance = window->vulkanInstance();
    auto *vkDevice = static_cast<VkDevice *>(ri->getResource(window, QSGRendererInterface::DeviceResource));
    auto *physical = static_cast<VkPhysicalDevice *>(ri->getResource(window, QSGRendererInterface::PhysicalDeviceResource));
    auto *vkQueue = static_cast<VkQueue *>(ri->getResource(window, QSGRendererInterface::CommandQueueResource));
    auto *queueFamily = static_cast<uint32_t *>(ri->getResource(window, QSGRendererInterface::GraphicsQueueFamilyIndexResource));
    if (!instance || !vkDevice || !physical || !vkQueue || !queueFamily)
        return QStringLiteral("Qt's Vulkan device is unavailable");
    device = *vkDevice;
    queue = *vkQueue;
    family = *queueFamily;
    functions = instance->deviceFunctions(device);
    auto importMemory = reinterpret_cast<PFN_vkGetMemoryWin32HandlePropertiesKHR>(
        instance->functions()->vkGetDeviceProcAddr(device, "vkGetMemoryWin32HandlePropertiesKHR"));
    if (!importMemory)
        return QStringLiteral("Qt's device cannot import Win32 Vulkan memory");
    VkPhysicalDeviceIDProperties id{};
    id.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES;
    VkPhysicalDeviceProperties2 properties{};
    properties.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2;
    properties.pNext = &id;
    auto getProperties = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties2>(
        instance->getInstanceProcAddr("vkGetPhysicalDeviceProperties2"));
    if (!getProperties)
        getProperties = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties2>(
            instance->getInstanceProcAddr("vkGetPhysicalDeviceProperties2KHR"));
    if (!getProperties)
        return QStringLiteral("Qt's Vulkan instance cannot query device identity");
    getProperties(*physical, &properties);
    if (frames.device_uuid.size() != VK_UUID_SIZE
        || std::memcmp(id.deviceUUID, frames.device_uuid.data(), VK_UUID_SIZE))
        return QStringLiteral("Qt and the game world selected different Vulkan GPUs");
    if (frames.handles.empty() || frames.allocation_sizes.size() != frames.handles.size()
        || frames.memory_types.size() != frames.handles.size())
        return QStringLiteral("invalid Vulkan ring metadata");
    size = QSize(int(frames.width), int(frames.height));
    for (size_t index = 0; index < frames.handles.size(); ++index) {
        images.emplace_back();
        Slot &slot = images.back();
        VkExternalMemoryImageCreateInfo external{};
        external.sType = VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO;
        external.handleTypes = VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32_BIT;
        VkImageCreateInfo image{};
        image.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
        image.pNext = &external;
        image.flags = VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT;
        image.imageType = VK_IMAGE_TYPE_2D;
        image.format = VK_FORMAT_R8G8B8A8_SRGB;
        image.extent = {frames.width, frames.height, 1};
        image.mipLevels = image.arrayLayers = 1;
        image.samples = VK_SAMPLE_COUNT_1_BIT;
        image.tiling = VK_IMAGE_TILING_OPTIMAL;
        image.usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_SAMPLED_BIT;
        VkResult result = functions->vkCreateImage(device, &image, nullptr, &slot.image);
        if (result != VK_SUCCESS)
            return failed("creating an imported image", result);
        VkMemoryRequirements requirements{};
        functions->vkGetImageMemoryRequirements(device, slot.image, &requirements);
        if (frames.memory_types[index] >= 32 || requirements.size > frames.allocation_sizes[index]
            || !(requirements.memoryTypeBits & (1u << frames.memory_types[index])))
            return QStringLiteral("incompatible Vulkan image allocation");
        VkImportMemoryWin32HandleInfoKHR imported{};
        imported.sType = VK_STRUCTURE_TYPE_IMPORT_MEMORY_WIN32_HANDLE_INFO_KHR;
        imported.handleType = VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32_BIT;
        imported.handle = reinterpret_cast<HANDLE>(frames.handles[index]);
        VkMemoryDedicatedAllocateInfo dedicated{};
        dedicated.sType = VK_STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO;
        dedicated.pNext = &imported;
        dedicated.image = slot.image;
        VkMemoryAllocateInfo allocation{};
        allocation.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
        allocation.pNext = &dedicated;
        allocation.allocationSize = frames.allocation_sizes[index];
        allocation.memoryTypeIndex = frames.memory_types[index];
        result = functions->vkAllocateMemory(device, &allocation, nullptr, &slot.memory);
        if (result != VK_SUCCESS)
            return failed("importing Vulkan memory", result);
        result = functions->vkBindImageMemory(device, slot.image, slot.memory, 0);
        if (result != VK_SUCCESS)
            return failed("binding imported Vulkan memory", result);
        slot.texture = QNativeInterface::QSGVulkanTexture::fromNative(
            slot.image, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, window, size);
        if (!slot.texture)
            return QStringLiteral("wrapping an imported Vulkan image failed");
    }
    return {};
}

QString GameVulkanRing::select(int index)
{
    if (index == held)
        return {};
    // Qt can have several frames in flight. Wait before returning an old
    // slot to the producer or recording ownership barriers on this queue.
    const VkResult result = functions->vkQueueWaitIdle(queue);
    if (result != VK_SUCCESS)
        return failed("waiting for Qt's Vulkan queue", result);
    if (held >= 0) {
        const QString error = transfer(held, false);
        if (!error.isEmpty())
            return error;
        held = -1;
    }
    const QString error = transfer(index, true);
    if (error.isEmpty())
        held = index;
    return error;
}

QString GameVulkanRing::transfer(int index, bool acquire)
{
    VkCommandPoolCreateInfo poolInfo{};
    poolInfo.sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO;
    poolInfo.queueFamilyIndex = family;
    VkCommandPool pool = VK_NULL_HANDLE;
    VkResult result = functions->vkCreateCommandPool(device, &poolInfo, nullptr, &pool);
    if (result != VK_SUCCESS)
        return failed("creating a Vulkan transfer pool", result);
    VkCommandBufferAllocateInfo allocation{};
    allocation.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO;
    allocation.commandPool = pool;
    allocation.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
    allocation.commandBufferCount = 1;
    VkCommandBuffer command = VK_NULL_HANDLE;
    result = functions->vkAllocateCommandBuffers(device, &allocation, &command);
    if (result == VK_SUCCESS) {
        VkCommandBufferBeginInfo begin{};
        begin.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO;
        begin.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
        result = functions->vkBeginCommandBuffer(command, &begin);
    }
    if (result == VK_SUCCESS) {
        VkImageMemoryBarrier barrier{};
        barrier.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
        barrier.oldLayout = barrier.newLayout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
        barrier.srcQueueFamilyIndex = acquire ? VK_QUEUE_FAMILY_EXTERNAL : family;
        barrier.dstQueueFamilyIndex = acquire ? family : VK_QUEUE_FAMILY_EXTERNAL;
        barrier.srcAccessMask = acquire ? 0 : VK_ACCESS_SHADER_READ_BIT;
        barrier.dstAccessMask = acquire ? VK_ACCESS_SHADER_READ_BIT : 0;
        barrier.image = images[index].image;
        barrier.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
        functions->vkCmdPipelineBarrier(command, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT,
            VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, 0, 0, nullptr, 0, nullptr, 1, &barrier);
        result = functions->vkEndCommandBuffer(command);
    }
    if (result == VK_SUCCESS) {
        VkSubmitInfo submit{};
        submit.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO;
        submit.commandBufferCount = 1;
        submit.pCommandBuffers = &command;
        result = functions->vkQueueSubmit(queue, 1, &submit, VK_NULL_HANDLE);
        if (result == VK_SUCCESS)
            result = functions->vkQueueWaitIdle(queue);
    }
    functions->vkDestroyCommandPool(device, pool, nullptr);
    return result == VK_SUCCESS ? QString{} : failed("transferring Vulkan image ownership", result);
}
