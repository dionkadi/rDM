// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod events;
mod logging;
mod native_host;
mod tray;

use dm_engine::manager::DownloadManager;
use dm_engine::storage::Storage;
use events::FrontendEvent;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

/// Type alias for the shared close-to-tray flag, managed in the
/// Tauri state and read by both the WindowEvent handler and the
/// `update_settings` Tauri command.
type CloseToTrayFlag = Arc<AtomicBool>;

fn main() {
    // File-based logging under ~/.local/share/DM/YYYYMMDD-HHMMSS.log
    // (with platform-appropriate fallbacks). The guard must live for
    // the entire process — dropping it removes the global `log`
    // implementation, and any later `log::*!` call would panic.
    let log_guard = logging::init();
    log::info!(
        "DM starting up (pid {}, log file: {})",
        std::process::id(),
        log_guard.path().display()
    );

    // Shared flag the WindowEvent::CloseRequested handler reads.
    // Mirrors the `closeToTray` setting in the user's stored
    // preferences. The initial value is loaded from the persisted
    // settings in `setup()`; every `update_settings` Tauri
    // command (which the SettingsTabs "Close to system tray"
    // toggle calls) refreshes the flag in place so toggling the
    // checkbox takes effect immediately for the next close
    // attempt — no app restart required.
    let close_to_tray: CloseToTrayFlag = Arc::new(AtomicBool::new(false));

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_opener::init())
        .setup({
            let close_to_tray = close_to_tray.clone();
            move |app| {
                let handle = app.handle().clone();

                // SQLite database lives in the app data directory.
                let data_dir = app
                    .path()
                    .app_data_dir()
                    .expect("app data directory must be resolvable");
                std::fs::create_dir_all(&data_dir).ok();
                let storage = Storage::open(&data_dir.join("dm.sqlite"))
                    .expect("open storage");

                // Hand the engine a spawner that drives download tasks on Tauri's
                // own async runtime. The engine is Tauri-free; it just sees a
                // closure that takes a boxed future. This is what lets
                // `DownloadManager::add()` work when called from a sync
                // `#[tauri::command]` (which is dispatched on a non-Tokio thread).
                let rt_handle = tauri::async_runtime::handle().clone();
                let spawn: dm_engine::manager::Spawner = std::sync::Arc::new(move |fut| {
                    rt_handle.spawn(fut);
                });
                let settings = dm_engine::config::load_or_default(&storage);
                // Pull the persisted closeToTray preference out of
                // settings so the WindowEvent handler agrees with
                // the UI toggle on the very first close attempt.
                close_to_tray.store(settings.close_to_tray, Ordering::Relaxed);

                let manager = DownloadManager::with_settings_and_spawner(
                    storage,
                    settings.clone(),
                    spawn,
                );

                // Forward engine lifecycle events to the webview.
                let sink_handle = handle.clone();
                manager.set_event_sink(Arc::new(move |e| {
                    let fe: FrontendEvent = e.into();
                    let _ = sink_handle.emit(events::EVENT_CHANNEL, fe);
                }));

                app.manage(manager.clone());
                app.manage(close_to_tray.clone());

                // Default save directory at startup — used by the
                // native-host listener to populate
                // `CapturedUrl.default_save_dir` so the dialog can
                // show "Save to: <dir>". The frontend can override
                // this once the user picks a category.
                let default_save_dir = settings
                    .default_directory
                    .to_string_lossy()
                    .into_owned();

                // Accept URLs forwarded from the browser extension
                // via the native-messaging host. The listener no
                // longer talks to the engine directly — it emits
                // a `Captured` event to the frontend, which shows
                // the IDM-style confirmation dialog. The user
                // has to click "Download" before the engine ever
                // sees the URL.
                let nh_status = native_host::NativeHostStatus::default();
                app.manage(nh_status.clone());
                native_host::start_native_host_listener(
                    handle.clone(),
                    native_host::DEFAULT_PORT,
                    default_save_dir,
                    nh_status,
                );

                // Resume any downloads that were active when the app last closed,
                // then keep the schedule window open/closed.
                tauri::async_runtime::spawn({
                    let manager = manager.clone();
                    async move {
                        manager.start().await;
                        manager.run_schedule_loop();
                    }
                });

                // Tray icon is best-effort: on locked-down systems (e.g. atomic
                // Fedora with a read-only /run) libayatana-appindicator cannot
                // write its icon cache and panics. Log + skip rather than
                // killing the whole app.
                if let Err(e) = tray::build_tray(app) {
                    log::warn!("tray icon unavailable: {e} — continuing without system tray");
                }
                Ok(())
            }
        })
        .on_window_event(|window, event| {
            // Close-to-tray interception. The flag is read on
            // every CloseRequested, so toggling it in SettingsTabs
            // takes effect for the *next* close attempt (no app
            // restart required). When the flag is on, we cancel
            // the default close and hide the window — the user
            // can bring it back via the tray menu's "Show DM"
            // item. When the flag is off, we do nothing and the
            // default close behaviour runs.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if let Some(state) =
                    window.app_handle().try_state::<CloseToTrayFlag>()
                {
                    if state.load(Ordering::Relaxed) {
                        api.prevent_close();
                        let _ = window.hide();
                        log::info!(
                            "close intercepted: closeToTray=true, \
                             window hidden to tray"
                        );
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::add_download,
            commands::list_downloads,
            commands::get_download,
            commands::pause_download,
            commands::resume_download,
            commands::cancel_download,
            commands::remove_download,
            commands::set_speed_limit,
            commands::set_global_speed_limit,
            commands::reorder_downloads,
            commands::set_download_priority,
            commands::set_download_auth,
            commands::set_download_mirrors,
            commands::set_proxy,
            commands::get_settings,
            commands::update_settings,
            commands::save_dir_for,
            commands::notify_on_complete,
            commands::probe_native_host,
            commands::open_folder,
            commands::open_file,
            commands::copy_text,
            commands::trash_download,
            commands::import_browser_cookies,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DM");
}
