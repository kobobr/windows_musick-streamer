use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use serde::Serialize;

use crate::{
    audio::sources::{AudioSourceInfo, CaptureSource},
    network::{
        airplay::AirplayHandle, cast::CastHandle, discovery::NetworkDevice, dlna::DlnaHandle,
        receiver::ReceiverHandle, sender::SenderHandle,
    },
};

pub enum NetworkStreamHandle {
    Airplay(AirplayHandle),
    Dlna(DlnaHandle),
    Cast(CastHandle),
}

impl NetworkStreamHandle {
    pub fn set_volume(&self, volume: f32) {
        match self {
            Self::Airplay(handle) => handle.set_volume(volume),
            Self::Dlna(handle) => handle.set_volume(volume),
            Self::Cast(handle) => handle.set_volume(volume),
        }
    }

    pub fn stop(self) {
        match self {
            Self::Airplay(handle) => handle.stop(),
            Self::Dlna(handle) => handle.stop(),
            Self::Cast(handle) => handle.stop(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StartedStreamInfo {
    pub id: String,
    pub protocol: String,
    pub codec: String,
    pub transport: String,
    pub target: String,
    pub volume: u8,
    pub sample_rate: u32,
    pub channels: u16,
    pub control_url: Option<String>,
    pub stream_url: Option<String>,
}

pub struct AppState {
    pub sender: Mutex<Option<SenderHandle>>,
    pub network: Mutex<HashMap<String, NetworkStreamHandle>>,
    pub receiver: Mutex<Option<ReceiverHandle>>,
    pub cached_devices: Mutex<Vec<NetworkDevice>>,
    pub cached_sources: Mutex<Vec<AudioSourceInfo>>,
    pub selected_connection_ids: Mutex<Vec<String>>,
    pub selected_source: Mutex<CaptureSource>,
    pub active_streams: Mutex<Vec<StartedStreamInfo>>,
    pub stream_starting: Mutex<bool>,
    pub stream_start_generation: AtomicU64,
    pub target_volumes: Mutex<HashMap<String, u8>>,
    pub muted: Mutex<bool>,
    pub hotkeys_enabled: Mutex<bool>,
    pub global_volume: Mutex<u8>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            sender: Mutex::new(None),
            network: Mutex::new(HashMap::new()),
            receiver: Mutex::new(None),
            cached_devices: Mutex::new(Vec::new()),
            cached_sources: Mutex::new(Vec::new()),
            selected_connection_ids: Mutex::new(Vec::new()),
            selected_source: Mutex::new(CaptureSource::System),
            active_streams: Mutex::new(Vec::new()),
            stream_starting: Mutex::new(false),
            stream_start_generation: AtomicU64::new(0),
            target_volumes: Mutex::new(HashMap::new()),
            muted: Mutex::new(false),
            hotkeys_enabled: Mutex::new(true),
            global_volume: Mutex::new(10),
        }
    }
}

impl AppState {
    pub fn next_start_generation(&self) -> u64 {
        self.stream_start_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current_start_generation(&self) -> u64 {
        self.stream_start_generation.load(Ordering::SeqCst)
    }
}
