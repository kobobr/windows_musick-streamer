use std::{
    io::{Read, Write},
    net::{IpAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use rust_cast::{
    channels::{
        media::{Media, StreamType},
        receiver::CastDeviceApp,
    },
    CastDevice,
};
use serde::Serialize;

use crate::{
    audio::{pcm, sources::CaptureSource, wasapi_loopback},
    network::discovery::ConnectionProfile,
};

type ClientSenders = Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>;

#[derive(Debug, Clone, Serialize)]
pub struct CastStreamInfo {
    pub renderer: String,
    pub stream_url: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct CastHandle {
    stop: Arc<AtomicBool>,
    volume: Arc<Mutex<f32>>,
    capture_thread: Option<JoinHandle<()>>,
    listener_thread: Option<JoinHandle<()>>,
    host: String,
    port: u16,
    app_session_id: Option<String>,
}

impl CastHandle {
    pub fn set_volume(&self, volume: f32) {
        if let Ok(mut current) = self.volume.lock() {
            *current = volume.clamp(0.0, 2.0);
        }
    }

    pub fn stop(mut self) {
        if let Some(session_id) = self.app_session_id.clone() {
            if let Ok(device) = CastDevice::connect_without_host_verification(&self.host, self.port)
            {
                let _ = device.receiver.stop_app(session_id);
            }
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.capture_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.listener_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for CastHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start_with_source(
    connection: &ConnectionProfile,
    codec: &str,
    volume: f32,
    source: CaptureSource,
) -> Result<(CastHandle, CastStreamInfo), String> {
    if codec != "pcm_s16le" {
        return Err(format!("Chromecast codec {codec} is not implemented yet"));
    }

    let host = connection.host.clone();
    let port = connection.port.unwrap_or(8009);
    let local_ip = local_ip_for(&host)?;
    let listener = TcpListener::bind(("0.0.0.0", 0))
        .map_err(|err| format!("failed to bind Cast HTTP stream server: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("failed to configure Cast HTTP stream server: {err}"))?;
    let http_port = listener
        .local_addr()
        .map_err(|err| format!("failed to read Cast HTTP stream server address: {err}"))?
        .port();
    let stream_url = format!("http://{local_ip}:{http_port}/stream.wav");

    let stop = Arc::new(AtomicBool::new(false));
    let volume = Arc::new(Mutex::new(volume.clamp(0.0, 2.0)));
    let clients: ClientSenders = Arc::new(Mutex::new(Vec::new()));

    let listener_stop = Arc::clone(&stop);
    let listener_clients = Arc::clone(&clients);
    let listener_thread = thread::spawn(move || {
        while !listener_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => spawn_http_client(stream, Arc::clone(&listener_clients)),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(40));
                }
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        }
    });

    let capture_stop = Arc::clone(&stop);
    let capture_clients = Arc::clone(&clients);
    let capture_volume = Arc::clone(&volume);
    let capture_thread = thread::spawn(move || {
        if let Err(err) = wasapi_loopback::capture_loop_for_source(capture_stop, source, move |chunk| {
            let gain = capture_volume.lock().map(|volume| *volume).unwrap_or(1.0);
            let bytes = pcm::samples_to_stereo_i16_bytes_with_gain(
                &chunk.samples,
                chunk.channels,
                chunk.sample_rate,
                gain,
            );
            if bytes.is_empty() {
                return;
            }
            if let Ok(mut senders) = capture_clients.lock() {
                senders.retain(|sender| sender.send(bytes.clone()).is_ok());
            }
        }) {
            eprintln!("Cast capture error: {err}");
        }
    });

    thread::sleep(Duration::from_millis(250));
    let app = CastDeviceApp::DefaultMediaReceiver;
    let device = CastDevice::connect_without_host_verification(&host, port)
        .map_err(|err| format!("failed to connect to Chromecast {host}:{port}: {err}"))?;
    let application = device
        .receiver
        .launch_app(&app)
        .map_err(|err| format!("failed to launch Cast media receiver: {err}"))?;
    let media = Media {
        content_id: stream_url.clone(),
        stream_type: StreamType::Live,
        content_type: "audio/wav".to_string(),
        metadata: None,
        duration: None,
    };
    let _status = device
        .media
        .load(
            application.transport_id.clone(),
            application.session_id.clone(),
            &media,
        )
        .map_err(|err| format!("failed to load Cast media stream: {err}"))?;

    Ok((
        CastHandle {
            stop,
            volume,
            capture_thread: Some(capture_thread),
            listener_thread: Some(listener_thread),
            host,
            port,
            app_session_id: Some(application.session_id),
        },
        CastStreamInfo {
            renderer: connection.name.clone(),
            stream_url,
            codec: "WAV PCM S16LE stereo".to_string(),
            sample_rate: pcm::OUTPUT_SAMPLE_RATE,
            channels: pcm::OUTPUT_CHANNELS,
        },
    ))
}

fn spawn_http_client(mut stream: TcpStream, clients: ClientSenders) {
    thread::spawn(move || {
        let mut request = [0u8; 1024];
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.read(&mut request);

        let response = "HTTP/1.1 200 OK\r\n\
                        Content-Type: audio/wav\r\n\
                        Cache-Control: no-cache\r\n\
                        Connection: close\r\n\
                        Access-Control-Allow-Origin: *\r\n\
                        \r\n";
        if stream.write_all(response.as_bytes()).is_err() {
            return;
        }
        if stream.write_all(&pcm::wav_stream_header()).is_err() {
            return;
        }

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        if let Ok(mut senders) = clients.lock() {
            senders.push(tx);
        }

        while let Ok(chunk) = rx.recv() {
            if stream.write_all(&chunk).is_err() {
                break;
            }
        }
    });
}

fn local_ip_for(remote_host: &str) -> Result<IpAddr, String> {
    let remote = (remote_host, 9)
        .to_socket_addrs()
        .map_err(|err| format!("failed to resolve Cast host {remote_host}: {err}"))?
        .next()
        .ok_or_else(|| format!("no address resolved for Cast host {remote_host}"))?;
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|err| format!("failed to bind local address probe socket: {err}"))?;
    socket
        .connect(remote)
        .map_err(|err| format!("failed to probe local route to Cast device: {err}"))?;
    Ok(socket
        .local_addr()
        .map_err(|err| format!("failed to read local route address: {err}"))?
        .ip())
}
