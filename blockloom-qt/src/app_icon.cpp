#include "app_icon.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QDebug>
#include <QtCore/QString>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QWindow>

void blockloom_apply_window_icon()
{
    // Displayed in about dialogs and used as a fallback identifier.
    QCoreApplication::setApplicationName(QStringLiteral("Blockloom"));
    QCoreApplication::setOrganizationName(QStringLiteral("Blockworked"));
    QGuiApplication::setApplicationDisplayName(QStringLiteral("Blockloom"));
    // Matches res/blockloom.desktop (shipped as com.blockworked.Blockloom on
    // Flatpak) and its StartupWMClass, so Wayland compositors and docks can
    // pair windows with the installed desktop entry and its Icon=blockloom.
    QGuiApplication::setDesktopFileName(QStringLiteral("com.blockworked.Blockloom"));

    // Embedded through CxxQtBuilder::qrc_resources in build.rs. Note: in C++
    // resources are addressed as ":/...", the "qrc:/..." form only works in QML.
    const QIcon icon(QStringLiteral(":/icons/blockloom.png"));
    if (icon.isNull()) {
        qWarning("Blockloom: failed to load the window icon from :/icons/blockloom.png");
        return;
    }
    QGuiApplication::setWindowIcon(icon);
}

void blockloom_apply_window_icon_to_windows()
{
    const QIcon icon = QGuiApplication::windowIcon();
    if (icon.isNull()) {
        return;
    }
    for (QWindow *window : QGuiApplication::allWindows()) {
        if (window->icon().isNull()) {
            window->setIcon(icon);
        }
    }
}
