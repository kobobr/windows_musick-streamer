import { useState } from "react";

const nodes = [
  {
    id: "wasapi",
    label: "Windows Audio Engine",
    sub: "WASAPI Loopback",
    color: "from-purple-600 to-purple-800",
    border: "border-purple-500/50",
    glow: "shadow-purple-500/20",
    icon: "🖥️",
    detail: "Captures system audio at the kernel level via WASAPI Loopback. No virtual devices or driver installation needed. Supports all modern Windows versions. Expected output: 48 kHz · Stereo · Float32.",
  },
  {
    id: "capture",
    label: "Audio Capture Layer",
    sub: "48 kHz · Stereo · Float32",
    color: "from-blue-600 to-blue-800",
    border: "border-blue-500/50",
    glow: "shadow-blue-500/20",
    icon: "🎙️",
    detail: "Receives raw PCM frames from WASAPI Loopback. Handles resampling if needed. Feeds data into the internal ring buffer at low latency.",
  },
  {
    id: "bus",
    label: "Internal Audio Bus",
    sub: "Ring Buffer · Fan-out",
    color: "from-cyan-600 to-cyan-800",
    border: "border-cyan-500/50",
    glow: "shadow-cyan-500/20",
    icon: "🔀",
    detail: "Single audio source, multiple subscribers. Each output module gets its own consumer handle on the ring buffer. Zero copy architecture. Enables simultaneous multi-device streaming without re-capture.",
  },
  {
    id: "codec",
    label: "Codec Layer",
    sub: "Opus · PCM · AAC · MP3",
    color: "from-green-600 to-green-800",
    border: "border-green-500/50",
    glow: "shadow-green-500/20",
    icon: "🔊",
    detail: "Encodes PCM frames per-output based on device capabilities. Default: Opus 128-192 kbps @ 48 kHz. Future: AAC for DLNA, PCM passthrough, FLAC lossless.",
  },
];

const outputs = [
  {
    id: "rtp",
    label: "RTP Output",
    sub: "UDP · Port 5000",
    color: "from-yellow-600 to-yellow-800",
    border: "border-yellow-500/50",
    icon: "📡",
    detail: "Primary MVP transport. Opus-encoded audio over RTP/UDP. Low latency (50–200 ms buffer). Compatible with any RTP-capable receiver. No server required.",
  },
  {
    id: "dlna",
    label: "DLNA Output",
    sub: "UPnP AV · SSDP",
    color: "from-pink-600 to-pink-800",
    border: "border-pink-500/50",
    icon: "📺",
    detail: "Discovers UPnP Media Renderers via SSDP (239.255.255.250:1900). Supports SetAVTransportURI, Play, Pause, Stop, SetVolume. Compatible with LG, Samsung, Sony TVs.",
  },
  {
    id: "airplay",
    label: "AirPlay Output",
    sub: "AirPlay 1 · mDNS",
    color: "from-gray-500 to-gray-700",
    border: "border-gray-400/50",
    icon: "🍎",
    detail: "AirPlay 1 protocol support via mDNS discovery. Compatible with Apple TV, AirPort Express, and select smart TVs. Planned for Phase 3.",
  },
  {
    id: "chromecast",
    label: "Chromecast Output",
    sub: "Cast SDK · mDNS",
    color: "from-red-600 to-red-800",
    border: "border-red-500/50",
    icon: "📻",
    detail: "Google Cast protocol via mDNS discovery. Supports Chromecast Audio, Google Nest, and Cast Built-In devices. Planned for Phase 4.",
  },
];

export default function ArchitectureSection() {
  const [activeNode, setActiveNode] = useState<string | null>(null);

  const allNodes = [...nodes, ...outputs];
  const active = allNodes.find((n) => n.id === activeNode);

  return (
    <section id="architecture" className="bg-gray-950 py-24 px-4">
      <div className="max-w-6xl mx-auto">
        {/* Header */}
        <div className="text-center mb-16">
          <span className="text-xs font-mono text-cyan-500 tracking-widest uppercase">System Design</span>
          <h2 className="text-4xl font-black text-white mt-2 mb-4">
            Audio Pipeline Architecture
          </h2>
          <p className="text-gray-400 max-w-xl mx-auto text-sm leading-relaxed">
            Sound is captured exactly once and distributed to any number of output modules
            through a lock-free ring buffer internal audio bus.
          </p>
        </div>

        {/* Pipeline visual */}
        <div className="relative">
          {/* Main pipeline */}
          <div className="flex flex-col items-center gap-0">
            {nodes.map((node, idx) => (
              <div key={node.id} className="flex flex-col items-center w-full max-w-md mx-auto">
                {/* Node card */}
                <button
                  onClick={() => setActiveNode(activeNode === node.id ? null : node.id)}
                  className={`w-full text-left bg-gradient-to-r ${node.color} border ${node.border} rounded-xl px-5 py-4 
                    shadow-lg ${node.glow} cursor-pointer
                    transition-all duration-200 hover:scale-105 hover:shadow-xl
                    ${activeNode === node.id ? "ring-2 ring-white/30 scale-105" : ""}
                  `}
                >
                  <div className="flex items-center gap-3">
                    <span className="text-2xl">{node.icon}</span>
                    <div>
                      <div className="text-white font-bold text-sm">{node.label}</div>
                      <div className="text-white/60 text-xs font-mono">{node.sub}</div>
                    </div>
                    <div className="ml-auto">
                      <svg
                        className={`w-4 h-4 text-white/50 transition-transform ${activeNode === node.id ? "rotate-180" : ""}`}
                        fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24"
                      >
                        <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7"/>
                      </svg>
                    </div>
                  </div>
                </button>

                {/* Connector */}
                {idx < nodes.length - 1 ? (
                  <div className="flex flex-col items-center py-1">
                    <div className="w-px h-6 bg-gradient-to-b from-cyan-500/80 to-cyan-500/40"/>
                    <div className="text-cyan-500 text-xs">▼</div>
                  </div>
                ) : (
                  /* Fan-out from bus to outputs */
                  <div className="w-full mt-2 mb-2">
                    <div className="flex justify-center">
                      <div className="w-px h-4 bg-cyan-500/60"/>
                    </div>
                    {/* Horizontal bar */}
                    <div className="relative flex justify-center">
                      <div className="h-px bg-gradient-to-r from-transparent via-cyan-500/60 to-transparent w-full max-w-2xl"/>
                    </div>
                    {/* Vertical drops to each output */}
                    <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 max-w-2xl mx-auto mt-0">
                      {outputs.map((out) => (
                        <div key={out.id} className="flex flex-col items-center">
                          <div className="w-px h-4 bg-cyan-500/40"/>
                          <div className="text-cyan-500/60 text-xs">▼</div>
                          <button
                            onClick={() => setActiveNode(activeNode === out.id ? null : out.id)}
                            className={`w-full text-left bg-gradient-to-b ${out.color} border ${out.border} rounded-xl px-3 py-3 
                              shadow-md cursor-pointer transition-all duration-200 hover:scale-105
                              ${activeNode === out.id ? "ring-2 ring-white/30 scale-105" : ""}
                            `}
                          >
                            <div className="text-center">
                              <div className="text-xl mb-1">{out.icon}</div>
                              <div className="text-white font-bold text-xs">{out.label}</div>
                              <div className="text-white/50 text-xs font-mono leading-tight mt-0.5">{out.sub}</div>
                            </div>
                          </button>
                        </div>
                      ))}
                    </div>
                    <div className="flex justify-center mt-4">
                      <div className="w-px h-4 bg-cyan-500/40"/>
                    </div>
                    <div className="flex justify-center">
                      <div className="bg-gray-800 border border-gray-700 rounded-xl px-6 py-3 text-center">
                        <div className="text-lg">📶</div>
                        <div className="text-white font-bold text-xs mt-1">Network Devices</div>
                        <div className="text-gray-400 text-xs font-mono">LAN · WiFi · Ethernet</div>
                      </div>
                    </div>
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>

        {/* Detail panel */}
        {active && (
          <div className="mt-8 max-w-2xl mx-auto bg-gray-900/80 border border-gray-700/60 rounded-2xl p-6 backdrop-blur-sm animate-fade-in">
            <div className="flex items-center gap-3 mb-3">
              <span className="text-2xl">{active.icon}</span>
              <div>
                <div className="text-white font-bold">{active.label}</div>
                <div className="text-gray-400 text-xs font-mono">{active.sub}</div>
              </div>
              <button
                onClick={() => setActiveNode(null)}
                className="ml-auto text-gray-500 hover:text-white"
              >
                ✕
              </button>
            </div>
            <p className="text-gray-300 text-sm leading-relaxed">{active.detail}</p>
          </div>
        )}

        {/* Principle callout */}
        <div className="mt-12 text-center">
          <div className="inline-block bg-cyan-950/40 border border-cyan-800/50 rounded-2xl px-8 py-5 backdrop-blur-sm">
            <p className="text-cyan-400 font-mono text-sm font-bold">
              🔑 Core Principle
            </p>
            <p className="text-white text-lg font-bold mt-1">
              Audio is captured <span className="text-cyan-400">exactly once</span>
            </p>
            <p className="text-gray-400 text-sm mt-1">
              The internal bus fans out to unlimited simultaneous outputs with zero re-encoding overhead per subscriber.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}
