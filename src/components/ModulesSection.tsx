import { useState } from "react";

const modules = [
  {
    id: "wasapi",
    title: "Audio Capture",
    icon: "🎙️",
    color: "text-purple-400",
    bg: "bg-purple-950/20 border-purple-500/30",
    tag: "audio/",
    files: ["capture.rs", "mixer.rs", "ring_buffer.rs", "resampler.rs"],
    description: "WASAPI Loopback capture layer. Reads audio frames directly from the Windows audio engine at the kernel buffer level.",
    details: [
      { key: "Sample Rate", value: "48,000 Hz" },
      { key: "Channels", value: "2 (Stereo)" },
      { key: "Format", value: "Float32 PCM" },
      { key: "API", value: "Windows WASAPI" },
      { key: "Mode", value: "Shared Loopback" },
      { key: "Buffer", value: "10–20 ms kernel" },
    ],
    code: `// WASAPI Loopback capture
let device = get_default_output_device()?;
let stream = device.build_input_stream(
    &config,
    move |data: &[f32], _| {
        ring_buffer.write(data);
    },
    err_fn,
)?;
stream.play()?;`,
  },
  {
    id: "bus",
    title: "Internal Audio Bus",
    icon: "🔀",
    color: "text-cyan-400",
    bg: "bg-cyan-950/20 border-cyan-500/30",
    tag: "audio/",
    files: ["ring_buffer.rs", "mixer.rs"],
    description: "Lock-free multi-producer, multi-consumer ring buffer. Distributes audio frames to all registered output modules without copying.",
    details: [
      { key: "Type", value: "SPMC Ring Buffer" },
      { key: "Latency", value: "< 1 ms fan-out" },
      { key: "Consumers", value: "Unlimited" },
      { key: "Copy", value: "Zero-copy slices" },
      { key: "Threading", value: "Lock-free" },
      { key: "Crate", value: "ringbuf" },
    ],
    code: `// Fan-out to all output subscribers
let bus = AudioBus::new(RING_SIZE);

// Each module gets its own consumer
let rtp_rx = bus.subscribe();
let dlna_rx = bus.subscribe();
let airplay_rx = bus.subscribe();

// Single write, multiple reads
bus.write(&pcm_frames);`,
  },
  {
    id: "rtp",
    title: "RTP Transport",
    icon: "📡",
    color: "text-yellow-400",
    bg: "bg-yellow-950/20 border-yellow-500/30",
    tag: "network/",
    files: ["rtp.rs", "udp.rs"],
    description: "Real-Time Protocol over UDP. MVP transport module. Encapsulates Opus-encoded frames in RTP packets and sends via UDP unicast or multicast.",
    details: [
      { key: "Protocol", value: "RTP (RFC 3550)" },
      { key: "Transport", value: "UDP" },
      { key: "Port", value: "5000 (default)" },
      { key: "Codec", value: "Opus" },
      { key: "Buffer", value: "50–200 ms" },
      { key: "Mode", value: "Unicast / Multicast" },
    ],
    code: `// RTP packet construction
let rtp_packet = RtpPacket {
    version: 2,
    payload_type: 111, // Opus dynamic PT
    sequence_number: seq.fetch_add(1),
    timestamp: audio_ts,
    ssrc: session_id,
    payload: opus_frame,
};
udp_socket.send_to(&rtp_packet.serialize(), target)?;`,
  },
  {
    id: "dlna",
    title: "DLNA Module",
    icon: "📺",
    color: "text-pink-400",
    bg: "bg-pink-950/20 border-pink-500/30",
    tag: "outputs/",
    files: ["dlna.rs"],
    description: "UPnP AV / DLNA Media Renderer controller. Discovers renderers via SSDP and controls playback using UPnP action commands.",
    details: [
      { key: "Discovery", value: "SSDP Multicast" },
      { key: "Address", value: "239.255.255.250:1900" },
      { key: "Protocol", value: "UPnP AV 1.0" },
      { key: "Commands", value: "Play/Pause/Stop/Vol" },
      { key: "Codecs", value: "PCM · MP3 · AAC" },
      { key: "Phase", value: "Phase 2" },
    ],
    code: `// DLNA SetAVTransportURI + Play
let action = UPnPAction::new("SetAVTransportURI")
    .arg("InstanceID", "0")
    .arg("CurrentURI", &stream_url)
    .arg("CurrentURIMetaData", &didl_metadata);

renderer.send_action(&action).await?;
renderer.play().await?;`,
  },
  {
    id: "discovery",
    title: "Discovery Engine",
    icon: "🔍",
    color: "text-green-400",
    bg: "bg-green-950/20 border-green-500/30",
    tag: "network/",
    files: ["ssdp.rs", "mdns.rs"],
    description: "Dual-protocol device discovery. SSDP for DLNA/UPnP devices, mDNS for AirPlay and Chromecast. Continuously monitors the LAN for new devices.",
    details: [
      { key: "SSDP", value: "239.255.255.250:1900" },
      { key: "mDNS", value: "224.0.0.251:5353" },
      { key: "Refresh", value: "Every 30s" },
      { key: "Protocol", value: "IPv4 / IPv6" },
      { key: "Crate", value: "mdns-sd, ssdp-client" },
      { key: "Output", value: "DeviceCapabilities" },
    ],
    code: `// mDNS AirPlay discovery
let mdns = ServiceDaemon::new()?;
let receiver = mdns.browse("_airplay._tcp.local.")?;

while let Ok(event) = receiver.recv() {
    if let ServiceEvent::ServiceResolved(info) = event {
        devices.insert(Device::from_mdns(info));
    }
}`,
  },
  {
    id: "devices",
    title: "Device Database",
    icon: "🗄️",
    color: "text-blue-400",
    bg: "bg-blue-950/20 border-blue-500/30",
    tag: "devices/",
    files: ["device.rs", "capabilities.rs"],
    description: "Persistent device registry with capability profiles. Each device stores its supported protocols, codecs, and current status.",
    details: [
      { key: "Storage", value: "In-memory + JSON" },
      { key: "Fields", value: "id, name, ip, model" },
      { key: "Caps", value: "PCM/MP3/AAC/Opus" },
      { key: "Status", value: "Online/Offline/Playing" },
      { key: "Protocol", value: "DLNA/AirPlay/Cast" },
      { key: "Auto-select", value: "Best codec match" },
    ],
    code: `#[derive(Serialize, Deserialize)]
struct Device {
    id: Uuid,
    name: String,
    ip: IpAddr,
    manufacturer: String,
    model: String,
    protocols: Vec<Protocol>,
    capabilities: DeviceCapabilities,
    status: DeviceStatus,
}`,
  },
];

export default function ModulesSection() {
  const [active, setActive] = useState("wasapi");
  const mod = modules.find((m) => m.id === active)!;

  return (
    <section id="modules" className="bg-gray-900 py-24 px-4">
      <div className="max-w-6xl mx-auto">
        {/* Header */}
        <div className="text-center mb-16">
          <span className="text-xs font-mono text-cyan-500 tracking-widest uppercase">Source Code</span>
          <h2 className="text-4xl font-black text-white mt-2 mb-4">
            Project Modules
          </h2>
          <p className="text-gray-400 max-w-xl mx-auto text-sm leading-relaxed">
            Explore the modular Rust architecture. Each module has a single responsibility
            and communicates through well-defined interfaces.
          </p>
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          {/* Module list */}
          <div className="lg:col-span-1 space-y-2">
            {modules.map((m) => (
              <button
                key={m.id}
                onClick={() => setActive(m.id)}
                className={`w-full text-left px-4 py-3 rounded-xl border transition-all duration-150 flex items-center gap-3
                  ${active === m.id
                    ? `${m.bg} ${m.color} font-bold`
                    : "border-gray-800 bg-gray-800/20 text-gray-400 hover:text-gray-200 hover:border-gray-700"
                  }`}
              >
                <span className="text-xl">{m.icon}</span>
                <div>
                  <div className="font-semibold text-sm">{m.title}</div>
                  <div className="text-xs font-mono opacity-60">
                    {m.tag}{m.files[0]}
                  </div>
                </div>
              </button>
            ))}
          </div>

          {/* Module detail */}
          <div className={`lg:col-span-2 border rounded-2xl overflow-hidden ${mod.bg}`}>
            {/* Module header */}
            <div className="px-6 py-5 border-b border-gray-700/40">
              <div className="flex items-center gap-3 mb-2">
                <span className="text-2xl">{mod.icon}</span>
                <h3 className={`text-xl font-black ${mod.color}`}>{mod.title}</h3>
              </div>
              <p className="text-gray-300 text-sm leading-relaxed">{mod.description}</p>
            </div>

            {/* Files */}
            <div className="px-6 py-3 flex flex-wrap gap-2 border-b border-gray-700/40">
              {mod.files.map((f) => (
                <span key={f} className="text-xs font-mono bg-gray-900/60 text-gray-400 px-2 py-1 rounded border border-gray-700/50">
                  src/{mod.tag}{f}
                </span>
              ))}
            </div>

            {/* Specs grid */}
            <div className="px-6 py-4 grid grid-cols-2 sm:grid-cols-3 gap-3 border-b border-gray-700/40">
              {mod.details.map((d) => (
                <div key={d.key} className="bg-gray-900/40 rounded-lg px-3 py-2">
                  <div className="text-gray-500 text-xs font-mono">{d.key}</div>
                  <div className="text-white text-sm font-bold">{d.value}</div>
                </div>
              ))}
            </div>

            {/* Code snippet */}
            <div className="px-6 py-4">
              <div className="text-xs font-mono text-gray-500 mb-2">// Rust pseudocode</div>
              <pre className="bg-gray-950/80 border border-gray-800/60 rounded-xl p-4 text-xs font-mono text-green-300 overflow-x-auto leading-relaxed whitespace-pre-wrap">
                {mod.code}
              </pre>
            </div>
          </div>
        </div>

        {/* Project structure */}
        <div className="mt-12 bg-gray-950/60 border border-gray-800/60 rounded-2xl p-6">
          <h3 className="text-white font-bold text-lg mb-4 flex items-center gap-2">
            <span>📁</span> Project Structure
          </h3>
          <pre className="text-xs font-mono text-gray-300 leading-6 overflow-x-auto">
{`src/
├── audio/
│   ├── capture.rs        # WASAPI Loopback capture
│   ├── mixer.rs          # Audio mixer / volume control
│   ├── ring_buffer.rs    # Lock-free SPMC ring buffer
│   └── resampler.rs      # Sample rate conversion
├── network/
│   ├── ssdp.rs           # SSDP discovery (DLNA)
│   ├── mdns.rs           # mDNS discovery (AirPlay/Cast)
│   ├── udp.rs            # UDP socket management
│   └── rtp.rs            # RTP packet construction
├── codecs/
│   ├── opus.rs           # Opus encoder/decoder
│   ├── pcm.rs            # Raw PCM passthrough
│   └── aac.rs            # AAC encoder (Phase 2)
├── outputs/
│   ├── rtp.rs            # RTP output module
│   ├── dlna.rs           # DLNA/UPnP AV output
│   ├── airplay.rs        # AirPlay 1 output
│   └── chromecast.rs     # Google Cast output
├── devices/
│   ├── device.rs         # Device model & registry
│   └── capabilities.rs   # Codec capability profiles
└── ui/
    └── tauri/            # Tauri commands & events`}
          </pre>
        </div>
      </div>
    </section>
  );
}
