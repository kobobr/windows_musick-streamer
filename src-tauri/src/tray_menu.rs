use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Manager,
};

use crate::{
    audio::sources,
    commands::{self, StreamTarget},
    network::{discovery, discovery::ConnectionProfile},
    state::AppState,
};

const TRAY_ID: &str = "main";
const ID_SCAN: &str = "tray:scan";
const ID_START: &str = "tray:start";
const ID_STOP: &str = "tray:stop";
const ID_QUIT: &str = "quit";
const ID_HOTKEYS: &str = "tray:hotkeys";

pub fn rebuild(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

pub fn handle_event(app: &AppHandle, id: &str) {
    match id {
        ID_SCAN => {
            if let Ok(devices) = discovery::discover_network_devices(Some(4000)) {
                if let Ok(mut cache) = app.state::<AppState>().cached_devices.lock() {
                    *cache = devices;
                }
            }
            commands::sync_app_state(app);
        }
        ID_START => {
            if let Err(err) = start_selected_outputs(app) {
                eprintln!("tray start failed: {err}");
            }
        }
        ID_STOP => {
            let state = app.state::<AppState>();
            let _ = commands::stop_network_stream_for_state(&state);
            commands::sync_app_state(app);
        }
        ID_HOTKEYS => {
            let state = app.state::<AppState>();
            if let Ok(mut enabled) = state.hotkeys_enabled.lock() {
                *enabled = !*enabled;
            }
            commands::sync_app_state(app);
        }
        id if id.starts_with("device:") => {
            toggle_device_connection(app, id.trim_start_matches("device:"));
            commands::sync_app_state(app);
        }
        _ => {}
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    let state = app.state::<AppState>();
    let streaming = state
        .network
        .lock()
        .map(|network| !network.is_empty())
        .unwrap_or(false)
        || state
            .stream_starting
            .lock()
            .map(|starting| *starting)
            .unwrap_or(false);
    let selected_count = app
        .state::<AppState>()
        .selected_connection_ids
        .lock()
        .map(|selected| selected.len())
        .unwrap_or_default();

    let scan = MenuItem::with_id(app, ID_SCAN, "Scan", !streaming, None::<&str>)?;
    let devices = devices_submenu(app)?;
    let start = MenuItem::with_id(
        app,
        ID_START,
        "Start selected",
        !streaming && selected_count > 0,
        None::<&str>,
    )?;
    let stop = MenuItem::with_id(app, ID_STOP, "Stop", streaming, None::<&str>)?;
    let hotkeys = hotkeys_submenu(app)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;

    menu.append(&scan)?;
    menu.append(&devices)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&start)?;
    menu.append(&stop)?;
    menu.append(&hotkeys)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;
    Ok(menu)
}

fn devices_submenu(app: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let submenu = Submenu::with_id(app, "tray:devices-root", "Devices", true)?;
    let devices = app
        .state::<AppState>()
        .cached_devices
        .lock()
        .map(|devices| devices.clone())
        .unwrap_or_default();
    let selected = app
        .state::<AppState>()
        .selected_connection_ids
        .lock()
        .map(|selected| selected.clone())
        .unwrap_or_default();

    if devices.is_empty() {
        let empty = MenuItem::with_id(
            app,
            "tray:no-devices",
            "No devices yet",
            false,
            None::<&str>,
        )?;
        submenu.append(&empty)?;
        return Ok(submenu);
    }

    for device in devices {
        let label = sources::compact_source_label(&device.name);
        let device_menu = Submenu::with_id(app, format!("tray:device:{}", device.id), label, true)?;
        let mut added = 0usize;
        for connection in protocol_profiles(&device.connections) {
            let checked = selected.contains(&connection.id);
            let enabled = commands::stream_selectable(connection);
            let label = format!(
                "{} / {}",
                connection.protocol,
                commands::auto_codec_for_connection(connection)
                    .unwrap_or_else(|| "pending".to_string())
                    .to_uppercase()
            );
            let item = CheckMenuItem::with_id(
                app,
                format!("device:{}", connection.id),
                label,
                enabled,
                checked,
                None::<&str>,
            )?;
            device_menu.append(&item)?;
            added += 1;
        }
        if added == 0 {
            let item = MenuItem::with_id(
                app,
                format!("tray:no-ready:{}", device.id),
                "No ready protocols",
                false,
                None::<&str>,
            )?;
            device_menu.append(&item)?;
        }
        submenu.append(&device_menu)?;
    }

    Ok(submenu)
}

fn hotkeys_submenu(app: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let submenu = Submenu::with_id(app, "tray:hotkeys-root", "Hotkeys", true)?;
    let enabled = app
        .state::<AppState>()
        .hotkeys_enabled
        .lock()
        .map(|enabled| *enabled)
        .unwrap_or(false);
    let item = CheckMenuItem::with_id(
        app,
        ID_HOTKEYS,
        "F1 mute / F2 down / F3 up",
        true,
        enabled,
        None::<&str>,
    )?;
    submenu.append(&item)?;
    Ok(submenu)
}

fn protocol_profiles(connections: &[ConnectionProfile]) -> Vec<&ConnectionProfile> {
    let mut profiles = connections
        .iter()
        .filter(|connection| {
            matches!(
                connection.protocol.as_str(),
                "DLNA" | "UPNP" | "AIRPLAY" | "CHROMECAST"
            )
        })
        .collect::<Vec<_>>();
    profiles.sort_by_key(|connection| protocol_rank(&connection.protocol));
    profiles
}

fn protocol_rank(protocol: &str) -> u8 {
    match protocol {
        "DLNA" => 0,
        "UPNP" => 1,
        "AIRPLAY" => 2,
        "CHROMECAST" => 3,
        _ => 9,
    }
}

fn toggle_device_connection(app: &AppHandle, connection_id: &str) {
    let state = app.state::<AppState>();
    let devices = state
        .cached_devices
        .lock()
        .map(|devices| devices.clone())
        .unwrap_or_default();
    let Some(owner) = devices.iter().find(|device| {
        device
            .connections
            .iter()
            .any(|connection| connection.id == connection_id)
    }) else {
        return;
    };
    let same_device_ids = owner
        .connections
        .iter()
        .map(|connection| connection.id.clone())
        .collect::<Vec<_>>();

    {
        let Ok(mut selected) = state.selected_connection_ids.lock() else {
            return;
        };
        if selected.iter().any(|id| id == connection_id) {
            selected.retain(|id| id != connection_id);
        } else {
            selected.retain(|id| !same_device_ids.contains(id));
            selected.push(connection_id.to_string());
        }
    }
}

fn start_selected_outputs(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let devices = state
        .cached_devices
        .lock()
        .map_err(|_| "device cache lock poisoned")?
        .clone();
    let selected = state
        .selected_connection_ids
        .lock()
        .map_err(|_| "selected output lock poisoned")?
        .clone();
    let volume = *state
        .global_volume
        .lock()
        .map_err(|_| "volume lock poisoned")?;
    let target_volumes = state
        .target_volumes
        .lock()
        .map_err(|_| "target volume lock poisoned")?
        .clone();

    let mut targets = Vec::<StreamTarget>::new();
    for connection in devices
        .iter()
        .flat_map(|device| device.connections.iter())
        .filter(|connection| selected.contains(&connection.id))
    {
        let Some(codec) = commands::auto_codec_for_connection(connection) else {
            continue;
        };
        targets.push(StreamTarget {
            connection: connection.clone(),
            codec,
            volume: target_volumes
                .get(&connection.id)
                .copied()
                .unwrap_or(volume),
        });
    }

    if targets.is_empty() {
        return Err("no ready tray outputs selected".to_string());
    }
    commands::start_stream_targets_background(app.clone(), targets)
}
