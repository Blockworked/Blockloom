#include "game_view.h"
#include "blockloom/src/game_view.cxx.h"
#include "pointer_lock.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QPointer>
#include <QtGui/QOpenGLContext>
#include <QtGui/QOpenGLExtraFunctions>
#include <QtGui/QOpenGLFunctions>
#include <QtQuick/QQuickWindow>
#include <QtQuick/QSGRendererInterface>
#include <QtQuick/QSGSimpleTextureNode>
#include <QtQuick/qsgtexture_platform.h>

#include <atomic>
#include <vector>

#ifdef __linux__
// Keep Xlib's macros (None, Bool, Status) out of a file that includes Qt.
#define EGL_NO_X11
#define MESA_EGL_NO_X11_HEADERS
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <QtGui/qguiapplication_platform.h>
#include <QtGui/qpa/qplatformnativeinterface.h>
#include <cstring>
#include <unistd.h>
#include <wayland-client.h>

#include "wayland/viewporter-client-protocol.h"
#endif

#ifndef GL_TEXTURE_EXTERNAL_OES
#define GL_TEXTURE_EXTERNAL_OES 0x8D65
#endif

namespace {

// Every live view, touched only on the GUI thread.
QList<QPointer<GameView>> &views()
{
    static QList<QPointer<GameView>> list;
    return list;
}

std::atomic<bool> wakePending{false};

constexpr uint32_t fourccXbgr8888 = 0x34324258; // "XB24"
constexpr uint64_t modifierLinear = 0;

#ifdef __linux__

using ImageTargetTexture = void (*)(GLenum, void *);

struct Egl {
    EGLDisplay display = EGL_NO_DISPLAY;
    PFNEGLCREATEIMAGEKHRPROC createImage = nullptr;
    PFNEGLDESTROYIMAGEKHRPROC destroyImage = nullptr;
    PFNEGLQUERYDMABUFMODIFIERSEXTPROC queryModifiers = nullptr;
    ImageTargetTexture imageTargetTexture = nullptr;
    bool modifiers = false;
};

Egl egl;

// The current context's EGL entry points, or why there are none.
QString loadEgl()
{
    egl.display = eglGetCurrentDisplay();
    if (egl.display == EGL_NO_DISPLAY)
        return QStringLiteral("Qt isn't rendering through EGL");
    const QByteArray extensions = eglQueryString(egl.display, EGL_EXTENSIONS);
    if (!extensions.contains("EGL_EXT_image_dma_buf_import"))
        return QStringLiteral("this driver can't import dma-bufs (EGL_EXT_image_dma_buf_import)");
    egl.modifiers = extensions.contains("EGL_EXT_image_dma_buf_import_modifiers");
    egl.createImage = reinterpret_cast<PFNEGLCREATEIMAGEKHRPROC>(eglGetProcAddress("eglCreateImageKHR"));
    egl.destroyImage = reinterpret_cast<PFNEGLDESTROYIMAGEKHRPROC>(eglGetProcAddress("eglDestroyImageKHR"));
    egl.imageTargetTexture = reinterpret_cast<ImageTargetTexture>(eglGetProcAddress("glEGLImageTargetTexture2DOES"));
    if (egl.modifiers)
        egl.queryModifiers = reinterpret_cast<PFNEGLQUERYDMABUFMODIFIERSEXTPROC>(eglGetProcAddress("eglQueryDmaBufModifiersEXT"));
    if (!egl.createImage || !egl.destroyImage || !egl.imageTargetTexture)
        return QStringLiteral("EGL image functions are missing");
    return {};
}

struct Modifier {
    EGLuint64KHR modifier;
    EGLBoolean externalOnly;
};

// Every modifier the driver imports `fourcc` in.
std::vector<Modifier> modifiersFor(uint32_t fourcc)
{
    if (!egl.queryModifiers)
        return {};
    EGLint count = 0;
    if (!egl.queryModifiers(egl.display, EGLint(fourcc), 0, nullptr, nullptr, &count) || count <= 0)
        return {};
    std::vector<EGLuint64KHR> modifiers(count);
    std::vector<EGLBoolean> external(count);
    egl.queryModifiers(egl.display, EGLint(fourcc), count, modifiers.data(), external.data(), &count);
    std::vector<Modifier> all;
    for (EGLint i = 0; i < count; ++i)
        all.push_back({modifiers[i], external[i]});
    return all;
}

// Whether the driver will only sample this format and modifier as an
// external texture - NVIDIA says so for linear buffers.
bool externalOnly(uint32_t fourcc, uint64_t modifier)
{
    for (const Modifier &known : modifiersFor(fourcc)) {
        if (known.modifier == modifier)
            return known.externalOnly;
    }
    return false;
}

// Tells the world which tiled layouts Qt can sample as a plain texture, so
// it draws straight into one. Once, on the render thread with GL current.
void offerModifiers()
{
    static bool offered = false;
    if (offered)
        return;
    offered = true;
    if (!egl.createImage && !loadEgl().isEmpty())
        return;
    std::vector<uint64_t> direct;
    for (const Modifier &known : modifiersFor(fourccXbgr8888)) {
        if (!known.externalOnly && known.modifier != modifierLinear)
            direct.push_back(known.modifier);
    }
    game_view_accept(rust::Slice<const uint64_t>(direct.data(), direct.size()));
}

// ─── The HDR plane ──────────────────────────────────────────────────────────
//
// Qt's window is 8-bit, so HDR frames can't be drawn into it. Instead the
// world presents them to a Wayland subsurface of its own, placed under the
// window, and the view leaves its own pixels see-through while it does. The
// plane is made once, for the first window with a Game view in it, and lives
// as long as the process: the world holds a swapchain on it.

bool onWayland()
{
    return QGuiApplication::platformName().startsWith(QLatin1String("wayland"));
}

struct Plane {
    bool tried = false;
    QQuickWindow *window = nullptr;
    wl_subcompositor *subcompositor = nullptr;
    wp_viewporter *viewporter = nullptr;
    wl_surface *surface = nullptr;
    wl_subsurface *subsurface = nullptr;
    wp_viewport *viewport = nullptr;
    // Where it was last put, in the window surface's logical pixels.
    QRect placed;
};

Plane plane;

void onPlaneGlobal(void *, wl_registry *registry, uint32_t name, const char *interface, uint32_t)
{
    if (!std::strcmp(interface, wl_subcompositor_interface.name))
        plane.subcompositor = static_cast<wl_subcompositor *>(wl_registry_bind(registry, name, &wl_subcompositor_interface, 1));
    else if (!std::strcmp(interface, wp_viewporter_interface.name))
        plane.viewporter = static_cast<wp_viewporter *>(wl_registry_bind(registry, name, &wp_viewporter_interface, 1));
}

void onPlaneGlobalRemove(void *, wl_registry *, uint32_t) {}

const wl_registry_listener planeRegistryListener = {onPlaneGlobal, onPlaneGlobalRemove};

// Makes the plane under `window` and offers it to the world. GUI thread,
// once the window has a surface; BLOCKLOOM_HDR_VIEW=0 turns it off.
void makePlane(QQuickWindow *window)
{
    if (plane.tried)
        return;
    if (!onWayland() || qEnvironmentVariable("BLOCKLOOM_HDR_VIEW") == QLatin1String("0")) {
        plane.tried = true;
        return;
    }
    auto *native = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>();
    auto *parent = static_cast<wl_surface *>(
        QGuiApplication::platformNativeInterface()->nativeResourceForWindow("surface", window));
    if (!native || !parent)
        return;
    plane.tried = true;
    wl_display *display = native->display();
    // Bound on a private queue so Qt's own dispatch is left alone; neither
    // global sends events.
    wl_event_queue *queue = wl_display_create_queue(display);
    auto *wrapper = static_cast<wl_display *>(wl_proxy_create_wrapper(display));
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(wrapper), queue);
    wl_registry *registry = wl_display_get_registry(wrapper);
    wl_proxy_wrapper_destroy(wrapper);
    wl_registry_add_listener(registry, &planeRegistryListener, nullptr);
    wl_display_roundtrip_queue(display, queue);
    wl_registry_destroy(registry);
    for (wl_proxy *proxy : {reinterpret_cast<wl_proxy *>(plane.subcompositor), reinterpret_cast<wl_proxy *>(plane.viewporter)}) {
        if (proxy)
            wl_proxy_set_queue(proxy, nullptr);
    }
    wl_event_queue_destroy(queue);
    if (!plane.subcompositor || !plane.viewporter || !(window->format().alphaBufferSize() > 0)) {
        qInfo("Game view: no HDR plane (needs wl_subcompositor, wp_viewporter and a window with alpha)");
        return;
    }
    wl_compositor *compositor = native->compositor();
    plane.window = window;
    plane.surface = wl_compositor_create_surface(compositor);
    plane.subsurface = wl_subcompositor_get_subsurface(plane.subcompositor, plane.surface, parent);
    wl_subsurface_place_below(plane.subsurface, parent);
    wl_subsurface_set_desync(plane.subsurface);
    // Clicks go straight through to the window above.
    wl_region *empty = wl_compositor_create_region(compositor);
    wl_surface_set_input_region(plane.surface, empty);
    wl_region_destroy(empty);
    plane.viewport = wp_viewporter_get_viewport(plane.viewporter, plane.surface);
    wl_surface_commit(plane.surface);
    wl_display_flush(display);
    game_view_offer_hdr(reinterpret_cast<size_t>(display), reinterpret_cast<size_t>(plane.surface));
}

// Puts the plane under `rect`, in the window's logical pixels. The position
// lands with the window's next commit, the size with the world's next present.
void placePlane(QQuickWindow *window, const QRect &rect)
{
    if (!plane.viewport || window != plane.window || rect == plane.placed || rect.isEmpty())
        return;
    plane.placed = rect;
    const QMargins frame = window->frameMargins();
    wl_subsurface_set_position(plane.subsurface, rect.x() + frame.left(), rect.y() + frame.top());
    wp_viewport_set_destination(plane.viewport, rect.width(), rect.height());
}

// Shrinks a plane nobody presents to out of the way, so a smaller window
// never leaves it poking out from underneath.
void parkPlane()
{
    if (!plane.viewport || plane.placed == QRect(0, 0, 1, 1))
        return;
    plane.placed = QRect(0, 0, 1, 1);
    wl_subsurface_set_position(plane.subsurface, 0, 0);
    wp_viewport_set_destination(plane.viewport, 1, 1);
    wl_surface_commit(plane.surface);
}

#endif

GLuint compile(QOpenGLFunctions *gl, GLenum type, const char *source)
{
    const GLuint shader = gl->glCreateShader(type);
    gl->glShaderSource(shader, 1, &source, nullptr);
    gl->glCompileShader(shader);
    GLint ok = 0;
    gl->glGetShaderiv(shader, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char log[512] = {};
        gl->glGetShaderInfoLog(shader, sizeof log, nullptr, log);
        qWarning("Game view: shader: %s", log);
    }
    return shader;
}

// The largest rectangle of `frame`'s shape that fits `bounds`, centred.
QRectF fit(const QRectF &bounds, const QSize &frame)
{
    if (frame.isEmpty())
        return bounds;
    const qreal scale = qMin(bounds.width() / frame.width(), bounds.height() / frame.height());
    const QSizeF size(frame.width() * scale, frame.height() * scale);
    return QRectF(bounds.center() - QPointF(size.width() / 2, size.height() / 2), size);
}

} // namespace

// One imported ring. Lives in the scene graph, so it is built and torn down
// on the render thread with the GL context current. The picture is a child
// that only exists while it has a live texture to show.
class GameViewNode : public QSGNode
{
public:
    explicit GameViewNode(GameViewNode **owner)
        : owner(owner)
    {
    }

    ~GameViewNode() override
    {
        release();
        QOpenGLContext *context = QOpenGLContext::currentContext();
        if (context && program)
            context->functions()->glDeleteProgram(program);
        if (context && vertices)
            context->functions()->glDeleteBuffers(1, &vertices);
        // The scene graph can drop nodes on its own; the item mustn't keep
        // drawing through a dead one.
        if (*owner == this)
            *owner = nullptr;
    }

    struct Slot {
#ifdef __linux__
        EGLImageKHR image = EGL_NO_IMAGE_KHR;
#endif
        GLuint texture = 0;
        QSGTexture *sgTexture = nullptr;
    };

    QSize size;
    std::vector<Slot> ring;
    // External-only rings are drawn through `copy` rather than shown direct.
    bool external = false;
    int pending = -1;

    // What to show for slot `index`.
    QSGTexture *textureFor(int index)
    {
        if (!external)
            return ring[index].sgTexture;
        pending = index;
        return copy.sgTexture;
    }

    void show(QSGTexture *texture, const QRectF &rect)
    {
        if (!image) {
            image = new QSGSimpleTextureNode;
            image->setOwnsTexture(false);
            image->setFiltering(QSGTexture::Linear);
            image->setTexture(texture);
            appendChildNode(image);
        } else {
            image->setTexture(texture);
        }
        image->setRect(rect);
        image->markDirty(QSGNode::DirtyMaterial);
    }

    // Shows through to the HDR plane over `rect`: a texture Qt thinks is
    // opaque, so it is drawn without blending and its zero alpha lands in
    // the window as is, while items over the view still draw on top.
    void showHole(const QRectF &rect, QQuickWindow *window)
    {
        if (!hole.sgTexture) {
            QOpenGLFunctions *gl = QOpenGLContext::currentContext()->functions();
            static const unsigned char clear[4] = {0, 0, 0, 0};
            gl->glGenTextures(1, &hole.texture);
            gl->glBindTexture(GL_TEXTURE_2D, hole.texture);
            gl->glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, clear);
            setSampling(gl, GL_TEXTURE_2D);
            gl->glBindTexture(GL_TEXTURE_2D, 0);
            hole.sgTexture = QNativeInterface::QSGOpenGLTexture::fromNative(hole.texture, window, QSize(1, 1));
        }
        show(hole.sgTexture, rect);
    }

    void hide()
    {
        if (!image)
            return;
        removeChildNode(image);
        delete image;
        image = nullptr;
    }

    void release()
    {
        // Before the textures go, so nothing is left pointing at them.
        hide();
        QOpenGLContext *context = QOpenGLContext::currentContext();
        QOpenGLFunctions *gl = context ? context->functions() : nullptr;
        for (Slot &slot : ring)
            drop(gl, slot);
        ring.clear();
        drop(gl, copy);
        drop(gl, hole);
        if (gl && framebuffer)
            gl->glDeleteFramebuffers(1, &framebuffer);
        framebuffer = 0;
        external = false;
        pending = -1;
        size = {};
    }

    // Imports every image of `frames`, closing their fds either way.
    QString import(const GameFrames &frames, QQuickWindow *window)
    {
        release();
        QString error;
        const QSize frameSize(int(frames.width), int(frames.height));
#ifdef __linux__
        if (!egl.createImage)
            error = loadEgl();
        QOpenGLFunctions *gl = QOpenGLContext::currentContext()->functions();
        if (error.isEmpty())
            external = externalOnly(frames.fourcc, frames.modifier);
        const GLenum target = external ? GL_TEXTURE_EXTERNAL_OES : GL_TEXTURE_2D;
        for (size_t i = 0; i < frames.fds.size(); ++i) {
            const int fd = frames.fds[i];
            if (error.isEmpty()) {
                std::vector<EGLint> attributes = {
                    EGL_WIDTH, EGLint(frames.width),
                    EGL_HEIGHT, EGLint(frames.height),
                    EGL_LINUX_DRM_FOURCC_EXT, EGLint(frames.fourcc),
                    EGL_DMA_BUF_PLANE0_FD_EXT, fd,
                    EGL_DMA_BUF_PLANE0_OFFSET_EXT, EGLint(frames.offsets[i]),
                    EGL_DMA_BUF_PLANE0_PITCH_EXT, EGLint(frames.strides[i]),
                };
                if (egl.modifiers) {
                    attributes.insert(attributes.end(), {
                        EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT, EGLint(frames.modifier & 0xffffffff),
                        EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT, EGLint(frames.modifier >> 32),
                    });
                }
                attributes.push_back(EGL_NONE);
                Slot slot;
                slot.image = egl.createImage(egl.display, EGL_NO_CONTEXT, EGL_LINUX_DMA_BUF_EXT, nullptr, attributes.data());
                if (slot.image == EGL_NO_IMAGE_KHR) {
                    error = QStringLiteral("importing a frame failed (EGL error 0x%1)").arg(eglGetError(), 0, 16);
                } else {
                    gl->glGenTextures(1, &slot.texture);
                    gl->glBindTexture(target, slot.texture);
                    egl.imageTargetTexture(target, slot.image);
                    setSampling(gl, target);
                    gl->glBindTexture(target, 0);
                    if (const GLenum glError = gl->glGetError())
                        error = QStringLiteral("binding a frame failed (GL error 0x%1)").arg(glError, 0, 16);
                    else if (!external)
                        slot.sgTexture = QNativeInterface::QSGOpenGLTexture::fromNative(slot.texture, window, frameSize);
                }
                ring.push_back(slot);
            }
            // EGL keeps its own reference to the buffer.
            ::close(fd);
        }
        if (error.isEmpty() && external)
            error = prepareCopy(gl, window, frameSize);
        if (error.isEmpty())
            qInfo("Game view: showing %dx%d frames %s (modifier 0x%llx)", frameSize.width(), frameSize.height(),
                  external ? "through an external-texture copy" : "directly", (unsigned long long)frames.modifier);
#else
        Q_UNUSED(window);
        error = QStringLiteral("the Game view isn't supported on this platform yet");
#endif
        if (!error.isEmpty())
            release();
        else
            size = frameSize;
        return error;
    }

    // Draws the pending external slot into the plain texture Qt samples.
    void drawPending()
    {
        if (!external || pending < 0 || pending >= int(ring.size()))
            return;
        QOpenGLContext *context = QOpenGLContext::currentContext();
        QOpenGLFunctions *gl = context->functions();
        if (context->format().majorVersion() >= 3)
            context->extraFunctions()->glBindVertexArray(0);
        gl->glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
        gl->glViewport(0, 0, size.width(), size.height());
        gl->glDisable(GL_BLEND);
        gl->glDisable(GL_DEPTH_TEST);
        gl->glDisable(GL_STENCIL_TEST);
        gl->glDisable(GL_SCISSOR_TEST);
        gl->glDisable(GL_CULL_FACE);
        gl->glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
        gl->glUseProgram(program);
        gl->glActiveTexture(GL_TEXTURE0);
        gl->glBindTexture(GL_TEXTURE_EXTERNAL_OES, ring[pending].texture);
        gl->glBindBuffer(GL_ARRAY_BUFFER, vertices);
        gl->glEnableVertexAttribArray(0);
        gl->glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, nullptr);
        gl->glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        gl->glDisableVertexAttribArray(0);
        gl->glBindBuffer(GL_ARRAY_BUFFER, 0);
        gl->glBindTexture(GL_TEXTURE_EXTERNAL_OES, 0);
        gl->glBindFramebuffer(GL_FRAMEBUFFER, 0);
        pending = -1;
    }

private:
    static void setSampling(QOpenGLFunctions *gl, GLenum target)
    {
        gl->glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        gl->glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
        gl->glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        gl->glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    }

    void drop(QOpenGLFunctions *gl, Slot &slot)
    {
        delete slot.sgTexture;
        slot.sgTexture = nullptr;
        if (gl && slot.texture)
            gl->glDeleteTextures(1, &slot.texture);
        slot.texture = 0;
#ifdef __linux__
        if (slot.image != EGL_NO_IMAGE_KHR && egl.destroyImage)
            egl.destroyImage(egl.display, slot.image);
        slot.image = EGL_NO_IMAGE_KHR;
#endif
    }

    // The plain texture an external ring is copied into, and what copies it.
    QString prepareCopy(QOpenGLFunctions *gl, QQuickWindow *window, const QSize &frameSize)
    {
        if (!program) {
            static const char *vertexSource =
                "#version 100\n"
                "attribute vec2 position;\n"
                "varying vec2 uv;\n"
                "void main() { uv = position * 0.5 + 0.5; gl_Position = vec4(position, 0.0, 1.0); }\n";
            static const char *fragmentSource =
                "#version 100\n"
                "#extension GL_OES_EGL_image_external : require\n"
                "precision mediump float;\n"
                "uniform samplerExternalOES frame;\n"
                "varying vec2 uv;\n"
                "void main() { gl_FragColor = vec4(texture2D(frame, uv).rgb, 1.0); }\n";
            const GLuint vertex = compile(gl, GL_VERTEX_SHADER, vertexSource);
            const GLuint fragment = compile(gl, GL_FRAGMENT_SHADER, fragmentSource);
            program = gl->glCreateProgram();
            gl->glAttachShader(program, vertex);
            gl->glAttachShader(program, fragment);
            gl->glBindAttribLocation(program, 0, "position");
            gl->glLinkProgram(program);
            gl->glDeleteShader(vertex);
            gl->glDeleteShader(fragment);
            GLint linked = 0;
            gl->glGetProgramiv(program, GL_LINK_STATUS, &linked);
            if (!linked)
                return QStringLiteral("the frame copy shader didn't build");
            gl->glUseProgram(program);
            gl->glUniform1i(gl->glGetUniformLocation(program, "frame"), 0);
            gl->glUseProgram(0);

            static const GLfloat quad[] = {-1, -1, 1, -1, -1, 1, 1, 1};
            gl->glGenBuffers(1, &vertices);
            gl->glBindBuffer(GL_ARRAY_BUFFER, vertices);
            gl->glBufferData(GL_ARRAY_BUFFER, sizeof quad, quad, GL_STATIC_DRAW);
            gl->glBindBuffer(GL_ARRAY_BUFFER, 0);
        }

        gl->glGenTextures(1, &copy.texture);
        gl->glBindTexture(GL_TEXTURE_2D, copy.texture);
        gl->glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, frameSize.width(), frameSize.height(), 0, GL_RGBA, GL_UNSIGNED_BYTE, nullptr);
        setSampling(gl, GL_TEXTURE_2D);
        gl->glBindTexture(GL_TEXTURE_2D, 0);
        gl->glGenFramebuffers(1, &framebuffer);
        gl->glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
        gl->glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, copy.texture, 0);
        const GLenum status = gl->glCheckFramebufferStatus(GL_FRAMEBUFFER);
        gl->glBindFramebuffer(GL_FRAMEBUFFER, 0);
        if (status != GL_FRAMEBUFFER_COMPLETE)
            return QStringLiteral("the frame copy target is incomplete (0x%1)").arg(status, 0, 16);
        copy.sgTexture = QNativeInterface::QSGOpenGLTexture::fromNative(copy.texture, window, frameSize);
        return {};
    }

    GameViewNode **owner;
    QSGSimpleTextureNode *image = nullptr;
    Slot copy;
    Slot hole;
    GLuint framebuffer = 0;
    GLuint program = 0;
    GLuint vertices = 0;
};

GameView::GameView(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    views().append(this);
    m_lock = new PointerLock(this);
    m_lock->setParent(this);
    m_lock->heldChanged = [this] { Q_EMIT pointerHeldChanged(); };
    m_lock->moved = [this](qreal dx, qreal dy) { Q_EMIT pointerMoved(dx, dy); };
    m_lock->released = [this] {
        m_pointerLocked = false;
        Q_EMIT pointerLockedChanged();
        Q_EMIT pointerReleased();
    };
    m_resizeTimer.setSingleShot(true);
    m_resizeTimer.setInterval(150);
    connect(&m_resizeTimer, &QTimer::timeout, this, &GameView::sendSize);
}

GameView::~GameView()
{
    views().removeAll(this);
    delete m_lock;
    m_lock = nullptr;
}

void GameView::setPointerLocked(bool locked)
{
    if (locked == m_pointerLocked)
        return;
    m_pointerLocked = locked;
    m_lock->setLocked(locked);
    Q_EMIT pointerLockedChanged();
}

bool GameView::pointerHeld() const
{
    return m_lock && m_lock->held();
}

void GameView::setResolution(const QSize &resolution)
{
    if (resolution == m_resolution)
        return;
    m_resolution = resolution;
    Q_EMIT resolutionChanged();
    resize();
}

void GameView::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickItem::geometryChange(newGeometry, oldGeometry);
    if (newGeometry.size() != oldGeometry.size())
        resize();
}

void GameView::resize()
{
    if (m_sentSize.isEmpty())
        sendSize();
    else
        m_resizeTimer.start();
}

void GameView::sendSize()
{
    // A hidden view's size means nothing; it is sent again once shown.
    if (!window() || !isVisible())
        return;
    const qreal dpr = window()->effectiveDevicePixelRatio();
    // A fixed resolution is a player's screen at 100%; a free one looks
    // the way the editor does.
    const bool fixed = !m_resolution.isEmpty();
    const QSize size = fixed ? m_resolution : QSize(qRound(width() * dpr), qRound(height() * dpr));
    const qreal scale = fixed ? 1.0 : dpr;
    if (size.width() < 16 || size.height() < 16 || (size == m_sentSize && scale == m_sentScale))
        return;
    m_resizeTimer.stop();
    m_sentSize = size;
    m_sentScale = scale;
    game_view_resize(uint32_t(size.width()), uint32_t(size.height()), float(scale));
}

void GameView::wake()
{
    const bool has = game_view_latest().valid || game_view_hdr_live();
    if (has != m_hasFrame) {
        m_hasFrame = has;
        Q_EMIT hasFrameChanged();
    }
    update();
}

void GameView::fail(const QString &message)
{
    // From the render thread: hand it to the GUI thread.
    QMetaObject::invokeMethod(this, [this, message] {
        if (m_error == message)
            return;
        m_error = message;
        qWarning("Game view: %s", qPrintable(message));
        Q_EMIT errorChanged();
    }, Qt::QueuedConnection);
}

void GameView::renderExternal()
{
#ifdef __linux__
    offerModifiers();
#endif
    if (!m_node || m_node->pending < 0)
        return;
    window()->beginExternalCommands();
    m_node->drawPending();
    window()->endExternalCommands();
}

void GameView::itemChange(ItemChange change, const ItemChangeData &value)
{
    // Hooked as soon as there is a window, before anything is drawn, so the
    // modifier offer reaches the world early.
    if (change == ItemSceneChange && value.window)
        hook(value.window);
    QQuickItem::itemChange(change, value);
    if ((change == ItemSceneChange && value.window) || change == ItemDevicePixelRatioHasChanged
        || (change == ItemVisibleHasChanged && value.boolValue))
        resize();
}

void GameView::hook(QQuickWindow *window)
{
    if (m_hooked == window)
        return;
    if (m_hooked)
        disconnect(m_hooked, nullptr, this, nullptr);
    m_hooked = window;
    connect(window, &QQuickWindow::beforeRendering, this, &GameView::renderExternal, Qt::DirectConnection);
    connect(window, &QQuickWindow::frameSwapped, this, &GameView::framePresented, Qt::DirectConnection);
}

void GameView::framePresented()
{
    game_view_presented();
#ifdef __linux__
    // Once the window has put a frame up it has a surface to hang the plane off.
    if (!plane.tried)
        QMetaObject::invokeMethod(this, [this] { if (window()) makePlane(window()); }, Qt::QueuedConnection);
#endif
    // A running world draws a frame per present, so keep presenting.
    if (m_generation)
        QMetaObject::invokeMethod(this, [this] { update(); }, Qt::QueuedConnection);
}

QSGNode *GameView::updatePaintNode(QSGNode *old, UpdatePaintNodeData *)
{
    auto *node = static_cast<GameViewNode *>(old);
    if (!window() || window()->rendererInterface()->graphicsApi() != QSGRendererInterface::OpenGL) {
        fail(QStringLiteral("the Game view needs Qt's OpenGL renderer"));
        delete node;
        return nullptr;
    }
    if (!node) {
        node = new GameViewNode(&m_node);
        m_node = node;
    }
    // Tried once per ring: a failed import waits for the next one.
    GameFrames frames;
    if (game_view_slots(m_generation, frames)) {
        m_generation = frames.generation;
        node->release();
        if (frames.generation) {
            const QString error = node->import(frames, window());
            // A tiled ring that won't import isn't the end: the world falls
            // back to linear frames once it hears.
            if (!error.isEmpty() && frames.modifier != modifierLinear) {
                qWarning("Game view: %s; asking for linear frames instead", qPrintable(error));
                game_view_refuse(frames.generation);
            } else if (!error.isEmpty()) {
                fail(error);
            }
        }
    }

#ifdef __linux__
    // HDR frames are on the plane under the window: show through to it,
    // letterboxed as the ring's frames would be.
    if (game_view_hdr_live() && !m_sentSize.isEmpty()) {
        const QRectF rect = fit(boundingRect(), m_sentSize);
        placePlane(window(), mapRectToScene(rect).toRect());
        node->showHole(rect, window());
        return node;
    }
    if (window() == plane.window)
        parkPlane();
#endif
    const GameFrame latest = game_view_latest();
    if (!latest.valid || latest.generation != m_generation || latest.index >= node->ring.size()) {
        // Nothing new: a frame from this ring stays up rather than flashing,
        // and with no ring there is nothing to show.
        if (node->ring.empty())
            node->hide();
        return node;
    }
    node->show(node->textureFor(int(latest.index)), fit(boundingRect(), node->size));
    game_view_hold(latest.generation, latest.index);
    return node;
}

void game_view_wake()
{
    // Frames land at the world's rate; one queued pass covers a burst.
    if (wakePending.exchange(true))
        return;
    QCoreApplication *app = QCoreApplication::instance();
    if (!app) {
        wakePending = false;
        return;
    }
    QMetaObject::invokeMethod(app, [] {
        wakePending = false;
        for (const QPointer<GameView> &view : views()) {
            if (view)
                view->wake();
        }
    }, Qt::QueuedConnection);
}

void game_view_prefer_opengl()
{
    QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
#ifdef __linux__
    // On Wayland the Game view may show through to an HDR plane under the
    // window, which needs the window to carry alpha. It stays opaque elsewhere.
    const QByteArray platform = qgetenv("QT_QPA_PLATFORM");
    if (qEnvironmentVariableIsSet("WAYLAND_DISPLAY") && !platform.startsWith("xcb")
        && qEnvironmentVariable("BLOCKLOOM_HDR_VIEW") != QLatin1String("0"))
        QQuickWindow::setDefaultAlphaBuffer(true);
#endif
}
