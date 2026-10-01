#ifndef BLOCKLOOM_GAME_VIEW_VULKAN_H
#define BLOCKLOOM_GAME_VIEW_VULKAN_H

#include <QtQuick/QSGTexture>
#include <QtGui/QVulkanFunctions>
#include <vector>
#include "blockloom/src/game_view.cxx.h"

// Render-thread owner of the imported images and their dedicated memory.
class GameVulkanRing
{
public:
    ~GameVulkanRing();
    QString import(const GameFrames &frames, QQuickWindow *window);
    QString select(int index);
    void release();
    QSGTexture *texture(int index) const { return images[index].texture; }
    size_t count() const { return images.size(); }
    QSize size;

private:
    struct Slot {
        VkImage image = VK_NULL_HANDLE;
        VkDeviceMemory memory = VK_NULL_HANDLE;
        QSGTexture *texture = nullptr;
    };
    QString transfer(int index, bool acquire);
    VkDevice device = VK_NULL_HANDLE;
    VkQueue queue = VK_NULL_HANDLE;
    uint32_t family = 0;
    QVulkanDeviceFunctions *functions = nullptr;
    std::vector<Slot> images;
    int held = -1;
};

#endif
