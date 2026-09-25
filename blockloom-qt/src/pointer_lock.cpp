#include "pointer_lock.h"

#include <QtCore/QTimer>
#include <QtGui/QCursor>
#include <QtGui/QGuiApplication>
#include <QtGui/QMouseEvent>
#include <QtGui/QScreen>
#include <QtQuick/QQuickItem>
#include <QtQuick/QQuickWindow>

#ifdef __linux__
#include <QtGui/qguiapplication_platform.h>
#include <QtGui/qpa/qplatformnativeinterface.h>
#include <cstring>
#include <wayland-client.h>

#include "wayland/pointer-constraints-unstable-v1-client-protocol.h"
#include "wayland/relative-pointer-unstable-v1-client-protocol.h"
#endif

namespace {

#ifdef __linux__

struct Globals {
    bool looked = false;
    zwp_pointer_constraints_v1 *constraints = nullptr;
    zwp_relative_pointer_manager_v1 *relative = nullptr;
};

Globals globals;

void onGlobal(void *, wl_registry *registry, uint32_t name, const char *interface, uint32_t)
{
    if (!std::strcmp(interface, zwp_pointer_constraints_v1_interface.name))
        globals.constraints = static_cast<zwp_pointer_constraints_v1 *>(
            wl_registry_bind(registry, name, &zwp_pointer_constraints_v1_interface, 1));
    else if (!std::strcmp(interface, zwp_relative_pointer_manager_v1_interface.name))
        globals.relative = static_cast<zwp_relative_pointer_manager_v1 *>(
            wl_registry_bind(registry, name, &zwp_relative_pointer_manager_v1_interface, 1));
}

void onGlobalRemove(void *, wl_registry *, uint32_t) {}

const wl_registry_listener registryListener = {onGlobal, onGlobalRemove};

// Binds both managers once, on a private queue so Qt's own dispatch is left
// alone, then moves them to the default queue Qt dispatches on this thread.
bool bindGlobals(wl_display *display)
{
    if (!globals.looked) {
        globals.looked = true;
        wl_event_queue *queue = wl_display_create_queue(display);
        auto *wrapper = static_cast<wl_display *>(wl_proxy_create_wrapper(display));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(wrapper), queue);
        wl_registry *registry = wl_display_get_registry(wrapper);
        wl_proxy_wrapper_destroy(wrapper);
        wl_registry_add_listener(registry, &registryListener, nullptr);
        wl_display_roundtrip_queue(display, queue);
        wl_registry_destroy(registry);
        if (globals.constraints)
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(globals.constraints), nullptr);
        if (globals.relative)
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(globals.relative), nullptr);
        wl_event_queue_destroy(queue);
        if (!globals.constraints || !globals.relative)
            qWarning("Game view: this compositor can't lock the pointer");
    }
    return globals.constraints && globals.relative;
}

void onLocked(void *data, zwp_locked_pointer_v1 *)
{
    static_cast<PointerLock *>(data)->onLocked();
}

void onUnlocked(void *data, zwp_locked_pointer_v1 *)
{
    static_cast<PointerLock *>(data)->onUnlocked();
}

const zwp_locked_pointer_v1_listener lockedListener = {onLocked, onUnlocked};

void onRelativeMotion(void *data, zwp_relative_pointer_v1 *, uint32_t, uint32_t,
                      wl_fixed_t, wl_fixed_t, wl_fixed_t dx, wl_fixed_t dy)
{
    // Unaccelerated, as a game's own window would read it.
    static_cast<PointerLock *>(data)->onMotion(wl_fixed_to_double(dx), wl_fixed_to_double(dy));
}

const zwp_relative_pointer_v1_listener relativeListener = {onRelativeMotion};

#endif

bool onWayland()
{
    return QGuiApplication::platformName().startsWith(QLatin1String("wayland"));
}

} // namespace

PointerLock::PointerLock(QQuickItem *item)
    : m_item(item)
{
}

PointerLock::~PointerLock()
{
    unlock();
}

void PointerLock::setLocked(bool locked)
{
    if (locked == m_wanted)
        return;
    m_wanted = locked;
    if (!locked) {
        unlock();
        return;
    }
    if (!m_item->window())
        return;
    if (onWayland()) {
        if (!lockWayland())
            m_wanted = false;
    } else {
        lockWarp();
    }
}

bool PointerLock::lockWayland()
{
#ifdef __linux__
    auto *native = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>();
    QQuickWindow *window = m_item->window();
    if (!native || !native->pointer() || !bindGlobals(native->display()))
        return false;
    auto *surface = static_cast<wl_surface *>(
        QGuiApplication::platformNativeInterface()->nativeResourceForWindow("surface", window));
    if (!surface)
        return false;

    // Only over the item: the lock waits until the pointer gets there.
    const QRect rect = m_item->mapRectToScene(m_item->boundingRect()).toAlignedRect();
    const QMargins frame = window->frameMargins();
    wl_region *region = wl_compositor_create_region(native->compositor());
    wl_region_add(region, rect.x() + frame.left(), rect.y() + frame.top(), rect.width(), rect.height());
    m_locked = zwp_pointer_constraints_v1_lock_pointer(globals.constraints, surface, native->pointer(), region,
                                                       ZWP_POINTER_CONSTRAINTS_V1_LIFETIME_ONESHOT);
    wl_region_destroy(region);
    zwp_locked_pointer_v1_add_listener(m_locked, &lockedListener, this);
    m_relative = zwp_relative_pointer_manager_v1_get_relative_pointer(globals.relative, native->pointer());
    zwp_relative_pointer_v1_add_listener(m_relative, &relativeListener, this);
    wl_display_flush(native->display());
    return true;
#else
    return false;
#endif
}

void PointerLock::lockWarp()
{
    m_warping = true;
    m_item->window()->installEventFilter(this);
    const QPoint center = m_item->mapToGlobal(m_item->boundingRect().center()).toPoint();
    QCursor::setPos(m_item->window()->screen(), center);
    setHeld(true);
}

void PointerLock::unlock()
{
#ifdef __linux__
    if (m_locked) {
        zwp_locked_pointer_v1_destroy(m_locked);
        m_locked = nullptr;
    }
    if (m_relative) {
        zwp_relative_pointer_v1_destroy(m_relative);
        m_relative = nullptr;
    }
    if (auto *native = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>())
        wl_display_flush(native->display());
#endif
    if (m_warping) {
        m_warping = false;
        if (m_item->window())
            m_item->window()->removeEventFilter(this);
    }
    setHeld(false);
}

void PointerLock::setHeld(bool held)
{
    if (held == m_held)
        return;
    m_held = held;
    if (heldChanged)
        heldChanged();
}

void PointerLock::onLocked()
{
    setHeld(true);
}

void PointerLock::onUnlocked()
{
    // A oneshot lock is spent once broken; asking again takes a new one.
    unlock();
    m_wanted = false;
    if (released)
        released();
}

void PointerLock::onMotion(qreal dx, qreal dy)
{
    // Relative motion arrives whether or not the lock has landed.
    if (!m_held)
        return;
    m_dx += dx;
    m_dy += dy;
    if (m_flushQueued)
        return;
    m_flushQueued = true;
    QTimer::singleShot(8, this, [this] {
        m_flushQueued = false;
        const qreal dx = m_dx, dy = m_dy;
        m_dx = m_dy = 0;
        if (moved && m_held && (dx != 0 || dy != 0))
            moved(dx, dy);
    });
}

bool PointerLock::eventFilter(QObject *watched, QEvent *event)
{
    if (!m_warping || event->type() != QEvent::MouseMove)
        return QObject::eventFilter(watched, event);
    // Whatever moved the cursor off centre is the motion; put it back.
    const auto *move = static_cast<QMouseEvent *>(event);
    const QPoint center = m_item->mapToGlobal(m_item->boundingRect().center()).toPoint();
    const QPointF delta = move->globalPosition() - QPointF(center);
    if (!delta.isNull()) {
        onMotion(delta.x(), delta.y());
        QCursor::setPos(m_item->window()->screen(), center);
    }
    // Swallowed: the game hears raw motion, not a pointer pinned in place.
    return true;
}
