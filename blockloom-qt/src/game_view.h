// A guard rather than #pragma once: the build copies this header, so it is
// reached by two paths.
#ifndef BLOCKLOOM_GAME_VIEW_H
#define BLOCKLOOM_GAME_VIEW_H

#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickItem>

class GameViewNode;
class PointerLock;

// The Game view: shows the embedded world's newest frame, straight from the
// GPU image it was drawn into. On Linux the images are dma-bufs imported
// through EGL, so Qt has to be on its OpenGL renderer.
class GameView : public QQuickItem
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(bool hasFrame READ hasFrame NOTIFY hasFrameChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
    // Asks for the pointer pinned over the view; held says it really is.
    Q_PROPERTY(bool pointerLocked READ pointerLocked WRITE setPointerLocked NOTIFY pointerLockedChanged)
    Q_PROPERTY(bool pointerHeld READ pointerHeld NOTIFY pointerHeldChanged)

public:
    explicit GameView(QQuickItem *parent = nullptr);
    ~GameView() override;

    bool hasFrame() const { return m_hasFrame; }
    QString error() const { return m_error; }
    bool pointerLocked() const { return m_pointerLocked; }
    void setPointerLocked(bool locked);
    bool pointerHeld() const;

    // A new frame, ring or no world at all: re-read and redraw. GUI thread.
    void wake();

Q_SIGNALS:
    void hasFrameChanged();
    void errorChanged();
    void pointerLockedChanged();
    void pointerHeldChanged();
    // Raw motion while held, coalesced.
    void pointerMoved(qreal dx, qreal dy);
    // The system broke the lock; asking again needs a fresh request.
    void pointerReleased();

protected:
    QSGNode *updatePaintNode(QSGNode *old, UpdatePaintNodeData *) override;

private:
    void fail(const QString &message);
    // Before Qt draws: copies an external-only frame into a plain texture.
    void renderExternal();

    // Render thread only: the ring imported, and the node holding it.
    quint64 m_generation = 0;
    GameViewNode *m_node = nullptr;
    QQuickWindow *m_hooked = nullptr;
    bool m_hasFrame = false;
    bool m_pointerLocked = false;
    PointerLock *m_lock = nullptr;
    QString m_error;
};

// Called by Rust from any thread when the exchange changes.
void game_view_wake();
// Before any window exists: the Game view imports through EGL into GL.
void game_view_prefer_opengl();

#endif
