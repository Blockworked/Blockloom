// A guard rather than #pragma once: the build copies this header, so it is
// reached by two paths.
#ifndef BLOCKLOOM_GAME_VIEW_H
#define BLOCKLOOM_GAME_VIEW_H

#include <QtCore/QTimer>
#include <QtCore/QPointer>
#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickItem>

class GameViewNode;
class PointerLock;

// The Game view: shows the embedded world's newest frame, straight from the
// GPU image it was drawn into. On Linux the images are dma-bufs imported
// through EGL, so Qt has to be on its OpenGL renderer. Where the GPU can't
// share, the world reads its target back and the view uploads those bytes.
// Windows shares Vulkan allocations through Win32 handles.
class GameView : public QQuickItem
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(bool hasFrame READ hasFrame NOTIFY hasFrameChanged)
    Q_PROPERTY(QString interfaceLayout READ interfaceLayout NOTIFY interfaceLayoutChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
    // Asks for the pointer pinned over the view; held says it really is.
    Q_PROPERTY(bool pointerLocked READ pointerLocked WRITE setPointerLocked NOTIFY pointerLockedChanged)
    Q_PROPERTY(bool pointerHeld READ pointerHeld NOTIFY pointerHeldChanged)
    // Physical pixels the world draws at; empty follows the item's own size.
    Q_PROPERTY(QSize resolution READ resolution WRITE setResolution NOTIFY resolutionChanged)

public:
    explicit GameView(QQuickItem *parent = nullptr);
    ~GameView() override;

    bool hasFrame() const { return m_hasFrame; }
    QString interfaceLayout() const { return m_interfaceLayout; }
    QString error() const { return m_error; }
    bool pointerLocked() const { return m_pointerLocked; }
    void setPointerLocked(bool locked);
    bool pointerHeld() const;
    QSize resolution() const { return m_resolution; }
    void setResolution(const QSize &resolution);

    // A new frame, ring or no world at all: re-read and redraw. GUI thread.
    void wake();

Q_SIGNALS:
    void hasFrameChanged();
    void errorChanged();
    void interfaceLayoutChanged();
    void pointerLockedChanged();
    void pointerHeldChanged();
    void resolutionChanged();
    // Raw motion while held, coalesced.
    void pointerMoved(qreal dx, qreal dy);
    // The system broke the lock; asking again needs a fresh request.
    void pointerReleased();

protected:
    QSGNode *updatePaintNode(QSGNode *old, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private:
    void fail(const QString &message);
    // Follows `window`'s render and swap signals. GUI thread.
    void hook(QQuickWindow *window);
    // Before Qt draws: copies an external-only frame into a plain texture.
    void renderExternal();
    // After each swap: paces the world, and asks for the next frame while one runs.
    void framePresented();
    // Tells the world what size to draw at. The first size goes straight
    // away; changes after it wait for a pause, since each one rebuilds the ring.
    void resize();
    void sendSize();

    // Render thread only: the ring imported, and the node holding it.
    quint64 m_generation = 0;
    // The read-back frame shown, by the exchange's generation.
    quint64 m_shmGeneration = 0;
    GameViewNode *m_node = nullptr;
    bool m_sharingOffered = false;
    // GUI thread only.
    QPointer<QQuickWindow> m_hooked;
    bool m_hasFrame = false;
    bool m_pointerLocked = false;
    PointerLock *m_lock = nullptr;
    QSize m_resolution;
    QSize m_sentSize;
    qreal m_sentScale = 0;
    QTimer m_resizeTimer;
    QString m_error;
    QString m_interfaceLayout;
    QString m_renderLayout;
};

// Called by Rust from any thread when the exchange changes.
void game_view_wake();
// Before any window exists: EGL/OpenGL on Linux, Vulkan on Windows.
void game_view_prefer_renderer();
void game_view_configure_windows();

#endif
