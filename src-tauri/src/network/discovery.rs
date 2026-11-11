use std::{
    collections::{HashMap, HashSet},
    io::{ErrorKind, Read, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs, UdpSocket},
    thread,
    time::{Duration, Instant},
};

use if_addrs::{get_if_addrs, IfAddr};
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};

const SSDP_ADDR: &str = "239.255.255.250:1900";
const MEDIA_RENDERER_ST: &str = "urn:schemas-upnp-org:device:MediaRenderer:1";
const RENDERER_SEARCH_TARGETS: &[&str] = &[
    MEDIA_RENDERER_ST,
    "urn:schemas-upnp-org:device:MediaRenderer:2",
    "urn:schemas-upnp-org:service:AVTransport:1",
    "urn:schemas-upnp-org:service:AVTransport:2",
];
const BROAD_SEARCH_TARGETS: &[&str] = &[
    "ssdp:all",
    "upnp:rootdevice",
    "urn:dial-multiscreen-org:service:dial:1",
    MEDIA_RENDERER_ST,
    "urn:schemas-upnp-org:device:MediaRenderer:2",
    "urn:schemas-upnp-org:service:AVTransport:1",
    "urn:schemas-upnp-org:service:AVTransport:2",
];
const MDNS_SERVICE_TYPES: &[&str] = &[
    "_googlecast._tcp.local.",
    "_airplay._tcp.local.",
    "_raop._tcp.local.",
];
const DEFAULT_TIMEOUT_MS: u64 = 3_000;
const MIN_TIMEOUT_MS: u64 = 250;
const MAX_TIMEOUT_MS: u64 = 10_000;
const DESCRIPTION_TIMEOUT_MS: u64 = 1_500;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveredRenderer {
    pub location: String,
    pub st: String,
    pub usn: String,
    pub server: String,
    pub remote_addr: String,
    #[serde(default)]
    pub friendly_name: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub control_url: Option<String>,
    #[serde(default)]
    pub rendering_control_url: Option<String>,
    #[serde(default)]
    pub rendering_control_service_type: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub service_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodecCapability {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub implemented: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub transport: String,
    pub host: String,
    pub port: Option<u16>,
    pub detail: String,
    pub supported: bool,
    pub codecs: Vec<CodecCapability>,
    pub renderer: Option<DiscoveredRenderer>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkDevice {
    pub id: String,
    pub name: String,
    pub host: String,
    pub detail: String,
    pub connections: Vec<ConnectionProfile>,
}

#[derive(Debug, Clone)]
struct DeviceDescription {
    friendly_name: Option<String>,
    model_name: Option<String>,
    device_type: Option<String>,
    base_url: String,
    services: Vec<ServiceDescription>,
    sink_protocol_info: Vec<String>,
}

#[derive(Debug, Clone)]
struct ServiceDescription {
    service_type: String,
    service_id: Option<String>,
    control_url: Option<String>,
}

pub fn discover_renderers(timeout_ms: Option<u64>) -> Result<Vec<DiscoveredRenderer>, String> {
    discover_ssdp(RENDERER_SEARCH_TARGETS, timeout_ms)
}

pub fn discover_network_devices(timeout_ms: Option<u64>) -> Result<Vec<NetworkDevice>, String> {
    let timeout_ms = timeout_ms
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);

    let renderer_thread =
        thread::spawn(move || discover_ssdp(RENDERER_SEARCH_TARGETS, Some(timeout_ms)));
    let ssdp_thread = thread::spawn(move || discover_ssdp(BROAD_SEARCH_TARGETS, Some(timeout_ms)));
    let mdns_thread = thread::spawn(move || discover_mdns_devices(Some(timeout_ms)));

    let mut ssdp_renderers = Vec::new();
    let mut connections = Vec::new();
    let mut seen = HashSet::new();
    let mut errors = Vec::new();

    match renderer_thread.join() {
        Ok(Ok(renderer_devices)) => {
            ssdp_renderers.extend(renderer_devices);
        }
        Ok(Err(err)) => errors.push(err),
        Err(_) => errors.push("DLNA renderer discovery thread panicked".to_string()),
    }

    match ssdp_thread.join() {
        Ok(Ok(ssdp_devices)) => {
            ssdp_renderers.extend(ssdp_devices);
        }
        Ok(Err(err)) => errors.push(err),
        Err(_) => errors.push("SSDP discovery thread panicked".to_string()),
    }

    let descriptions = fetch_device_descriptions(&ssdp_renderers);
    for device in ssdp_renderers {
        for connection in ssdp_to_connection_profiles(&device, descriptions.get(&device.location)) {
            add_unique_connection(connection, &mut connections, &mut seen);
        }
    }

    match mdns_thread.join() {
        Ok(Ok(mdns_devices)) => {
            for device in mdns_devices {
                add_unique_connection(device, &mut connections, &mut seen);
            }
        }
        Ok(Err(err)) => errors.push(err),
        Err(_) => errors.push("mDNS discovery thread panicked".to_string()),
    }

    let devices = group_connections(connections);

    if devices.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }

    Ok(devices)
}

fn discover_ssdp(
    targets: &[&str],
    timeout_ms: Option<u64>,
) -> Result<Vec<DiscoveredRenderer>, String> {
    let timeout = Duration::from_millis(
        timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS),
    );
    let deadline = Instant::now() + timeout;
    let sockets = discovery_sockets()?;

    if send_searches(&sockets, targets) == 0 {
        return Err("failed to send SSDP M-SEARCH on any network interface".to_string());
    }

    let mut renderers = Vec::new();
    let mut seen_usns = HashSet::new();
    let mut seen_locations = HashSet::new();
    let mut buffer = [0u8; 8192];

    while Instant::now() < deadline {
        let mut received_any = false;

        for socket in &sockets {
            loop {
                match socket.recv_from(&mut buffer) {
                    Ok((len, addr)) => {
                        received_any = true;
                        let Some(renderer) = parse_response(&buffer[..len], addr) else {
                            continue;
                        };
                        add_unique_renderer(
                            renderer,
                            &mut renderers,
                            &mut seen_usns,
                            &mut seen_locations,
                        );
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }

        if !received_any {
            thread::sleep(Duration::from_millis(30));
        }
    }

    Ok(renderers)
}

fn discover_mdns_devices(timeout_ms: Option<u64>) -> Result<Vec<ConnectionProfile>, String> {
    let timeout = Duration::from_millis(
        timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS),
    );
    let deadline = Instant::now() + timeout;
    let mdns =
        ServiceDaemon::new().map_err(|err| format!("failed to start mDNS discovery: {err}"))?;
    let mut receivers = Vec::new();

    for service_type in MDNS_SERVICE_TYPES {
        if let Ok(receiver) = mdns.browse(service_type) {
            receivers.push(receiver);
        }
    }

    let mut devices = Vec::new();
    let mut seen = HashSet::new();

    while Instant::now() < deadline {
        let mut received_any = false;
        for receiver in &receivers {
            loop {
                match receiver.try_recv() {
                    Ok(ServiceEvent::ServiceResolved(service)) => {
                        received_any = true;
                        if let Some(connection) = mdns_to_connection_profile(&service) {
                            add_unique_connection(connection, &mut devices, &mut seen);
                        }
                    }
                    Ok(_) => {
                        received_any = true;
                    }
                    Err(_) => break,
                }
            }
        }

        if !received_any {
            thread::sleep(Duration::from_millis(30));
        }
    }

    let _ = mdns.shutdown();
    Ok(devices)
}

fn discovery_sockets() -> Result<Vec<UdpSocket>, String> {
    let mut sockets = Vec::new();
    let mut seen_ips = HashSet::new();

    if let Ok(interfaces) = get_if_addrs() {
        for interface in interfaces {
            if interface.is_loopback() {
                continue;
            }

            let IfAddr::V4(addr) = interface.addr else {
                continue;
            };
            let ip = addr.ip;
            if !is_lan_ipv4(ip) || !seen_ips.insert(ip) {
                continue;
            }

            if let Ok(socket) = bind_socket(SocketAddrV4::new(ip, 0).into()) {
                sockets.push(socket);
            }
        }
    }

    if sockets.is_empty() {
        sockets.push(bind_socket(
            "0.0.0.0:0".parse().expect("valid fallback bind address"),
        )?);
    }

    Ok(sockets)
}

fn bind_socket(addr: SocketAddr) -> Result<UdpSocket, String> {
    let socket = UdpSocket::bind(addr)
        .map_err(|err| format!("failed to bind SSDP discovery socket: {err}"))?;
    socket
        .set_multicast_ttl_v4(4)
        .map_err(|err| format!("failed to configure SSDP multicast TTL: {err}"))?;
    socket
        .set_nonblocking(true)
        .map_err(|err| format!("failed to configure SSDP nonblocking socket: {err}"))?;
    Ok(socket)
}

fn is_lan_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || (octets[0] == 169 && octets[1] == 254))
}

fn send_searches(sockets: &[UdpSocket], targets: &[&str]) -> usize {
    let mut sent = 0;
    for socket in sockets {
        for target in targets {
            let request = search_request(target);
            if socket.send_to(request.as_bytes(), SSDP_ADDR).is_ok() {
                sent += 1;
            }
        }
    }
    sent
}

fn search_request(target: &str) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\n\
         HOST: 239.255.255.250:1900\r\n\
         MAN: \"ssdp:discover\"\r\n\
         MX: 2\r\n\
         ST: {target}\r\n\
         USER-AGENT: Windows/10 UPnP/1.1 NARfW/0.1\r\n\
         \r\n"
    )
}

fn add_unique_connection(
    connection: ConnectionProfile,
    connections: &mut Vec<ConnectionProfile>,
    seen: &mut HashSet<String>,
) {
    let key = if connection.id.trim().is_empty() {
        format!(
            "{}|{}|{:?}|{}",
            connection.protocol.to_ascii_lowercase(),
            connection.host.to_ascii_lowercase(),
            connection.port,
            connection.detail.to_ascii_lowercase()
        )
    } else {
        connection.id.to_ascii_lowercase()
    };
    if seen.insert(key) {
        connections.push(connection);
    }
}

fn group_connections(mut connections: Vec<ConnectionProfile>) -> Vec<NetworkDevice> {
    connections.sort_by(|a, b| {
        device_key(&a.host)
            .cmp(&device_key(&b.host))
            .then_with(|| protocol_rank(&a.protocol).cmp(&protocol_rank(&b.protocol)))
            .then_with(|| a.detail.to_lowercase().cmp(&b.detail.to_lowercase()))
    });

    let mut devices = Vec::<NetworkDevice>::new();
    for connection in connections {
        let key = device_key(&connection.host);
        if let Some(device) = devices.iter_mut().find(|device| device.id == key) {
            let candidate_name = device_name_from_connection(&connection);
            if prefers_device_name(&device.name, &candidate_name, &device.host) {
                device.name = candidate_name;
            }
            device.connections.push(connection);
            device.connections.sort_by(|a, b| {
                protocol_rank(&a.protocol)
                    .cmp(&protocol_rank(&b.protocol))
                    .then_with(|| a.detail.to_lowercase().cmp(&b.detail.to_lowercase()))
            });
            continue;
        }

        devices.push(NetworkDevice {
            id: key,
            name: device_name_from_connection(&connection),
            host: connection.host.clone(),
            detail: "Multiple connection profiles may be available for this device.".to_string(),
            connections: vec![connection],
        });
    }

    devices.sort_by_key(|device| device.name.to_lowercase());
    devices
}

fn ssdp_to_connection_profiles(
    renderer: &DiscoveredRenderer,
    description: Option<&DeviceDescription>,
) -> Vec<ConnectionProfile> {
    let st = renderer.st.to_ascii_lowercase();
    let server = renderer.server.to_ascii_lowercase();
    let host = host_from_location(&renderer.location)
        .unwrap_or_else(|| host_from_remote(&renderer.remote_addr));
    let port = port_from_location(&renderer.location);
    let name = device_name_from_ssdp(renderer, description, &host);
    let model = description
        .and_then(|device| device.model_name.clone())
        .or_else(|| sanitize_server_name(&renderer.server));
    let is_dlna = st.contains("mediarenderer") || st.contains("avtransport");
    let is_chromecast = st.contains("dial-multiscreen") || server.contains("chromecast");
    let is_root_device = st.contains("upnp:rootdevice");
    let rendering_control = description.and_then(first_rendering_control);
    let mut profiles = Vec::new();

    if let Some(device) = description {
        let av_services = av_transport_services(device);
        for (index, service) in av_services.iter().enumerate() {
            let Some(control_url) = service
                .control_url
                .as_ref()
                .map(|value| absolutize_url(&device.base_url, &renderer.location, value))
            else {
                continue;
            };
            let service_name = service_label(service, index);
            let detail = format!(
                "{} | {}",
                service_name,
                model
                    .clone()
                    .unwrap_or_else(|| compact_url_label(&control_url))
            );
            profiles.push(ConnectionProfile {
                id: format!(
                    "dlna:{}:{}",
                    normalize_key(&host),
                    normalize_key(&control_url)
                ),
                name: name.clone(),
                protocol: "DLNA".to_string(),
                transport: "SSDP/UPnP".to_string(),
                host: host.clone(),
                port,
                detail,
                supported: true,
                codecs: dlna_codecs(true, description),
                renderer: Some(enriched_renderer(
                    renderer,
                    description,
                    Some(control_url),
                    rendering_control.as_ref().map(|control| control.0.clone()),
                    rendering_control.as_ref().map(|control| control.1.clone()),
                    Some(service_name),
                    Some(service.service_type.clone()),
                )),
            });
        }
    }

    if profiles.is_empty() && is_dlna {
        profiles.push(ConnectionProfile {
            id: format!(
                "dlna:{}:{}",
                normalize_key(&host),
                normalize_key(&renderer.location)
            ),
            name: name.clone(),
            protocol: "DLNA".to_string(),
            transport: "SSDP/UPnP".to_string(),
            host: host.clone(),
            port,
            detail: format!(
                "AVTransport | {}",
                model
                    .clone()
                    .unwrap_or_else(|| compact_url_label(&renderer.location))
            ),
            supported: true,
            codecs: dlna_codecs(true, description),
            renderer: Some(enriched_renderer(
                renderer,
                description,
                None,
                rendering_control.as_ref().map(|control| control.0.clone()),
                rendering_control.as_ref().map(|control| control.1.clone()),
                Some("AVTransport".to_string()),
                Some(renderer.st.clone()),
            )),
        });
    }

    if is_chromecast {
        let cast_ready = port.unwrap_or(8009) > 0;
        profiles.push(ConnectionProfile {
            id: format!("chromecast:{}", normalize_key(&host)),
            name: name.clone(),
            protocol: "CHROMECAST".to_string(),
            transport: "SSDP/DIAL".to_string(),
            host: host.clone(),
            port,
            detail: format!(
                "DIAL | {}",
                model
                    .clone()
                    .unwrap_or_else(|| compact_url_label(&renderer.location))
            ),
            supported: cast_ready,
            codecs: codecs_for_protocol("CHROMECAST", cast_ready),
            renderer: None,
        });
    }

    let should_expose_upnp = is_root_device
        || (!is_dlna && !is_chromecast)
        || description
            .and_then(|device| device.device_type.as_ref())
            .is_some_and(|device_type| {
                let device_type = device_type.to_ascii_lowercase();
                device_type.contains("rootdevice") || device_type.contains("mediarenderer")
            });

    if should_expose_upnp {
        let preferred_renderer = profiles
            .iter()
            .find(|profile| profile.protocol == "DLNA" && profile.renderer.is_some())
            .and_then(|profile| profile.renderer.clone());
        let upnp_supported = preferred_renderer
            .as_ref()
            .and_then(|renderer| renderer.control_url.as_ref())
            .is_some();
        let upnp_control_url = preferred_renderer
            .as_ref()
            .and_then(|renderer| renderer.control_url.as_deref())
            .unwrap_or(&renderer.location);
        let upnp_detail = preferred_renderer
            .as_ref()
            .and_then(|renderer| renderer.service_name.clone())
            .unwrap_or_else(|| {
                if upnp_supported {
                    "AVTransport".to_string()
                } else {
                    "Root device".to_string()
                }
            });

        profiles.push(ConnectionProfile {
            id: format!(
                "upnp:{}:{}",
                normalize_key(&host),
                normalize_key(upnp_control_url)
            ),
            name,
            protocol: "UPNP".to_string(),
            transport: "SSDP/UPnP".to_string(),
            host: host.clone(),
            port,
            detail: format!(
                "{} | {}",
                upnp_detail,
                model.unwrap_or_else(|| compact_url_label(&renderer.location))
            ),
            supported: upnp_supported,
            codecs: dlna_codecs(upnp_supported, description),
            renderer: preferred_renderer,
        });
    }

    if profiles.is_empty() {
        profiles.push(ConnectionProfile {
            id: format!(
                "upnp:{}:{}",
                normalize_key(&host),
                normalize_key(&renderer.location)
            ),
            name: device_name_from_ssdp(renderer, description, &host),
            protocol: "UPNP".to_string(),
            transport: "SSDP/UPnP".to_string(),
            host,
            port,
            detail: compact_url_label(&renderer.location),
            supported: false,
            codecs: codecs_for_protocol("UPNP", false),
            renderer: None,
        });
    }

    profiles
}

fn mdns_to_connection_profile(service: &ResolvedService) -> Option<ConnectionProfile> {
    let service_type = service.ty_domain.to_ascii_lowercase();
    let is_raop = service_type.contains("_raop");
    let protocol = if service_type.contains("_googlecast") {
        "CHROMECAST"
    } else if is_raop || service_type.contains("_airplay") {
        "AIRPLAY"
    } else {
        return None;
    };
    let host = service
        .get_addresses_v4()
        .into_iter()
        .next()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| service.get_hostname().trim_end_matches('.').to_string());
    let name = mdns_display_name(service).unwrap_or_else(|| instance_name(service.get_fullname()));
    let model = txt_value(service, "md")
        .or_else(|| txt_value(service, "model"))
        .or_else(|| txt_value(service, "am"))
        .unwrap_or_else(|| service.ty_domain.clone());
    let password_required = txt_value(service, "pw")
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value == "true" || value == "1" || value == "yes"
        })
        .unwrap_or(false);
    let airplay = airplay_capability(service, is_raop, password_required);
    let airplay_ready = protocol == "AIRPLAY" && airplay.stream_ready;
    let detail = if protocol == "AIRPLAY" {
        format!(
            "Bonjour | {} | {} | port {}{}",
            airplay.mode,
            model,
            service.get_port(),
            airplay.txt_detail
        )
    } else {
        format!("Bonjour | {} | port {}", model, service.get_port())
    };

    Some(ConnectionProfile {
        id: format!(
            "{}:{}:{}",
            protocol.to_ascii_lowercase(),
            if is_raop { "raop" } else { "mdns" },
            normalize_key(&host)
        ),
        name,
        protocol: protocol.to_string(),
        transport: "mDNS/Bonjour".to_string(),
        host,
        port: Some(service.get_port()),
        detail,
        supported: if protocol == "AIRPLAY" {
            airplay_ready
        } else {
            false
        },
        codecs: codecs_for_protocol(protocol, airplay_ready),
        renderer: None,
    })
}

struct AirplayCapability {
    stream_ready: bool,
    mode: &'static str,
    txt_detail: String,
}

fn airplay_capability(
    service: &ResolvedService,
    is_raop: bool,
    password_required: bool,
) -> AirplayCapability {
    let ft = txt_value(service, "ft").or_else(|| txt_value(service, "features"));
    let sf = txt_value(service, "sf").or_else(|| txt_value(service, "flags"));
    let pk = txt_value(service, "pk");
    let pi = txt_value(service, "pi").or_else(|| txt_value(service, "gid"));
    let vn = txt_value(service, "vn");
    let et = txt_value(service, "et");
    let ek = txt_value(service, "ek");
    let cn = txt_value(service, "cn");
    let protovers = txt_value(service, "protovers");
    let srcvers = txt_value(service, "srcvers").or_else(|| txt_value(service, "vs"));
    let has_homekit_or_ap2 = pk.is_some()
        || pi.is_some()
        || protovers
            .as_deref()
            .is_some_and(|value| value.trim() != "1.0")
        || vn
            .as_deref()
            .is_some_and(|value| value.trim() != "3" && value.trim() != "1")
        || ft
            .as_deref()
            .is_some_and(|value| value.contains(',') || feature_flag_has_ap2_bits(value))
        || sf.as_deref().is_some_and(|value| value.trim() != "0x0");
    let legacy_encryption_advertised = et
        .as_deref()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|part| part == "0" || part == "1")
        })
        .unwrap_or(true);
    let alac_advertised = cn
        .as_deref()
        .map(|value| value.split(',').map(str::trim).any(|part| part == "1"))
        .unwrap_or(true);
    let auth_setup_advertised = et
        .as_deref()
        .map(|value| value.split(',').map(str::trim).any(|part| part == "4"))
        .unwrap_or(false);

    let mode = if password_required {
        "RAOP password required"
    } else if has_homekit_or_ap2 && is_raop && auth_setup_advertised {
        "RAOP auth-setup audio"
    } else if has_homekit_or_ap2 && is_raop {
        "RAOP HomeKit-compatible audio"
    } else if has_homekit_or_ap2 {
        "AirPlay 2 control pending"
    } else if is_raop && !legacy_encryption_advertised {
        "RAOP unsupported encryption"
    } else if is_raop && !alac_advertised {
        "RAOP unsupported codec"
    } else if !is_raop {
        "AirPlay control"
    } else {
        "RAOP audio"
    };

    let stream_ready =
        !password_required && is_raop && legacy_encryption_advertised && alac_advertised;
    let mut txt = Vec::new();
    for (key, value) in [
        ("et", et),
        ("ek", ek),
        ("cn", cn),
        ("vn", vn),
        ("ft", ft),
        ("sf", sf),
        ("pk", pk.map(|_| "present".to_string())),
        ("pi", pi.map(|_| "present".to_string())),
        ("protovers", protovers),
        ("srcvers", srcvers),
    ] {
        if let Some(value) = value {
            txt.push(format!("{key}={value}"));
        }
    }

    AirplayCapability {
        stream_ready,
        mode,
        txt_detail: if txt.is_empty() {
            String::new()
        } else {
            format!(" | {}", txt.join(" "))
        },
    }
}

fn feature_flag_has_ap2_bits(value: &str) -> bool {
    parse_airplay_features(value).is_some_and(|features| {
        // AP2/HomeKit/CoreUtils/PTP/buffered-audio feature bits.
        [40u8, 41, 46, 47, 48, 51, 59, 60]
            .iter()
            .any(|bit| features & (1u64 << bit) != 0)
    })
}

fn parse_airplay_features(value: &str) -> Option<u64> {
    let parts = value
        .split(',')
        .map(|part| part.trim().trim_start_matches("0x"))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [single] => u64::from_str_radix(single, 16).ok(),
        [lo, hi] => {
            let lo = u64::from_str_radix(lo, 16).ok()?;
            let hi = u64::from_str_radix(hi, 16).ok()?;
            Some((hi << 32) | lo)
        }
        _ => None,
    }
}

fn device_key(host: &str) -> String {
    format!("device:{}", host.trim().to_ascii_lowercase())
}

fn device_name_from_connection(connection: &ConnectionProfile) -> String {
    if !connection.name.trim().is_empty() {
        return connection.name.clone();
    }

    if let Some(renderer) = &connection.renderer {
        if let Some(name) = renderer
            .friendly_name
            .as_ref()
            .filter(|name| !name.trim().is_empty())
        {
            return name.clone();
        }
        if !renderer.server.trim().is_empty() {
            return renderer.server.clone();
        }
        if let Some(label) = label_from_usn(&renderer.usn) {
            return label;
        }
    }

    connection.host.clone()
}

fn protocol_rank(protocol: &str) -> u8 {
    match protocol {
        "DLNA" => 0,
        "AIRPLAY" => 1,
        "CHROMECAST" => 2,
        "UPNP" => 3,
        _ => 9,
    }
}

fn codecs_for_protocol(protocol: &str, stream_ready: bool) -> Vec<CodecCapability> {
    match protocol {
        "DLNA" => dlna_codecs(stream_ready, None),
        "CHROMECAST" => vec![
            codec(
                "pcm_s16le",
                "WAV PCM",
                "48 kHz stereo PCM over local HTTP stream to Cast Default Media Receiver",
                stream_ready,
            ),
            codec(
                "flac",
                "FLAC",
                "Cast-capable devices often support FLAC; live encoder backend pending",
                false,
            ),
            codec(
                "opus",
                "Opus",
                "Low-latency Cast/WebRTC-style option; encoder backend pending",
                false,
            ),
            codec(
                "aac_lc",
                "AAC-LC",
                "Common Cast-compatible audio option; encoder backend pending",
                false,
            ),
            codec(
                "mp3",
                "MP3",
                "Legacy Cast-compatible fallback; backend pending",
                false,
            ),
        ],
        "AIRPLAY" => vec![
            codec(
                "alac",
                "ALAC",
                if stream_ready {
                    "44.1 kHz stereo Apple Lossless over AirPlay 1 RAOP"
                } else {
                    "AirPlay 2/FairPlay sender backend pending for this profile"
                },
                stream_ready,
            ),
            codec(
                "aac_lc",
                "AAC-LC",
                "AirPlay compressed option; backend pending",
                false,
            ),
            codec(
                "pcm_s16le",
                "PCM",
                "Uncompressed AirPlay-style path; backend pending",
                false,
            ),
        ],
        _ => vec![
            codec(
                "pcm_s16le",
                "PCM",
                "Raw PCM fallback; protocol backend pending",
                false,
            ),
            codec(
                "aac_lc",
                "AAC-LC",
                "Common network audio codec; backend pending",
                false,
            ),
        ],
    }
}

fn dlna_codecs(
    stream_ready: bool,
    description: Option<&DeviceDescription>,
) -> Vec<CodecCapability> {
    let sink = description
        .map(|device| device.sink_protocol_info.as_slice())
        .unwrap_or(&[]);
    let wav_supported =
        sink.is_empty() || sink_supports(sink, &["audio/wav", "audio/x-wav", "lpcm", "wav"]);
    let l16_supported = sink_supports(sink, &["audio/l16"]);
    let flac_supported = sink_supports(sink, &["audio/flac", "audio/x-flac", "flac"]);
    let aac_supported = sink_supports(sink, &["audio/aac", "audio/mp4", "aac", "mp4"]);
    let mp3_supported = sink_supports(sink, &["audio/mpeg", "mp3"]);

    vec![
        codec(
            "flac",
            "FLAC",
            if flac_supported {
                "Device advertises FLAC; live encoder backend pending"
            } else {
                "FLAC not advertised by this renderer"
            },
            false,
        ),
        codec(
            "pcm_l16be",
            "L16 PCM",
            if l16_supported {
                "Device advertises L16 PCM; live compatibility check pending"
            } else {
                "L16 PCM not advertised by this renderer"
            },
            false,
        ),
        codec(
            "pcm_s16le",
            "WAV PCM",
            if wav_supported {
                "48 kHz stereo PCM over HTTP/WAV"
            } else {
                "WAV/PCM not advertised, but may still work on permissive DLNA renderers"
            },
            stream_ready && wav_supported,
        ),
        codec(
            "aac_lc",
            "AAC-LC",
            if aac_supported {
                "Device advertises AAC; live encoder backend pending"
            } else {
                "AAC not advertised by this renderer"
            },
            false,
        ),
        codec(
            "mp3",
            "MP3",
            if mp3_supported {
                "Device advertises MP3; live encoder backend pending"
            } else {
                "MP3 not advertised by this renderer"
            },
            false,
        ),
    ]
}

fn sink_supports(sink_protocol_info: &[String], needles: &[&str]) -> bool {
    sink_protocol_info.iter().any(|entry| {
        let entry = entry.to_ascii_lowercase();
        needles.iter().any(|needle| entry.contains(needle))
    })
}

fn fetch_device_descriptions(
    renderers: &[DiscoveredRenderer],
) -> HashMap<String, DeviceDescription> {
    let mut locations = Vec::new();
    let mut seen = HashSet::new();
    for renderer in renderers {
        if seen.insert(renderer.location.clone()) {
            locations.push(renderer.location.clone());
        }
    }

    let mut workers = Vec::new();
    for location in locations {
        workers.push(thread::spawn(move || {
            let description = fetch_device_description(&location);
            (location, description)
        }));
    }

    let mut descriptions = HashMap::new();
    for worker in workers {
        if let Ok((location, Some(description))) = worker.join() {
            descriptions.insert(location, description);
        }
    }
    descriptions
}

fn fetch_device_description(location: &str) -> Option<DeviceDescription> {
    let xml = http_get(location).ok()?;
    let base_url = tag_text(&xml, "URLBase").unwrap_or_else(|| base_url(location));
    let services = collect_services(&xml);
    let sink_protocol_info = fetch_sink_protocol_info(&base_url, location, &services);
    Some(DeviceDescription {
        friendly_name: tag_text(&xml, "friendlyName"),
        model_name: tag_text(&xml, "modelName"),
        device_type: tag_text(&xml, "deviceType"),
        base_url,
        services,
        sink_protocol_info,
    })
}

fn fetch_sink_protocol_info(
    base_url: &str,
    location: &str,
    services: &[ServiceDescription],
) -> Vec<String> {
    let Some(service) = services.iter().find(|service| {
        service
            .service_type
            .to_ascii_lowercase()
            .contains("connectionmanager")
    }) else {
        return Vec::new();
    };
    let Some(control_path) = service.control_url.as_deref() else {
        return Vec::new();
    };
    let control_url = absolutize_url(base_url, location, control_path);
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:GetProtocolInfo xmlns:u="urn:schemas-upnp-org:service:ConnectionManager:1" />
  </s:Body>
</s:Envelope>"#;
    let response =
        http_post_soap(&control_url, &service.service_type, "GetProtocolInfo", body).ok();
    response
        .and_then(|response| tag_text(&response, "Sink"))
        .map(|sink| {
            xml_unescape(&sink)
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn collect_services(xml: &str) -> Vec<ServiceDescription> {
    let mut services = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<service") {
        rest = &rest[start..];
        let Some(end) = rest.find("</service>") else {
            break;
        };
        let service = &rest[..end];
        services.push(ServiceDescription {
            service_type: tag_text(service, "serviceType").unwrap_or_default(),
            service_id: tag_text(service, "serviceId"),
            control_url: tag_text(service, "controlURL"),
        });
        rest = &rest[end + "</service>".len()..];
    }
    services
}

fn av_transport_services(device: &DeviceDescription) -> Vec<ServiceDescription> {
    device
        .services
        .iter()
        .filter(|service| {
            service
                .service_type
                .to_ascii_lowercase()
                .contains("avtransport")
        })
        .cloned()
        .collect()
}

fn first_rendering_control(device: &DeviceDescription) -> Option<(String, String)> {
    device
        .services
        .iter()
        .find(|service| {
            service
                .service_type
                .to_ascii_lowercase()
                .contains("renderingcontrol")
        })
        .and_then(|service| {
            service.control_url.as_ref().map(|value| {
                (
                    absolutize_url(&device.base_url, "", value),
                    service.service_type.clone(),
                )
            })
        })
}

fn enriched_renderer(
    renderer: &DiscoveredRenderer,
    description: Option<&DeviceDescription>,
    control_url: Option<String>,
    rendering_control_url: Option<String>,
    rendering_control_service_type: Option<String>,
    service_name: Option<String>,
    service_type: Option<String>,
) -> DiscoveredRenderer {
    DiscoveredRenderer {
        location: renderer.location.clone(),
        st: renderer.st.clone(),
        usn: renderer.usn.clone(),
        server: renderer.server.clone(),
        remote_addr: renderer.remote_addr.clone(),
        friendly_name: description.and_then(|device| device.friendly_name.clone()),
        model_name: description.and_then(|device| device.model_name.clone()),
        control_url,
        rendering_control_url,
        rendering_control_service_type,
        service_name,
        service_type,
    }
}

fn device_name_from_ssdp(
    renderer: &DiscoveredRenderer,
    description: Option<&DeviceDescription>,
    host: &str,
) -> String {
    description
        .and_then(|device| device.friendly_name.clone())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| label_from_usn(&renderer.usn))
        .or_else(|| sanitize_server_name(&renderer.server))
        .unwrap_or_else(|| host.to_string())
}

fn prefers_device_name(current: &str, candidate: &str, host: &str) -> bool {
    name_quality(candidate, host) < name_quality(current, host)
}

fn name_quality(name: &str, host: &str) -> u8 {
    let normalized = name.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == host.to_ascii_lowercase() {
        return 5;
    }
    if normalized.starts_with("uuid:") {
        return 4;
    }
    if normalized.contains("upnp/")
        || normalized.contains("bonjour")
        || normalized.contains("linux/")
        || normalized.contains("windows/")
        || normalized.contains("http/")
    {
        return 3;
    }
    if normalized
        .chars()
        .all(|ch| ch.is_ascii_hexdigit() || ch == '-' || ch == '_')
    {
        return 2;
    }
    0
}

fn sanitize_server_name(server: &str) -> Option<String> {
    let trimmed = server.trim();
    (!trimmed.is_empty() && name_quality(trimmed, "") < 3).then(|| trimmed.to_string())
}

fn service_label(service: &ServiceDescription, index: usize) -> String {
    let version = service
        .service_type
        .rsplit(':')
        .next()
        .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))
        .map(|value| format!("v{value}"));
    let suffix = service
        .service_id
        .as_deref()
        .and_then(service_id_suffix)
        .or(version)
        .unwrap_or_else(|| format!("#{}", index + 1));
    format!("AVTransport {suffix}")
}

fn service_id_suffix(service_id: &str) -> Option<String> {
    service_id
        .trim()
        .rsplit([':', '/'])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn compact_url_label(url: &str) -> String {
    let Some(rest) = url.strip_prefix("http://") else {
        return url.to_string();
    };
    let (_, path) = rest
        .split_once('/')
        .map(|(authority, path)| (authority, format!("/{path}")))
        .unwrap_or((rest, "/".to_string()));
    path
}

fn codec(id: &str, label: &str, detail: &str, implemented: bool) -> CodecCapability {
    CodecCapability {
        id: id.to_string(),
        label: label.to_string(),
        detail: detail.to_string(),
        implemented,
    }
}

fn add_unique_renderer(
    renderer: DiscoveredRenderer,
    renderers: &mut Vec<DiscoveredRenderer>,
    seen_usns: &mut HashSet<String>,
    seen_locations: &mut HashSet<String>,
) {
    let normalized_usn = normalize_key(&renderer.usn);
    let normalized_location = normalize_key(&format!("{}|{}", renderer.location, renderer.st));

    let duplicate_usn = !normalized_usn.is_empty() && seen_usns.contains(&normalized_usn);
    let duplicate_location =
        !normalized_location.is_empty() && seen_locations.contains(&normalized_location);
    if duplicate_usn || duplicate_location {
        return;
    }

    if !normalized_usn.is_empty() {
        seen_usns.insert(normalized_usn);
    }
    if !normalized_location.is_empty() {
        seen_locations.insert(normalized_location);
    }
    renderers.push(renderer);
}

fn parse_response(bytes: &[u8], addr: SocketAddr) -> Option<DiscoveredRenderer> {
    let response = String::from_utf8_lossy(bytes);
    let mut location = String::new();
    let mut st = String::new();
    let mut usn = String::new();
    let mut server = String::new();

    for line in response.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match name.trim().to_ascii_lowercase().as_str() {
            "location" => location = value,
            "st" => st = value,
            "usn" => usn = value,
            "server" => server = value,
            _ => {}
        }
    }

    if location.is_empty() {
        return None;
    }

    Some(DiscoveredRenderer {
        location,
        st,
        usn,
        server,
        remote_addr: addr.to_string(),
        friendly_name: None,
        model_name: None,
        control_url: None,
        rendering_control_url: None,
        rendering_control_service_type: None,
        service_name: None,
        service_type: None,
    })
}

fn http_get(url: &str) -> Result<String, String> {
    let parts = parse_http_url(url)?;
    let mut stream =
        TcpStream::connect_timeout(&parts.addr, Duration::from_millis(DESCRIPTION_TIMEOUT_MS))
            .map_err(|err| format!("failed to connect to {}: {err}", parts.host))?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(DESCRIPTION_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(DESCRIPTION_TIMEOUT_MS)));
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

fn http_post_soap(
    url: &str,
    service_type: &str,
    action: &str,
    body: &str,
) -> Result<String, String> {
    let parts = parse_http_url(url)?;
    let mut stream =
        TcpStream::connect_timeout(&parts.addr, Duration::from_millis(DESCRIPTION_TIMEOUT_MS))
            .map_err(|err| format!("failed to connect to {}: {err}", parts.host))?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(DESCRIPTION_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(DESCRIPTION_TIMEOUT_MS)));
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
        service_type.trim(),
        action,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("failed to send SOAP {action}: {err}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| format!("failed to read SOAP {action} response: {err}"))?;
    let status_ok = response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200");
    if !status_ok {
        let status = response.lines().next().unwrap_or("empty SOAP response");
        return Err(format!("SOAP {action} failed: {status}"));
    }
    Ok(response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or(response))
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

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn host_from_remote(remote: &str) -> String {
    remote
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(remote)
        .to_string()
}

fn host_from_location(location: &str) -> Option<String> {
    let rest = location.strip_prefix("http://")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    Some(
        authority
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(authority)
            .to_string(),
    )
}

fn port_from_location(location: &str) -> Option<u16> {
    let rest = location.strip_prefix("http://")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    authority
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
}

fn label_from_usn(usn: &str) -> Option<String> {
    let cleaned = usn
        .trim()
        .trim_start_matches("uuid:")
        .split("::")
        .next()
        .unwrap_or("")
        .trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

fn instance_name(fullname: &str) -> String {
    let instance = fullname.split("._").next().unwrap_or(fullname).trim();
    instance
        .rsplit_once('@')
        .map(|(_, name)| name)
        .unwrap_or(instance)
        .replace("\\.", ".")
}

fn mdns_display_name(service: &ResolvedService) -> Option<String> {
    txt_value(service, "fn")
        .or_else(|| txt_value(service, "name"))
        .or_else(|| {
            txt_value(service, "manufacturer")
                .zip(txt_value(service, "model"))
                .map(|(m, model)| format!("{m} {model}"))
        })
}

fn txt_value(service: &ResolvedService, key: &str) -> Option<String> {
    service
        .get_property_val_str(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssdp_response_headers_case_insensitively() {
        let response = b"HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.2/root.xml\r\nST: urn:schemas-upnp-org:device:MediaRenderer:1\r\nUSN: uuid:device::urn:schemas-upnp-org:device:MediaRenderer:1\r\nSERVER: Windows UPnP/1.1 Renderer/1.0\r\n\r\n";

        let renderer =
            parse_response(response, "192.168.1.2:1900".parse::<SocketAddr>().unwrap()).unwrap();

        assert_eq!(renderer.location, "http://192.168.1.2/root.xml");
        assert_eq!(renderer.st, MEDIA_RENDERER_ST);
        assert_eq!(
            renderer.usn,
            "uuid:device::urn:schemas-upnp-org:device:MediaRenderer:1"
        );
        assert_eq!(renderer.server, "Windows UPnP/1.1 Renderer/1.0");
        assert_eq!(renderer.remote_addr, "192.168.1.2:1900");
    }

    #[test]
    fn ignores_responses_without_location() {
        let response =
            b"HTTP/1.1 200 OK\r\nST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n\r\n";

        assert!(parse_response(response, "192.168.1.2:1900".parse().unwrap()).is_none());
    }

    #[test]
    fn builds_search_request_for_each_target() {
        let request = search_request("urn:test");

        assert!(request.contains("M-SEARCH * HTTP/1.1"));
        assert!(request.contains("ST: urn:test"));
        assert!(request.contains("MAN: \"ssdp:discover\""));
    }

    #[test]
    fn keeps_rootdevice_and_mediarenderer_with_same_location() {
        let mut renderers = Vec::new();
        let mut seen_usns = HashSet::new();
        let mut seen_locations = HashSet::new();
        let location = "http://192.168.1.20:8008/ssdp/device-desc.xml";

        add_unique_renderer(
            DiscoveredRenderer {
                location: location.to_string(),
                st: "upnp:rootdevice".to_string(),
                usn: "uuid:device::upnp:rootdevice".to_string(),
                server: "test".to_string(),
                remote_addr: "192.168.1.20:1900".to_string(),
                friendly_name: None,
                model_name: None,
                control_url: None,
                rendering_control_url: None,
                rendering_control_service_type: None,
                service_name: None,
                service_type: None,
            },
            &mut renderers,
            &mut seen_usns,
            &mut seen_locations,
        );
        add_unique_renderer(
            DiscoveredRenderer {
                location: location.to_string(),
                st: MEDIA_RENDERER_ST.to_string(),
                usn: format!("uuid:device::{MEDIA_RENDERER_ST}"),
                server: "test".to_string(),
                remote_addr: "192.168.1.20:1900".to_string(),
                friendly_name: None,
                model_name: None,
                control_url: None,
                rendering_control_url: None,
                rendering_control_service_type: None,
                service_name: None,
                service_type: None,
            },
            &mut renderers,
            &mut seen_usns,
            &mut seen_locations,
        );

        assert_eq!(renderers.len(), 2);
        assert!(renderers
            .iter()
            .any(|renderer| renderer.st == MEDIA_RENDERER_ST));
    }

    #[test]
    fn strips_mdns_instance_name() {
        assert_eq!(
            instance_name("AA1122334455@Living Room._raop._tcp.local."),
            "Living Room"
        );
        assert_eq!(
            instance_name("Kitchen\\032TV._googlecast._tcp.local."),
            "Kitchen\\032TV"
        );
    }

    #[test]
    fn parses_airplay_feature_pairs() {
        assert_eq!(
            parse_airplay_features("0x405D4A00,0x1C340"),
            Some(0x0001_C340_405D_4A00)
        );
        assert_eq!(parse_airplay_features("0x7"), Some(0x7));
    }

    #[test]
    fn detects_airplay2_feature_bits() {
        assert!(feature_flag_has_ap2_bits("0x405D4A00,0x1C340"));
        assert!(!feature_flag_has_ap2_bits("0x7"));
    }

    #[test]
    fn prefers_friendly_name_over_generic_server_string() {
        let devices = group_connections(vec![
            ConnectionProfile {
                id: "upnp:denon".to_string(),
                name: "LINUX UPnP/1.0 Denon-Heos/1.571.160".to_string(),
                protocol: "UPNP".to_string(),
                transport: "SSDP/UPnP".to_string(),
                host: "192.168.1.50".to_string(),
                port: Some(60006),
                detail: "Root device | /upnp/desc.xml".to_string(),
                supported: false,
                codecs: Vec::new(),
                renderer: None,
            },
            ConnectionProfile {
                id: "dlna:denon".to_string(),
                name: "kostya".to_string(),
                protocol: "DLNA".to_string(),
                transport: "SSDP/UPnP".to_string(),
                host: "192.168.1.50".to_string(),
                port: Some(60006),
                detail: "AVTransport #1 | Denon HEOS".to_string(),
                supported: true,
                codecs: Vec::new(),
                renderer: None,
            },
        ]);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "kostya");
    }

    #[test]
    fn builds_distinct_dlna_profiles_for_each_avtransport_service() {
        let renderer = DiscoveredRenderer {
            location: "http://192.168.1.50:60006/upnp/desc.xml".to_string(),
            st: MEDIA_RENDERER_ST.to_string(),
            usn: "uuid:device::urn:schemas-upnp-org:device:MediaRenderer:1".to_string(),
            server: "LINUX UPnP/1.0 Denon-Heos/1.571.160".to_string(),
            remote_addr: "192.168.1.50:1900".to_string(),
            friendly_name: None,
            model_name: None,
            control_url: None,
            rendering_control_url: None,
            rendering_control_service_type: None,
            service_name: None,
            service_type: None,
        };
        let description = DeviceDescription {
            friendly_name: Some("kostya".to_string()),
            model_name: Some("Denon HEOS".to_string()),
            device_type: Some(MEDIA_RENDERER_ST.to_string()),
            base_url: "http://192.168.1.50:60006/".to_string(),
            services: vec![
                ServiceDescription {
                    service_type: "urn:schemas-upnp-org:service:AVTransport:1".to_string(),
                    service_id: Some("urn:upnp-org:serviceId:AVTransport".to_string()),
                    control_url: Some("/upnp/control/avtransport1".to_string()),
                },
                ServiceDescription {
                    service_type: "urn:schemas-upnp-org:service:AVTransport:2".to_string(),
                    service_id: Some("urn:upnp-org:serviceId:AVTransport2".to_string()),
                    control_url: Some("/upnp/control/avtransport2".to_string()),
                },
            ],
            sink_protocol_info: Vec::new(),
        };

        let profiles = ssdp_to_connection_profiles(&renderer, Some(&description));
        let dlna_profiles = profiles
            .iter()
            .filter(|profile| profile.protocol == "DLNA")
            .collect::<Vec<_>>();

        assert_eq!(dlna_profiles.len(), 2);
        assert_eq!(dlna_profiles[0].name, "kostya");
        assert_ne!(dlna_profiles[0].id, dlna_profiles[1].id);
        assert!(
            dlna_profiles.iter().all(|profile| profile.supported),
            "all AVTransport-backed profiles should be ready"
        );
    }

    #[test]
    fn exposes_ready_upnp_profile_when_root_device_has_avtransport() {
        let renderer = DiscoveredRenderer {
            location: "http://192.168.1.60:1400/xml/device.xml".to_string(),
            st: "upnp:rootdevice".to_string(),
            usn: "uuid:device::upnp:rootdevice".to_string(),
            server: "UPnP/1.0 Renderer".to_string(),
            remote_addr: "192.168.1.60:1900".to_string(),
            friendly_name: None,
            model_name: None,
            control_url: None,
            rendering_control_url: None,
            rendering_control_service_type: None,
            service_name: None,
            service_type: None,
        };
        let description = DeviceDescription {
            friendly_name: Some("Living Room".to_string()),
            model_name: Some("UPnP Renderer".to_string()),
            device_type: Some("urn:schemas-upnp-org:device:MediaRenderer:1".to_string()),
            base_url: "http://192.168.1.60:1400/".to_string(),
            services: vec![ServiceDescription {
                service_type: "urn:schemas-upnp-org:service:AVTransport:1".to_string(),
                service_id: Some("urn:upnp-org:serviceId:AVTransport".to_string()),
                control_url: Some("/MediaRenderer/AVTransport/Control".to_string()),
            }],
            sink_protocol_info: vec!["http-get:*:audio/wav:DLNA.ORG_PN=WAV".to_string()],
        };

        let profiles = ssdp_to_connection_profiles(&renderer, Some(&description));
        let upnp = profiles
            .iter()
            .find(|profile| profile.protocol == "UPNP")
            .expect("ready UPnP profile");

        assert!(upnp.supported);
        assert!(upnp
            .renderer
            .as_ref()
            .and_then(|renderer| renderer.control_url.as_ref())
            .is_some());
        assert!(upnp
            .codecs
            .iter()
            .any(|codec| codec.id == "pcm_s16le" && codec.implemented));
    }
}
