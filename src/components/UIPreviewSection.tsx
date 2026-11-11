import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface BackendStatus {
  backend: string;
  sender_running: boolean;
  receiver_running: boolean;
  capture: string;
  transport: string;
}

interface StartedStreamInfo {
  id: string;
  protocol: string;
  codec: string;
  transport: string;
  target: string;
  volume: number;
  sample_rate: number;
  channels: number;
  control_url?: string | null;
  stream_url?: string | null;
}

interface DiscoveredRenderer {
  location: string;
  st: string;
  usn: string;
  server: string;
  remote_addr: string;
  friendly_name?: string | null;
  model_name?: string | null;
  control_url?: string | null;
  rendering_control_url?: string | null;
  rendering_control_service_type?: string | null;
  service_name?: string | null;
  service_type?: string | null;
}

interface CodecCapability {
  id: string;
  label: string;
  detail: string;
  implemented: boolean;
}

interface ConnectionProfile {
  id: string;
  name: string;
  protocol: string;
  transport: string;
  host: string;
  port?: number | null;
  detail: string;
  supported: boolean;
  codecs: CodecCapability[];
  renderer?: DiscoveredRenderer | null;
}

interface NetworkDevice {
  id: string;
  name: string;
  host: string;
  detail: string;
  connections: ConnectionProfile[];
}

interface HotkeyStatus {
  enabled: boolean;
  decrease_key: string;
  increase_key: string;
}

interface AudioSourceInfo {
  id: string;
  name: string;
  process_id?: number | null;
  kind: string;
  selected: boolean;
}

interface CaptureSourceStatus {
  source: unknown;
  label: string;
}

interface AppSnapshot {
  status: BackendStatus;
  devices: NetworkDevice[];
  sources: AudioSourceInfo[];
  selected_connection_ids: string[];
  selected_source: CaptureSourceStatus;
  streams: StartedStreamInfo[];
  stream_starting: boolean;
  target_volumes: Record<string, number>;
  global_volume: number;
  muted: boolean;
  hotkeys: HotkeyStatus;
}

interface StreamErrorPayload {
  message: string;
}

interface StreamTarget {
  connection: ConnectionProfile;
  codec: string;
  volume: number;
}

const defaultStatus: BackendStatus = {
  backend: "Rust/Tauri",
  sender_running: false,
  receiver_running: false,
  capture: "WASAPI Loopback",
  transport: "Network audio",
};

const codecCatalog = [
  { id: "flac", label: "FLAC", quality: 10 },
  { id: "alac", label: "ALAC", quality: 9 },
  { id: "pcm_s16le", label: "WAV PCM", quality: 8.5 },
  { id: "pcm_l16be", label: "L16 PCM", quality: 8 },
  { id: "ldac", label: "LDAC", quality: 7 },
  { id: "lhdc", label: "LHDC", quality: 7 },
  { id: "aac_lc", label: "AAC-LC", quality: 5 },
  { id: "opus", label: "Opus", quality: 5 },
  { id: "mp3", label: "MP3", quality: 3 },
  { id: "sbc", label: "SBC", quality: 1 },
];

const protocolOrder: Record<string, number> = {
  DLNA: 0,
  AIRPLAY: 1,
  CHROMECAST: 2,
  UPNP: 3,
};

const targetVolumeStorageKey = "narfw.targetVolumes";

function loadTargetVolumes() {
  try {
    const raw = window.localStorage.getItem(targetVolumeStorageKey);
    return raw ? (JSON.parse(raw) as Record<string, number>) : {};
  } catch {
    return {};
  }
}

function protocolLabel(protocol: string) {
  return protocol.split("_").join(" ");
}

function codecLabel(id: string) {
  return codecCatalog.find((codec) => codec.id === id)?.label ?? id.toUpperCase();
}

function codecQuality(id: string) {
  return codecCatalog.find((codec) => codec.id === id)?.quality ?? 0;
}

function implementedCodecs(connection: ConnectionProfile) {
  return connection.codecs
    .filter((codec) => codec.implemented)
    .sort((a, b) => codecQuality(b.id) - codecQuality(a.id));
}

function autoCodec(connection: ConnectionProfile | undefined) {
  if (!connection) return "";
  const implemented = implementedCodecs(connection)[0]?.id;
  if (implemented) return implemented;
  if (connection.protocol === "AIRPLAY" && connection.codecs.some((codec) => codec.id === "alac")) {
    return "alac";
  }
  return "";
}

function streamReady(connection: ConnectionProfile) {
  return connection.supported && Boolean(autoCodec(connection));
}

function streamSelectable(connection: ConnectionProfile) {
  return streamReady(connection) || connection.protocol === "AIRPLAY";
}

function connectionRank(connection: ConnectionProfile) {
  return protocolOrder[connection.protocol] ?? 9;
}

function bestConnection(device: NetworkDevice | undefined) {
  if (!device) return undefined;
  const connections = [...device.connections].sort((a, b) => connectionRank(a) - connectionRank(b));
  return connections.find(streamReady) ?? connections[0];
}

function protocolSummary(device: NetworkDevice) {
  const byProtocol = new Map<string, ConnectionProfile>();
  for (const connection of device.connections.slice().sort((a, b) => connectionRank(a) - connectionRank(b))) {
    const current = byProtocol.get(connection.protocol);
    if (!current || protocolConnectionScore(connection) > protocolConnectionScore(current)) {
      byProtocol.set(connection.protocol, connection);
    }
  }
  return Array.from(byProtocol.values()).sort((a, b) => connectionRank(a) - connectionRank(b));
}

function protocolConnectionScore(connection: ConnectionProfile) {
  const detail = connection.detail.toLowerCase();
  let score = streamReady(connection) ? 100 : 0;
  if (streamSelectable(connection)) score += 20;
  if (connection.id.toLowerCase().includes(":raop:")) score += 20;
  if (detail.includes("raop audio")) score += 10;
  if (detail.includes("airplay control")) score -= 10;
  return score;
}

function Spinner() {
  return (
    <svg className="h-4 w-4 animate-spin" viewBox="0 0 24 24" aria-hidden="true">
      <circle className="opacity-20" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" fill="none" />
      <path className="opacity-90" fill="currentColor" d="M12 2a10 10 0 0 1 10 10h-3a7 7 0 0 0-7-7V2z" />
    </svg>
  );
}

export default function UIPreviewSection() {
  const [status, setStatus] = useState(defaultStatus);
  const [devices, setDevices] = useState<NetworkDevice[]>([]);
  const [audioSources, setAudioSources] = useState<AudioSourceInfo[]>([]);
  const [captureSourceLabel, setCaptureSourceLabel] = useState("System audio");
  const [selectedDeviceId, setSelectedDeviceId] = useState("");
  const [selectedConnectionId, setSelectedConnectionId] = useState("");
  const [selectedConnectionIds, setSelectedConnectionIds] = useState<string[]>([]);
  const [targetVolumes, setTargetVolumes] = useState(loadTargetVolumes);
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [streamStarting, setStreamStarting] = useState(false);
  const [streamInfos, setStreamInfos] = useState<StartedStreamInfo[]>([]);
  const [muted, setMuted] = useState(false);
  const [message, setMessage] = useState("Ready");
  const [error, setError] = useState<string | null>(null);

  const streaming = (status.sender_running || streamInfos.length > 0) && !streamStarting;
  const controlsBusy = busy || streamStarting;
  const allConnections = useMemo(
    () =>
      devices.flatMap((device) =>
        device.connections.map((connection) => ({
          device,
          connection,
        })),
      ),
    [devices],
  );
  const selectedDevice = useMemo(
    () => devices.find((device) => device.id === selectedDeviceId),
    [devices, selectedDeviceId],
  );
  const selectedConnection = useMemo(
    () => selectedDevice?.connections.find((connection) => connection.id === selectedConnectionId),
    [selectedDevice, selectedConnectionId],
  );
  const selectedTargets = useMemo(
    () => allConnections.filter(({ connection }) => selectedConnectionIds.includes(connection.id)),
    [allConnections, selectedConnectionIds],
  );
  const activeStreamIds = useMemo(() => new Set(streamInfos.map((stream) => stream.id)), [streamInfos]);
  const readyCount = allConnections.filter(({ connection }) => streamReady(connection)).length;
  const selectedSourceIds = useMemo(
    () => audioSources.filter((source) => source.selected).map((source) => source.id),
    [audioSources],
  );

  const applySnapshot = (snapshot: AppSnapshot) => {
    setStatus(snapshot.status);
    setDevices(snapshot.devices);
    setAudioSources(snapshot.sources);
    setCaptureSourceLabel(snapshot.selected_source.label);
    setSelectedConnectionIds(snapshot.selected_connection_ids);
    setStreamInfos(snapshot.streams);
    setStreamStarting(snapshot.stream_starting);
    setMuted(snapshot.muted);
    setTargetVolumes((current) => {
      const next = { ...current, ...snapshot.target_volumes };
      for (const id of snapshot.selected_connection_ids) {
        next[id] = next[id] ?? snapshot.global_volume;
      }
      for (const stream of snapshot.streams) {
        next[stream.id] = stream.volume;
      }
      return next;
    });

    const selectedConnection = snapshot.devices
      .flatMap((device) => device.connections.map((connection) => ({ device, connection })))
      .find(({ connection }) => snapshot.selected_connection_ids.includes(connection.id));
    const fallbackDevice = selectedConnection?.device ?? snapshot.devices[0];
    const fallbackConnection = selectedConnection?.connection ?? bestConnection(fallbackDevice);

    setSelectedDeviceId((current) =>
      snapshot.devices.some((device) => device.id === current) ? current : fallbackDevice?.id ?? "",
    );
    setSelectedConnectionId((current) =>
      snapshot.devices.some((device) => device.connections.some((connection) => connection.id === current))
        ? current
        : fallbackConnection?.id ?? "",
    );
  };

  const refreshSnapshot = async () => {
    try {
      const snapshot = await invoke<AppSnapshot>("app_snapshot");
      applySnapshot(snapshot);
      return snapshot;
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      return null;
    }
  };

  const run = async <T,>(action: () => Promise<T>, ok: string) => {
    setBusy(true);
    setError(null);
    try {
      const result = await action();
      setMessage(ok);
      await refreshSnapshot();
      return result;
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setMessage("Error");
      return null;
    } finally {
      setBusy(false);
    }
  };

  const selectDevice = (device: NetworkDevice) => {
    const connection = bestConnection(device);
    setSelectedDeviceId(device.id);
    setSelectedConnectionId(connection?.id ?? "");
    setError(null);
  };

  const selectConnection = (device: NetworkDevice, connection: ConnectionProfile) => {
    setSelectedDeviceId(device.id);
    setSelectedConnectionId(connection.id);
    setError(null);
  };

  const toggleConnection = (connection: ConnectionProfile) => {
    if (!streamSelectable(connection)) {
      setError(`${protocolLabel(connection.protocol)} is visible, but streaming backend for this protocol is not ready yet.`);
      return;
    }
    if (!streamReady(connection)) {
      setError("AirPlay 2/FairPlay is selectable for diagnostics; Start will probe the device handshake.");
    }
    const owner = allConnections.find((item) => item.connection.id === connection.id);
    const sameDeviceIds = owner?.device.connections.map((candidate) => candidate.id) ?? [connection.id];
    setSelectedConnectionIds((current) => {
      const next = current.includes(connection.id)
        ? current.filter((id) => id !== connection.id)
        : [...current.filter((id) => !sameDeviceIds.includes(id)), connection.id];
      void invoke("set_selected_connection_ids", { connectionIds: next }).catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      });
      return next;
    });
  };

  const setVolume = (connectionId: string, volume: number) => {
    const nextVolume = Math.max(0, Math.min(150, volume));
    setTargetVolumes((current) => ({ ...current, [connectionId]: nextVolume }));
    void invoke("set_network_stream_volume", { streamId: connectionId, volume: nextVolume }).catch((err) => {
      setError(err instanceof Error ? err.message : String(err));
    });
  };

  const toggleSource = async (source: AudioSourceInfo) => {
    if (streaming || streamStarting) return;

    const nextIds =
      source.kind === "system"
        ? ["system"]
        : source.selected
          ? selectedSourceIds.filter((id) => id !== source.id && id !== "system")
          : [...selectedSourceIds.filter((id) => id !== "system"), source.id];
    const finalIds = nextIds.length === 0 ? ["system"] : nextIds;

    setAudioSources((current) =>
      current.map((candidate) => ({
        ...candidate,
        selected: finalIds.includes(candidate.id),
      })),
    );
    setError(null);
    try {
      await invoke("set_capture_source_ids", { sourceIds: finalIds });
      await refreshSnapshot();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      await refreshSnapshot();
    }
  };

  const scan = async () => {
    setScanning(true);
    const found = await run(
      () => invoke<NetworkDevice[]>("discover_network_devices", { timeoutMs: 4000 }),
      "Scan complete",
    );
    setScanning(false);
    if (!found) return;

    setDevices(found);

    const firstReady = found
      .flatMap((device) => device.connections.map((connection) => ({ device, connection })))
      .find(({ connection }) => streamReady(connection));
    if (firstReady) {
      setSelectedDeviceId(firstReady.device.id);
      setSelectedConnectionId(firstReady.connection.id);
    } else {
      setSelectedDeviceId(found[0]?.id ?? "");
      setSelectedConnectionId(bestConnection(found[0])?.id ?? "");
    }

    const profiles = found.reduce((sum, device) => sum + device.connections.length, 0);
    setMessage(found.length === 0 ? "No network audio devices found." : `Found: ${found.length} devices / ${profiles} profiles`);
  };

  const start = async () => {
    if (selectedTargets.length === 0) {
      setError("No outputs selected.");
      setMessage("Select output");
      return;
    }

    const targets: StreamTarget[] = selectedTargets.map(({ connection }) => ({
      connection,
      codec: autoCodec(connection),
      volume: targetVolumes[connection.id] ?? 100,
    }));
    const unavailable = targets.find((target) => !target.codec);
    if (unavailable) {
      setError(`${protocolLabel(unavailable.connection.protocol)} is visible, but streaming backend for this protocol is not ready yet.`);
      setMessage("Protocol unavailable");
      return;
    }

    const connectionIds = targets.map((target) => target.connection.id);
    const volumes = Object.fromEntries(targets.map((target) => [target.connection.id, target.volume]));
    const started = await run(
      () => invoke<void>("start_selected_network_streams", { connectionIds, volumes }),
      `Starting ${targets.length} output${targets.length === 1 ? "" : "s"}`,
    );
    if (started !== null) {
      setStreamStarting(true);
    }
  };

  const stop = async () => {
    await run(() => invoke<void>("stop_network_stream"), "Streaming stopped");
    setStreamInfos([]);
    setStreamStarting(false);
  };

  const hideToTray = async () => {
    await run(() => invoke<void>("hide_main_window"), "Window hidden to tray");
  };

  const targetLabel =
    streamInfos.length > 0
      ? [...new Set(streamInfos.map((stream) => stream.target))].join(", ")
      : selectedTargets.length > 0
        ? [...new Set(selectedTargets.map(({ device }) => device.name))].join(", ")
        : selectedConnection
          ? `${selectedConnection.name} / ${protocolLabel(selectedConnection.protocol)}`
          : "No output";

  useEffect(() => {
    const refreshIfVisible = () => {
      if (document.visibilityState === "visible") void refreshSnapshot();
    };
    void refreshSnapshot();
    window.addEventListener("focus", refreshIfVisible);
    document.addEventListener("visibilitychange", refreshIfVisible);
    const id = window.setInterval(refreshIfVisible, 1500);
    return () => {
      window.clearInterval(id);
      window.removeEventListener("focus", refreshIfVisible);
      document.removeEventListener("visibilitychange", refreshIfVisible);
    };
  }, []);

  useEffect(() => {
    let unlistenVolume: (() => void) | undefined;
    let unlistenSnapshot: (() => void) | undefined;
    let unlistenStarted: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;
    void listen<{ volume: number; muted?: boolean }>("hotkey-volume-changed", (event) => {
      const volume = event.payload.volume;
      setMuted(Boolean(event.payload.muted));
      setTargetVolumes((current) => {
        const next = { ...current };
        for (const id of activeStreamIds) {
          next[id] = volume;
        }
        return next;
      });
    }).then((handler) => {
      unlistenVolume = handler;
    });
    void listen<AppSnapshot>("app-state-changed", (event) => {
      applySnapshot(event.payload);
    }).then((handler) => {
      unlistenSnapshot = handler;
    });
    void listen<StartedStreamInfo[]>("network-stream-started", (event) => {
      setStreamStarting(false);
      setStreamInfos(event.payload);
      setMessage(`Streaming to ${event.payload.length} output${event.payload.length === 1 ? "" : "s"}`);
      setError(null);
      void refreshSnapshot();
    }).then((handler) => {
      unlistenStarted = handler;
    });
    void listen<StreamErrorPayload>("network-stream-error", (event) => {
      setStreamStarting(false);
      setMessage("Error");
      setError(event.payload.message);
      void refreshSnapshot();
    }).then((handler) => {
      unlistenError = handler;
    });
    return () => {
      unlistenVolume?.();
      unlistenSnapshot?.();
      unlistenStarted?.();
      unlistenError?.();
    };
  }, [activeStreamIds]);

  useEffect(() => {
    window.localStorage.setItem(targetVolumeStorageKey, JSON.stringify(targetVolumes));
  }, [targetVolumes]);

  return (
    <main className="min-h-screen bg-gray-950 p-4 sm:p-6">
      <section className="mx-auto flex min-h-[calc(100vh-3rem)] max-w-5xl flex-col overflow-hidden rounded-lg border border-gray-800 bg-gray-900 shadow-2xl shadow-black/40">
        <header className="flex flex-wrap items-center justify-between gap-3 border-b border-gray-800 px-5 py-4">
          <div className="min-w-0">
            <h1 className="text-lg font-bold text-white">Network Audio Router</h1>
            <p className="mt-0.5 break-words text-xs font-mono text-gray-500 [overflow-wrap:anywhere]">{message}</p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <span className={`h-2.5 w-2.5 rounded-full ${streaming ? "animate-pulse bg-cyan-400" : "bg-gray-600"}`} />
            <span className={`text-xs font-mono ${streaming ? "text-cyan-400" : "text-gray-500"}`}>
              {streamStarting ? "STARTING" : muted ? "MUTED" : streaming ? "STREAMING" : "IDLE"}
            </span>
          </div>
        </header>

        <div className="grid min-w-0 gap-4 p-5 lg:grid-cols-[1.05fr_0.95fr]">
          <div className="grid min-w-0 gap-4">
            <div className="grid grid-cols-2 gap-3">
              <button
                onClick={scan}
                disabled={controlsBusy || scanning || streaming}
                className="flex h-11 min-w-0 items-center justify-center gap-2 rounded-md bg-gray-800 text-sm font-semibold text-gray-200 hover:bg-gray-700 disabled:cursor-not-allowed disabled:opacity-50"
              >
                {scanning ? <Spinner /> : null}
                Scan
              </button>
              <button
                onClick={hideToTray}
                disabled={controlsBusy}
                className="h-11 min-w-0 rounded-md border border-gray-700 px-2 text-sm font-semibold text-gray-300 hover:border-gray-500 hover:text-white disabled:opacity-50"
              >
                Hide to tray
              </button>
            </div>

            <div className="min-w-0 rounded-md border border-gray-800 bg-gray-950/50">
              <div className="flex flex-wrap items-center justify-between gap-2 border-b border-gray-800 px-4 py-3">
                <h2 className="text-sm font-bold text-white">Devices</h2>
                <span className="text-xs font-mono text-gray-500">
                  {devices.length} devices / {readyCount} ready
                </span>
              </div>

              <div className="max-h-[28rem] min-w-0 overflow-y-auto p-2">
                {devices.length === 0 ? (
                  <div className="rounded-md border border-dashed border-gray-800 px-3 py-6 text-center text-sm font-semibold text-gray-400">
                    No network devices yet
                  </div>
                ) : null}

                {devices.map((device) => {
                  const readyConnections = device.connections.filter(streamReady);
                  const selectedOnDevice = device.connections.filter((connection) => selectedConnectionIds.includes(connection.id)).length;
                  return (
                    <div
                      key={device.id}
                      className={`mb-2 min-w-0 rounded-md border px-3 py-3 transition-colors last:mb-0 ${
                        selectedDevice?.id === device.id ? "border-cyan-600 bg-cyan-950/30" : "border-gray-800 bg-gray-900"
                      }`}
                    >
                      <div className="grid min-w-0 gap-3 sm:grid-cols-[1fr_auto]">
                        <button onClick={() => selectDevice(device)} className="min-w-0 text-left">
                          <div className="break-words text-sm font-semibold text-white [overflow-wrap:anywhere]">{device.name}</div>
                          <div className="mt-1 break-all text-xs font-mono text-gray-500">{device.host}</div>
                        </button>
                        <div className="flex flex-wrap items-start gap-2 sm:justify-end">
                          <span className="rounded-sm border border-gray-700 px-1.5 py-0.5 text-[10px] font-bold uppercase text-gray-400">
                            {device.connections.length} profiles
                          </span>
                          <span className={`rounded-sm border px-1.5 py-0.5 text-[10px] font-bold uppercase ${
                            readyConnections.length > 0 ? "border-cyan-700 text-cyan-300" : "border-gray-700 text-gray-500"
                          }`}>
                            {readyConnections.length} ready
                          </span>
                          <span className={`rounded-sm border px-1.5 py-0.5 text-[10px] font-bold uppercase ${
                            selectedOnDevice > 0 ? "border-emerald-700 text-emerald-300" : "border-gray-700 text-gray-500"
                          }`}>
                            {selectedOnDevice} selected
                          </span>
                        </div>
                      </div>

                      <div className="mt-3 flex flex-wrap gap-2">
                        {protocolSummary(device).map((connection) => {
                          const checked = selectedConnectionIds.includes(connection.id);
                          const ready = streamReady(connection);
                          const selectable = streamSelectable(connection);
                          return (
                            <button
                              key={`${device.id}:${connection.protocol}`}
                              onClick={() => {
                                selectConnection(device, connection);
                                if (selectable) toggleConnection(connection);
                              }}
                              disabled={streaming}
                              className={`rounded-sm border px-2 py-1 text-[10px] font-bold uppercase disabled:opacity-40 ${
                                checked
                                  ? "border-emerald-600 bg-emerald-500/10 text-emerald-300"
                                  : selectedConnection?.id === connection.id
                                    ? "border-cyan-600 text-cyan-300"
                                    : ready
                                      ? "border-gray-700 text-gray-300"
                                      : selectable
                                        ? "border-amber-700 text-amber-300"
                                      : "border-gray-800 text-gray-600"
                              }`}
                            >
                              {protocolLabel(connection.protocol)}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          </div>

          <div className="grid min-w-0 content-start gap-4">
            <div className="min-w-0 rounded-md border border-gray-800 bg-gray-950/50">
              <div className="flex items-center justify-between gap-2 border-b border-gray-800 px-4 py-3">
                <h2 className="text-sm font-bold text-white">Capture</h2>
                <span className="max-w-[12rem] truncate text-right text-xs font-mono text-gray-500">{captureSourceLabel}</span>
              </div>
              <div className="grid max-h-44 min-w-0 gap-2 overflow-y-auto p-2">
                {audioSources.map((source) => (
                  <button
                    key={source.id}
                    onClick={() => void toggleSource(source)}
                    disabled={streaming || streamStarting}
                    className={`grid min-h-10 min-w-0 grid-cols-[1fr_auto] items-center gap-3 rounded-md border px-3 py-2 text-left disabled:opacity-40 ${
                      source.selected
                        ? "border-emerald-600 bg-emerald-500/10"
                        : "border-gray-800 bg-gray-900 hover:border-gray-700"
                    }`}
                  >
                    <span className="min-w-0 truncate text-xs font-semibold text-white">{source.name}</span>
                    <span
                      className={`rounded-sm border px-1.5 py-0.5 text-[10px] font-bold uppercase ${
                        source.kind === "system"
                          ? "border-cyan-700 text-cyan-300"
                          : source.selected
                            ? "border-emerald-700 text-emerald-300"
                            : "border-gray-700 text-gray-500"
                      }`}
                    >
                      {source.kind === "system" ? "system" : source.selected ? "on" : "app"}
                    </span>
                  </button>
                ))}
              </div>
            </div>

            <div className="min-w-0 rounded-md border border-gray-800 bg-gray-950/50">
              <div className="flex items-center justify-between gap-2 border-b border-gray-800 px-4 py-3">
                <h2 className="text-sm font-bold text-white">Outputs</h2>
                <span className="text-xs font-mono text-gray-500">{selectedTargets.length} selected</span>
              </div>
              <div className="grid max-h-72 min-w-0 gap-2 overflow-y-auto p-2">
                {selectedTargets.length === 0 ? (
                  <div className="h-12 rounded-md border border-gray-800 bg-gray-900" aria-hidden="true" />
                ) : null}
                {selectedTargets.map(({ device, connection }) => {
                  const volume = targetVolumes[connection.id] ?? 100;
                  const codec = autoCodec(connection);
                  return (
                    <div key={connection.id} className="min-w-0 rounded-md border border-gray-800 bg-gray-900 px-3 py-3">
                      <div className="grid min-w-0 gap-2 sm:grid-cols-[1fr_auto]">
                        <div className="min-w-0">
                          <div className="break-words text-sm font-bold text-white [overflow-wrap:anywhere]">{device.name}</div>
                          <div className="break-words text-[11px] font-mono text-gray-500 [overflow-wrap:anywhere]">
                            {protocolLabel(connection.protocol)} / {codec ? codecLabel(codec) : "No codec"}
                          </div>
                        </div>
                        <button
                          onClick={() => toggleConnection(connection)}
                          disabled={streaming}
                          className="h-8 rounded-md border border-gray-700 px-2 text-[11px] font-bold text-gray-400 disabled:opacity-40"
                        >
                          Remove
                        </button>
                      </div>
                      <label className="mt-3 grid grid-cols-[1fr_52px] items-center gap-3">
                        <input
                          type="range"
                          min={0}
                          max={150}
                          value={volume}
                          onChange={(event) => setVolume(connection.id, Number(event.target.value))}
                          className="h-2 w-full cursor-pointer accent-cyan-400"
                        />
                        <span className="text-right text-xs font-mono text-cyan-300">{volume}%</span>
                      </label>
                    </div>
                  );
                })}
              </div>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <button
                onClick={start}
                disabled={controlsBusy || streaming || selectedTargets.length === 0}
                className="h-12 rounded-md bg-cyan-500 text-sm font-black text-gray-950 shadow-lg shadow-cyan-500/20 hover:bg-cyan-400 disabled:cursor-not-allowed disabled:bg-gray-700 disabled:text-gray-500 disabled:shadow-none"
              >
                Start
              </button>
              <button
                onClick={stop}
                disabled={busy || (!streaming && !streamStarting)}
                className="h-12 rounded-md border border-red-500/40 bg-red-600/15 text-sm font-bold text-red-300 hover:bg-red-600/25 disabled:cursor-not-allowed disabled:border-gray-800 disabled:bg-gray-800 disabled:text-gray-600"
              >
                Stop
              </button>
            </div>
          </div>

          <div className="min-w-0 rounded-md border border-gray-800 bg-black/20 px-4 py-3 font-mono text-xs leading-5 lg:col-span-2">
            <div className="text-gray-400">
              Capture: <span className="text-white">{captureSourceLabel}</span>
            </div>
            <div className="text-gray-400">
              Transport: <span className="text-white">
                {streamInfos.length > 0
                  ? [...new Set(streamInfos.map((s) => s.transport))].join(", ")
                  : status.transport}
              </span>
            </div>
            <div className="text-gray-400">
              Codec: <span className="text-white">
                {streamInfos.length > 0
                  ? [...new Set(streamInfos.map((s) => s.codec))].join(", ")
                  : codecLabel(autoCodec(selectedConnection) || "pcm_s16le")}
              </span>
            </div>
            <div className="text-gray-400">
              Target: <span className="break-words text-white [overflow-wrap:anywhere]">{targetLabel}</span>
            </div>
            {streamInfos[0]?.control_url ? (
              <div className="text-gray-400">
                Control: <span className="break-words text-white [overflow-wrap:anywhere]">{streamInfos[0].control_url}</span>
              </div>
            ) : null}
            {error ? <div className="mt-2 whitespace-pre-wrap break-words text-red-300 [overflow-wrap:anywhere]">{error}</div> : null}
          </div>
        </div>
      </section>
    </main>
  );
}
