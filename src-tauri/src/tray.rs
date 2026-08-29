//! System tray: show/hide the window and quit. Extended in M4 with
//! "add URL" and completion notifications.

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    App, Manager,
};

/// Build the tray icon + menu and wire menu events.
pub fn build_tray(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show DM", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "Hide to tray", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit DM", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("a default window icon must be configured");

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("DM — Download Manager")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "hide" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}
