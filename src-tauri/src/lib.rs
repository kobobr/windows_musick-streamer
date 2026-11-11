mod audio;
mod commands;
mod hotkeys;
mod network;
mod state;
mod tray_menu;

use state::AppState;
use tauri::Manager;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("Network Audio Router")
                .show_menu_on_left_click(false);

            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;
            tray_menu::rebuild(app.handle())?;
            hotkeys::start(app.handle().clone());

            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if let Ok(devices) = network::discovery::discover_network_devices(Some(5000)) {
                    let state = app_handle.state::<AppState>();
                    if let Ok(mut cache) = state.cached_devices.lock() {
                        *cache = devices;
                    }
                    let _ = tray_menu::rebuild(&app_handle);
                }
            });

            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            id => tray_menu::handle_event(app, id),
        })
        .on_tray_icon_event(|app, event| {
            let should_rebuild = matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            );

            if should_rebuild {
                let _ = tray_menu::rebuild(app);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::backend_status,
            commands::app_snapshot,
            commands::list_audio_outputs,
            commands::list_audio_sources,
            commands::set_capture_source,
            commands::set_capture_source_ids,
            commands::capture_source_status,
            commands::discover_dlna_renderers,
            commands::discover_network_devices,
            commands::start_network_stream,
            commands::start_network_streams,
            commands::start_selected_network_streams,
            commands::set_network_stream_volume,
            commands::set_selected_connection_ids,
            commands::set_hotkeys_enabled,
            commands::hotkeys_status,
            commands::stop_network_stream,
            commands::start_dlna_stream,
            commands::stop_dlna_stream,
            commands::start_rtp_stream,
            commands::stop_rtp_stream,
            commands::start_rtp_receiver,
            commands::stop_rtp_receiver,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Network Audio Router");
}
