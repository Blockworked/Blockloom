#ifndef BLOCKLOOM_POINTER_LOCK_H
#define BLOCKLOOM_POINTER_LOCK_H

#include <QtCore/QObject>

#include <functional>

class QQuickItem;
struct zwp_locked_pointer_v1;
struct zwp_relative_pointer_v1;

// Pins the pointer over an item and reports raw motion instead: Wayland's
// pointer constraints there, cursor warping elsewhere. GUI thread only.
class PointerLock : public QObject
{
public:
    explicit PointerLock(QQuickItem *item);
    ~PointerLock() override;

    void setLocked(bool locked);
    // The pointer really is pinned, not just asked for.
    bool held() const { return m_held; }

    std::function<void()> heldChanged;
    // The system let go on its own (a compositor shortcut, a lost grab).
    std::function<void()> released;
    // Motion since the last call, a few times a frame at most.
    std::function<void(qreal, qreal)> moved;

    // Wayland listener entry points.
    void onLocked();
    void onUnlocked();
    void onMotion(qreal dx, qreal dy);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    bool lockWayland();
    void lockWarp();
    void unlock();
    void setHeld(bool held);

    QQuickItem *m_item;
    bool m_wanted = false;
    bool m_held = false;
    bool m_warping = false;
    zwp_locked_pointer_v1 *m_locked = nullptr;
    zwp_relative_pointer_v1 *m_relative = nullptr;
    qreal m_dx = 0;
    qreal m_dy = 0;
    bool m_flushQueued = false;
};

#endif
