use std::{
    collections::VecDeque,
    net::UdpSocket,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
};
use serde::Serialize;

use crate::network::rtp;

#[derive(Debug, Clone, Serialize)]
pub struct ReceiverInfo {
    pub listen_port: u16,
    pub codec: String,
    pub transport: String,
    pub output_device: String,
}

pub struct ReceiverHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _stream: Stream,
}

impl ReceiverHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ReceiverHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start(port: u16) -> Result<(ReceiverHandle, ReceiverInfo), String> {
    let socket = UdpSocket::bind(("0.0.0.0", port))
        .map_err(|err| format!("failed to bind UDP receiver socket: {err}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|err| format!("failed to configure UDP receiver socket: {err}"))?;

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default output audio device found".to_string())?;
    let output_device = device
        .name()
        .unwrap_or_else(|_| "Default output".to_string());
    let supported = device
        .default_output_config()
        .map_err(|err| format!("failed to read default output config: {err}"))?;

    let mut config: StreamConfig = supported.clone().into();
    config.channels = 2;
    config.sample_rate = cpal::SampleRate(48_000);

    let samples = Arc::new(Mutex::new(VecDeque::<i16>::with_capacity(48_000)));
    let playback_samples = Arc::clone(&samples);
    let err_fn = |err| eprintln!("audio output stream error: {err}");

    let stream = match supported.sample_format() {
        SampleFormat::I16 => device
            .build_output_stream(
                &config,
                move |data: &mut [i16], _| fill_i16(data, &playback_samples),
                err_fn,
                None,
            )
            .map_err(|err| format!("failed to build i16 output stream: {err}"))?,
        SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| fill_f32(data, &playback_samples),
                err_fn,
                None,
            )
            .map_err(|err| format!("failed to build f32 output stream: {err}"))?,
        SampleFormat::U16 => device
            .build_output_stream(
                &config,
                move |data: &mut [u16], _| fill_u16(data, &playback_samples),
                err_fn,
                None,
            )
            .map_err(|err| format!("failed to build u16 output stream: {err}"))?,
        other => return Err(format!("unsupported output sample format: {other:?}")),
    };
    stream
        .play()
        .map_err(|err| format!("failed to start output stream: {err}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread_samples = Arc::clone(&samples);
    let thread = thread::spawn(move || {
        let mut packet = vec![0u8; 2048];
        while !thread_stop.load(Ordering::Relaxed) {
            let Ok((len, _)) = socket.recv_from(&mut packet) else {
                continue;
            };
            let Some(payload) = rtp::payload(&packet[..len]) else {
                continue;
            };
            if payload.len() % 2 != 0 {
                continue;
            }
            if let Ok(mut queue) = thread_samples.lock() {
                for bytes in payload.chunks_exact(2) {
                    if queue.len() > 96_000 {
                        let _ = queue.pop_front();
                    }
                    queue.push_back(i16::from_le_bytes([bytes[0], bytes[1]]));
                }
            }
        }
    });

    Ok((
        ReceiverHandle {
            stop,
            thread: Some(thread),
            _stream: stream,
        },
        ReceiverInfo {
            listen_port: port,
            codec: "PCM S16LE stereo".to_string(),
            transport: "RTP/UDP dynamic payload 96".to_string(),
            output_device,
        },
    ))
}

fn fill_i16(data: &mut [i16], samples: &Arc<Mutex<VecDeque<i16>>>) {
    if let Ok(mut queue) = samples.lock() {
        for sample in data {
            *sample = queue.pop_front().unwrap_or_default();
        }
    }
}

fn fill_f32(data: &mut [f32], samples: &Arc<Mutex<VecDeque<i16>>>) {
    if let Ok(mut queue) = samples.lock() {
        for sample in data {
            *sample = queue.pop_front().unwrap_or_default() as f32 / i16::MAX as f32;
        }
    }
}

fn fill_u16(data: &mut [u16], samples: &Arc<Mutex<VecDeque<i16>>>) {
    if let Ok(mut queue) = samples.lock() {
        for sample in data {
            let signed = queue.pop_front().unwrap_or_default() as i32;
            *sample = (signed + 32768).clamp(0, u16::MAX as i32) as u16;
        }
    }
}
