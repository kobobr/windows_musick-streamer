use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::Serialize;

use crate::{
    audio::{pcm, sources::CaptureSource, wasapi_loopback},
    network::discovery::DiscoveredRenderer,
};

type ClientSenders = Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlnaStreamFormat {
    WavPcm,
    L16Pcm,
}

impl DlnaStreamFormat {
    pub fn from_codec_id(codec: &str) -> Result<Self, String> {
        match codec {
            "pcm_s16le" => Ok(Self::WavPcm),
            "pcm_l16be" => Ok(Self::L16Pcm),
            other => Err(format!("DLNA codec {other} is not implemented yet")),
        }
    }

    fn codec_label(self) -> &'static str {
        match self {
            Self::WavPcm => "WAV PCM S16LE stereo",
            Self::L16Pcm => "L16 PCM S16BE stereo",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::WavPcm => "audio/wav",
            Self::L16Pcm => "audio/L16;rate=48000;channels=2",
        }
    }

    fn protocol_info(self) -> &'static str {
        match self {
            Self::WavPcm => "http-get:*:audio/wav:DLNA.ORG_PN=WAV;DLNA.ORG_OP=01",
            Self::L16Pcm => "http-get:*:audio/L16:DLNA.ORG_OP=01",
        }
    }

    fn stream_path(self) -> &'static str {
        match self {
            Self::WavPcm => "/stream.wav",
            Self::L16Pcm => "/stream.l16",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DlnaStreamInfo {
    pub renderer: String,
    pub control_url: String,
    pub stream_url: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone)]
struct RendererControl {
    name: String,
    control_url: String,
    av_transport_service_type: String,
    rendering_control_url: Option<String>,
    rendering_control_service_type: Option<String>,
}

pub struct DlnaHandle {
    stop: Arc<AtomicBool>,
    volume: Arc<Mutex<f32>>,
    capture_thread: Option<JoinHandle<()>>,
    listener_thread: Option<JoinHandle<()>>,
    control_url: String,
    av_transport_service_type: String,
    rendering_control_url: Option<String>,
    rendering_control_service_type: Option<String>,
}

impl DlnaHandle {
    pub fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 2.0);
        if let Ok(mut current) = self.volume.lock() {
            *current = volume;
        }
        if let (Some(control_url), Some(service_type)) = (
            self.rendering_control_url.as_deref(),
            self.rendering_control_service_type.as_deref(),
        ) {
            let desired_volume = (volume * 100.0).round().clamp(0.0, 150.0) as u8;
            let body = format!(
                "<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>{desired_volume}</DesiredVolume>"
            );
            let _ = soap_action(control_url, service_type, "SetVolume", &body);
        }
    }

    pub fn stop(mut self) {
        let _ = soap_action(
            &self.control_url,
            &self.av_transport_service_type,
            "Stop",
            "<InstanceID>0</InstanceID>",
        );
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.capture_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.listener_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for DlnaHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start(renderer: DiscoveredRenderer) -> Result<(DlnaHandle, DlnaStreamInfo), String> {
    start_with_volume(renderer, 1.0)
}

pub fn start_with_volume(
    renderer: DiscoveredRenderer,
    volume: f32,
) -> Result<(DlnaHandle, DlnaStreamInfo), String> {
    start_with_format(renderer, volume, DlnaStreamFormat::WavPcm)
}

pub fn start_with_format(
    renderer: DiscoveredRenderer,
    volume: f32,
    format: DlnaStreamFormat,
) -> Result<(DlnaHandle, DlnaStreamInfo), String> {
    start_with_format_and_source(renderer, volume, format, CaptureSource::System)
}

pub fn start_with_format_and_source(
    renderer: DiscoveredRenderer,
    volume: f32,
    format: DlnaStreamFormat,
    source: CaptureSource,
) -> Result<(DlnaHandle, DlnaStreamInfo), String> {
    let control = resolve_renderer_control(&renderer)?;
    let renderer_host = host_from_url(&renderer.location)
        .or_else(|| host_from_remote(&renderer.remote_addr))
        .ok_or_else(|| "failed to resolve renderer host".to_string())?;
    let local_ip = local_ip_for(&renderer_host)?;

    let listener = TcpListener::bind(("0.0.0.0", 0))
        .map_err(|err| format!("failed to bind DLNA HTTP stream server: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("failed to configure DLNA HTTP stream server: {err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("failed to read DLNA HTTP stream server address: {err}"))?
        .port();
    let stream_url = format!("http://{local_ip}:{port}{}", format.stream_path());

    let stop = Arc::new(AtomicBool::new(false));
    let client_seen = Arc::new(AtomicBool::new(false));
    let volume = Arc::new(Mutex::new(volume.clamp(0.0, 2.0)));
    let clients: ClientSenders = Arc::new(Mutex::new(Vec::new()));

    let listener_stop = Arc::clone(&stop);
    let listener_client_seen = Arc::clone(&client_seen);
    let listener_clients = Arc::clone(&clients);
    let listener_thread = thread::spawn(move || {
        while !listener_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    listener_client_seen.store(true, Ordering::Relaxed);
                    spawn_http_client(stream, Arc::clone(&listener_clients), format);
                }
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
            let bytes = match format {
                DlnaStreamFormat::WavPcm => pcm::samples_to_stereo_i16_bytes_with_gain(
                    &chunk.samples,
                    chunk.channels,
                    chunk.sample_rate,
                    gain,
                ),
                DlnaStreamFormat::L16Pcm => pcm::samples_to_stereo_i16_be_bytes_with_gain(
                    &chunk.samples,
                    chunk.channels,
                    chunk.sample_rate,
                    gain,
                ),
            };
            if bytes.is_empty() {
                return;
            }
            if let Ok(mut senders) = capture_clients.lock() {
                senders.retain(|sender| sender.send(bytes.clone()).is_ok());
            }
        }) {
            eprintln!("DLNA capture error: {err}");
        }
    });

    thread::sleep(Duration::from_millis(250));
    let metadata = didl_metadata(&stream_url, format);
    let set_uri_body = format!(
        "<InstanceID>0</InstanceID>\
         <CurrentURI>{}</CurrentURI>\
         <CurrentURIMetaData>{}</CurrentURIMetaData>",
        xml_escape(&stream_url),
        xml_escape(&metadata)
    );
    let _ = soap_action(
        &control.control_url,
        &control.av_transport_service_type,
        "Stop",
        "<InstanceID>0</InstanceID>",
    );
    soap_action(
        &control.control_url,
        &control.av_transport_service_type,
        "SetAVTransportURI",
        &set_uri_body,
    )?;
    soap_action(
        &control.control_url,
        &control.av_transport_service_type,
        "Play",
        "<InstanceID>0</InstanceID><Speed>1</Speed>",
    )?;

    if !wait_for_first_client(&client_seen, Duration::from_secs(5)) {
        stop.store(true, Ordering::Relaxed);
        let _ = capture_thread.join();
        let _ = listener_thread.join();
        return Err(format!(
            "{} accepted Play, but did not request the local audio stream at {}",
            control.name, stream_url
        ));
    }

    Ok((
        DlnaHandle {
            stop,
            volume,
            capture_thread: Some(capture_thread),
            listener_thread: Some(listener_thread),
            control_url: control.control_url.clone(),
            av_transport_service_type: control.av_transport_service_type.clone(),
            rendering_control_url: control.rendering_control_url.clone(),
            rendering_control_service_type: control.rendering_control_service_type.clone(),
        },
        DlnaStreamInfo {
            renderer: control.name,
            control_url: control.control_url,
            stream_url,
            codec: format.codec_label().to_string(),
            sample_rate: pcm::OUTPUT_SAMPLE_RATE,
            channels: pcm::OUTPUT_CHANNELS,
        },
    ))
}

fn wait_for_first_client(client_seen: &AtomicBool, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if client_seen.load(Ordering::Relaxed) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    client_seen.load(Ordering::Relaxed)
}

fn spawn_http_client(mut stream: TcpStream, clients: ClientSenders, format: DlnaStreamFormat) {
    thread::spawn(move || {
        let mut request = [0u8; 1024];
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.read(&mut request);

        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: {}\r\n\
             Cache-Control: no-cache\r\n\
             transferMode.dlna.org: Streaming\r\n\
             Connection: close\r\n\
             Access-Control-Allow-Origin: *\r\n\
             \r\n",
            format.content_type()
        );
        if stream.write_all(response.as_bytes()).is_err() {
            return;
        }
        if format == DlnaStreamFormat::WavPcm
            && stream.write_all(&pcm::wav_stream_header()).is_err()
        {
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

fn resolve_renderer_control(renderer: &DiscoveredRenderer) -> Result<RendererControl, String> {
    if let Some(control_url) = renderer.control_url.clone() {
        let name = stream_target_name(
            renderer.friendly_name.as_deref(),
            renderer.service_name.as_deref(),
            renderer.location.as_str(),
        );
        return Ok(RendererControl {
            name,
            control_url,
            av_transport_service_type: renderer
                .service_type
                .clone()
                .unwrap_or_else(default_av_transport_service_type),
            rendering_control_url: renderer.rendering_control_url.clone(),
            rendering_control_service_type: renderer.rendering_control_service_type.clone(),
        });
    }

    let xml = http_get(&renderer.location)?;
    let base = tag_text(&xml, "URLBase").unwrap_or_else(|| base_url(&renderer.location));
    let friendly_name = renderer
        .friendly_name
        .clone()
        .or_else(|| tag_text(&xml, "friendlyName"))
        .unwrap_or_else(|| {
            host_from_url(&renderer.location).unwrap_or_else(|| "DLNA Renderer".to_string())
        });
    let control_path = find_service_control_url(&xml, "AVTransport")
        .ok_or_else(|| "renderer description does not expose AVTransport service".to_string())?;
    let control_url = absolutize_url(&base, &renderer.location, &control_path);

    Ok(RendererControl {
        name: stream_target_name(
            Some(&friendly_name),
            renderer.service_name.as_deref(),
            renderer.location.as_str(),
        ),
        control_url,
        av_transport_service_type: renderer
            .service_type
            .clone()
            .unwrap_or_else(default_av_transport_service_type),
        rendering_control_url: renderer.rendering_control_url.clone(),
        rendering_control_service_type: renderer.rendering_control_service_type.clone(),
    })
}

fn default_av_transport_service_type() -> String {
    "urn:schemas-upnp-org:service:AVTransport:1".to_string()
}

fn stream_target_name(
    friendly_name: Option<&str>,
    service_name: Option<&str>,
    location: &str,
) -> String {
    let base_name = friendly_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| host_from_url(location).unwrap_or_else(|| "DLNA Renderer".to_string()));

    match service_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(service) => format!("{base_name} ({service})"),
        None => base_name,
    }
}

fn find_service_control_url(xml: &str, service_keyword: &str) -> Option<String> {
    let mut rest = xml;
    while let Some(start) = rest.find("<service") {
        rest = &rest[start..];
        let end = rest.find("</service>")?;
        let service = &rest[..end];
        if service
            .to_ascii_lowercase()
            .contains(&service_keyword.to_ascii_lowercase())
        {
            return tag_text(service, "controlURL");
        }
        rest = &rest[end + "</service>".len()..];
    }
    None
}

fn http_get(url: &str) -> Result<String, String> {
    let parts = parse_http_url(url)?;
    let mut stream = TcpStream::connect_timeout(&parts.addr, Duration::from_secs(4))
        .map_err(|err| format!("failed to connect to {}: {err}", parts.host))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .map_err(|err| format!("failed to configure HTTP GET read timeout: {err}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(4)))
        .map_err(|err| format!("failed to configure HTTP GET write timeout: {err}"))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: NARfW/0.1\r\nConnection: close\r\n\r\n",
        parts.path, parts.host_header
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("failed to send HTTP GET: {err}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| format!("failed to read HTTP response: {err}"))?;
    let status_ok = response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200");
    if !status_ok {
        let status = response.lines().next().unwrap_or("empty HTTP response");
        return Err(format!("device description HTTP request failed: {status}"));
    }
    Ok(response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or(response))
}

fn soap_action(
    control_url: &str,
    service_type: &str,
    action: &str,
    inner: &str,
) -> Result<(), String> {
    let parts = parse_http_url(control_url)?;
    let service_type = if service_type.trim().is_empty() {
        "urn:schemas-upnp-org:service:AVTransport:1"
    } else {
        service_type.trim()
    };
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:{action} xmlns:u="{service_type}">
      {inner}
    </u:{action}>
  </s:Body>
</s:Envelope>"#
    );

    let request = format!(
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: text/xml; charset=\"utf-8\"\r\n\
         SOAPAction: \"{}#{}\"\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        parts.path,
        parts.host_header,
        service_type,
        action,
        body.len(),
        body
    );
    let mut stream = TcpStream::connect_timeout(&parts.addr, Duration::from_secs(4))
        .map_err(|err| format!("failed to connect to DLNA control endpoint: {err}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .map_err(|err| format!("failed to configure DLNA {action} read timeout: {err}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(4)))
        .map_err(|err| format!("failed to configure DLNA {action} write timeout: {err}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("failed to send DLNA {action}: {err}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| format!("failed to read DLNA {action} response: {err}"))?;
    let ok = response.starts_with("HTTP/1.1 200")
        || response.starts_with("HTTP/1.0 200")
        || response.starts_with("HTTP/1.1 204")
        || response.starts_with("HTTP/1.0 204");
    if !ok {
        let status = response.lines().next().unwrap_or("empty SOAP response");
        return Err(format!("DLNA {action} failed: {status}"));
    }
    Ok(())
}

struct HttpUrl {
    host: String,
    host_header: String,
    path: String,
    addr: SocketAddr,
}

fn parse_http_url(url: &str) -> Result<HttpUrl, String> {
    let Some(rest) = url.strip_prefix("http://") else {
        return Err(format!("only http:// URLs are supported: {url}"));
    };
    let (authority, path) = rest
        .split_once('/')
        .map(|(authority, path)| (authority, format!("/{path}")))
        .unwrap_or((rest, "/".to_string()));
    let (host, port) = authority
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host, port)))
        .unwrap_or((authority, 80));
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|err| format!("failed to resolve {authority}: {err}"))?
        .next()
        .ok_or_else(|| format!("no socket address found for {authority}"))?;

    Ok(HttpUrl {
        host: host.to_string(),
        host_header: authority.to_string(),
        path,
        addr,
    })
}

fn base_url(location: &str) -> String {
    let Some(rest) = location.strip_prefix("http://") else {
        return location.to_string();
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    format!("http://{authority}/")
}

fn absolutize_url(url_base: &str, location: &str, value: &str) -> String {
    if value.starts_with("http://") {
        return value.to_string();
    }

    let base = if !url_base.trim().is_empty() {
        url_base.trim().to_string()
    } else {
        base_url(location)
    };
    if value.starts_with('/') {
        let Some(rest) = base.strip_prefix("http://") else {
            return format!("{base}{value}");
        };
        let authority = rest.split('/').next().unwrap_or(rest);
        format!("http://{authority}{value}")
    } else if base.ends_with('/') {
        format!("{base}{value}")
    } else {
        format!("{base}/{value}")
    }
}

fn tag_text(xml: &str, tag: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let open = format!("<{}", tag.to_ascii_lowercase());
    let start = lower.find(&open)?;
    let after_open = lower[start..].find('>')? + start + 1;
    let close = format!("</{}>", tag.to_ascii_lowercase());
    let end = lower[after_open..].find(&close)? + after_open;
    Some(xml[after_open..end].trim().to_string())
}

fn host_from_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    Some(
        authority
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(authority)
            .to_string(),
    )
}

fn host_from_remote(remote: &str) -> Option<String> {
    Some(
        remote
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(remote)
            .to_string(),
    )
}

fn local_ip_for(remote_host: &str) -> Result<IpAddr, String> {
    let remote = (remote_host, 9)
        .to_socket_addrs()
        .map_err(|err| format!("failed to resolve renderer host {remote_host}: {err}"))?
        .next()
        .ok_or_else(|| format!("no address resolved for renderer host {remote_host}"))?;
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|err| format!("failed to bind local address probe socket: {err}"))?;
    socket
        .connect(remote)
        .map_err(|err| format!("failed to probe local route to renderer: {err}"))?;
    Ok(socket
        .local_addr()
        .map_err(|err| format!("failed to read local route address: {err}"))?
        .ip())
}

fn didl_metadata(stream_url: &str, format: DlnaStreamFormat) -> String {
    format!(
        r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/">
<item id="narfw-stream" parentID="0" restricted="1">
<dc:title>Windows System Audio</dc:title>
<upnp:class>object.item.audioItem.musicTrack</upnp:class>
<res protocolInfo="{}">{}</res>
</item>
</DIDL-Lite>"#,
        format.protocol_info(),
        xml_escape(stream_url)
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_avtransport_control_url() {
        let xml = r#"
          <root><device><friendlyName>Living TV</friendlyName><serviceList>
            <service>
              <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
              <controlURL>/rc</controlURL>
            </service>
            <service>
              <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
              <controlURL>/upnp/control/avtransport1</controlURL>
            </service>
          </serviceList></device></root>
        "#;
        assert_eq!(
            find_service_control_url(xml, "AVTransport").unwrap(),
            "/upnp/control/avtransport1"
        );
    }

    #[test]
    fn resolves_relative_control_url() {
        assert_eq!(
            absolutize_url(
                "",
                "http://192.168.1.20:1400/xml/device.xml",
                "/MediaRenderer/AVTransport/Control"
            ),
            "http://192.168.1.20:1400/MediaRenderer/AVTransport/Control"
        );
    }
}
