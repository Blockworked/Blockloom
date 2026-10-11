#include "hub_service.h"
#include <QMouseEvent>
#include <QQuickItem>
#include <QWheelEvent>
#include <QtCore/QCoreApplication>
#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QFileInfo>
#include <QtCore/QJsonDocument>
#include <QtCore/QProcessEnvironment>
#include <QtCore/QStandardPaths>
#include <QtCore/QTimer>
#include <QtCore/QUrl>
#include <QtGui/QGuiApplication>
#include <QtGui/QDesktopServices>
#include <QtGui/QFontDatabase>
#include <QtQuick/QQuickWindow>
#include <cstdio>
#include <functional>

HubService::HubService(QObject *parent) : QObject(parent)
{
    if (smokeTest()) {
        const QString font = qEnvironmentVariable("BLOCKLOOM_HUB_SMOKE_FONT");
        if (!font.isEmpty()) {
            const int id = QFontDatabase::addApplicationFont(font);
            const auto families = QFontDatabase::applicationFontFamilies(id);
            if (!families.isEmpty()) QGuiApplication::setFont(QFont(families.first()));
        }
        const QString mono = qEnvironmentVariable("BLOCKLOOM_HUB_SMOKE_MONO");
        if (!mono.isEmpty()) QFontDatabase::addApplicationFont(mono);
        QTimer::singleShot(20000, this, [] { QCoreApplication::exit(2); });
    }
    // Extract the embedded service so imports work without a source checkout.
    for (const auto &name : {"hub.py", "hub_install.py", "hub_process.py", "hub_download.py", "hub_github.py", "hub_tools.py", "replace.py"}) {
        if (!m_scripts.isValid() || !QFile::copy(QString(":/hub-service/") + name,
                m_scripts.filePath(name))) {
            m_setupError = "Could not prepare the Hub installation service.";
            break;
        }
    }
    connect(&m_poll, qOverload<int, QProcess::ExitStatus>(&QProcess::finished),
        this, [this](int code, QProcess::ExitStatus status) {
            const QString output = QString::fromUtf8(m_poll.readAllStandardOutput());
            const bool ok = code == 0 && status == QProcess::NormalExit;
            emit polled(m_pollCommand, ok, ok ? output : QString::fromUtf8(m_poll.readAllStandardError()));
        });
    connect(&m_process, &QProcess::readyReadStandardOutput, this, [this] {
        m_output += m_process.readAllStandardOutput();
    });
    connect(&m_process, &QProcess::readyReadStandardError, this, [this] {
        const QString text = QString::fromUtf8(m_process.readAllStandardError());
        m_errorOutput += text;
        if (m_errorOutput.size() > 256 * 1024) m_errorOutput = m_errorOutput.right(256 * 1024);
        appendLog(text);
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error == QProcess::FailedToStart)
            finish(false, "Could not start Python 3.11+. Set BLOCKLOOM_HUB_PYTHON to its executable.\n" + m_process.errorString());
    });
    connect(&m_process, qOverload<int, QProcess::ExitStatus>(&QProcess::finished),
        this, [this](int code, QProcess::ExitStatus status) {
            m_output += m_process.readAllStandardOutput();
            const QString text = QString::fromUtf8(m_process.readAllStandardError());
            m_errorOutput += text;
            appendLog(text);
            if (m_cancelRequested && code == 130)
                finish(false, "Operation cancelled.");
            else finish(code == 0 && status == QProcess::NormalExit);
        });
}

namespace {
// What the Rust state tool answers; every other command is the installer.
bool isStateCommand(const QString &command)
{
    static const QStringList commands {
        "projects", "installations", "remember", "forget", "relocate", "bind", "backups",
        "backup-project", "restore-backup", "focus", "open", "new-project"
    };
    return commands.contains(command);
}
}

QString HubService::localPath(const QUrl &url) const
{
    return url.toLocalFile();
}

bool HubService::canCancel() const
{
    return m_busy && (m_command == "prepare-dev" || m_command == "rebuild" || m_command == "install-dev"
        || m_command == "check-releases" || m_command == "download-release" || m_command == "install"
        || m_command == "bind" || m_command == "backup-project" || m_command == "restore-backup"
        || m_command == "add-tools"
        || m_command == "uninstall" || m_command == "prune-tools");
}

bool HubService::showBackup(const QString &path) const
{
    const QFileInfo file(path);
    return file.isFile() && QDesktopServices::openUrl(QUrl::fromLocalFile(file.absolutePath()));
}

bool HubService::showFolder(const QString &path) const
{
    const QFileInfo dir(path);
    return dir.isDir() && QDesktopServices::openUrl(QUrl::fromLocalFile(dir.absoluteFilePath()));
}

void HubService::cancel()
{
    if (!canCancel() || m_cancelRequested) return;
    QFile request(m_scripts.filePath("cancel"));
    if (!request.open(QIODevice::WriteOnly)) {
        appendLog("Could not request cancellation.\n");
        return;
    }
    request.close();
    m_cancelRequested = true;
    emit cancellingChanged();
    appendLog("Cancellation requested. Stopping build processes...\n");
}

bool HubService::smokeTest() const
{
    return QCoreApplication::arguments().contains("--smoke-test");
}

QString HubService::smokePage() const
{
    const auto args = QCoreApplication::arguments();
    const int page = args.indexOf("--smoke-page");
    return smokeTest() && page >= 0 && page + 1 < args.size() ? args[page + 1] : QString();
}

void HubService::smokeReady(int projects, int installations)
{
    if (!smokeTest()) return;
    if (smokePage().startsWith("checkbox-")) {
        for (auto *window : QGuiApplication::allWindows()) {
            if (auto *quick = qobject_cast<QQuickWindow *>(window)) {
                auto *item = quick->contentItem()->findChild<QQuickItem *>("smokeCheckbox");
                if (!item) { QCoreApplication::exit(4); return; }
                const auto position = item->mapToScene(QPointF(12, item->height() / 2));
                QMouseEvent move(QEvent::MouseMove, position, quick->mapToGlobal(position),
                                 Qt::NoButton, Qt::NoButton, Qt::NoModifier);
                QCoreApplication::sendEvent(quick, &move);
                if (!item->property("hovered").toBool()) { QCoreApplication::exit(5); return; }
            }
        }
    }
    if (smokePage() == "log") appendLog("[Editor] Building Blockloom...\n[Web] Compiling player...\n[Native] Waiting for editor build.\n");
    if (smokePage() == "log-scroll") {
        for (int line = 0; line < 200; ++line) appendLog(QString("Build output line %1\n").arg(line));
        QTimer::singleShot(150, this, [this, projects, installations] {
            for (auto *window : QGuiApplication::allWindows()) {
                auto *quick = qobject_cast<QQuickWindow *>(window);
                if (!quick) continue;
                auto *root = quick->contentItem();
                auto *viewport = root->findChild<QQuickItem *>("smokeLogViewport");
                auto *scrollbar = root->findChild<QQuickItem *>("smokeLogScrollBar");
                auto *follow = root->findChild<QQuickItem *>("smokeLogFollow");
                auto *cancel = root->findChild<QQuickItem *>("smokeLogCancel");
                auto *close = root->findChild<QQuickItem *>("smokeLogClose");
                if (!viewport || !scrollbar || !follow || !cancel || !close) { QCoreApplication::exit(6); return; }
                const auto viewportRight = viewport->mapToScene(QPointF(viewport->width(), 0));
                const auto scrollbarTop = scrollbar->mapToScene(QPointF(0, 0));
                if (!scrollbar->isVisible() || scrollbar->width() < 12
                    || qAbs(scrollbar->height() - viewport->height()) > 4
                    || qAbs(scrollbarTop.x() - viewportRight.x()) > 4
                    || qAbs(scrollbarTop.y() - viewportRight.y()) > 4) {
                    QCoreApplication::exit(12); return;
                }
                cancel->setVisible(true);
                const double bottom = viewport->property("contentHeight").toDouble() - viewport->height();
                if (qAbs(viewport->property("contentY").toDouble() - bottom) > 1) { QCoreApplication::exit(7); return; }
                const auto position = viewport->mapToScene(QPointF(100, 100));
                QWheelEvent wheel(position, quick->mapToGlobal(position), QPoint(), QPoint(0, 120),
                                  Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
                QCoreApplication::sendEvent(quick, &wheel);
                QTimer::singleShot(350, this, [this, viewport, follow, cancel, close, bottom, projects, installations] {
                    const double reading = viewport->property("contentY").toDouble();
                    if (follow->property("checked").toBool() || reading >= bottom - 1) { QCoreApplication::exit(8); return; }
                    const auto cancelCenter = cancel->mapToScene(QPointF(0, cancel->height() / 2));
                    const auto closeCenter = close->mapToScene(QPointF(0, close->height() / 2));
                    if (qAbs(cancelCenter.y() - closeCenter.y()) > 1) { QCoreApplication::exit(9); return; }
                    appendLog("More output while reading earlier lines.\n");
                    QTimer::singleShot(100, this, [this, viewport, follow, reading, projects, installations] {
                        if (qAbs(viewport->property("contentY").toDouble() - reading) > 1) { QCoreApplication::exit(10); return; }
                        follow->setProperty("checked", true);
                        QTimer::singleShot(100, this, [this, viewport, projects, installations] {
                            const double bottom = viewport->property("contentHeight").toDouble() - viewport->height();
                            if (qAbs(viewport->property("contentY").toDouble() - bottom) > 1) { QCoreApplication::exit(11); return; }
                            finishSmoke(projects, installations);
                        });
                    });
                });
                return;
            }
            QCoreApplication::exit(6);
        });
        return;
    }
    finishSmoke(projects, installations);
}

void HubService::finishSmoke(int projects, int installations)
{
    QTimer::singleShot(400, this, [projects, installations] {
        // --smoke-expect a,b: each named item exists and is visible.
        // --smoke-absent a,b: none is visible (or none exists).
        const auto checks = QCoreApplication::arguments();
        for (const QString &flag : {QString("--smoke-expect"), QString("--smoke-absent")}) {
            const int at = checks.indexOf(flag);
            if (at < 0 || at + 1 >= checks.size()) continue;
            for (const QString &name : checks[at + 1].split(',', Qt::SkipEmptyParts)) {
                bool found = false;
                for (auto *window : QGuiApplication::allWindows()) {
                    auto *quick = qobject_cast<QQuickWindow *>(window);
                    if (!quick) continue;
                    // Delegates are parented visually only, so walk the item tree.
                    std::function<void(QQuickItem *)> walk = [&](QQuickItem *item) {
                        if (item->objectName() == name && item->isVisible() && item->width() > 0)
                            found = true;
                        for (auto *child : item->childItems()) walk(child);
                    };
                    walk(quick->contentItem());
                }
                if (found != (flag == "--smoke-expect")) {
                    std::fprintf(stderr, "Smoke check failed: %s %s\n", qPrintable(flag), qPrintable(name));
                    QCoreApplication::exit(13);
                    return;
                }
            }
        }
        const auto args = QCoreApplication::arguments();
        const int output = args.indexOf("--smoke-output");
        if (output >= 0 && output + 1 < args.size()) {
            bool saved = false;
            for (auto *window : QGuiApplication::allWindows()) {
                if (auto *quick = qobject_cast<QQuickWindow *>(window)) {
                    saved = quick->grabWindow().save(args[output + 1]);
                    if (saved) break;
                }
            }
            if (!saved) { QCoreApplication::exit(3); return; }
        }
        std::fprintf(stdout, "{\"projects\":%d,\"installations\":%d}\n", projects, installations);
        std::fflush(stdout);
        QCoreApplication::exit(0);
    });
}

void HubService::appendLog(const QString &text)
{
    if (text.isEmpty()) return;
    m_log += text;
    // Keep the visible log bounded during long builds.
    if (m_log.size() > 256 * 1024) m_log = m_log.right(256 * 1024);
    emit logChanged();
}

void HubService::finish(bool ok, const QString &error)
{
    if (!m_busy) return;
    if (!error.isEmpty()) { appendLog(error); m_errorOutput += error; }
    const QString command = m_command;
    const QString response = ok ? QString::fromUtf8(m_output)
        : (error.startsWith("Operation cancelled") ? error
           : m_errorOutput.isEmpty() ? "The Hub operation failed." : m_errorOutput);
    m_busy = false;
    emit busyChanged();
    emit completed(command, ok, response);
}

void HubService::run(const QString &command, const QStringList &arguments)
{
    if (m_busy) return;
    m_command = command;
    QFile::remove(m_scripts.filePath("cancel"));
    m_cancelRequested = false;
    emit cancellingChanged();
    m_output.clear();
    m_errorOutput.clear();
    if (command != "projects" && command != "installations") m_log.clear();
    m_busy = true;
    emit logChanged();
    emit busyChanged();
    if (!m_setupError.isEmpty()) {
        finish(false, m_setupError);
        return;
    }
    m_process.setProcessEnvironment(serviceEnvironment());
    if (isStateCommand(command)) {
        QStringList args {"--hubctl", command};
        args.append(arguments);
        m_process.start(QCoreApplication::applicationFilePath(), args);
        return;
    }
    QStringList args {"-B", m_scripts.filePath("hub.py"), command};
    args.append(arguments);
    m_process.start(pythonPath(), args);
}

QString HubService::pythonPath() const
{
    QString python = qEnvironmentVariable("BLOCKLOOM_HUB_PYTHON");
    if (!python.isEmpty()) return python;
    const QString bundled = QDir(QCoreApplication::applicationDirPath()).filePath(
#ifdef Q_OS_WIN
        "python/python.exe"
#else
        "python/bin/python3"
#endif
    );
    if (QFile::exists(bundled)) return bundled;
#ifdef Q_OS_WIN
    return "python";
#else
    return "python3";
#endif
}

QProcessEnvironment HubService::serviceEnvironment() const
{
    auto environment = QProcessEnvironment::systemEnvironment();
    environment.insert("PYTHONUTF8", "1");
    environment.insert("PYTHONUNBUFFERED", "1");
    environment.insert("BLOCKLOOM_HUB_CANCEL_FILE", m_scripts.filePath("cancel"));
    // The installer service asks the same tool the window uses.
    environment.insert("BLOCKLOOM_HUBCTL", QCoreApplication::applicationFilePath());
    environment.insert("BLOCKLOOM_HUBCTL_ARGS", "--hubctl");
    return environment;
}

void HubService::poll(const QString &command)
{
    if (m_poll.state() != QProcess::NotRunning || !m_setupError.isEmpty()) return;
    m_pollCommand = command;
    m_poll.setProcessEnvironment(serviceEnvironment());
    if (isStateCommand(command)) {
        m_poll.start(QCoreApplication::applicationFilePath(), {"--hubctl", command});
        return;
    }
    m_poll.start(pythonPath(), {"-B", m_scripts.filePath("hub.py"), command});
}
