// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod events;
mod native_host;
mod tray;

use dm_engine::manager::DownloadManager;
use dm_engine::storage::Storage;
use events::FrontendEvent;
use std::sync::Arc;
use tauri::Emitter;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // SQLite database lives in the app data directory.
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app data directory must be resolvable");
            std::fs::create_dir_all(&data_dir).ok();
            let storage = Storage::open(data_dir.join("dm.sqlite")).expect("open storage");

            let manager = DownloadManager::new(storage);

            // Forward engine lifecycle events to the webview.
            let sink_handle = handle.clone();
            manager.set_event_sink(Arc::new(move |e| {
                let fe: FrontendEvent = e.into();
                let _ = sink_handle.emit(events::EVENT_CHANNEL, fe);
            }));

            app.manage(manager.clone());

            // Accept URLs forwarded from the browser extension via the
            // native-messaging host.
            native_host::start_native_host_listener(manager.clone(), native_host::DEFAULT_PORT);

            // Resume any downloads that were active when the app last closed,
            // then keep the schedule window open/closed.
            tauri::async_runtime::spawn(async move {
                manager.start().await;
                manager.run_schedule_loop();
            });

            tray::build_tray(app)?;
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
            commands::get_settings,
            commands::update_settings,
            commands::save_dir_for,
            commands::notify_on_complete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DM");
}
