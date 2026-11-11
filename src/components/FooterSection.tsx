export default function FooterSection() {
  return (
    <footer className="bg-gray-950 border-t border-gray-800/60 py-12 px-4">
      <div className="max-w-6xl mx-auto">
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-8 mb-8">
          {/* Brand */}
          <div>
            <div className="flex items-center gap-3 mb-3">
              <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-cyan-400 to-blue-600 flex items-center justify-center">
                <svg viewBox="0 0 24 24" fill="none" className="w-4 h-4 text-white" stroke="currentColor" strokeWidth="2">
                  <path d="M9 19V6l12-3v13" strokeLinecap="round" strokeLinejoin="round"/>
                  <circle cx="6" cy="19" r="3"/>
                  <circle cx="18" cy="16" r="3"/>
                </svg>
              </div>
              <span className="font-bold text-white">
                Win<span className="text-cyan-400">Audio</span>Router
              </span>
            </div>
            <p className="text-gray-500 text-xs leading-relaxed">
              The network equivalent of PipeWire for the Windows ecosystem.
              Stream system audio to any LAN device — natively.
            </p>
          </div>

          {/* Links */}
          <div>
            <h4 className="text-white font-bold text-sm mb-3">Documentation</h4>
            <ul className="space-y-2">
              {[
                { label: "Architecture Overview", href: "#architecture" },
                { label: "Technology Stack", href: "#techstack" },
                { label: "Development Roadmap", href: "#roadmap" },
                { label: "Module Reference", href: "#modules" },
                { label: "UI Preview", href: "#ui-preview" },
              ].map((l) => (
                <li key={l.href}>
                  <a href={l.href} className="text-gray-500 hover:text-cyan-400 text-xs transition-colors">
                    {l.label}
                  </a>
                </li>
              ))}
            </ul>
          </div>

          {/* Protocols */}
          <div>
            <h4 className="text-white font-bold text-sm mb-3">Supported Protocols</h4>
            <ul className="space-y-2">
              {[
                { label: "RTP / UDP", status: "MVP", color: "text-cyan-400" },
                { label: "DLNA / UPnP AV", status: "Phase 2", color: "text-pink-400" },
                { label: "AirPlay 1", status: "Phase 3", color: "text-gray-400" },
                { label: "Google Cast", status: "Phase 4", color: "text-red-400" },
              ].map((p) => (
                <li key={p.label} className="flex items-center justify-between">
                  <span className={`text-xs ${p.color}`}>{p.label}</span>
                  <span className="text-xs text-gray-600 font-mono">{p.status}</span>
                </li>
              ))}
            </ul>
          </div>
        </div>

        {/* Bottom bar */}
        <div className="border-t border-gray-800/60 pt-6 flex flex-col sm:flex-row items-center justify-between gap-3">
          <div className="text-xs text-gray-600 font-mono">
            Technical Specification · Version 0.1 · 2026
          </div>
          <div className="flex items-center gap-4 text-xs text-gray-600">
            <span className="flex items-center gap-1.5">
              <span className="text-orange-400">🦀</span>
              Built with Rust + Tauri
            </span>
            <span>·</span>
            <span>WASAPI · Opus · RTP</span>
          </div>
        </div>
      </div>
    </footer>
  );
}
