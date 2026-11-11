use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aes::Aes128;
use alac_encoder::{AlacEncoder, FormatDescription};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use cbc::cipher::{
    block_padding::NoPadding, generic_array::GenericArray, BlockEncrypt, BlockEncryptMut, KeyInit,
    KeyIvInit,
};
use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, Nonce,
};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use num_bigint::BigUint;
use num_traits::Zero;
use plist::{Dictionary as PlistDictionary, Value as PlistValue};
use rand::RngCore;
use rsa::{Oaep, RsaPublicKey};
use serde::Serialize;
use sha2::{Digest, Sha512};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use crate::{
    audio::{pcm, sources::CaptureSource, wasapi_loopback},
    network::discovery::ConnectionProfile,
};

const AIRPLAY_SAMPLE_RATE: u32 = 44_100;
const AIRPLAY_CHANNELS: u16 = 2;
const AIRPLAY_PAYLOAD_TYPE: u8 = 96;
const AIRPLAY_INITIAL_LATENCY_FRAMES: u32 = 66_150;
const AIRPLAY_PACKET_BACKLOG: usize = 512;
const AIRPORT_PUBLIC_KEY_MODULUS_B64: &str = "tYEChgTK2NQoUw+IRijq/K2yBrLUUKGuAeaZlbmINl0+QLbSQEvCyoCRiSzZ/u1pFLxryCQntbqzHYeR+0RxYkKewyKfAn0KRmGaOJoSYnwZ58qfNlvE9JXwgsTMwQSoWSKrFkaesKUIvw2F3G0zWf+E+1v3gfP05Dzf/RU+oOE=";
const AIRPORT_PUBLIC_KEY_EXPONENT_B64: &str = "AQAB";
const AIRPLAY_AUTH_SETUP_PUBLIC_KEY: [u8; 32] = [
    0x59, 0x02, 0xed, 0xe9, 0x0d, 0x4e, 0xf2, 0xbd, 0x4c, 0xb6, 0x8a, 0x63, 0x30, 0x03, 0x82, 0x07,
    0xa9, 0x4d, 0xbd, 0x50, 0xd8, 0xaa, 0x46, 0x5b, 0x5d, 0x8c, 0x01, 0x2a, 0x0c, 0x7e, 0x1d, 0x4e,
];
const AP2_TRANSIENT_PIN: &str = "3939";
const AP2_SRP_USERNAME: &str = "Pair-Setup";
const AP2_N_3072_LEN: usize = 384;
const AP2_MAX_RTSP_BLOCK: usize = 0x400;
const AP2_RTSP_TAG_LEN: usize = 16;
const AP2_RTP_NONCE_TRAIL_LEN: usize = 8;
const ENABLE_EXPERIMENTAL_AP2: bool = false;
const AP2_N_3072_HEX: &str = "\
FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B\
139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485\
B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1F\
E649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23\
DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32\
905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF69558\
17183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521\
ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D7\
1E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B1817\
7B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82\
D120A93AD2CAFFFFFFFFFFFFFFFF";

type Aes128CbcEnc = cbc::Encryptor<Aes128>;

#[derive(Debug, Clone, Serialize)]
pub struct AirplayStreamInfo {
    pub renderer: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct AirplayHandle {
    stop: Arc<AtomicBool>,
    volume: Arc<Mutex<f32>>,
    rtsp: Arc<Mutex<RtspClient>>,
    session_uri: String,
    client_instance: String,
    active_remote: String,
    rtsp_session: Option<String>,
    capture_thread: Option<JoinHandle<()>>,
    timing_thread: Option<JoinHandle<()>>,
    control_thread: Option<JoinHandle<()>>,
}

impl AirplayHandle {
    pub fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 2.0);
        if let Ok(mut current) = self.volume.lock() {
            *current = volume;
        }
        let body = format!("volume: {:.6}\r\n", airplay_volume_db(volume));
        if let Ok(mut rtsp) = self.rtsp.lock() {
            let mut headers = vec![
                ("X-Apple-Client-Name", "Network Audio Router"),
                ("Client-Instance", self.client_instance.as_str()),
                ("DACP-ID", self.client_instance.as_str()),
                ("Active-Remote", self.active_remote.as_str()),
            ];
            if let Some(session) = self.rtsp_session.as_deref() {
                headers.push(("Session", session));
            }
            headers.push(("Content-Type", "text/parameters"));
            let _ = rtsp.request(
                "SET_PARAMETER",
                &self.session_uri,
                &headers,
                body.as_bytes(),
            );
        }
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut rtsp) = self.rtsp.lock() {
            let mut headers = vec![
                ("X-Apple-Client-Name", "Network Audio Router"),
                ("Client-Instance", self.client_instance.as_str()),
                ("DACP-ID", self.client_instance.as_str()),
                ("Active-Remote", self.active_remote.as_str()),
            ];
            if let Some(session) = self.rtsp_session.as_deref() {
                headers.push(("Session", session));
            }
            let _ = rtsp.request("TEARDOWN", &self.session_uri, &headers, &[]);
        }
        if let Some(thread) = self.capture_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.timing_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.control_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AirplayHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start_with_source(
    connection: &ConnectionProfile,
    codec: &str,
    volume: f32,
    source: CaptureSource,
) -> Result<(AirplayHandle, AirplayStreamInfo), String> {
    if codec != "alac" {
        return Err(format!("AirPlay codec {codec} is not implemented yet"));
    }

    if is_airplay2_profile(connection) && !is_raop_profile(connection) {
        let raop_error = match start_airplay1(connection, volume, source.clone()) {
            Ok(session) => return Ok(session),
            Err(err) => err,
        };
        if ENABLE_EXPERIMENTAL_AP2 {
            return start_airplay2(connection, volume, source);
        }
        return Err(fairplay_probe_pending_message(
            connection,
            Some(&raop_error),
        ));
    }

    start_airplay1(connection, volume, source)
}

fn is_raop_profile(connection: &ConnectionProfile) -> bool {
    let id = connection.id.to_ascii_lowercase();
    let detail = connection.detail.to_ascii_lowercase();
    id.contains(":raop:") || detail.contains("raop")
}

fn start_airplay1(
    connection: &ConnectionProfile,
    volume: f32,
    source: CaptureSource,
) -> Result<(AirplayHandle, AirplayStreamInfo), String> {
    let variants = airplay_announce_variants(connection);
    let mut errors = Vec::new();

    for variant in variants {
        match try_start_airplay1(connection, volume, variant, source.clone()) {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                errors.push(format!("{}: {err}", variant.label()));
                if should_stop_after_airplay_error(&err) {
                    break;
                }
            }
        }
    }

    Err(format!(
        "AirPlay start failed for all RAOP variants: {}",
        errors.join(" | ")
    ))
}

fn should_stop_after_airplay_error(err: &str) -> bool {
    let err = err.to_ascii_lowercase();
    err.contains("10060")
        || err.contains("timed out")
        || err.contains("failed while reading response")
}

fn try_start_airplay1(
    connection: &ConnectionProfile,
    volume: f32,
    variant: AnnounceVariant,
    source: CaptureSource,
) -> Result<(AirplayHandle, AirplayStreamInfo), String> {
    let host = connection.host.clone();
    let port = connection.port.unwrap_or(5000);
    let remote_addr = resolve_socket_addr(&host, port)?;
    let local_ip = local_ip_for(&host)?;

    let control_socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|err| format!("failed to bind AirPlay control socket: {err}"))?;
    let timing_socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|err| format!("failed to bind AirPlay timing socket: {err}"))?;
    let data_socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|err| format!("failed to bind AirPlay RTP socket: {err}"))?;
    let control_port = control_socket
        .local_addr()
        .map_err(|err| format!("failed to read AirPlay control port: {err}"))?
        .port();
    let timing_port = timing_socket
        .local_addr()
        .map_err(|err| format!("failed to read AirPlay timing port: {err}"))?
        .port();

    let stop = Arc::new(AtomicBool::new(false));
    let timing_peer = Arc::new(Mutex::new(None));
    let control_peer = Arc::new(Mutex::new(None));
    let packet_backlog: PacketBacklog = Arc::new(Mutex::new(VecDeque::new()));
    let start_sequence = rand::thread_rng().next_u32() as u16;
    let start_timestamp = AIRPLAY_INITIAL_LATENCY_FRAMES;
    let ssrc = rand::thread_rng().next_u32();
    let sync_timestamp = Arc::new(AtomicU32::new(start_timestamp));
    let timing_stop = Arc::clone(&stop);
    let timing_thread = spawn_timing_responder(
        timing_socket,
        Arc::clone(&timing_peer),
        Arc::clone(&timing_stop),
    );
    let control_stop = Arc::clone(&stop);
    let control_thread = spawn_control_loop(
        control_socket,
        Arc::clone(&control_peer),
        Arc::clone(&sync_timestamp),
        Arc::clone(&packet_backlog),
        control_stop,
    );

    let mut session =
        match try_announce_airplay_session(&host, port, &local_ip, &remote_addr.ip(), variant) {
            Ok(session) => session,
            Err(err) => {
                stop.store(true, Ordering::Relaxed);
                let _ = timing_thread.join();
                let _ = control_thread.join();
                return Err(err);
            }
        };

    let transport = format!(
        "RTP/AVP/UDP;unicast;interleaved=0-1;mode=record;control_port={control_port};timing_port={timing_port}"
    );
    let setup = session.rtsp.request(
        "SETUP",
        &session.session_uri,
        &[
            ("X-Apple-Client-Name", "Network Audio Router"),
            ("Client-Instance", &session.client_instance),
            ("DACP-ID", &session.client_instance),
            ("Active-Remote", &session.active_remote),
            ("Transport", &transport),
        ],
        &[],
    );
    let setup = match setup {
        Ok(setup) => setup,
        Err(err) => {
            let _ = session.rtsp.request_no_response(
                "TEARDOWN",
                &session.session_uri,
                &[
                    ("X-Apple-Client-Name", "Network Audio Router"),
                    ("Client-Instance", &session.client_instance),
                    ("DACP-ID", &session.client_instance),
                    ("Active-Remote", &session.active_remote),
                ],
                &[],
            );
            stop.store(true, Ordering::Relaxed);
            let _ = timing_thread.join();
            let _ = control_thread.join();
            return Err(err);
        }
    };
    let transport_response = setup
        .header("transport")
        .ok_or_else(|| "AirPlay SETUP response did not include Transport".to_string())?;
    let rtsp_session = setup.header("session");
    let data_port = transport_port(&transport_response, "server_port").ok_or_else(|| {
        format!("AirPlay SETUP response did not include server_port: {transport_response}")
    })?;
    let remote_control_port = transport_port(&transport_response, "control_port");
    let remote_timing_port = transport_port(&transport_response, "timing_port");
    if let Some(port) = remote_timing_port.filter(|port| *port > 0) {
        if let Ok(mut peer) = timing_peer.lock() {
            *peer = Some(SocketAddr::new(remote_addr.ip(), port));
        }
    }
    if let Some(port) = remote_control_port.filter(|port| *port > 0) {
        if let Ok(mut peer) = control_peer.lock() {
            *peer = Some(SocketAddr::new(remote_addr.ip(), port));
        }
    }

    if let Err(err) = data_socket.connect(SocketAddr::new(remote_addr.ip(), data_port)) {
        stop.store(true, Ordering::Relaxed);
        let _ = timing_thread.join();
        let _ = control_thread.join();
        return Err(format!("failed to connect AirPlay RTP socket: {err}"));
    }

    let mut record_headers = vec![
        ("X-Apple-Client-Name", "Network Audio Router"),
        ("Client-Instance", session.client_instance.as_str()),
        ("DACP-ID", session.client_instance.as_str()),
        ("Active-Remote", session.active_remote.as_str()),
        ("Range", "npt=0-"),
    ];
    if let Some(rtsp_session) = rtsp_session.as_deref() {
        record_headers.push(("Session", rtsp_session));
    }
    let rtp_info = format!("seq={start_sequence};rtptime={start_timestamp}");
    record_headers.push(("RTP-Info", rtp_info.as_str()));
    if let Err(err) = session
        .rtsp
        .request("RECORD", &session.session_uri, &record_headers, &[])
    {
        let mut teardown_headers = vec![
            ("X-Apple-Client-Name", "Network Audio Router"),
            ("Client-Instance", session.client_instance.as_str()),
            ("DACP-ID", session.client_instance.as_str()),
            ("Active-Remote", session.active_remote.as_str()),
        ];
        if let Some(rtsp_session) = rtsp_session.as_deref() {
            teardown_headers.push(("Session", rtsp_session));
        }
        let _ = session.rtsp.request_no_response(
            "TEARDOWN",
            &session.session_uri,
            &teardown_headers,
            &[],
        );
        stop.store(true, Ordering::Relaxed);
        let _ = timing_thread.join();
        let _ = control_thread.join();
        return Err(err);
    }
    let mut flush_headers = vec![
        ("X-Apple-Client-Name", "Network Audio Router"),
        ("Client-Instance", session.client_instance.as_str()),
        ("DACP-ID", session.client_instance.as_str()),
        ("Active-Remote", session.active_remote.as_str()),
        ("Range", "npt=0-"),
    ];
    if let Some(rtsp_session) = rtsp_session.as_deref() {
        flush_headers.push(("Session", rtsp_session));
    }
    flush_headers.push(("RTP-Info", rtp_info.as_str()));
    let _ = session
        .rtsp
        .request("FLUSH", &session.session_uri, &flush_headers, &[]);
    let body = format!("volume: {:.6}\r\n", airplay_volume_db(volume));
    let mut volume_headers = vec![
        ("X-Apple-Client-Name", "Network Audio Router"),
        ("Client-Instance", session.client_instance.as_str()),
        ("DACP-ID", session.client_instance.as_str()),
        ("Active-Remote", session.active_remote.as_str()),
    ];
    if let Some(rtsp_session) = rtsp_session.as_deref() {
        volume_headers.push(("Session", rtsp_session));
    }
    volume_headers.push(("Content-Type", "text/parameters"));
    let _ = session.rtsp.request(
        "SET_PARAMETER",
        &session.session_uri,
        &volume_headers,
        body.as_bytes(),
    );

    let volume = Arc::new(Mutex::new(volume.clamp(0.0, 2.0)));
    let rtsp = Arc::new(Mutex::new(session.rtsp));

    let capture_stop = Arc::clone(&stop);
    let capture_volume = Arc::clone(&volume);
    let aes_key = session.aes_key;
    let aes_iv = session.aes_iv;
    let frame_samples = session.frame_samples;
    let capture_thread = thread::spawn(move || {
        let runtime = RaopAudioRuntime {
            socket: data_socket,
            aes_key,
            aes_iv,
            frame_samples,
            start_sequence,
            start_timestamp,
            ssrc,
            packet_backlog,
            sync_timestamp,
        };
        if let Err(err) = stream_alac_rtp(runtime, capture_stop, capture_volume, source) {
            eprintln!("AirPlay capture error: {err}");
        }
    });

    Ok((
        AirplayHandle {
            stop,
            volume,
            rtsp,
            session_uri: session.session_uri,
            client_instance: session.client_instance,
            active_remote: session.active_remote,
            rtsp_session,
            capture_thread: Some(capture_thread),
            timing_thread: Some(timing_thread),
            control_thread: Some(control_thread),
        },
        AirplayStreamInfo {
            renderer: connection.name.clone(),
            codec: "ALAC 44.1 kHz stereo".to_string(),
            sample_rate: AIRPLAY_SAMPLE_RATE,
            channels: AIRPLAY_CHANNELS,
        },
    ))
}

struct AirplaySession {
    rtsp: RtspClient,
    session_uri: String,
    client_instance: String,
    active_remote: String,
    aes_key: Option<[u8; 16]>,
    aes_iv: Option<[u8; 16]>,
    frame_samples: usize,
}

fn is_airplay2_profile(connection: &ConnectionProfile) -> bool {
    let detail = connection.detail.to_ascii_lowercase();
    detail.contains("airplay 2")
        || detail.contains("homekit")
        || detail.contains("fairplay")
        || detail.contains("pk=present")
}

fn fairplay_probe_pending_message(
    connection: &ConnectionProfile,
    raop_error: Option<&str>,
) -> String {
    let raop_prefix = raop_error
        .map(|err| format!("RAOP-compatible path failed first: {err}\n"))
        .unwrap_or_default();
    match fairplay_m1_probe(connection) {
        Ok(probe) => format!(
            "{raop_prefix}AirPlay 2/FairPlay handshake reached /fp-setup M1: mode {}, RTSP {} {}, {} byte FPLY response. Audio is still blocked because FairPlay fpaeskey/ekey generation is not implemented yet.",
            probe.mode, probe.status_code, probe.reason, probe.body_len
        ),
        Err(err) => format!(
            "{raop_prefix}AirPlay 2/FairPlay sender backend is pending for this device; FairPlay /fp-setup probe did not complete: {err}"
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FairplayProbe {
    mode: u8,
    status_code: u16,
    reason: String,
    body_len: usize,
}

fn fairplay_m1_probe(connection: &ConnectionProfile) -> Result<FairplayProbe, String> {
    let host = connection.host.clone();
    let port = connection.port.unwrap_or(7000);
    let mut failures = Vec::new();

    for mode in [0_u8, 1, 2, 3] {
        let mut rtsp = match RtspClient::connect(&host, port) {
            Ok(client) => client,
            Err(err) => {
                failures.push(format!("mode {mode}: {err}"));
                continue;
            }
        };
        let _ = rtsp.set_timeouts(Duration::from_secs(2), Duration::from_secs(2));
        rtsp.set_user_agent("AirPlay/750.3.1 NARfW/0.1");

        match rtsp.request_raw(
            "POST",
            "/fp-setup",
            &[("Content-Type", "application/octet-stream")],
            &fairplay_m1_body(mode),
        ) {
            Ok(response) if is_fairplay_m1_response(&response) => {
                return Ok(FairplayProbe {
                    mode,
                    status_code: response.status_code,
                    reason: response.reason,
                    body_len: response.body.len(),
                });
            }
            Ok(response) => failures.push(format!(
                "mode {mode}: RTSP {} {}{}",
                response.status_code,
                response.reason,
                response.diagnostic_suffix()
            )),
            Err(err) => failures.push(format!("mode {mode}: {err}")),
        }
    }

    Err(failures.join(" | "))
}

fn fairplay_m1_body(mode: u8) -> [u8; 16] {
    let mut body = [0_u8; 16];
    body[4] = 0x03;
    body[14] = mode;
    body
}

fn is_fairplay_m1_response(response: &RtspResponse) -> bool {
    (200..300).contains(&response.status_code)
        && response.body.len() == 142
        && response.body.starts_with(b"FPLY")
}

fn start_airplay2(
    connection: &ConnectionProfile,
    volume: f32,
    source: CaptureSource,
) -> Result<(AirplayHandle, AirplayStreamInfo), String> {
    let host = connection.host.clone();
    let port = connection.port.unwrap_or(7000);
    let remote_addr = resolve_socket_addr(&host, port)?;
    let mut rtsp = RtspClient::connect(&host, port)?;
    rtsp.set_user_agent("AirPlay/750.3.1 NARfW/0.1");
    let session_uri = format!("rtsp://{host}:{port}/{}", random_numeric_id());
    let session_key = ap2_transient_pair_setup(&mut rtsp)
        .map_err(|err| format!("AirPlay 2 pair-setup failed: {err}"))?;
    rtsp.enable_ap2_control_cipher(&session_key)?;

    let mut initial_setup = PlistDictionary::new();
    initial_setup.insert("timingProtocol".into(), PlistValue::String("PTP".into()));
    initial_setup.insert("isRemoteControlOnly".into(), PlistValue::Boolean(false));
    let initial_response = ap2_plist_request(&mut rtsp, "SETUP", &session_uri, initial_setup)
        .map_err(|err| format!("AirPlay 2 initial SETUP failed: {err}"))?;
    let _event_port = initial_response
        .get("eventPort")
        .and_then(|value| value.as_unsigned_integer())
        .unwrap_or(0);

    let mut shk = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut shk);
    let mut stream = PlistDictionary::new();
    stream.insert("type".into(), PlistValue::Integer(96.into()));
    stream.insert(
        "audioFormat".into(),
        PlistValue::Integer(0x40000_u64.into()),
    );
    stream.insert(
        "sr".into(),
        PlistValue::Integer((AIRPLAY_SAMPLE_RATE as u64).into()),
    );
    stream.insert("spf".into(), PlistValue::Integer(352_u64.into()));
    stream.insert("shk".into(), PlistValue::Data(shk.to_vec()));

    let mut stream_setup = PlistDictionary::new();
    stream_setup.insert(
        "streams".into(),
        PlistValue::Array(vec![PlistValue::Dictionary(stream)]),
    );
    let stream_response = ap2_plist_request(&mut rtsp, "SETUP", &session_uri, stream_setup)
        .map_err(|err| format!("AirPlay 2 audio SETUP failed: {err}"))?;
    let (data_port, control_port) = ap2_stream_ports(&stream_response)?;

    let data_socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|err| format!("failed to bind AirPlay 2 RTP socket: {err}"))?;
    data_socket
        .connect(SocketAddr::new(remote_addr.ip(), data_port))
        .map_err(|err| format!("failed to connect AirPlay 2 RTP socket: {err}"))?;

    let mut record = PlistDictionary::new();
    record.insert("streamID".into(), PlistValue::Integer(1_u64.into()));
    record.insert("rtpTime".into(), PlistValue::Integer(0_u64.into()));
    record.insert("rate".into(), PlistValue::Real(1.0));
    let _ = ap2_plist_request(&mut rtsp, "RECORD", &session_uri, record).or_else(|_| {
        rtsp.request("RECORD", &session_uri, &[("Range", "npt=0-")], &[])
            .map(|_| PlistDictionary::new())
    });

    let body = format!("volume: {:.6}\r\n", airplay_volume_db(volume));
    let _ = rtsp.request(
        "SET_PARAMETER",
        &session_uri,
        &[("Content-Type", "text/parameters")],
        body.as_bytes(),
    );

    let stop = Arc::new(AtomicBool::new(false));
    let volume = Arc::new(Mutex::new(volume.clamp(0.0, 2.0)));
    let rtsp = Arc::new(Mutex::new(rtsp));
    let capture_stop = Arc::clone(&stop);
    let capture_volume = Arc::clone(&volume);
    let capture_thread = thread::spawn(move || {
        if let Err(err) = stream_ap2_alac_rtp(data_socket, shk, 352, capture_stop, capture_volume, source) {
            eprintln!("AirPlay 2 capture error: {err}");
        }
    });

    if let Some(port) = control_port {
        let _ = send_sync_hint(remote_addr.ip(), port);
    }
    let client_instance = random_hex(8);
    let active_remote = (rand::thread_rng().next_u32() & 0x7fff_ffff).to_string();

    Ok((
        AirplayHandle {
            stop,
            volume,
            rtsp,
            session_uri,
            client_instance,
            active_remote,
            rtsp_session: None,
            capture_thread: Some(capture_thread),
            timing_thread: None,
            control_thread: None,
        },
        AirplayStreamInfo {
            renderer: connection.name.clone(),
            codec: "ALAC 44.1 kHz stereo over AirPlay 2".to_string(),
            sample_rate: AIRPLAY_SAMPLE_RATE,
            channels: AIRPLAY_CHANNELS,
        },
    ))
}

fn ap2_plist_request(
    rtsp: &mut RtspClient,
    method: &str,
    uri: &str,
    dictionary: PlistDictionary,
) -> Result<PlistDictionary, String> {
    let mut body = Vec::new();
    plist::to_writer_binary(&mut body, &PlistValue::Dictionary(dictionary))
        .map_err(|err| format!("failed to encode AirPlay 2 plist: {err}"))?;
    let response = rtsp.request(
        method,
        uri,
        &[("Content-Type", "application/x-apple-binary-plist")],
        &body,
    )?;
    if response.body.is_empty() {
        return Ok(PlistDictionary::new());
    }
    let value: PlistValue = plist::from_bytes(&response.body)
        .map_err(|err| format!("failed to decode AirPlay 2 plist response: {err}"))?;
    value
        .into_dictionary()
        .ok_or_else(|| "AirPlay 2 plist response was not a dictionary".to_string())
}

fn ap2_stream_ports(response: &PlistDictionary) -> Result<(u16, Option<u16>), String> {
    let stream = response
        .get("streams")
        .and_then(|value| value.as_array())
        .and_then(|streams| streams.first())
        .and_then(|value| value.as_dictionary())
        .ok_or_else(|| "AirPlay 2 audio SETUP response did not include streams".to_string())?;
    let data_port = stream
        .get("dataPort")
        .and_then(|value| value.as_unsigned_integer())
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| "AirPlay 2 audio SETUP response did not include dataPort".to_string())?;
    let control_port = stream
        .get("controlPort")
        .and_then(|value| value.as_unsigned_integer())
        .and_then(|value| u16::try_from(value).ok());
    Ok((data_port, control_port))
}

fn stream_ap2_alac_rtp(
    socket: UdpSocket,
    shk: [u8; 32],
    frame_samples: usize,
    stop: Arc<AtomicBool>,
    volume: Arc<Mutex<f32>>,
    source: CaptureSource,
) -> Result<(), String> {
    let input_format = FormatDescription::pcm::<i16>(AIRPLAY_SAMPLE_RATE as f64, 2);
    let output_format =
        FormatDescription::alac(AIRPLAY_SAMPLE_RATE as f64, frame_samples as u32, 2);
    let mut encoder = AlacEncoder::new(&output_format);
    let cipher = ChaCha20Poly1305::new((&shk).into());
    let mut alac_output = vec![0u8; output_format.max_packet_size()];
    let mut pending_pcm = Vec::<u8>::new();
    let mut sequence = rand::thread_rng().next_u32() as u16;
    let mut timestamp = rand::thread_rng().next_u32();
    let ssrc = rand::thread_rng().next_u32();
    let mut nonce_counter = 0u64;
    let frame_bytes = frame_samples * AIRPLAY_CHANNELS as usize * 2;

    wasapi_loopback::capture_loop_for_source(stop, source, move |chunk| {
        let gain = volume.lock().map(|volume| *volume).unwrap_or(1.0);
        let bytes = pcm::samples_to_stereo_i16_bytes_at_sample_rate_with_gain(
            &chunk.samples,
            chunk.channels,
            chunk.sample_rate,
            AIRPLAY_SAMPLE_RATE,
            gain,
        );
        pending_pcm.extend_from_slice(&bytes);

        while pending_pcm.len() >= frame_bytes {
            let frame = pending_pcm.drain(..frame_bytes).collect::<Vec<_>>();
            let size = encoder.encode(&input_format, &frame, &mut alac_output);
            if let Ok(packet) = ap2_rtp_packet(
                &cipher,
                sequence,
                timestamp,
                ssrc,
                &alac_output[..size],
                nonce_counter,
            ) {
                let _ = socket.send(&packet);
            }
            sequence = sequence.wrapping_add(1);
            timestamp = timestamp.wrapping_add(frame_samples as u32);
            nonce_counter = nonce_counter.wrapping_add(1);
        }
    })
}

fn ap2_rtp_packet(
    cipher: &ChaCha20Poly1305,
    sequence: u16,
    timestamp: u32,
    ssrc: u32,
    payload: &[u8],
    nonce_counter: u64,
) -> Result<Vec<u8>, String> {
    let mut header = Vec::with_capacity(12);
    header.push(0x80);
    header.push(AIRPLAY_PAYLOAD_TYPE);
    header.extend_from_slice(&sequence.to_be_bytes());
    header.extend_from_slice(&timestamp.to_be_bytes());
    header.extend_from_slice(&ssrc.to_be_bytes());

    let nonce_tail = nonce_counter.to_le_bytes();
    let mut nonce = [0u8; 12];
    nonce[4..12].copy_from_slice(&nonce_tail);
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: payload,
                aad: &header[4..12],
            },
        )
        .map_err(|_| "failed to encrypt AirPlay 2 RTP payload".to_string())?;

    let mut packet = Vec::with_capacity(header.len() + ciphertext.len() + AP2_RTP_NONCE_TRAIL_LEN);
    packet.extend_from_slice(&header);
    packet.extend_from_slice(&ciphertext);
    packet.extend_from_slice(&nonce_tail);
    Ok(packet)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct AnnounceVariant {
    frame_samples: usize,
    encrypted: bool,
    paired: bool,
    auth_setup: bool,
    session_uri_kind: SessionUriKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionUriKind {
    Receiver,
    Local,
}

impl AnnounceVariant {
    fn label(self) -> String {
        format!(
            "ALAC {} {}{}{}{}",
            self.frame_samples,
            if self.encrypted { "RSA-AES" } else { "clear" },
            if self.auth_setup { " auth-setup" } else { "" },
            if self.paired { " paired" } else { "" },
            match self.session_uri_kind {
                SessionUriKind::Receiver => " receiver-uri",
                SessionUriKind::Local => " local-uri",
            }
        )
    }
}

fn airplay_announce_variants(connection: &ConnectionProfile) -> Vec<AnnounceVariant> {
    let auth_setup = airplay_supports_auth_setup(connection);
    let encrypted = airplay_requires_legacy_encryption(connection);
    let mut variants = Vec::new();

    push_announce_variant(&mut variants, encrypted, auth_setup, SessionUriKind::Local);
    push_announce_variant(
        &mut variants,
        encrypted,
        auth_setup,
        SessionUriKind::Receiver,
    );

    if encrypted {
        push_announce_variant(&mut variants, false, auth_setup, SessionUriKind::Local);
    } else if auth_setup {
        push_announce_variant(&mut variants, false, false, SessionUriKind::Local);
    }

    variants
}

fn push_announce_variant(
    variants: &mut Vec<AnnounceVariant>,
    encrypted: bool,
    auth_setup: bool,
    session_uri_kind: SessionUriKind,
) {
    let variant = AnnounceVariant {
        frame_samples: 352,
        encrypted,
        paired: false,
        auth_setup,
        session_uri_kind,
    };
    if !variants.contains(&variant) {
        variants.push(variant);
    }
}

fn airplay_supports_auth_setup(connection: &ConnectionProfile) -> bool {
    airplay_detail_has_csv_value(&connection.detail, "et", "4")
}

fn airplay_requires_legacy_encryption(connection: &ConnectionProfile) -> bool {
    airplay_detail_has_csv_value(&connection.detail, "ek", "1")
}

fn airplay_detail_has_csv_value(detail: &str, key: &str, wanted: &str) -> bool {
    let prefix = format!("{}=", key.to_ascii_lowercase());
    detail
        .to_ascii_lowercase()
        .split_whitespace()
        .filter_map(|part| part.strip_prefix(&prefix))
        .flat_map(|value| value.split(','))
        .any(|value| value.trim() == wanted)
}

fn try_announce_airplay_session(
    host: &str,
    port: u16,
    local_ip: &IpAddr,
    remote_ip: &IpAddr,
    variant: AnnounceVariant,
) -> Result<AirplaySession, String> {
    let session_id = random_numeric_id();
    let session_uri = match variant.session_uri_kind {
        SessionUriKind::Receiver => format!("rtsp://{remote_ip}/{session_id}"),
        SessionUriKind::Local => format!("rtsp://{local_ip}/{session_id}"),
    };
    let client_instance = random_hex(8);
    let active_remote = (rand::thread_rng().next_u32() & 0x7fff_ffff).to_string();
    let mut rtsp = RtspClient::connect(host, port)?;
    rtsp.set_user_agent("iTunes/7.6.2 (Windows; N;)");
    rtsp.set_include_host_header(false);
    rtsp.set_cseq(1);
    let (aes_key, aes_iv, rsa_aes_key, aes_iv_b64) = if variant.encrypted {
        let aes_key = random_bytes_16();
        let aes_iv = random_bytes_16();
        (
            Some(aes_key),
            Some(aes_iv),
            Some(encrypt_airplay_aes_key(&aes_key)?),
            Some(BASE64.encode(aes_iv)),
        )
    } else {
        (None, None, None, None)
    };
    let sdp = airplay_sdp(
        local_ip,
        remote_ip,
        &session_id,
        variant.frame_samples,
        rsa_aes_key.as_deref(),
        aes_iv_b64.as_deref(),
    );

    let options = rtsp.request(
        "OPTIONS",
        "*",
        &[
            ("X-Apple-Client-Name", "Network Audio Router"),
            ("Client-Instance", &client_instance),
            ("DACP-ID", &client_instance),
            ("Active-Remote", &active_remote),
        ],
        &[],
    )?;
    if options.status_code == 401 || options.status_code == 403 {
        return Err("device requires authentication or pairing".to_string());
    }
    if variant.auth_setup {
        airplay_auth_setup(&mut rtsp, &client_instance, &active_remote)?;
    }
    if variant.paired {
        legacy_pair_verify(&mut rtsp)?;
    }

    let announce_challenge = random_challenge_unpadded();
    let mut announce_headers = vec![
        ("X-Apple-Client-Name", "Network Audio Router"),
        ("Client-Instance", client_instance.as_str()),
        ("DACP-ID", client_instance.as_str()),
        ("Active-Remote", active_remote.as_str()),
        ("Content-Type", "application/sdp"),
    ];
    if variant.encrypted {
        announce_headers.push(("Apple-Challenge", announce_challenge.as_str()));
    }
    let response = rtsp.request_raw("ANNOUNCE", &session_uri, &announce_headers, sdp.as_bytes())?;
    if !(200..300).contains(&response.status_code) {
        return Err(format!(
            "RTSP {} {}{}",
            response.status_code,
            response.reason,
            response.diagnostic_suffix()
        ));
    }

    Ok(AirplaySession {
        rtsp,
        session_uri,
        client_instance,
        active_remote,
        aes_key,
        aes_iv,
        frame_samples: variant.frame_samples,
    })
}

fn airplay_auth_setup(
    rtsp: &mut RtspClient,
    client_instance: &str,
    active_remote: &str,
) -> Result<(), String> {
    let body = airplay_auth_setup_body();
    let response = rtsp.request(
        "POST",
        "/auth-setup",
        &[
            ("X-Apple-Client-Name", "Network Audio Router"),
            ("Client-Instance", client_instance),
            ("DACP-ID", client_instance),
            ("Active-Remote", active_remote),
            ("Content-Type", "application/octet-stream"),
        ],
        &body,
    )?;
    if !(200..300).contains(&response.status_code) {
        return Err(format!(
            "AirPlay auth-setup failed: RTSP {} {}{}",
            response.status_code,
            response.reason,
            response.diagnostic_suffix()
        ));
    }
    Ok(())
}

fn airplay_auth_setup_body() -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + AIRPLAY_AUTH_SETUP_PUBLIC_KEY.len());
    body.push(0x01);
    body.extend_from_slice(&AIRPLAY_AUTH_SETUP_PUBLIC_KEY);
    body
}

fn legacy_pair_verify(rtsp: &mut RtspClient) -> Result<(), String> {
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let client_ed_public = signing_key.verifying_key().to_bytes();

    let pair_setup_response = rtsp.request(
        "POST",
        "/pair-setup",
        &[("Content-Type", "application/octet-stream")],
        &client_ed_public,
    )?;
    let server_ed_public: [u8; 32] =
        pair_setup_response
            .body
            .as_slice()
            .try_into()
            .map_err(|_| {
                format!(
                    "AirPlay pair-setup returned {} bytes, expected 32",
                    pair_setup_response.body.len()
                )
            })?;

    let client_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
    let client_x_public = X25519PublicKey::from(&client_secret);
    let mut m1 = Vec::with_capacity(68);
    m1.extend_from_slice(&[1, 0, 0, 0]);
    m1.extend_from_slice(client_x_public.as_bytes());
    m1.extend_from_slice(&client_ed_public);

    let pair_verify_m2 = rtsp.request(
        "POST",
        "/pair-verify",
        &[("Content-Type", "application/octet-stream")],
        &m1,
    )?;
    if pair_verify_m2.body.len() != 96 {
        return Err(format!(
            "AirPlay pair-verify M2 returned {} bytes, expected 96",
            pair_verify_m2.body.len()
        ));
    }
    let server_x_public: [u8; 32] = pair_verify_m2.body[..32]
        .try_into()
        .map_err(|_| "invalid AirPlay pair-verify public key".to_string())?;
    let encrypted_server_signature: [u8; 64] = pair_verify_m2.body[32..96]
        .try_into()
        .map_err(|_| "invalid AirPlay pair-verify signature".to_string())?;
    let server_x_public = X25519PublicKey::from(server_x_public);
    let shared = client_secret.diffie_hellman(&server_x_public);
    let (aes_key, aes_iv) = pair_verify_aes(shared.as_bytes());

    let mut server_signature = encrypted_server_signature;
    AesCtrCompat::new(&aes_key, &aes_iv).encrypt(&mut server_signature);
    let mut server_message = Vec::with_capacity(64);
    server_message.extend_from_slice(server_x_public.as_bytes());
    server_message.extend_from_slice(client_x_public.as_bytes());
    let server_verifying_key = VerifyingKey::from_bytes(&server_ed_public)
        .map_err(|err| format!("invalid AirPlay pair-setup public key: {err}"))?;
    server_verifying_key
        .verify(
            &server_message,
            &ed25519_dalek::Signature::from_bytes(&server_signature),
        )
        .map_err(|err| format!("AirPlay pair-verify server signature failed: {err}"))?;

    let mut client_message = Vec::with_capacity(64);
    client_message.extend_from_slice(client_x_public.as_bytes());
    client_message.extend_from_slice(server_x_public.as_bytes());
    let mut client_signature = signing_key.sign(&client_message).to_bytes();
    let mut aes = AesCtrCompat::new(&aes_key, &aes_iv);
    let mut dummy = [0u8; 64];
    aes.encrypt(&mut dummy);
    aes.encrypt(&mut client_signature);

    let mut m3 = Vec::with_capacity(68);
    m3.extend_from_slice(&[0, 0, 0, 0]);
    m3.extend_from_slice(&client_signature);
    rtsp.request(
        "POST",
        "/pair-verify",
        &[("Content-Type", "application/octet-stream")],
        &m3,
    )?;

    Ok(())
}

fn pair_verify_aes(shared_secret: &[u8; 32]) -> ([u8; 16], [u8; 16]) {
    let mut key_hash = Sha512::new();
    key_hash.update(b"Pair-Verify-AES-Key");
    key_hash.update(shared_secret);
    let key_hash = key_hash.finalize();
    let mut iv_hash = Sha512::new();
    iv_hash.update(b"Pair-Verify-AES-IV");
    iv_hash.update(shared_secret);
    let iv_hash = iv_hash.finalize();

    let mut key = [0u8; 16];
    let mut iv = [0u8; 16];
    key.copy_from_slice(&key_hash[..16]);
    iv.copy_from_slice(&iv_hash[..16]);
    (key, iv)
}

struct AesCtrCompat {
    cipher: Aes128,
    counter: [u8; 16],
    state: [u8; 16],
    available: usize,
}

impl AesCtrCompat {
    fn new(key: &[u8; 16], iv: &[u8; 16]) -> Self {
        Self {
            cipher: Aes128::new(key.into()),
            counter: *iv,
            state: [0u8; 16],
            available: 0,
        }
    }

    fn encrypt(&mut self, data: &mut [u8]) {
        let mut index = 0;
        while index < data.len() {
            if self.available == 0 {
                let mut block = GenericArray::clone_from_slice(&self.counter);
                self.cipher.encrypt_block(&mut block);
                self.state.copy_from_slice(&block);
                self.available = 16;
                self.increment_counter();
            }

            let offset = 16 - self.available;
            let count = self.available.min(data.len() - index);
            for state_index in offset..offset + count {
                data[index] ^= self.state[state_index];
                index += 1;
            }
            self.available -= count;
        }
    }

    fn increment_counter(&mut self) {
        let mut carry = 1u16;
        for byte in self.counter.iter_mut().rev() {
            carry += *byte as u16;
            *byte = carry as u8;
            carry >>= 8;
        }
    }
}

struct RaopAudioRuntime {
    socket: UdpSocket,
    aes_key: Option<[u8; 16]>,
    aes_iv: Option<[u8; 16]>,
    frame_samples: usize,
    start_sequence: u16,
    start_timestamp: u32,
    ssrc: u32,
    packet_backlog: PacketBacklog,
    sync_timestamp: Arc<AtomicU32>,
}

fn stream_alac_rtp(
    runtime: RaopAudioRuntime,
    stop: Arc<AtomicBool>,
    volume: Arc<Mutex<f32>>,
    source: CaptureSource,
) -> Result<(), String> {
    let RaopAudioRuntime {
        socket,
        aes_key,
        aes_iv,
        frame_samples,
        start_sequence,
        start_timestamp,
        ssrc,
        packet_backlog,
        sync_timestamp,
    } = runtime;
    let input_format = FormatDescription::pcm::<i16>(AIRPLAY_SAMPLE_RATE as f64, 2);
    let output_format =
        FormatDescription::alac(AIRPLAY_SAMPLE_RATE as f64, frame_samples as u32, 2);
    let mut encoder = AlacEncoder::new(&output_format);
    let mut alac_output = vec![0u8; output_format.max_packet_size()];
    let mut pending_pcm = Vec::<u8>::new();
    let mut sequence = start_sequence;
    let mut timestamp = start_timestamp;
    let mut first_packet = true;
    let frame_bytes = frame_samples * AIRPLAY_CHANNELS as usize * 2;

    wasapi_loopback::capture_loop_for_source(stop, source, move |chunk| {
        let gain = volume.lock().map(|volume| *volume).unwrap_or(1.0);
        let bytes = pcm::samples_to_stereo_i16_bytes_at_sample_rate_with_gain(
            &chunk.samples,
            chunk.channels,
            chunk.sample_rate,
            AIRPLAY_SAMPLE_RATE,
            gain,
        );
        pending_pcm.extend_from_slice(&bytes);

        while pending_pcm.len() >= frame_bytes {
            let frame = pending_pcm.drain(..frame_bytes).collect::<Vec<_>>();
            let size = encoder.encode(&input_format, &frame, &mut alac_output);
            let payload = match (aes_key.as_ref(), aes_iv.as_ref()) {
                (Some(aes_key), Some(aes_iv)) => {
                    encrypt_alac_payload(&alac_output[..size], aes_key, aes_iv)
                }
                _ => alac_output[..size].to_vec(),
            };
            let packet = rtp_packet(sequence, timestamp, ssrc, first_packet, &payload);
            let _ = socket.send(&packet);
            store_packet_backlog(&packet_backlog, sequence, packet);
            first_packet = false;
            sequence = sequence.wrapping_add(1);
            timestamp = timestamp.wrapping_add(frame_samples as u32);
            sync_timestamp.store(timestamp, Ordering::Relaxed);
        }
    })
}

fn rtp_packet(sequence: u16, timestamp: u32, ssrc: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(12 + payload.len());
    packet.push(0x80);
    packet.push(if marker {
        AIRPLAY_PAYLOAD_TYPE | 0x80
    } else {
        AIRPLAY_PAYLOAD_TYPE
    });
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

fn store_packet_backlog(backlog: &PacketBacklog, sequence: u16, packet: Vec<u8>) {
    let Ok(mut backlog) = backlog.lock() else {
        return;
    };
    backlog.push_back((sequence, packet));
    while backlog.len() > AIRPLAY_PACKET_BACKLOG {
        backlog.pop_front();
    }
}

fn encrypt_alac_payload(payload: &[u8], aes_key: &[u8; 16], aes_iv: &[u8; 16]) -> Vec<u8> {
    let encrypted_len = (payload.len() / 16) * 16;
    if encrypted_len == 0 {
        return payload.to_vec();
    }

    let encrypted = Aes128CbcEnc::new(aes_key.into(), aes_iv.into())
        .encrypt_padded_vec_mut::<NoPadding>(&payload[..encrypted_len]);
    let mut output = encrypted;
    output.extend_from_slice(&payload[encrypted_len..]);
    output
}

fn airplay_sdp(
    local_ip: &IpAddr,
    remote_ip: &IpAddr,
    session_id: &str,
    frame_samples: usize,
    rsa_aes_key: Option<&str>,
    aes_iv: Option<&str>,
) -> String {
    let mut sdp = format!(
        "v=0\r\n\
         o=iTunes {session_id} 0 IN IP4 {local_ip}\r\n\
         s=iTunes\r\n\
         c=IN IP4 {remote_ip}\r\n\
         t=0 0\r\n\
         m=audio 0 RTP/AVP {AIRPLAY_PAYLOAD_TYPE}\r\n\
         a=rtpmap:{AIRPLAY_PAYLOAD_TYPE} AppleLossless\r\n\
         a=fmtp:{AIRPLAY_PAYLOAD_TYPE} {frame_samples} 0 16 40 10 14 2 255 0 0 {AIRPLAY_SAMPLE_RATE}\r\n"
    );
    if let (Some(rsa_aes_key), Some(aes_iv)) = (rsa_aes_key, aes_iv) {
        sdp.push_str(&format!("a=rsaaeskey:{rsa_aes_key}\r\n"));
        sdp.push_str(&format!("a=aesiv:{aes_iv}\r\n"));
    }
    sdp.push_str("a=min-latency:11025\r\n");
    sdp.push_str("a=max-latency:88200\r\n");
    sdp
}

fn encrypt_airplay_aes_key(aes_key: &[u8; 16]) -> Result<String, String> {
    let modulus = BASE64
        .decode(AIRPORT_PUBLIC_KEY_MODULUS_B64)
        .map_err(|err| format!("failed to decode AirPlay public key modulus: {err}"))?;
    let exponent = BASE64
        .decode(AIRPORT_PUBLIC_KEY_EXPONENT_B64)
        .map_err(|err| format!("failed to decode AirPlay public key exponent: {err}"))?;
    let public_key = RsaPublicKey::new(
        rsa::BigUint::from_bytes_be(&modulus),
        rsa::BigUint::from_bytes_be(&exponent),
    )
    .map_err(|err| format!("failed to build AirPlay public key: {err}"))?;
    let encrypted = public_key
        .encrypt(&mut rand::thread_rng(), Oaep::new::<sha1::Sha1>(), aes_key)
        .map_err(|err| format!("failed to encrypt AirPlay AES key: {err}"))?;
    Ok(BASE64.encode(encrypted))
}

fn spawn_timing_responder(
    socket: UdpSocket,
    remote_timing: Arc<Mutex<Option<SocketAddr>>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let _ = socket.set_read_timeout(Some(Duration::from_millis(250)));
        let mut buffer = [0u8; 128];

        while !stop.load(Ordering::Relaxed) {
            match socket.recv_from(&mut buffer) {
                Ok((len, addr)) if len >= 32 && buffer[1] & 0x7f == 0x52 => {
                    let response = timing_response(&buffer[..32]);
                    let _ = socket.send_to(&response, addr);
                }
                Ok(_) => {}
                Err(err)
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
            if let Some(remote_timing) = remote_timing.lock().ok().and_then(|peer| *peer) {
                let _ = socket.send_to(&timing_request(), remote_timing);
            }
        }
    })
}

type PacketBacklog = Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>;

fn spawn_control_loop(
    socket: UdpSocket,
    remote_control: Arc<Mutex<Option<SocketAddr>>>,
    sync_timestamp: Arc<AtomicU32>,
    packet_backlog: PacketBacklog,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let _ = socket.set_read_timeout(Some(Duration::from_millis(250)));
        let mut buffer = [0u8; 1500];
        let mut first_sync = true;
        let mut last_sync = Instant::now() - Duration::from_secs(2);
        while !stop.load(Ordering::Relaxed) {
            match socket.recv_from(&mut buffer) {
                Ok((len, addr)) if len >= 8 && buffer[1] & 0x7f == 0x55 => {
                    handle_retransmit_request(&socket, &packet_backlog, &buffer[..len], addr);
                }
                Ok(_) => {}
                Err(err)
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
            if last_sync.elapsed() >= Duration::from_secs(1) {
                if let Some(remote_control) = remote_control.lock().ok().and_then(|peer| *peer) {
                    let packet = sync_packet(
                        first_sync,
                        sync_timestamp.load(Ordering::Relaxed),
                        AIRPLAY_INITIAL_LATENCY_FRAMES,
                    );
                    let _ = socket.send_to(&packet, remote_control);
                    first_sync = false;
                    last_sync = Instant::now();
                }
            }
        }
    })
}

fn handle_retransmit_request(
    socket: &UdpSocket,
    packet_backlog: &PacketBacklog,
    request: &[u8],
    addr: SocketAddr,
) {
    let first = u16::from_be_bytes([request[4], request[5]]);
    let count = u16::from_be_bytes([request[6], request[7]]);
    let Ok(backlog) = packet_backlog.lock() else {
        return;
    };
    for offset in 0..count {
        let seq = first.wrapping_add(offset);
        if let Some((_, packet)) = backlog.iter().find(|(packet_seq, _)| *packet_seq == seq) {
            let mut response = Vec::with_capacity(4 + packet.len());
            response.push(0x80);
            response.push(0xd6);
            response.extend_from_slice(&seq.to_be_bytes());
            response.extend_from_slice(packet);
            let _ = socket.send_to(&response, addr);
        }
    }
}

fn sync_packet(first: bool, rtptime: u32, latency: u32) -> [u8; 20] {
    let mut packet = [0u8; 20];
    packet[0] = if first { 0x90 } else { 0x80 };
    packet[1] = 0xd4;
    packet[2..4].copy_from_slice(&7u16.to_be_bytes());
    packet[4..8].copy_from_slice(&rtptime.saturating_sub(latency).to_be_bytes());
    let (secs, frac) = ntp_now();
    put_ntp(&mut packet, 8, secs, frac);
    packet[16..20].copy_from_slice(&rtptime.to_be_bytes());
    packet
}

fn timing_request() -> [u8; 32] {
    let mut request = [0u8; 32];
    request[0] = 0x80;
    request[1] = 0xd2;
    request[2] = 0x00;
    request[3] = 0x07;
    let (secs, frac) = ntp_now();
    put_ntp(&mut request, 24, secs, frac);
    request
}

fn timing_response(request: &[u8]) -> [u8; 32] {
    let mut response = [0u8; 32];
    response.copy_from_slice(&request[..32]);
    response[1] = 0xd3;
    response[8..16].copy_from_slice(&request[24..32]);
    let (secs, frac) = ntp_now();
    put_ntp(&mut response, 16, secs, frac);
    put_ntp(&mut response, 24, secs, frac);
    response
}

fn put_ntp(buffer: &mut [u8], offset: usize, secs: u32, frac: u32) {
    buffer[offset..offset + 4].copy_from_slice(&secs.to_be_bytes());
    buffer[offset + 4..offset + 8].copy_from_slice(&frac.to_be_bytes());
}

fn ntp_now() -> (u32, u32) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = (now.as_secs() + 0x83AA7E80) as u32;
    let frac = ((now.subsec_nanos() as u64) << 32) / 1_000_000_000;
    (secs, frac as u32)
}

fn send_sync_hint(host: IpAddr, port: u16) -> Result<(), String> {
    let socket =
        UdpSocket::bind(("0.0.0.0", 0)).map_err(|err| format!("sync socket failed: {err}"))?;
    let mut packet = [0u8; 20];
    packet[0] = 0x80;
    packet[1] = 0xd4;
    let (secs, frac) = ntp_now();
    put_ntp(&mut packet, 4, secs, frac);
    put_ntp(&mut packet, 12, secs, frac);
    socket
        .send_to(&packet, SocketAddr::new(host, port))
        .map(|_| ())
        .map_err(|err| format!("sync hint failed: {err}"))
}

fn airplay_volume_db(volume: f32) -> f32 {
    let volume = volume.clamp(0.0, 1.0);
    if volume <= 0.001 {
        -144.0
    } else {
        -30.0 + (volume * 30.0)
    }
}

fn random_bytes_16() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

fn random_hex(bytes: usize) -> String {
    let mut raw = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut raw);
    raw.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn random_numeric_id() -> String {
    let mut rng = rand::thread_rng();
    let value = ((rng.next_u32() as u64) << 32) | rng.next_u32() as u64;
    value.to_string()
}

fn random_challenge() -> String {
    BASE64.encode(random_bytes_16())
}

fn random_challenge_unpadded() -> String {
    random_challenge().trim_end_matches('=').to_string()
}

fn resolve_socket_addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    (host, port)
        .to_socket_addrs()
        .map_err(|err| format!("failed to resolve AirPlay host {host}:{port}: {err}"))?
        .next()
        .ok_or_else(|| format!("no address resolved for AirPlay host {host}:{port}"))
}

fn local_ip_for(remote_host: &str) -> Result<IpAddr, String> {
    let remote = (remote_host, 9)
        .to_socket_addrs()
        .map_err(|err| format!("failed to resolve AirPlay host {remote_host}: {err}"))?
        .next()
        .ok_or_else(|| format!("no address resolved for AirPlay host {remote_host}"))?;
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|err| format!("failed to bind local address probe socket: {err}"))?;
    socket
        .connect(remote)
        .map_err(|err| format!("failed to probe local route to AirPlay device: {err}"))?;
    Ok(socket
        .local_addr()
        .map_err(|err| format!("failed to read local route address: {err}"))?
        .ip())
}

fn transport_port(transport: &str, key: &str) -> Option<u16> {
    transport.split(';').find_map(|part| {
        let part = part.trim();
        let (name, value) = part.split_once('=')?;
        (name.eq_ignore_ascii_case(key))
            .then(|| value.parse::<u16>().ok())
            .flatten()
    })
}

fn ap2_transient_pair_setup(rtsp: &mut RtspClient) -> Result<Vec<u8>, String> {
    let (m2, content_type, attempt_label) = ap2_pair_setup_m1(rtsp)?;
    let m2 = tlv_decode(&m2.body)?;
    let salt = tlv_required(&m2, 2)?;
    let server_public = tlv_required(&m2, 3)?;

    let n = BigUint::parse_bytes(AP2_N_3072_HEX.as_bytes(), 16)
        .ok_or_else(|| "failed to parse AirPlay 2 SRP group".to_string())?;
    let g = BigUint::from(5u32);
    let mut a_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut a_bytes);
    let a = BigUint::from_bytes_be(&a_bytes);
    let big_a = g.modpow(&a, &n);
    let big_b = BigUint::from_bytes_be(server_public);
    if (&big_b % &n).is_zero() {
        return Err("AirPlay 2 SRP server public key is invalid".to_string());
    }

    let salt_bn = BigUint::from_bytes_be(salt);
    let k = ap2_h_nn_pad(&n, &g);
    let u = ap2_h_nn_pad(&big_a, &big_b);
    let x = ap2_calculate_x(&salt_bn);
    let gx = g.modpow(&x, &n);
    let kgx = (&k * &gx) % &n;
    let base = (&big_b + &n - &kgx) % &n;
    let exponent = &a + &u * &x;
    let big_s = base.modpow(&exponent, &n);
    let session_key = sha512_bytes(&ap2_to_bytes_be(&big_s)).to_vec();
    let client_proof = ap2_calculate_m(&n, &g, &salt_bn, &big_a, &big_b, &session_key);

    let a_bytes = ap2_to_bytes_be(&big_a);
    let m3 = tlv_encode(&[(6, &[3][..]), (3, &a_bytes), (4, &client_proof)]);
    let m4 = rtsp
        .request(
            "POST",
            "/pair-setup",
            &[("Content-Type", content_type)],
            &m3,
        )
        .map_err(|err| format!("{attempt_label} M3 failed: {err}"))?;
    let m4 = tlv_decode(&m4.body)?;
    if let Some(error) = m4.get(&7).and_then(|value| value.first()) {
        return Err(format!("receiver returned TLV error {error}"));
    }
    let server_proof = tlv_required(&m4, 4)?;
    let expected_proof = ap2_calculate_h_amk(&big_a, &client_proof, &session_key);
    if server_proof != expected_proof.as_slice() {
        return Err("receiver SRP proof did not match".to_string());
    }

    Ok(session_key)
}

fn ap2_pair_setup_m1(
    rtsp: &mut RtspClient,
) -> Result<(RtspResponse, &'static str, &'static str), String> {
    let method_state_flags = tlv_encode(&[(0, &[0][..]), (6, &[1][..]), (19, &[0x10][..])]);
    let state_method_flags = tlv_encode(&[(6, &[1][..]), (0, &[0][..]), (19, &[0x10][..])]);
    let method_state = tlv_encode(&[(0, &[0][..]), (6, &[1][..])]);
    let attempts = [
        (
            "HomeKit transient tlv8",
            "application/pairing+tlv8",
            method_state_flags.as_slice(),
        ),
        (
            "HomeKit transient octet",
            "application/octet-stream",
            method_state_flags.as_slice(),
        ),
        (
            "HomeKit transient state-first tlv8",
            "application/pairing+tlv8",
            state_method_flags.as_slice(),
        ),
        (
            "HomeKit SRP no transient flag tlv8",
            "application/pairing+tlv8",
            method_state.as_slice(),
        ),
    ];
    let mut errors = Vec::new();
    for (label, content_type, body) in attempts {
        match rtsp.request_raw(
            "POST",
            "/pair-setup",
            &[("Content-Type", content_type)],
            body,
        ) {
            Ok(response) if (200..300).contains(&response.status_code) => {
                return Ok((response, content_type, label));
            }
            Ok(response) => errors.push(format!(
                "{label}: RTSP {} {}{}",
                response.status_code,
                response.reason,
                response.diagnostic_suffix()
            )),
            Err(err) => errors.push(format!("{label}: {err}")),
        }
    }
    Err(format!(
        "all AP2 pair-setup M1 variants failed: {}",
        errors.join(" | ")
    ))
}

fn tlv_encode(fields: &[(u8, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, value) in fields {
        if value.is_empty() {
            out.push(*tag);
            out.push(0);
            continue;
        }
        for chunk in value.chunks(255) {
            out.push(*tag);
            out.push(chunk.len() as u8);
            out.extend_from_slice(chunk);
        }
    }
    out
}

fn tlv_decode(data: &[u8]) -> Result<HashMap<u8, Vec<u8>>, String> {
    let mut fields = HashMap::new();
    let mut index = 0;
    while index + 2 <= data.len() {
        let tag = data[index];
        let len = data[index + 1] as usize;
        index += 2;
        if index + len > data.len() {
            return Err("truncated AirPlay 2 TLV".to_string());
        }
        fields
            .entry(tag)
            .or_insert_with(Vec::new)
            .extend_from_slice(&data[index..index + len]);
        index += len;
    }
    if index != data.len() {
        return Err("trailing byte in AirPlay 2 TLV".to_string());
    }
    Ok(fields)
}

fn tlv_required(fields: &HashMap<u8, Vec<u8>>, tag: u8) -> Result<&[u8], String> {
    fields
        .get(&tag)
        .map(Vec::as_slice)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("AirPlay 2 TLV missing tag {tag}"))
}

fn sha512_bytes(data: &[u8]) -> [u8; 64] {
    let mut hasher = Sha512::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn ap2_to_bytes_be(value: &BigUint) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    if bytes.is_empty() {
        vec![0]
    } else {
        bytes
    }
}

fn ap2_to_bytes_be_padded(value: &BigUint) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    if bytes.len() >= AP2_N_3072_LEN {
        bytes
    } else {
        let mut padded = vec![0u8; AP2_N_3072_LEN - bytes.len()];
        padded.extend_from_slice(&bytes);
        padded
    }
}

fn ap2_h_nn_pad(left: &BigUint, right: &BigUint) -> BigUint {
    let mut bytes = Vec::with_capacity(AP2_N_3072_LEN * 2);
    bytes.extend_from_slice(&ap2_to_bytes_be_padded(left));
    bytes.extend_from_slice(&ap2_to_bytes_be_padded(right));
    BigUint::from_bytes_be(&sha512_bytes(&bytes))
}

fn ap2_calculate_x(salt: &BigUint) -> BigUint {
    let mut inner = Sha512::new();
    inner.update(AP2_SRP_USERNAME.as_bytes());
    inner.update(b":");
    inner.update(AP2_TRANSIENT_PIN.as_bytes());
    let inner = inner.finalize();

    let mut bytes = ap2_to_bytes_be(salt);
    bytes.extend_from_slice(&inner);
    BigUint::from_bytes_be(&sha512_bytes(&bytes))
}

fn ap2_calculate_m(
    n: &BigUint,
    g: &BigUint,
    salt: &BigUint,
    big_a: &BigUint,
    big_b: &BigUint,
    session_key: &[u8],
) -> [u8; 64] {
    let h_n = sha512_bytes(&ap2_to_bytes_be(n));
    let h_g = sha512_bytes(&ap2_to_bytes_be(g));
    let mut h_xor = [0u8; 64];
    for index in 0..64 {
        h_xor[index] = h_n[index] ^ h_g[index];
    }
    let h_i = sha512_bytes(AP2_SRP_USERNAME.as_bytes());

    let mut hasher = Sha512::new();
    hasher.update(h_xor);
    hasher.update(h_i);
    hasher.update(ap2_to_bytes_be(salt));
    hasher.update(ap2_to_bytes_be(big_a));
    hasher.update(ap2_to_bytes_be(big_b));
    hasher.update(session_key);
    hasher.finalize().into()
}

fn ap2_calculate_h_amk(big_a: &BigUint, proof: &[u8], session_key: &[u8]) -> [u8; 64] {
    let mut hasher = Sha512::new();
    hasher.update(ap2_to_bytes_be(big_a));
    hasher.update(proof);
    hasher.update(session_key);
    hasher.finalize().into()
}

struct Ap2CipherContext {
    key: [u8; 32],
    counter: u64,
}

impl Ap2CipherContext {
    fn new(key: [u8; 32]) -> Self {
        Self { key, counter: 0 }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[4..12].copy_from_slice(&self.counter.to_le_bytes());
        nonce
    }

    fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let mut out = Vec::with_capacity(plaintext.len() + plaintext.len() / 32);
        for chunk in plaintext.chunks(AP2_MAX_RTSP_BLOCK) {
            let len = (chunk.len() as u16).to_le_bytes();
            let nonce = self.nonce();
            let encrypted = cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: chunk,
                        aad: &len,
                    },
                )
                .map_err(|_| "AirPlay 2 RTSP encryption failed".to_string())?;
            out.extend_from_slice(&len);
            out.extend_from_slice(&encrypted);
            self.counter = self.counter.wrapping_add(1);
        }
        Ok(out)
    }

    fn decrypt_blocks(&mut self, encrypted: &[u8]) -> Result<(Vec<u8>, usize), String> {
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let mut plaintext = Vec::new();
        let mut index = 0;
        while index + 2 <= encrypted.len() {
            let len_bytes = [encrypted[index], encrypted[index + 1]];
            let len = u16::from_le_bytes(len_bytes) as usize;
            let frame_len = 2 + len + AP2_RTSP_TAG_LEN;
            if index + frame_len > encrypted.len() {
                break;
            }
            let nonce = self.nonce();
            let decrypted = cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &encrypted[index + 2..index + frame_len],
                        aad: &len_bytes,
                    },
                )
                .map_err(|_| "AirPlay 2 RTSP decryption failed".to_string())?;
            plaintext.extend_from_slice(&decrypted);
            index += frame_len;
            self.counter = self.counter.wrapping_add(1);
        }
        Ok((plaintext, index))
    }
}

struct Ap2RtspCipher {
    write: Ap2CipherContext,
    read: Ap2CipherContext,
    encrypted_read_buffer: Vec<u8>,
    decrypted_read_buffer: Vec<u8>,
}

impl Ap2RtspCipher {
    fn control_for_client(shared_secret: &[u8]) -> Result<Self, String> {
        let write = ap2_hkdf_key(
            shared_secret,
            "Control-Salt",
            "Control-Write-Encryption-Key",
        )?;
        let read = ap2_hkdf_key(shared_secret, "Control-Salt", "Control-Read-Encryption-Key")?;
        Ok(Self {
            write: Ap2CipherContext::new(write),
            read: Ap2CipherContext::new(read),
            encrypted_read_buffer: Vec::new(),
            decrypted_read_buffer: Vec::new(),
        })
    }
}

fn ap2_hkdf_key(shared_secret: &[u8], salt: &str, info: &str) -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    Hkdf::<Sha512>::new(Some(salt.as_bytes()), shared_secret)
        .expand(info.as_bytes(), &mut key)
        .map_err(|_| format!("AirPlay 2 HKDF failed for {info}"))?;
    Ok(key)
}

struct RtspClient {
    stream: TcpStream,
    host_header: String,
    include_host_header: bool,
    user_agent: String,
    cseq: u32,
    ap2_cipher: Option<Ap2RtspCipher>,
}

impl RtspClient {
    fn connect(host: &str, port: u16) -> Result<Self, String> {
        let addr = resolve_socket_addr(host, port)?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(4))
            .map_err(|err| format!("failed to connect to AirPlay RTSP {host}:{port}: {err}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|err| format!("failed to set AirPlay RTSP read timeout: {err}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|err| format!("failed to set AirPlay RTSP write timeout: {err}"))?;
        Ok(Self {
            stream,
            host_header: format!("{host}:{port}"),
            include_host_header: true,
            user_agent: "iTunes/12.10 NARfW/0.1".to_string(),
            cseq: 1,
            ap2_cipher: None,
        })
    }

    fn set_user_agent(&mut self, user_agent: &str) {
        self.user_agent = user_agent.to_string();
    }

    fn set_include_host_header(&mut self, include: bool) {
        self.include_host_header = include;
    }

    fn set_cseq(&mut self, cseq: u32) {
        self.cseq = cseq;
    }

    fn set_timeouts(&self, read: Duration, write: Duration) -> Result<(), String> {
        self.stream
            .set_read_timeout(Some(read))
            .map_err(|err| format!("failed to set AirPlay RTSP read timeout: {err}"))?;
        self.stream
            .set_write_timeout(Some(write))
            .map_err(|err| format!("failed to set AirPlay RTSP write timeout: {err}"))
    }

    fn enable_ap2_control_cipher(&mut self, shared_secret: &[u8]) -> Result<(), String> {
        self.ap2_cipher = Some(Ap2RtspCipher::control_for_client(shared_secret)?);
        Ok(())
    }

    fn request(
        &mut self,
        method: &str,
        uri: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<RtspResponse, String> {
        let response = self.request_raw(method, uri, headers, body)?;
        if !(200..300).contains(&response.status_code) {
            return Err(format!(
                "AirPlay {method} failed: RTSP {} {}{}",
                response.status_code,
                response.reason,
                response.diagnostic_suffix()
            ));
        }
        Ok(response)
    }

    fn request_raw(
        &mut self,
        method: &str,
        uri: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<RtspResponse, String> {
        self.write_request(method, uri, headers, body)?;
        if self.ap2_cipher.is_some() {
            self.read_ap2_response()
                .map_err(|err| format!("AirPlay {method} failed while reading response: {err}"))
        } else {
            read_rtsp_response(&mut self.stream)
                .map_err(|err| format!("AirPlay {method} failed while reading response: {err}"))
        }
    }

    fn request_no_response(
        &mut self,
        method: &str,
        uri: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<(), String> {
        self.write_request(method, uri, headers, body)
    }

    fn write_request(
        &mut self,
        method: &str,
        uri: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<(), String> {
        let cseq = self.cseq;
        self.cseq = self.cseq.wrapping_add(1);
        let mut request = format!(
            "{method} {uri} RTSP/1.0\r\n\
             CSeq: {cseq}\r\n\
             User-Agent: {}\r\n",
            self.user_agent
        );
        if self.include_host_header {
            request.push_str("Host: ");
            request.push_str(&self.host_header);
            request.push_str("\r\n");
        }
        for (name, value) in headers {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        if !body.is_empty() {
            request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        request.push_str("\r\n");

        let mut plaintext = request.into_bytes();
        plaintext.extend_from_slice(body);
        let wire_bytes = if let Some(cipher) = self.ap2_cipher.as_mut() {
            cipher.write.encrypt(&plaintext)?
        } else {
            plaintext
        };

        self.stream
            .write_all(&wire_bytes)
            .map_err(|err| format!("failed to send AirPlay {method}: {err}"))
    }

    fn read_ap2_response(&mut self) -> Result<RtspResponse, String> {
        loop {
            let parsed = {
                let buffer = self.ap2_plain_buffer_mut()?;
                try_parse_rtsp_response(buffer)?
            };
            if let Some(response) = parsed {
                return Ok(response);
            }

            let mut buffer = [0u8; 2048];
            let len = self
                .stream
                .read(&mut buffer)
                .map_err(|err| format!("failed to read AirPlay 2 RTSP response: {err}"))?;
            if len == 0 {
                return Err("AirPlay 2 RTSP connection closed".to_string());
            }

            let cipher = self
                .ap2_cipher
                .as_mut()
                .ok_or_else(|| "AirPlay 2 RTSP cipher is not active".to_string())?;
            cipher
                .encrypted_read_buffer
                .extend_from_slice(&buffer[..len]);
            let (plain, consumed) = cipher.read.decrypt_blocks(&cipher.encrypted_read_buffer)?;
            if consumed > 0 {
                cipher.encrypted_read_buffer.drain(..consumed);
            }
            cipher.decrypted_read_buffer.extend_from_slice(&plain);
        }
    }

    fn ap2_plain_buffer_mut(&mut self) -> Result<&mut Vec<u8>, String> {
        self.ap2_cipher
            .as_mut()
            .map(|cipher| &mut cipher.decrypted_read_buffer)
            .ok_or_else(|| "AirPlay 2 RTSP cipher is not active".to_string())
    }
}

#[derive(Debug)]
struct RtspResponse {
    status_code: u16,
    reason: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl RtspResponse {
    fn header(&self, name: &str) -> Option<String> {
        self.headers.get(&name.to_ascii_lowercase()).cloned()
    }

    fn diagnostic_suffix(&self) -> String {
        let mut parts = Vec::new();
        for key in [
            "server",
            "www-authenticate",
            "public",
            "apple-response",
            "audio-jack-status",
        ] {
            if let Some(value) = self.headers.get(key) {
                parts.push(format!("{key}: {value}"));
            }
        }
        if !self.body.is_empty() && self.body.len() <= 512 {
            let body = String::from_utf8_lossy(&self.body)
                .replace('\r', "\\r")
                .replace('\n', "\\n");
            if !body.trim().is_empty() {
                parts.push(format!("body: {body}"));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", parts.join("; "))
        }
    }
}

fn read_rtsp_response(stream: &mut TcpStream) -> Result<RtspResponse, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 2048];
    let header_end = loop {
        let len = stream
            .read(&mut buffer)
            .map_err(|err| format!("failed to read AirPlay RTSP response: {err}"))?;
        if len == 0 {
            return Err("AirPlay RTSP connection closed".to_string());
        }
        bytes.extend_from_slice(&buffer[..len]);
        if let Some(pos) = find_header_end(&bytes) {
            break pos;
        }
    };

    let (status_code, reason, headers) = parse_rtsp_header_block(&bytes[..header_end])?;

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    while bytes.len().saturating_sub(body_start) < content_length {
        let len = stream
            .read(&mut buffer)
            .map_err(|err| format!("failed to read AirPlay RTSP response body: {err}"))?;
        if len == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..len]);
    }

    let body = if bytes.len() > body_start {
        bytes[body_start..bytes.len().min(body_start + content_length)].to_vec()
    } else {
        Vec::new()
    };

    Ok(RtspResponse {
        status_code,
        reason,
        headers,
        body,
    })
}

fn parse_rtsp_header_block(bytes: &[u8]) -> Result<(u16, String, HashMap<String, String>), String> {
    let header_text = String::from_utf8_lossy(bytes).to_string();
    let mut lines = header_text.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut status_parts = status.splitn(3, ' ');
    let _version = status_parts.next();
    let status_code = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("invalid AirPlay RTSP response: {status}"))?;
    let reason = status_parts.next().unwrap_or("").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    Ok((status_code, reason, headers))
}

fn try_parse_rtsp_response(bytes: &mut Vec<u8>) -> Result<Option<RtspResponse>, String> {
    let Some(header_end) = find_header_end(bytes) else {
        return Ok(None);
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]).to_string();
    let mut lines = header_text.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut status_parts = status.splitn(3, ' ');
    let _version = status_parts.next();
    let status_code = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("invalid AirPlay RTSP response: {status}"))?;
    let reason = status_parts.next().unwrap_or("").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    let required = body_start + content_length;
    if bytes.len() < required {
        return Ok(None);
    }
    let body = bytes[body_start..required].to_vec();
    bytes.drain(..required);
    Ok(Some(RtspResponse {
        status_code,
        reason,
        headers,
        body,
    }))
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_transport_ports() {
        let transport =
            "RTP/AVP/UDP;unicast;mode=record;timing_port=6002;control_port=6001;server_port=6000";
        assert_eq!(transport_port(transport, "server_port"), Some(6000));
        assert_eq!(transport_port(transport, "control_port"), Some(6001));
        assert_eq!(transport_port(transport, "timing_port"), Some(6002));
    }

    #[test]
    fn maps_airplay_volume() {
        assert!(airplay_volume_db(0.0) < -100.0);
        assert_eq!(airplay_volume_db(1.0), 0.0);
        assert_eq!(airplay_volume_db(0.5), -15.0);
    }

    #[test]
    fn numeric_session_id_contains_only_digits() {
        assert!(random_numeric_id().chars().all(|ch| ch.is_ascii_digit()));
    }

    #[test]
    fn builds_clear_and_encrypted_sdp() {
        let local_ip = "192.168.1.10".parse::<IpAddr>().unwrap();
        let remote_ip = "192.168.1.172".parse::<IpAddr>().unwrap();
        let clear = airplay_sdp(&local_ip, &remote_ip, "123", 352, None, None);
        assert!(clear.contains("o=iTunes 123 0 IN IP4 192.168.1.10"));
        assert!(clear.contains("c=IN IP4 192.168.1.172"));
        assert!(clear.contains("a=fmtp:96 352"));
        assert!(!clear.contains("a=rsaaeskey"));

        let encrypted = airplay_sdp(&local_ip, &remote_ip, "123", 4096, Some("key"), Some("iv"));
        assert!(encrypted.contains("a=fmtp:96 4096"));
        assert!(encrypted.contains("a=rsaaeskey:key"));
        assert!(encrypted.contains("a=aesiv:iv"));
    }

    #[test]
    fn starts_auth_setup_raop_with_owntone_style_local_uri() {
        let variants = airplay_announce_variants(&test_airplay_connection(
            "Bonjour | RAOP auth-setup audio | Denon Home 150 | port 7000 | et=0,4 cn=0,1",
        ));
        let first = variants.first().copied().unwrap();
        assert_eq!(first.frame_samples, 352);
        assert!(!first.encrypted);
        assert!(first.auth_setup);
        assert!(!first.paired);
        assert!(matches!(first.session_uri_kind, SessionUriKind::Local));
        assert!(variants.len() <= 3);
    }

    fn test_airplay_connection(detail: &str) -> ConnectionProfile {
        ConnectionProfile {
            id: "airplay:test:192.168.1.172".to_string(),
            name: "Kostya".to_string(),
            protocol: "AIRPLAY".to_string(),
            transport: "mDNS/Bonjour".to_string(),
            host: "192.168.1.172".to_string(),
            port: Some(7000),
            detail: detail.to_string(),
            supported: true,
            codecs: Vec::new(),
            renderer: None,
        }
    }

    #[test]
    fn fairplay_m1_body_uses_expected_probe_shape() {
        let body = fairplay_m1_body(2);
        assert_eq!(body.len(), 16);
        assert_eq!(body[4], 0x03);
        assert_eq!(body[14], 2);
        assert!(body
            .iter()
            .enumerate()
            .all(|(index, value)| index == 4 || index == 14 || *value == 0));
    }

    #[test]
    fn detects_fairplay_m1_setup_response() {
        let mut body = vec![0_u8; 142];
        body[..4].copy_from_slice(b"FPLY");
        let response = RtspResponse {
            status_code: 200,
            reason: "OK".to_string(),
            headers: HashMap::new(),
            body,
        };
        assert!(is_fairplay_m1_response(&response));

        let bad = RtspResponse {
            status_code: 200,
            reason: "OK".to_string(),
            headers: HashMap::new(),
            body: b"FPLY".to_vec(),
        };
        assert!(!is_fairplay_m1_response(&bad));
    }

    #[test]
    fn builds_auth_setup_body_with_no_encryption_flag() {
        let body = airplay_auth_setup_body();
        assert_eq!(body.len(), 33);
        assert_eq!(body[0], 0x01);
        assert_eq!(&body[1..], &AIRPLAY_AUTH_SETUP_PUBLIC_KEY);
    }

    #[test]
    fn marks_first_raop_audio_packet() {
        let first = rtp_packet(10, 66_150, 0x1234_5678, true, b"frame");
        assert_eq!(first[0], 0x80);
        assert_eq!(first[1], 0xe0);
        assert_eq!(&first[2..4], &10u16.to_be_bytes());
        assert_eq!(&first[4..8], &66_150u32.to_be_bytes());

        let next = rtp_packet(11, 66_502, 0x1234_5678, false, b"frame");
        assert_eq!(next[1], 0x60);
    }

    #[test]
    fn builds_raop_sync_packet_with_rtp_time_fields() {
        let packet = sync_packet(true, 66_502, AIRPLAY_INITIAL_LATENCY_FRAMES);
        assert_eq!(packet[0], 0x90);
        assert_eq!(packet[1], 0xd4);
        assert_eq!(&packet[2..4], &7u16.to_be_bytes());
        assert_eq!(&packet[4..8], &352u32.to_be_bytes());
        assert_eq!(&packet[16..20], &66_502u32.to_be_bytes());
    }

    #[test]
    fn parses_rtsp_header_block_without_body() {
        let (status, reason, headers) = parse_rtsp_header_block(
            b"RTSP/1.0 200 OK\r\nContent-Length: 1076\r\nServer: AirTunes/366.0\r\nCSeq: 0",
        )
        .unwrap();
        assert_eq!(status, 200);
        assert_eq!(reason, "OK");
        assert_eq!(headers.get("content-length").unwrap(), "1076");
        assert_eq!(headers.get("server").unwrap(), "AirTunes/366.0");
    }

    #[test]
    fn airplay_challenge_omits_base64_padding() {
        for _ in 0..16 {
            let challenge = random_challenge_unpadded();
            assert!(!challenge.contains('='));
            assert!(challenge.len() >= 21);
        }
    }

    #[test]
    fn airplay2_tlv_roundtrips_chunked_values() {
        let long_value = vec![0x42; 300];
        let encoded = tlv_encode(&[(6, &[1][..]), (3, &long_value)]);
        let decoded = tlv_decode(&encoded).unwrap();
        assert_eq!(decoded.get(&6).unwrap(), &[1]);
        assert_eq!(decoded.get(&3).unwrap(), &long_value);
    }

    #[test]
    fn airplay2_rtp_packet_uses_chacha_payload_with_trailing_nonce() {
        let shk = [0x11u8; 32];
        let cipher = ChaCha20Poly1305::new((&shk).into());
        let packet = ap2_rtp_packet(&cipher, 7, 42, 99, b"alac-frame", 5).unwrap();
        assert_eq!(packet[0], 0x80);
        assert_eq!(packet[1], AIRPLAY_PAYLOAD_TYPE);
        assert_eq!(&packet[2..4], &7u16.to_be_bytes());
        assert_eq!(&packet[4..8], &42u32.to_be_bytes());
        assert_eq!(&packet[8..12], &99u32.to_be_bytes());

        let mut nonce = [0u8; 12];
        nonce[4..12].copy_from_slice(&packet[packet.len() - AP2_RTP_NONCE_TRAIL_LEN..]);
        let decrypted = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &packet[12..packet.len() - AP2_RTP_NONCE_TRAIL_LEN],
                    aad: &packet[4..12],
                },
            )
            .unwrap();
        assert_eq!(decrypted, b"alac-frame");
    }
}
