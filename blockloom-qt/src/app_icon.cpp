#include "app_icon.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QDebug>
#include <QtCore/QString>
#include <QtGui/QFont>
#include <QtGui/QFontDatabase>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QWindow>

// Blockworked's typeface (Atkinson Hyperlegible Next, OFL, res/fonts), so the
// editor reads the same on every desktop. Falls back to the platform font.
static void blockloom_use_bundled_font()
{
    static const char *const files[] = {"Regular", "Medium", "SemiBold", "Bold", "ExtraBold", "Italic"};
    QString family;
    for (const char *name : files) {
        const int id = QFontDatabase::addApplicationFont(QStringLiteral(":/fonts/AtkinsonHyperlegibleNext-%1.ttf").arg(QLatin1String(name)));
        if (id >= 0 && family.isEmpty())
            family = QFontDatabase::applicationFontFamilies(id).value(0);
    }
    if (family.isEmpty()) {
        qWarning("Blockloom: bundled font missing from :/fonts, using the platform font");
        return;
    }
    QFont font = QGuiApplication::font();
    font.setFamilies({family});
    QGuiApplication::setFont(font);
}

void blockloom_apply_window_icon()
{
    blockloom_use_bundled_font();
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
