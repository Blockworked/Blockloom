#pragma once

// Applies the Blockloom window icon and desktop metadata to the running
// QGuiApplication. Must be called after the QGuiApplication instance exists.
void blockloom_apply_window_icon();

// Re-applies the application icon to every existing top-level window that
// has none of its own. Call after the QML engine has loaded the main window.
void blockloom_apply_window_icon_to_windows();
