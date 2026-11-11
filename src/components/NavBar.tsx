import { useState } from "react";

const navLinks = [
  { label: "Overview", href: "#overview" },
  { label: "Architecture", href: "#architecture" },
  { label: "Tech Stack", href: "#techstack" },
  { label: "Roadmap", href: "#roadmap" },
  { label: "Modules", href: "#modules" },
  { label: "UI Preview", href: "#ui-preview" },
];

export default function NavBar() {
  const [open, setOpen] = useState(false);

  return (
    <nav className="fixed top-0 left-0 right-0 z-50 bg-gray-950/90 backdrop-blur-md border-b border-cyan-900/40">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div className="flex items-center justify-between h-16">
          {/* Logo */}
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-cyan-400 to-blue-600 flex items-center justify-center">
              <svg viewBox="0 0 24 24" fill="none" className="w-5 h-5 text-white" stroke="currentColor" strokeWidth="2">
                <path d="M9 19V6l12-3v13" strokeLinecap="round" strokeLinejoin="round"/>
                <circle cx="6" cy="19" r="3"/>
                <circle cx="18" cy="16" r="3"/>
              </svg>
            </div>
            <span className="font-bold text-white text-lg tracking-tight">
              Win<span className="text-cyan-400">Audio</span>Router
            </span>
            <span className="hidden sm:inline text-xs text-gray-500 border border-gray-700 rounded px-1.5 py-0.5 font-mono">v0.1</span>
          </div>

          {/* Desktop Links */}
          <div className="hidden md:flex items-center gap-1">
            {navLinks.map((l) => (
              <a
                key={l.href}
                href={l.href}
                className="text-sm text-gray-400 hover:text-cyan-400 px-3 py-1.5 rounded-md hover:bg-cyan-950/40 transition-all duration-150"
              >
                {l.label}
              </a>
            ))}
            <a
              href="#roadmap"
              className="ml-3 text-sm font-semibold bg-cyan-500 hover:bg-cyan-400 text-gray-950 px-4 py-1.5 rounded-full transition-colors"
            >
              Get Started
            </a>
          </div>

          {/* Mobile hamburger */}
          <button
            className="md:hidden text-gray-400 hover:text-white p-2"
            onClick={() => setOpen(!open)}
            aria-label="Toggle menu"
          >
            {open ? (
              <svg className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12"/>
              </svg>
            ) : (
              <svg className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 6h16M4 12h16M4 18h16"/>
              </svg>
            )}
          </button>
        </div>
      </div>

      {/* Mobile Menu */}
      {open && (
        <div className="md:hidden bg-gray-950 border-t border-cyan-900/30 px-4 pb-4 pt-2 space-y-1">
          {navLinks.map((l) => (
            <a
              key={l.href}
              href={l.href}
              onClick={() => setOpen(false)}
              className="block text-sm text-gray-400 hover:text-cyan-400 py-2 px-3 rounded-md hover:bg-cyan-950/30 transition-colors"
            >
              {l.label}
            </a>
          ))}
        </div>
      )}
    </nav>
  );
}
