const stack = [
  {
    category: "Language",
    name: "Rust",
    version: "2021 Edition",
    icon: "🦀",
    color: "border-orange-500/40 bg-orange-950/20",
    tagColor: "bg-orange-500/20 text-orange-300",
    reasons: [
      "Memory safety without GC",
      "Zero-cost abstractions",
      "Sub-millisecond audio latency",
      "Fearless concurrency",
      "Cross-platform build toolchain",
    ],
    crates: ["cpal", "windows-rs", "tokio", "ringbuf", "opus"],
  },
  {
    category: "UI Framework",
    name: "Tauri",
    version: "v2.x",
    icon: "⚡",
    color: "border-blue-500/40 bg-blue-950/20",
    tagColor: "bg-blue-500/20 text-blue-300",
    reasons: [
      "~10 MB binary size",
      "Native WebView (no Electron)",
      "Rust backend ↔ JS bridge",
      "React / Vue / Svelte frontend",
      "Windows/macOS/Linux support",
    ],
    crates: ["tauri", "tauri-build", "serde", "serde_json"],
  },
  {
    category: "Audio Capture",
    name: "WASAPI",
    version: "Loopback Mode",
    icon: "🎵",
    color: "border-purple-500/40 bg-purple-950/20",
    tagColor: "bg-purple-500/20 text-purple-300",
    reasons: [
      "No virtual audio devices",
      "Full system audio capture",
      "Per-device or global capture",
      "Low latency kernel buffer",
      "Native Windows API",
    ],
    crates: ["windows-rs", "cpal", "wasapi"],
  },
  {
    category: "Audio Codec",
    name: "Opus",
    version: "1.5",
    icon: "🔊",
    color: "border-green-500/40 bg-green-950/20",
    tagColor: "bg-green-500/20 text-green-300",
    reasons: [
      "Ultra-low latency (20ms frames)",
      "128–192 kbps stereo",
      "48 kHz native sample rate",
      "Excellent RTP support",
      "Royalty-free open standard",
    ],
    crates: ["audiopus", "opus-sys"],
  },
  {
    category: "Network Transport",
    name: "RTP / UDP",
    version: "RFC 3550",
    icon: "📡",
    color: "border-yellow-500/40 bg-yellow-950/20",
    tagColor: "bg-yellow-500/20 text-yellow-300",
    reasons: [
      "Real-time transport protocol",
      "50–200 ms configurable buffer",
      "Port 5000 default",
      "Multicast capable",
      "Compatible with VLC/ffmpeg",
    ],
    crates: ["rtp", "tokio-net", "bytes"],
  },
  {
    category: "Discovery",
    name: "SSDP + mDNS",
    version: "UPnP 1.1 / RFC 6762",
    icon: "🔍",
    color: "border-cyan-500/40 bg-cyan-950/20",
    tagColor: "bg-cyan-500/20 text-cyan-300",
    reasons: [
      "SSDP: DLNA / UPnP Renderer discovery",
      "mDNS: AirPlay & Chromecast",
      "Zero-config LAN scanning",
      "239.255.255.250:1900 multicast",
      "224.0.0.251:5353 mDNS",
    ],
    crates: ["mdns-sd", "ssdp-client"],
  },
];

const futureCodecs = [
  { name: "PCM", note: "Lossless passthrough · low CPU" },
  { name: "AAC", note: "DLNA compatible · wide support" },
  { name: "MP3", note: "Universal fallback" },
  { name: "FLAC", note: "Lossless compressed" },
];

export default function TechStackSection() {
  return (
    <section id="techstack" className="bg-gray-900 py-24 px-4">
      <div className="max-w-6xl mx-auto">
        {/* Header */}
        <div className="text-center mb-16">
          <span className="text-xs font-mono text-cyan-500 tracking-widest uppercase">Technologies</span>
          <h2 className="text-4xl font-black text-white mt-2 mb-4">
            Technology Stack
          </h2>
          <p className="text-gray-400 max-w-xl mx-auto text-sm leading-relaxed">
            Every component chosen for maximum performance, minimal footprint, and native Windows integration.
          </p>
        </div>

        {/* Stack grid */}
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-5">
          {stack.map((item) => (
            <div
              key={item.name}
              className={`border rounded-2xl p-5 ${item.color} hover:scale-[1.02] transition-transform duration-200`}
            >
              {/* Header */}
              <div className="flex items-start justify-between mb-4">
                <div>
                  <span className={`text-xs font-mono px-2 py-0.5 rounded ${item.tagColor} mb-2 inline-block`}>
                    {item.category}
                  </span>
                  <div className="flex items-center gap-2">
                    <span className="text-2xl">{item.icon}</span>
                    <h3 className="text-white font-black text-xl">{item.name}</h3>
                  </div>
                  <div className="text-gray-500 text-xs font-mono mt-0.5">{item.version}</div>
                </div>
              </div>

              {/* Reasons */}
              <ul className="space-y-1.5 mb-4">
                {item.reasons.map((r) => (
                  <li key={r} className="flex items-start gap-2 text-xs text-gray-300">
                    <span className="text-cyan-500 mt-0.5 flex-shrink-0">✓</span>
                    {r}
                  </li>
                ))}
              </ul>

              {/* Crates */}
              <div className="flex flex-wrap gap-1.5 mt-3 pt-3 border-t border-gray-700/50">
                {item.crates.map((c) => (
                  <span
                    key={c}
                    className="text-xs font-mono bg-gray-800/80 text-gray-400 px-2 py-0.5 rounded border border-gray-700/50"
                  >
                    {c}
                  </span>
                ))}
              </div>
            </div>
          ))}
        </div>

        {/* Codec roadmap */}
        <div className="mt-14">
          <h3 className="text-white font-bold text-lg mb-5 text-center">
            🎚️ Codec Roadmap
          </h3>
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-4">
            {/* Opus — default */}
            <div className="bg-green-950/30 border border-green-500/40 rounded-xl p-4 text-center">
              <div className="text-green-400 text-xs font-mono font-bold mb-1">MVP · DEFAULT</div>
              <div className="text-white font-black text-lg">Opus</div>
              <div className="text-gray-400 text-xs mt-1">128–192 kbps · 48 kHz</div>
            </div>
            {futureCodecs.map((c) => (
              <div key={c.name} className="bg-gray-800/40 border border-gray-700/50 rounded-xl p-4 text-center opacity-70">
                <div className="text-gray-500 text-xs font-mono font-bold mb-1">FUTURE</div>
                <div className="text-white font-black text-lg">{c.name}</div>
                <div className="text-gray-400 text-xs mt-1">{c.note}</div>
              </div>
            ))}
          </div>
        </div>

        {/* Why not alternatives */}
        <div className="mt-14 bg-gray-950/60 border border-gray-800/60 rounded-2xl p-6">
          <h3 className="text-white font-bold text-lg mb-5">
            ❌ Why Not Existing Solutions?
          </h3>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            {[
              {
                name: "Stream What You Hear",
                problems: ["Limited compatibility", "Unstable device search", "Issues with modern TVs", "No unified architecture"],
              },
              {
                name: "Virtual Audio Cable",
                problems: ["Manual setup required", "Additional latency", "Extra audio conversions", "User-unfriendly"],
              },
              {
                name: "WSL2",
                problems: ["SSDP problems", "mDNS issues", "Unstable multicast", "No native Windows audio"],
              },
              {
                name: "Docker",
                problems: ["No audio device access", "Network isolation", "High complexity", "Not intended for audio"],
              },
            ].map((alt) => (
              <div key={alt.name} className="bg-red-950/20 border border-red-800/30 rounded-xl p-4">
                <div className="text-red-400 font-bold text-sm mb-2">{alt.name}</div>
                <ul className="space-y-1">
                  {alt.problems.map((p) => (
                    <li key={p} className="flex items-center gap-2 text-xs text-gray-400">
                      <span className="text-red-500">✗</span>
                      {p}
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
          <div className="mt-4 p-4 bg-cyan-950/30 border border-cyan-800/40 rounded-xl">
            <p className="text-cyan-400 font-bold text-sm">
              ✅ Conclusion: Native Windows Application is the only viable approach.
            </p>
            <p className="text-gray-400 text-xs mt-1">
              Direct WASAPI access, native networking, no intermediate layers, no compatibility shims.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}
