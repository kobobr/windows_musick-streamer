import { useState } from "react";

const phases = [
  {
    phase: "MVP",
    title: "Core Audio Engine",
    status: "active",
    statusLabel: "In Development",
    color: "border-cyan-500 bg-cyan-950/20",
    dotColor: "bg-cyan-500",
    labelColor: "bg-cyan-500/20 text-cyan-400",
    items: [
      { done: false, text: "WASAPI Loopback audio capture" },
      { done: false, text: "Ring buffer internal audio bus" },
      { done: false, text: "Opus codec encoding pipeline" },
      { done: false, text: "RTP/UDP transport (port 5000)" },
      { done: false, text: "Basic RTP receiver client" },
      { done: false, text: "Tauri desktop app shell" },
      { done: false, text: "Stability & latency benchmarking" },
    ],
    goal: "Validate the core capture-encode-transmit pipeline. No device discovery yet.",
  },
  {
    phase: "Phase 2",
    title: "DLNA / UPnP Streaming",
    status: "planned",
    statusLabel: "Planned",
    color: "border-pink-500/50 bg-pink-950/10",
    dotColor: "bg-pink-500",
    labelColor: "bg-pink-500/20 text-pink-400",
    items: [
      { done: false, text: "SSDP device discovery (239.255.255.250:1900)" },
      { done: false, text: "UPnP Media Renderer enumeration" },
      { done: false, text: "SetAVTransportURI command" },
      { done: false, text: "Play / Pause / Stop control" },
      { done: false, text: "SetVolume UPnP command" },
      { done: false, text: "AAC codec for DLNA compatibility" },
      { done: false, text: "Device capability profiles" },
    ],
    goal: "Stream to LG, Samsung, Sony, and other DLNA-capable smart TVs.",
  },
  {
    phase: "Phase 3",
    title: "AirPlay Support",
    status: "planned",
    statusLabel: "Planned",
    color: "border-gray-500/50 bg-gray-800/20",
    dotColor: "bg-gray-400",
    labelColor: "bg-gray-500/20 text-gray-400",
    items: [
      { done: false, text: "mDNS device discovery (_airplay._tcp)" },
      { done: false, text: "AirPlay 1 protocol implementation" },
      { done: false, text: "Apple TV compatibility" },
      { done: false, text: "AirPort Express compatibility" },
      { done: false, text: "PCM / ALAC audio transport" },
      { done: false, text: "AirPlay-compatible smart TV support" },
    ],
    goal: "Enable streaming to Apple TV, AirPort Express, and AirPlay-enabled speakers.",
  },
  {
    phase: "Phase 4",
    title: "Chromecast Support",
    status: "planned",
    statusLabel: "Planned",
    color: "border-red-500/50 bg-red-950/10",
    dotColor: "bg-red-500",
    labelColor: "bg-red-500/20 text-red-400",
    items: [
      { done: false, text: "mDNS discovery (_googlecast._tcp)" },
      { done: false, text: "Google Cast protocol implementation" },
      { done: false, text: "Chromecast Audio support" },
      { done: false, text: "Google Nest support" },
      { done: false, text: "Cast Built-In device support" },
      { done: false, text: "Opus / AAC transport" },
    ],
    goal: "Full Google Cast ecosystem coverage.",
  },
  {
    phase: "v2.0",
    title: "Advanced Features",
    status: "future",
    statusLabel: "Future",
    color: "border-violet-500/40 bg-violet-950/10",
    dotColor: "bg-violet-500",
    labelColor: "bg-violet-500/20 text-violet-400",
    items: [
      { done: false, text: "Multi-device synchronization" },
      { done: false, text: "Latency compensation engine" },
      { done: false, text: "Built-in equalizer / DSP" },
      { done: false, text: "Stream recording to file" },
      { done: false, text: "Internet streaming (WAN)" },
      { done: false, text: "Web UI (browser remote control)" },
      { done: false, text: "Mobile companion app" },
      { done: false, text: "PipeWire-style audio graph routing" },
    ],
    goal: "Transform WinAudioRouter into a full-featured network audio platform.",
  },
];

export default function RoadmapSection() {
  const [expanded, setExpanded] = useState<string>("MVP");

  return (
    <section id="roadmap" className="bg-gray-950 py-24 px-4">
      <div className="max-w-4xl mx-auto">
        {/* Header */}
        <div className="text-center mb-16">
          <span className="text-xs font-mono text-cyan-500 tracking-widest uppercase">Development Plan</span>
          <h2 className="text-4xl font-black text-white mt-2 mb-4">
            Project Roadmap
          </h2>
          <p className="text-gray-400 max-w-xl mx-auto text-sm leading-relaxed">
            Phased development from a stable MVP core to a full-featured network audio platform.
          </p>
        </div>

        {/* Timeline */}
        <div className="relative">
          {/* Vertical line */}
          <div className="absolute left-6 top-0 bottom-0 w-px bg-gradient-to-b from-cyan-500 via-gray-700 to-transparent hidden sm:block"/>

          <div className="space-y-4">
            {phases.map((p) => (
              <div key={p.phase} className="relative sm:pl-16">
                {/* Dot */}
                <div className={`absolute left-4 top-5 w-4 h-4 rounded-full border-2 border-gray-950 ${p.dotColor} hidden sm:block z-10`}/>

                {/* Card */}
                <div
                  className={`border rounded-2xl overflow-hidden transition-all duration-300 ${p.color}`}
                >
                  {/* Header */}
                  <button
                    className="w-full text-left px-5 py-4 flex items-center gap-4"
                    onClick={() => setExpanded(expanded === p.phase ? "" : p.phase)}
                  >
                    <div className="flex-1">
                      <div className="flex items-center gap-3 flex-wrap">
                        <span className="text-white font-black text-lg">{p.phase}</span>
                        <span className="text-gray-300 font-semibold text-sm">— {p.title}</span>
                        <span className={`text-xs font-mono px-2 py-0.5 rounded-full ${p.labelColor}`}>
                          {p.statusLabel}
                        </span>
                      </div>
                    </div>
                    <svg
                      className={`w-4 h-4 text-gray-400 transition-transform flex-shrink-0 ${expanded === p.phase ? "rotate-180" : ""}`}
                      fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24"
                    >
                      <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7"/>
                    </svg>
                  </button>

                  {/* Body */}
                  {expanded === p.phase && (
                    <div className="px-5 pb-5 border-t border-gray-800/60">
                      <p className="text-gray-400 text-xs italic mt-4 mb-4">
                        🎯 Goal: {p.goal}
                      </p>
                      <ul className="space-y-2">
                        {p.items.map((item) => (
                          <li key={item.text} className="flex items-start gap-3 text-sm">
                            <span className={`w-5 h-5 flex-shrink-0 rounded border flex items-center justify-center mt-0.5 ${
                              item.done
                                ? "bg-cyan-500 border-cyan-500 text-white"
                                : "border-gray-600 text-transparent"
                            }`}>
                              {item.done ? "✓" : ""}
                            </span>
                            <span className={item.done ? "text-gray-300 line-through" : "text-gray-300"}>
                              {item.text}
                            </span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Vision callout */}
        <div className="mt-12 text-center p-8 bg-gradient-to-r from-cyan-950/40 via-blue-950/40 to-purple-950/40 border border-cyan-800/30 rounded-2xl">
          <div className="text-3xl mb-3">🌐</div>
          <h3 className="text-white font-black text-xl mb-2">Final Vision</h3>
          <p className="text-gray-300 text-sm leading-relaxed max-w-2xl mx-auto">
            A universal Windows Audio Router that discovers LAN devices and streams system audio via
            DLNA, AirPlay, Chromecast, and RTP — without Virtual Audio Cable, WSL2, Docker, or any
            third-party audio drivers.
            <br/><br/>
            <span className="text-cyan-400 font-bold">The network equivalent of PipeWire, for the Windows ecosystem.</span>
          </p>
        </div>
      </div>
    </section>
  );
}
