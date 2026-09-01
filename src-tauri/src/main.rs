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
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;

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

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // SQLite database lives in the app data directory.
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app data directory must be resolvable");
            std::fs::create_dir_all(&data_dir).ok();
            let storage = Storage::open(&data_dir.join("dm.sqlite")).expect("open storage");

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
            let manager = DownloadManager::with_settings_and_spawner(storage, settings, spawn);

            // Forward engine lifecycle events to the webview.
            let sink_handle = handle.clone();
            manager.set_event_sink(Arc::new(move |e| {
                let fe: FrontendEvent = e.into();
                let _ = sink_handle.emit(events::EVENT_CHANNEL, fe);
            }));

            app.manage(manager.clone());

            // Wrap in an Arc for the background native-host listener (which
            // needs a shareable handle) and the schedule loop.
            let manager = Arc::new(manager);

            // Accept URLs forwarded from the browser extension via the
            // native-messaging host.
            let nh_status = native_host::NativeHostStatus::default();
            app.manage(nh_status.clone());
            native_host::start_native_host_listener(
                manager.clone(),
                native_host::DEFAULT_PORT,
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
            commands::set_proxy,
            commands::get_settings,
            commands::update_settings,
            commands::save_dir_for,
            commands::notify_on_complete,
            commands::probe_native_host,
            commands::open_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DM");
}
