# Network Audio Router for Windows

A Windows desktop application that captures system audio and streams it to network devices over DLNA, AirPlay, Chromecast, and RTP.

## Overview

Network Audio Router for Windows (NARfW) is a native Windows application built with Tauri 2 and Rust. It captures system audio via WASAPI Loopback without virtual audio cables and transmits it to any compatible device on your local network.

## Features

- **WASAPI Loopback Capture** — Captures all system audio directly from the Windows audio engine
- **RTP/UDP Streaming** — Low-latency PCM S16LE streaming over UDP
- **DLNA/UPnP** — Control and stream to DLNA MediaRenderer devices (TVs, speakers, etc.)
- **mDNS Discovery** — Automatically discover Chromecast, AirPlay, and RAOP devices
- **Multi-output** — Stream to multiple devices simultaneously from a single capture
- **Per-device codec selection** — PCM, AAC, Opus, ALAC, FLAC, MP3, LDAC, LHDC, SBC
- **System tray** — Compact tray-first control window with scan, select, start/stop
- **NSIS and MSI installers** — Ready-to-distribute installer builds

## Architecture

```
Windows Audio Engine
        |
        v
   WASAPI Loopback
        |
        v
  Audio Capture Layer
        |
        v
  Internal Audio Bus
        |
   ┌────┼────────┐
   v    v        v
 RTP  DLNA  AirPlay/Chromecast
   └────┼────────┘
        v
   Network Devices
```

Sound is captured once and distributed to any number of output modules without duplication.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Backend | Rust |
| Frontend | React 19 + Vite + TypeScript + Tailwind CSS |
| Desktop Shell | Tauri 2 |
| Audio Capture | WASAPI Loopback (via `cpal` / `windows` crate) |
| Transport | RTP/UDP, DLNA/AVTransport, mDNS |
| Codecs | PCM S16LE (current), Opus, AAC, ALAC, FLAC, MP3 planned |

## Build Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (with `cargo` in PATH)
- [Node.js](https://nodejs.org/) (v18+)
- npm

## Build

```powershell
# Install frontend dependencies
npm install

# Build the application
npm run tauri:build
```

If Rust was just installed, refresh PATH first:

```powershell
$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"
npm run tauri:build
```

## Output Artifacts

After a successful build:

```
src-tauri/target/release/network-audio-router-windows.exe
src-tauri/target/release/bundle/nsis/Network Audio Router for Windows_0.1.0_x64-setup.exe
src-tauri/target/release/bundle/msi/Network Audio Router for Windows_0.1.0_x64_en-US.msi
```

## Usage

1. Launch the application
2. Click **Scan** to discover network devices
3. Select a discovered device
4. Choose a connection profile (DLNA, RTP, AirPlay, etc.)
5. Select a codec and click **Start**
6. Use **Hide to tray** to keep streaming in the background
7. Right-click the tray icon to show the window or quit

## Project Structure

```
NARfW/
├── src/                    # React frontend
│   ├── App.tsx
│   ├── main.tsx
│   ├── index.css
│   └── components/         # UI components
├── src-tauri/              # Rust backend
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── build.rs
│   └── src/
│       ├── main.rs
│       ├── lib.rs
│       ├── commands.rs     # Tauri command handlers
│       ├── state.rs        # Application state
│       ├── tray_menu.rs    # System tray
│       ├── hotkeys.rs      # Keyboard shortcuts
│       ├── audio/          # Audio capture & devices
│       │   ├── wasapi_loopback.rs
│       │   ├── devices.rs
│       │   ├── sources.rs
│       │   └── pcm.rs
│       └── network/        # Network protocols
│           ├── rtp.rs
│           ├── dlna.rs
│           ├── airplay.rs
│           ├── cast.rs
│           ├── discovery.rs
│           ├── receiver.rs
│           └── sender.rs
├── package.json
├── vite.config.ts
├── tsconfig.json
└── .gitignore
```

## Current Status

MVP with working:
- WASAPI loopback capture
- RTP/UDP streaming
- DLNA discovery and AVTransport control
- mDNS device discovery (Chromecast, AirPlay, RAOP)
- Codec priority UI
- Multi-output selection
- System tray integration

Planned:
- Opus encoding (currently PCM only)
- Full AirPlay protocol support
- Chromecast streaming backend
- Multi-device synchronization
- EQ/DSP
- Recording
- Internet streaming
- Web UI

## License

MIT
