import { useEffect, useRef } from "react";

const badges = [
  { label: "Rust", color: "bg-orange-500/20 text-orange-300 border-orange-500/40" },
  { label: "Tauri", color: "bg-blue-500/20 text-blue-300 border-blue-500/40" },
  { label: "WASAPI", color: "bg-purple-500/20 text-purple-300 border-purple-500/40" },
  { label: "Opus Codec", color: "bg-green-500/20 text-green-300 border-green-500/40" },
  { label: "RTP/UDP", color: "bg-yellow-500/20 text-yellow-300 border-yellow-500/40" },
  { label: "DLNA", color: "bg-pink-500/20 text-pink-300 border-pink-500/40" },
];

function AnimatedWave({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 400 80" className={className} preserveAspectRatio="none">
      <defs>
        <linearGradient id="wg" x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stopColor="#06b6d4" stopOpacity="0.0"/>
          <stop offset="30%" stopColor="#06b6d4" stopOpacity="0.8"/>
          <stop offset="70%" stopColor="#3b82f6" stopOpacity="0.8"/>
          <stop offset="100%" stopColor="#3b82f6" stopOpacity="0.0"/>
        </linearGradient>
      </defs>
      <polyline
        points="0,40 30,28 60,45 90,20 120,50 150,15 180,42 210,18 240,48 270,22 300,44 330,16 360,40 400,30"
        fill="none"
        stroke="url(#wg)"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export default function HeroSection() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let animId: number;
    let t = 0;

    const resize = () => {
      canvas.width = canvas.offsetWidth;
      canvas.height = canvas.offsetHeight;
    };
    resize();
    window.addEventListener("resize", resize);

    const draw = () => {
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      const W = canvas.width;
      const H = canvas.height;

      // Draw multiple audio waveform lines
      const waves = [
        { amp: 30, freq: 0.015, speed: 0.04, color: "rgba(6,182,212,0.25)", y: H * 0.4 },
        { amp: 20, freq: 0.025, speed: 0.06, color: "rgba(59,130,246,0.2)", y: H * 0.5 },
        { amp: 40, freq: 0.01, speed: 0.03, color: "rgba(139,92,246,0.15)", y: H * 0.6 },
      ];

      waves.forEach(({ amp, freq, speed, color, y }) => {
        ctx.beginPath();
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5;
        for (let x = 0; x <= W; x += 2) {
          const vy = y + Math.sin(x * freq + t * speed) * amp * Math.sin(t * speed * 0.5);
          if (x === 0) ctx.moveTo(x, vy);
          else ctx.lineTo(x, vy);
        }
        ctx.stroke();
      });

      // Floating particles
      const particles = 12;
      for (let i = 0; i < particles; i++) {
        const px = (W / particles) * i + Math.sin(t * 0.02 + i) * 30;
        const py = H * 0.5 + Math.cos(t * 0.015 + i * 1.2) * 60;
        const r = 1.5 + Math.sin(t * 0.03 + i) * 1;
        ctx.beginPath();
        ctx.fillStyle = `rgba(6,182,212,${0.3 + Math.sin(t * 0.02 + i) * 0.2})`;
        ctx.arc(px, py, r, 0, Math.PI * 2);
        ctx.fill();
      }

      t++;
      animId = requestAnimationFrame(draw);
    };

    draw();
    return () => {
      cancelAnimationFrame(animId);
      window.removeEventListener("resize", resize);
    };
  }, []);

  return (
    <section
      id="overview"
      className="relative min-h-screen flex flex-col items-center justify-center overflow-hidden bg-gray-950 pt-16"
    >
      {/* Background grid */}
      <div
        className="absolute inset-0 opacity-10"
        style={{
          backgroundImage: `linear-gradient(rgba(6,182,212,0.3) 1px, transparent 1px),
                            linear-gradient(90deg, rgba(6,182,212,0.3) 1px, transparent 1px)`,
          backgroundSize: "60px 60px",
        }}
      />

      {/* Radial glow */}
      <div className="absolute inset-0 bg-radial-gradient pointer-events-none"
        style={{
          background: "radial-gradient(ellipse 80% 60% at 50% 40%, rgba(6,182,212,0.08) 0%, transparent 70%)"
        }}
      />

      {/* Canvas waveform */}
      <canvas
        ref={canvasRef}
        className="absolute inset-0 w-full h-full pointer-events-none"
      />

      <div className="relative z-10 max-w-5xl mx-auto px-4 text-center">
        {/* Status badge */}
        <div className="inline-flex items-center gap-2 bg-cyan-950/60 border border-cyan-700/50 rounded-full px-4 py-1.5 text-xs text-cyan-400 font-mono mb-8 backdrop-blur-sm">
          <span className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse"/>
          Technical Specification · Version 0.1 · 2026
        </div>

        {/* Main heading */}
        <h1 className="text-5xl sm:text-6xl lg:text-7xl font-black text-white mb-6 leading-tight tracking-tight">
          Network Audio{" "}
          <span className="text-transparent bg-clip-text bg-gradient-to-r from-cyan-400 to-blue-500">
            Router
          </span>
          <br />
          <span className="text-3xl sm:text-4xl lg:text-5xl font-bold text-gray-300">
            for Windows
          </span>
        </h1>

        {/* Animated wave decoration */}
        <AnimatedWave className="w-full h-12 my-4 opacity-80" />

        {/* Subtitle */}
        <p className="text-lg sm:text-xl text-gray-400 max-w-2xl mx-auto mb-8 leading-relaxed">
          Stream system audio to any device on your local network —
          no Virtual Audio Cable, no WSL2, no Docker.
          Pure native Windows audio routing powered by{" "}
          <span className="text-orange-400 font-semibold">Rust</span> &{" "}
          <span className="text-blue-400 font-semibold">Tauri</span>.
        </p>

        {/* Tech badges */}
        <div className="flex flex-wrap justify-center gap-2 mb-10">
          {badges.map((b) => (
            <span
              key={b.label}
              className={`text-xs font-mono font-semibold px-3 py-1 rounded-full border ${b.color}`}
            >
              {b.label}
            </span>
          ))}
        </div>

        {/* CTA Buttons */}
        <div className="flex flex-col sm:flex-row gap-4 justify-center">
          <a
            href="#architecture"
            className="px-8 py-3 bg-cyan-500 hover:bg-cyan-400 text-gray-950 font-bold rounded-full transition-all duration-200 text-sm shadow-lg shadow-cyan-500/25 hover:shadow-cyan-400/40 hover:-translate-y-0.5"
          >
            Explore Architecture
          </a>
          <a
            href="#roadmap"
            className="px-8 py-3 bg-transparent border border-cyan-700/60 hover:border-cyan-500 text-gray-300 hover:text-white font-semibold rounded-full transition-all duration-200 text-sm hover:-translate-y-0.5"
          >
            View Roadmap
          </a>
        </div>

        {/* Problem statement */}
        <div className="mt-16 grid grid-cols-1 sm:grid-cols-3 gap-4 text-left">
          {[
            {
              icon: "🔌",
              title: "No Virtual Cables",
              desc: "WASAPI Loopback captures system audio natively — zero driver installation required.",
            },
            {
              icon: "📡",
              title: "Auto Discovery",
              desc: "SSDP & mDNS scan your LAN for DLNA, AirPlay, and Chromecast devices automatically.",
            },
            {
              icon: "🔀",
              title: "Multi-Cast",
              desc: "One capture → infinite outputs. Stream to TV, speakers, and Chromecast simultaneously.",
            },
          ].map((item) => (
            <div
              key={item.title}
              className="bg-gray-900/60 backdrop-blur-sm border border-gray-800/60 rounded-2xl p-5 hover:border-cyan-800/60 transition-colors"
            >
              <div className="text-3xl mb-3">{item.icon}</div>
              <h3 className="text-white font-bold text-sm mb-1">{item.title}</h3>
              <p className="text-gray-400 text-xs leading-relaxed">{item.desc}</p>
            </div>
          ))}
        </div>
      </div>

      {/* Scroll indicator */}
      <div className="absolute bottom-8 left-1/2 -translate-x-1/2 flex flex-col items-center gap-2 text-gray-600">
        <span className="text-xs font-mono">scroll to explore</span>
        <svg className="w-4 h-4 animate-bounce" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7"/>
        </svg>
      </div>
    </section>
  );
}
