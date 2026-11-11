use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

use serde::Serialize;

use crate::{
    audio::{pcm, wasapi_loopback},
    network::rtp,
};

const FRAMES_PER_PACKET: usize = 240;

#[derive(Debug, Clone, Serialize)]
pub struct SenderInfo {
    pub target: String,
    pub port: u16,
    pub codec: String,
    pub transport: String,
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct SenderHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SenderHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SenderHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start(target: String, port: u16) -> Result<(SenderHandle, SenderInfo), String> {
    let addr: SocketAddr = format!("{target}:{port}")
        .parse()
        .map_err(|err| format!("invalid target address: {err}"))?;
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|err| format!("failed to bind UDP sender socket: {err}"))?;
    socket
        .connect(addr)
        .map_err(|err| format!("failed to connect UDP sender socket: {err}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        let mut sequence = 0u16;
        let mut timestamp = 0u32;
        let ssrc = 0x4e415246;
        let mut pcm_bytes = Vec::<u8>::with_capacity(FRAMES_PER_PACKET * 4);

        let _ = wasapi_loopback::capture_loop(thread_stop, move |chunk| {
            let bytes =
                pcm::samples_to_stereo_i16_bytes(&chunk.samples, chunk.channels, chunk.sample_rate);
            for frame in bytes.chunks_exact(4) {
                pcm_bytes.extend_from_slice(frame);
                if pcm_bytes.len() >= FRAMES_PER_PACKET * 2 * std::mem::size_of::<i16>() {
                    let packet = rtp::build_packet(sequence, timestamp, ssrc, &pcm_bytes);
                    let _ = socket.send(&packet);
                    pcm_bytes.clear();
                    sequence = sequence.wrapping_add(1);
                    timestamp = timestamp.wrapping_add(FRAMES_PER_PACKET as u32);
                }
            }
        });
    });

    Ok((
        SenderHandle {
            stop,
            thread: Some(thread),
        },
        SenderInfo {
            target,
            port,
            codec: "PCM S16LE stereo".to_string(),
            transport: "RTP/UDP dynamic payload 96".to_string(),
            sample_rate: pcm::OUTPUT_SAMPLE_RATE,
            channels: 2,
        },
    ))
}
