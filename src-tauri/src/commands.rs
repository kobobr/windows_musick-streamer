use std::{collections::HashMap, thread, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    audio::{
        devices::{self, AudioDeviceInfo},
        sources::{self, AudioSourceInfo, CaptureSource},
    },
    network::{
        discovery::{self, ConnectionProfile, DiscoveredRenderer, NetworkDevice},
        dlna::{DlnaStreamFormat, DlnaStreamInfo},
        receiver::ReceiverInfo,
        sender::SenderInfo,
    },
    state::{AppState, NetworkStreamHandle, StartedStreamInfo},
};

#[derive(Debug, Clone, Serialize)]
pub struct BackendStatus {
    pub backend: String,
    pub sender_running: bool,
    pub receiver_running: bool,
    pub capture: String,
    pub transport: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamTarget {
    pub connection: ConnectionProfile,
    pub codec: String,
    pub volume: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureSourceStatus {
    pub source: CaptureSource,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyStatus {
    pub enabled: bool,
    pub decrease_key: String,
    pub increase_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppSnapshot {
    pub status: BackendStatus,
    pub devices: Vec<NetworkDevice>,
    pub sources: Vec<AudioSourceInfo>,
    pub selected_connection_ids: Vec<String>,
    pub selected_source: CaptureSourceStatus,
    pub streams: Vec<StartedStreamInfo>,
    pub stream_starting: bool,
    pub target_volumes: std::collections::HashMap<String, u8>,
    pub global_volume: u8,
    pub muted: bool,
    pub hotkeys: HotkeyStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamErrorPayload {
    pub message: String,
}

#[tauri::command]
pub fn backend_status(state: State<'_, AppState>) -> Result<BackendStatus, String> {
    backend_status_for_state(&state)
}

fn backend_status_for_state(state: &AppState) -> Result<BackendStatus, String> {
    let sender_running = state
        .sender
        .lock()
        .map_err(|_| "sender lock poisoned")?
        .is_some();
    let network_running = !state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?
        .is_empty();
    let receiver_running = state
        .receiver
        .lock()
        .map_err(|_| "receiver lock poisoned")?
        .is_some();
    let stream_starting = *state
        .stream_starting
        .lock()
        .map_err(|_| "stream start lock poisoned")?;

    Ok(BackendStatus {
        backend: "Rust/Tauri".to_string(),
        sender_running: sender_running || network_running || stream_starting,
        receiver_running,
        capture: "WASAPI Loopback".to_string(),
        transport: if network_running {
            "AVTransport network stream".to_string()
        } else {
            "RTP/UDP PCM S16LE".to_string()
        },
    })
}

#[tauri::command]
pub fn app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    app_snapshot_for_state(&state)
}

pub fn app_snapshot_for_state(state: &AppState) -> Result<AppSnapshot, String> {
    let status = backend_status_for_state(state)?;
    let sources = normalized_audio_sources_for_state(state)?;
    let source = state
        .selected_source
        .lock()
        .map_err(|_| "selected source lock poisoned")?
        .clone();
    let devices = state
        .cached_devices
        .lock()
        .map_err(|_| "device cache lock poisoned")?
        .clone();
    let selected_connection_ids = state
        .selected_connection_ids
        .lock()
        .map_err(|_| "selected output lock poisoned")?
        .clone();
    let streams = state
        .active_streams
        .lock()
        .map_err(|_| "active stream lock poisoned")?
        .clone();
    let stream_starting = *state
        .stream_starting
        .lock()
        .map_err(|_| "stream start lock poisoned")?;
    let global_volume = *state
        .global_volume
        .lock()
        .map_err(|_| "volume lock poisoned")?;
    let target_volumes = state
        .target_volumes
        .lock()
        .map_err(|_| "target volume lock poisoned")?
        .clone();
    let muted = *state.muted.lock().map_err(|_| "mute lock poisoned")?;
    let hotkeys_enabled = *state
        .hotkeys_enabled
        .lock()
        .map_err(|_| "hotkeys lock poisoned")?;

    Ok(AppSnapshot {
        status,
        devices,
        sources,
        selected_connection_ids,
        selected_source: CaptureSourceStatus {
            label: source.label(),
            source,
        },
        streams,
        stream_starting,
        target_volumes,
        global_volume,
        muted,
        hotkeys: HotkeyStatus {
            enabled: hotkeys_enabled,
            decrease_key: "F1 mute / F2".to_string(),
            increase_key: "F3".to_string(),
        },
    })
}

pub fn sync_app_state(app: &AppHandle) {
    let _ = crate::tray_menu::rebuild(app);
    emit_app_state_changed(app);
}

pub fn emit_app_state_changed(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Ok(snapshot) = app_snapshot_for_state(&state) {
        let _ = app.emit("app-state-changed", snapshot);
    }
}

fn normalized_audio_sources_for_state(state: &AppState) -> Result<Vec<AudioSourceInfo>, String> {
    let selected = state
        .selected_source
        .lock()
        .map_err(|_| "selected source lock poisoned")?
        .clone();
    let list = sources::list_audio_sources(&selected);

    *state
        .cached_sources
        .lock()
        .map_err(|_| "source cache lock poisoned")? = list.clone();
    Ok(list)
}

#[tauri::command]
pub fn list_audio_outputs() -> Result<Vec<AudioDeviceInfo>, String> {
    devices::list_output_devices()
}

#[tauri::command]
pub fn list_audio_sources(state: State<'_, AppState>) -> Result<Vec<AudioSourceInfo>, String> {
    normalized_audio_sources_for_state(&state)
}

#[tauri::command]
pub fn set_capture_source(
    app: AppHandle,
    state: State<'_, AppState>,
    source: CaptureSource,
) -> Result<CaptureSourceStatus, String> {
    let status = set_capture_source_inner(&state, source)?;
    sync_app_state(&app);
    Ok(status)
}

#[tauri::command]
pub fn set_capture_source_ids(
    app: AppHandle,
    state: State<'_, AppState>,
    source_ids: Vec<String>,
) -> Result<CaptureSourceStatus, String> {
    let fresh = normalized_audio_sources_for_state(&state)?;
    let known = state
        .cached_sources
        .lock()
        .map_err(|_| "source cache lock poisoned")?
        .clone();
    if source_ids.iter().any(|id| id == "system") || source_ids.is_empty() {
        let status = set_capture_source_inner(&state, CaptureSource::System)?;
        sync_app_state(&app);
        return Ok(status);
    }
    let source =
        sources::source_from_app_ids(&source_ids, if known.is_empty() { &fresh } else { &known });
    let status = set_capture_source_inner(&state, source)?;
    sync_app_state(&app);
    Ok(status)
}

#[tauri::command]
pub fn capture_source_status(state: State<'_, AppState>) -> Result<CaptureSourceStatus, String> {
    normalized_audio_sources_for_state(&state)?;
    let source = state
        .selected_source
        .lock()
        .map_err(|_| "selected source lock poisoned")?
        .clone();
    Ok(CaptureSourceStatus {
        label: source.label(),
        source,
    })
}

fn set_capture_source_inner(
    state: &AppState,
    source: CaptureSource,
) -> Result<CaptureSourceStatus, String> {
    *state
        .selected_source
        .lock()
        .map_err(|_| "selected source lock poisoned")? = source.clone();
    Ok(CaptureSourceStatus {
        label: source.label(),
        source,
    })
}

#[tauri::command]
pub fn discover_dlna_renderers(timeout_ms: Option<u64>) -> Result<Vec<DiscoveredRenderer>, String> {
    discovery::discover_renderers(timeout_ms)
}

#[tauri::command]
pub fn discover_network_devices(
    app: AppHandle,
    state: State<'_, AppState>,
    timeout_ms: Option<u64>,
) -> Result<Vec<NetworkDevice>, String> {
    let devices = discovery::discover_network_devices(timeout_ms)?;
    prune_selected_connections_for_devices(&state, &devices)?;
    *state
        .cached_devices
        .lock()
        .map_err(|_| "device cache lock poisoned")? = devices.clone();
    sync_app_state(&app);
    Ok(devices)
}

fn prune_selected_connections_for_devices(
    state: &AppState,
    devices: &[NetworkDevice],
) -> Result<(), String> {
    let known = devices
        .iter()
        .flat_map(|device| {
            device
                .connections
                .iter()
                .map(|connection| connection.id.clone())
        })
        .collect::<std::collections::HashSet<_>>();
    let mut selected = state
        .selected_connection_ids
        .lock()
        .map_err(|_| "selected output lock poisoned")?;
    selected.retain(|id| known.contains(id));
    Ok(())
}

#[tauri::command]
pub fn start_rtp_stream(
    state: State<'_, AppState>,
    target: String,
    port: u16,
) -> Result<SenderInfo, String> {
    let mut sender = state.sender.lock().map_err(|_| "sender lock poisoned")?;
    if sender.is_some() {
        return Err("RTP sender is already running".to_string());
    }
    if !state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?
        .is_empty()
    {
        return Err("network stream is already running".to_string());
    }
    let (handle, info) = crate::network::sender::start(target, port)?;
    *sender = Some(handle);
    Ok(info)
}

#[tauri::command]
pub fn stop_rtp_stream(state: State<'_, AppState>) -> Result<(), String> {
    let mut sender = state.sender.lock().map_err(|_| "sender lock poisoned")?;
    if let Some(handle) = sender.take() {
        handle.stop();
    }
    Ok(())
}

#[tauri::command]
pub fn start_dlna_stream(
    state: State<'_, AppState>,
    renderer: DiscoveredRenderer,
) -> Result<DlnaStreamInfo, String> {
    if state
        .sender
        .lock()
        .map_err(|_| "sender lock poisoned")?
        .is_some()
    {
        return Err("RTP sender is already running".to_string());
    }

    let mut network = state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?;
    if !network.is_empty() {
        return Err("network stream is already running".to_string());
    }

    let (handle, info) = crate::network::dlna::start(renderer)?;
    network.insert(info.control_url.clone(), NetworkStreamHandle::Dlna(handle));
    Ok(info)
}

#[tauri::command]
pub fn start_network_stream(
    app: AppHandle,
    state: State<'_, AppState>,
    connection: ConnectionProfile,
    codec: String,
) -> Result<StartedStreamInfo, String> {
    let mut streams = start_network_streams_for_state(
        &state,
        vec![StreamTarget {
            connection,
            codec,
            volume: 100,
        }],
    )?;
    sync_app_state(&app);
    streams
        .pop()
        .ok_or_else(|| "stream did not start".to_string())
}

#[tauri::command]
pub fn start_network_streams(
    app: AppHandle,
    state: State<'_, AppState>,
    targets: Vec<StreamTarget>,
) -> Result<Vec<StartedStreamInfo>, String> {
    let streams = start_network_streams_for_state(&state, targets)?;
    sync_app_state(&app);
    Ok(streams)
}

#[tauri::command]
pub fn start_selected_network_streams(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_ids: Vec<String>,
    volumes: HashMap<String, u8>,
) -> Result<(), String> {
    let generation = begin_background_start(&state)?;
    emit_app_state_changed(&app);

    let worker_app = app.clone();
    thread::spawn(move || {
        let result = {
            let state = worker_app.state::<AppState>();
            selected_stream_targets_for_state(&state, &connection_ids, &volumes)
                .and_then(|targets| start_network_streams_for_state(&state, targets))
        };
        finish_background_start(&worker_app, generation, result);
    });

    let watchdog_app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(25));
        fail_stale_background_start(
            &watchdog_app,
            generation,
            "Start timed out. The UI stayed alive, but the selected device did not finish its streaming handshake.",
        );
    });

    Ok(())
}

pub fn start_stream_targets_background(
    app: AppHandle,
    targets: Vec<StreamTarget>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let generation = begin_background_start(&state)?;
    emit_app_state_changed(&app);

    let worker_app = app.clone();
    thread::spawn(move || {
        let result = {
            let state = worker_app.state::<AppState>();
            start_network_streams_for_state(&state, targets)
        };
        finish_background_start(&worker_app, generation, result);
    });

    let watchdog_app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(25));
        fail_stale_background_start(
            &watchdog_app,
            generation,
            "Start timed out. The tray stayed alive, but the selected device did not finish its streaming handshake.",
        );
    });

    Ok(())
}

fn begin_background_start(state: &AppState) -> Result<u64, String> {
    if state
        .sender
        .lock()
        .map_err(|_| "sender lock poisoned")?
        .is_some()
    {
        return Err("RTP sender is already running".to_string());
    }
    if !state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?
        .is_empty()
    {
        return Err("network stream is already running".to_string());
    }

    let mut starting = state
        .stream_starting
        .lock()
        .map_err(|_| "stream start lock poisoned")?;
    if *starting {
        return Err("network stream is already starting".to_string());
    }
    *starting = true;
    Ok(state.next_start_generation())
}

fn finish_background_start(
    app: &AppHandle,
    generation: u64,
    result: Result<Vec<StartedStreamInfo>, String>,
) {
    let state = app.state::<AppState>();
    let is_current = state.current_start_generation() == generation;
    let mut was_starting = false;
    if is_current {
        if let Ok(mut starting) = state.stream_starting.lock() {
            was_starting = *starting;
            *starting = false;
        }
    }

    match result {
        Ok(streams) if is_current && was_starting => {
            let _ = app.emit("network-stream-started", streams);
        }
        Ok(streams) => {
            let ids = streams
                .into_iter()
                .map(|stream| stream.id)
                .collect::<Vec<_>>();
            let _ = stop_network_stream_ids_for_state(&state, &ids);
        }
        Err(message) if is_current && was_starting => {
            let _ = app.emit("network-stream-error", StreamErrorPayload { message });
        }
        Err(_) => {}
    }

    sync_app_state(app);
}

fn fail_stale_background_start(app: &AppHandle, generation: u64, message: &str) {
    let state = app.state::<AppState>();
    if state.current_start_generation() != generation {
        return;
    }
    let mut should_emit = false;
    if let Ok(mut starting) = state.stream_starting.lock() {
        if *starting {
            *starting = false;
            should_emit = true;
        }
    }
    if should_emit {
        let _ = app.emit(
            "network-stream-error",
            StreamErrorPayload {
                message: message.to_string(),
            },
        );
        sync_app_state(app);
    }
}

pub fn start_network_streams_for_state(
    state: &AppState,
    targets: Vec<StreamTarget>,
) -> Result<Vec<StartedStreamInfo>, String> {
    if targets.is_empty() {
        return Err("no stream targets selected".to_string());
    }
    if state
        .sender
        .lock()
        .map_err(|_| "sender lock poisoned")?
        .is_some()
    {
        return Err("RTP sender is already running".to_string());
    }
    if !state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?
        .is_empty()
    {
        return Err("network stream is already running".to_string());
    }

    let mut pending: Vec<(String, NetworkStreamHandle)> = Vec::new();
    let mut infos = Vec::new();
    let mut seen_routes = std::collections::HashSet::new();
    let source = state
        .selected_source
        .lock()
        .map_err(|_| "selected source lock poisoned")?
        .clone();
    let first_volume = targets
        .first()
        .map(|target| target_volume_for_start(state, target))
        .unwrap_or(10);
    *state
        .global_volume
        .lock()
        .map_err(|_| "volume lock poisoned")? = first_volume;
    let muted = *state.muted.lock().map_err(|_| "mute lock poisoned")?;

    for mut target in targets {
        target.volume = target_volume_for_start(state, &target);
        state
            .target_volumes
            .lock()
            .map_err(|_| "target volume lock poisoned")?
            .insert(target.connection.id.clone(), target.volume);
        let route_key = stream_route_key(&target.connection);
        if !seen_routes.insert(route_key.clone()) {
            for (_, handle) in pending {
                handle.stop();
            }
            return Err(format!(
                "duplicate route selected for {}; choose one protocol for this endpoint",
                target.connection.name
            ));
        }
        match start_one_network_target(target, source.clone(), muted) {
            Ok((id, handle, info)) => {
                pending.push((id, handle));
                infos.push(info);
            }
            Err(err) => {
                stop_pending_streams(pending);
                return Err(err);
            }
        }
    }

    let mut network = state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?;
    for (id, handle) in pending {
        network.insert(id, handle);
    }
    *state
        .active_streams
        .lock()
        .map_err(|_| "active stream lock poisoned")? = infos.clone();

    Ok(infos)
}

fn stop_pending_streams(pending: Vec<(String, NetworkStreamHandle)>) {
    for (_, handle) in pending {
        handle.stop();
    }
}

fn selected_stream_targets_for_state(
    state: &AppState,
    connection_ids: &[String],
    volumes: &HashMap<String, u8>,
) -> Result<Vec<StreamTarget>, String> {
    if connection_ids.is_empty() {
        return Err("no stream targets selected".to_string());
    }

    let devices = state
        .cached_devices
        .lock()
        .map_err(|_| "device cache lock poisoned")?
        .clone();
    let mut targets = Vec::new();

    for connection_id in connection_ids {
        let connection = devices
            .iter()
            .flat_map(|device| device.connections.iter())
            .find(|connection| &connection.id == connection_id)
            .ok_or_else(|| {
                format!("selected output {connection_id} is no longer in the scan cache")
            })?;
        let codec = auto_codec_for_connection(connection).ok_or_else(|| {
            format!(
                "{} is visible, but no implemented codec is available",
                connection.name
            )
        })?;
        targets.push(StreamTarget {
            connection: connection.clone(),
            codec,
            volume: volumes.get(connection_id).copied().unwrap_or(10),
        });
    }

    *state
        .selected_connection_ids
        .lock()
        .map_err(|_| "selected output lock poisoned")? = connection_ids.to_vec();
    {
        let mut stored_volumes = state
            .target_volumes
            .lock()
            .map_err(|_| "target volume lock poisoned")?;
        for (connection_id, volume) in volumes {
            stored_volumes.insert(connection_id.clone(), (*volume).min(150));
        }
    }

    Ok(targets)
}

fn target_volume_for_start(state: &AppState, target: &StreamTarget) -> u8 {
    state
        .target_volumes
        .lock()
        .ok()
        .and_then(|volumes| volumes.get(&target.connection.id).copied())
        .unwrap_or(target.volume)
        .min(150)
}

fn stream_route_key(connection: &ConnectionProfile) -> String {
    connection.host.to_ascii_lowercase()
}

pub fn auto_codec_for_connection(connection: &ConnectionProfile) -> Option<String> {
    connection
        .codecs
        .iter()
        .filter(|codec| codec.implemented)
        .max_by(|left, right| {
            codec_quality(&left.id)
                .partial_cmp(&codec_quality(&right.id))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|codec| codec.id.clone())
        .or_else(|| {
            if connection.protocol == "AIRPLAY"
                && connection.codecs.iter().any(|codec| codec.id == "alac")
            {
                Some("alac".to_string())
            } else {
                None
            }
        })
}

pub fn stream_selectable(connection: &ConnectionProfile) -> bool {
    connection.supported && auto_codec_for_connection(connection).is_some()
        || connection.protocol == "AIRPLAY"
}

fn codec_quality(id: &str) -> f32 {
    match id {
        "flac" => 10.0,
        "alac" => 9.0,
        "pcm_s16le" => 8.5,
        "pcm_l16be" => 8.0,
        "ldac" | "lhdc" => 7.0,
        "aac_lc" | "opus" => 5.0,
        "mp3" => 3.0,
        "sbc" => 1.0,
        _ => 0.0,
    }
}

fn start_one_network_target(
    target: StreamTarget,
    source: CaptureSource,
    muted: bool,
) -> Result<(String, NetworkStreamHandle, StartedStreamInfo), String> {
    let connection = target.connection;
    let codec = target.codec;
    let logical_volume = target.volume.min(150);
    let volume = if muted { 0 } else { logical_volume };
    let protocol = connection.protocol.clone();
    let codec_supported = connection
        .codecs
        .iter()
        .any(|candidate| candidate.id == codec && candidate.implemented);
    let airplay_probe = connection.protocol == "AIRPLAY" && codec == "alac";
    if !codec_supported && !airplay_probe {
        return Err(format!(
            "{} / {} is visible, but this streaming backend is not implemented yet",
            connection.protocol, codec
        ));
    }

    match protocol.as_str() {
        "DLNA" | "UPNP" => {
            let renderer = connection
                .renderer
                .ok_or_else(|| format!("{protocol} connection profile is missing renderer data"))?;
            let format = DlnaStreamFormat::from_codec_id(&codec)?;

            let id = connection.id.clone();
            let (handle, info) = crate::network::dlna::start_with_format_and_source(
                renderer,
                volume as f32 / 100.0,
                format,
                source,
            )?;
            let transport = match protocol.as_str() {
                "UPNP" => format!("UPnP AVTransport / {}", info.codec),
                _ => format!("DLNA AVTransport / {}", info.codec),
            };

            Ok((
                id.clone(),
                NetworkStreamHandle::Dlna(handle),
                StartedStreamInfo {
                    id,
                    protocol,
                    codec: info.codec,
                    transport,
                    target: info.renderer,
                    volume: logical_volume,
                    sample_rate: info.sample_rate,
                    channels: info.channels,
                    control_url: Some(info.control_url),
                    stream_url: Some(info.stream_url),
                },
            ))
        }
        "CHROMECAST" => {
            if codec != "pcm_s16le" {
                return Err(format!("Chromecast codec {codec} is not implemented yet"));
            }

            let id = connection.id.clone();
            let (handle, info) = crate::network::cast::start_with_source(
                &connection,
                &codec,
                volume as f32 / 100.0,
                source,
            )?;

            Ok((
                id.clone(),
                NetworkStreamHandle::Cast(handle),
                StartedStreamInfo {
                    id,
                    protocol,
                    codec: info.codec,
                    transport: "Chromecast Default Media Receiver / WAV PCM".to_string(),
                    target: info.renderer,
                    volume: logical_volume,
                    sample_rate: info.sample_rate,
                    channels: info.channels,
                    control_url: None,
                    stream_url: Some(info.stream_url),
                },
            ))
        }
        "AIRPLAY" => {
            if codec != "alac" {
                return Err(format!("AirPlay codec {codec} is not implemented yet"));
            }

            let id = connection.id.clone();
            let (handle, info) = crate::network::airplay::start_with_source(
                &connection,
                &codec,
                volume as f32 / 100.0,
                source,
            )?;
            let transport = if info.codec.contains("AirPlay 2") {
                "AirPlay 2 / ALAC over encrypted RTP".to_string()
            } else {
                "AirPlay 1 RAOP / ALAC over RTP".to_string()
            };

            Ok((
                id.clone(),
                NetworkStreamHandle::Airplay(handle),
                StartedStreamInfo {
                    id,
                    protocol,
                    codec: info.codec,
                    transport,
                    target: info.renderer,
                    volume: logical_volume,
                    sample_rate: info.sample_rate,
                    channels: info.channels,
                    control_url: None,
                    stream_url: None,
                },
            ))
        }
        other => Err(format!(
            "{other} discovery is available, but streaming is not implemented yet"
        )),
    }
}

#[tauri::command]
pub fn stop_network_stream(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    stop_network_stream_for_state(&state)?;
    sync_app_state(&app);
    Ok(())
}

pub fn stop_network_stream_for_state(state: &AppState) -> Result<(), String> {
    state.next_start_generation();
    if let Ok(mut starting) = state.stream_starting.lock() {
        *starting = false;
    }
    let mut network = state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?;
    let handles = network
        .drain()
        .map(|(_, handle)| handle)
        .collect::<Vec<_>>();
    drop(network);
    for handle in handles {
        handle.stop();
    }
    state
        .active_streams
        .lock()
        .map_err(|_| "active stream lock poisoned")?
        .clear();

    let mut sender = state.sender.lock().map_err(|_| "sender lock poisoned")?;
    if let Some(handle) = sender.take() {
        handle.stop();
    }
    Ok(())
}

fn stop_network_stream_ids_for_state(state: &AppState, ids: &[String]) -> Result<(), String> {
    let mut network = state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?;
    let handles = ids
        .iter()
        .filter_map(|id| network.remove(id))
        .collect::<Vec<_>>();
    drop(network);
    for handle in handles {
        handle.stop();
    }
    if let Ok(mut streams) = state.active_streams.lock() {
        streams.retain(|stream| !ids.contains(&stream.id));
    }
    Ok(())
}

#[tauri::command]
pub fn set_network_stream_volume(
    app: AppHandle,
    state: State<'_, AppState>,
    stream_id: String,
    volume: u8,
) -> Result<(), String> {
    let logical_volume = volume.min(150);
    *state
        .global_volume
        .lock()
        .map_err(|_| "volume lock poisoned")? = logical_volume;
    state
        .target_volumes
        .lock()
        .map_err(|_| "target volume lock poisoned")?
        .insert(stream_id.clone(), logical_volume);
    let effective_volume = effective_volume_for_state(&state, logical_volume)?;

    let network = state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?;
    if let Some(handle) = network.get(&stream_id) {
        handle.set_volume(effective_volume);
    }
    if let Ok(mut streams) = state.active_streams.lock() {
        for stream in streams.iter_mut().filter(|stream| stream.id == stream_id) {
            stream.volume = logical_volume;
        }
    }
    sync_app_state(&app);
    Ok(())
}

pub fn adjust_all_network_stream_volumes(state: &AppState, delta: i16) -> Result<u8, String> {
    let next_volume = {
        let mut volume = state
            .global_volume
            .lock()
            .map_err(|_| "volume lock poisoned")?;
        let next = (*volume as i16 + delta).clamp(0, 150) as u8;
        *volume = next;
        next
    };
    update_selected_target_volumes(state, next_volume)?;
    let effective_volume = effective_volume_for_state(state, next_volume)?;

    let network = state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?;
    for handle in network.values() {
        handle.set_volume(effective_volume);
    }
    if let Ok(mut streams) = state.active_streams.lock() {
        for stream in streams.iter_mut() {
            stream.volume = next_volume;
        }
    }
    Ok(next_volume)
}

pub fn toggle_mute_for_state(state: &AppState) -> Result<(u8, bool), String> {
    let muted = {
        let mut muted = state.muted.lock().map_err(|_| "mute lock poisoned")?;
        *muted = !*muted;
        *muted
    };
    let volume = *state
        .global_volume
        .lock()
        .map_err(|_| "volume lock poisoned")?;
    let effective_volume = if muted { 0.0 } else { volume as f32 / 100.0 };

    let network = state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?;
    for handle in network.values() {
        handle.set_volume(effective_volume);
    }
    Ok((volume, muted))
}

fn effective_volume_for_state(state: &AppState, logical_volume: u8) -> Result<f32, String> {
    let muted = *state.muted.lock().map_err(|_| "mute lock poisoned")?;
    Ok(if muted {
        0.0
    } else {
        logical_volume.min(150) as f32 / 100.0
    })
}

fn update_selected_target_volumes(state: &AppState, volume: u8) -> Result<(), String> {
    let selected = state
        .selected_connection_ids
        .lock()
        .map_err(|_| "selected output lock poisoned")?
        .clone();
    let active = state
        .active_streams
        .lock()
        .map(|streams| {
            streams
                .iter()
                .map(|stream| stream.id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut volumes = state
        .target_volumes
        .lock()
        .map_err(|_| "target volume lock poisoned")?;
    for id in selected.into_iter().chain(active) {
        volumes.insert(id, volume);
    }
    Ok(())
}

#[tauri::command]
pub fn set_hotkeys_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<HotkeyStatus, String> {
    *state
        .hotkeys_enabled
        .lock()
        .map_err(|_| "hotkeys lock poisoned")? = enabled;
    let status = hotkeys_status(state)?;
    sync_app_state(&app);
    Ok(status)
}

#[tauri::command]
pub fn hotkeys_status(state: State<'_, AppState>) -> Result<HotkeyStatus, String> {
    let enabled = *state
        .hotkeys_enabled
        .lock()
        .map_err(|_| "hotkeys lock poisoned")?;
    Ok(HotkeyStatus {
        enabled,
        decrease_key: "F1 mute / F2".to_string(),
        increase_key: "F3".to_string(),
    })
}

#[tauri::command]
pub fn set_selected_connection_ids(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let connection_ids = sanitize_connection_ids_for_state(&state, connection_ids)?;
    *state
        .selected_connection_ids
        .lock()
        .map_err(|_| "selected output lock poisoned")? = connection_ids.clone();
    let default_volume = *state
        .global_volume
        .lock()
        .map_err(|_| "volume lock poisoned")?;
    if let Ok(mut volumes) = state.target_volumes.lock() {
        for id in &connection_ids {
            volumes.entry(id.clone()).or_insert(default_volume);
        }
    }
    sync_app_state(&app);
    Ok(connection_ids)
}

fn sanitize_connection_ids_for_state(
    state: &AppState,
    connection_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let devices = state
        .cached_devices
        .lock()
        .map_err(|_| "device cache lock poisoned")?
        .clone();
    let mut selected = Vec::<String>::new();

    for connection_id in connection_ids {
        let Some(device) = devices.iter().find(|device| {
            device
                .connections
                .iter()
                .any(|connection| connection.id == connection_id)
        }) else {
            continue;
        };
        let same_device_ids = device
            .connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect::<Vec<_>>();
        selected.retain(|id| !same_device_ids.contains(id));
        selected.push(connection_id);
    }

    Ok(selected)
}

#[tauri::command]
pub fn stop_dlna_stream(state: State<'_, AppState>) -> Result<(), String> {
    let mut network = state
        .network
        .lock()
        .map_err(|_| "network stream lock poisoned")?;
    let handles = network
        .drain()
        .map(|(_, handle)| handle)
        .collect::<Vec<_>>();
    drop(network);
    for handle in handles {
        handle.stop();
    }
    state
        .active_streams
        .lock()
        .map_err(|_| "active stream lock poisoned")?
        .clear();
    Ok(())
}

#[tauri::command]
pub fn start_rtp_receiver(state: State<'_, AppState>, port: u16) -> Result<ReceiverInfo, String> {
    let mut receiver = state
        .receiver
        .lock()
        .map_err(|_| "receiver lock poisoned")?;
    if receiver.is_some() {
        return Err("RTP receiver is already running".to_string());
    }
    let (handle, info) = crate::network::receiver::start(port)?;
    *receiver = Some(handle);
    Ok(info)
}

#[tauri::command]
pub fn stop_rtp_receiver(state: State<'_, AppState>) -> Result<(), String> {
    let mut receiver = state
        .receiver
        .lock()
        .map_err(|_| "receiver lock poisoned")?;
    if let Some(handle) = receiver.take() {
        handle.stop();
    }
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
