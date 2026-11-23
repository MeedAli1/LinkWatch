/// engine/overlay.rs - True transparent click-through overlay window manager.
///
/// In tauri.conf.json, the overlay window is statically declared with:
///   - transparent: true
///   - decorations: false
///   - shadow: false
///   - visible: false
///
/// When opening, we get the window, reposition it to the configured corner,
/// set click-through (set_ignore_cursor_events), and show it.
/// If it was closed/destroyed, we re-create it using WebviewWindowBuilder.

use tauri::Manager;

fn calc_bounds(app: &tauri::AppHandle, cfg: &crate::engine::overlay_config::OverlayConfig) -> (f64, f64, f64, f64) {
    // Single line stats HUD height based on font size + padding
    let win_h = (cfg.font_size as f64 * 1.8).max(36.0);
    let win_w = 650.0;
    let margin = cfg.margin as f64;

    let (pos_x, pos_y) = if let Ok(Some(monitor)) = app.primary_monitor() {
        let size = monitor.size();
        let scale = monitor.scale_factor();
        let screen_w = size.width as f64 / scale;
        let screen_h = size.height as f64 / scale;

        match cfg.corner {
            crate::engine::overlay_config::OverlayCorner::TopLeft => (margin, margin),
            crate::engine::overlay_config::OverlayCorner::TopRight => (screen_w - win_w - margin, margin),
            crate::engine::overlay_config::OverlayCorner::BottomLeft => (margin, screen_h - win_h - margin),
            crate::engine::overlay_config::OverlayCorner::BottomRight => (screen_w - win_w - margin, screen_h - win_h - margin),
        }
    } else {
        (margin, margin)
    };

    (pos_x, pos_y, win_w, win_h)
}

/// Open (or focus/show) the transparent overlay window.
pub fn open_overlay(app: &tauri::AppHandle) -> Result<(), String> {
    let cfg = crate::engine::overlay_config::load_overlay_config();
    let (pos_x, pos_y, win_w, win_h) = calc_bounds(app, &cfg);

    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.set_size(tauri::Size::Logical(tauri::LogicalSize { width: win_w, height: win_h }));
        let _ = w.set_position(tauri::Position::Logical(tauri::LogicalPosition { x: pos_x, y: pos_y }));
        let _ = w.show();
        let _ = w.set_ignore_cursor_events(true);
        return Ok(());
    }

    // Window does not exist yet: load clean index.html with query parameter
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "overlay",
        tauri::WebviewUrl::App("index.html?overlay=1".into()),
    )
    .title("LinkWatch Overlay")
    .inner_size(win_w, win_h)
    .position(pos_x, pos_y)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .resizable(false)
    .visible(true)
    .build()
    .map_err(|e| e.to_string())?;

    // Safe cursor pass-through
    let _ = window.set_ignore_cursor_events(true);

    Ok(())
}

/// Close (hide) the overlay window.
pub fn close_overlay(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.hide();
    }
    Ok(())
}

/// Show the overlay if hidden or missing, hide it if visible. Existence is not
/// the test: close_overlay only hides the window, so the WebviewWindow handle
/// lives on after every "close" and a missing-vs-present check wedges the toggle.
/// Returns the new visibility so callers can sync UI state with it.
pub fn toggle_overlay(app: &tauri::AppHandle) -> Result<bool, String> {
    let visible = app
        .get_webview_window("overlay")
        .map_or(false, |w| w.is_visible().unwrap_or(false));
    if visible {
        close_overlay(app)?;
        Ok(false)
    } else {
        open_overlay(app)?;
        Ok(true)
    }
}

/// Reposition & resize overlay if config changed while running
pub fn reposition_overlay(app: &tauri::AppHandle, cfg: &crate::engine::overlay_config::OverlayConfig) {
    if let Some(w) = app.get_webview_window("overlay") {
        let (pos_x, pos_y, win_w, win_h) = calc_bounds(app, cfg);
        let _ = w.set_size(tauri::Size::Logical(tauri::LogicalSize { width: win_w, height: win_h }));
        let _ = w.set_position(tauri::Position::Logical(tauri::LogicalPosition { x: pos_x, y: pos_y }));
    }
}
