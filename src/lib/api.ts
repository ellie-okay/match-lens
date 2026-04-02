import { invoke } from "@tauri-apps/api/core";

export interface Match {
  id: number;
  game_id: string;
  summoner_name: string;
  champion: string;
  kills: number;
  deaths: number;
  assists: number;
  result: string;   // "Win" | "Loss" | "Remake" | "Unknown"
  duration_sec: number;
  recorded_at: string;
  video_path: string;
  file_size_bytes: number;
}

export interface GameEvent {
  id: number;
  match_id: number;
  event_type: string;  // Kill, Death, Assist, Dragon, Baron, Herald, Turret, Inhibitor
  timestamp_sec: number;
  is_local_player: boolean;
  raw_data: string;
}

export interface NetworkSample {
  id: number;
  match_id: number;
  timestamp_sec: number;
  target: string;
  rtt_ms: number | null;
  timed_out: boolean;
  error: string | null;
}

export interface EventFilters {
  kill: boolean;
  death: boolean;
  assist: boolean;
  dragon: boolean;
  baron: boolean;
  herald: boolean;
  turret: boolean;
  inhibitor: boolean;
}

export interface Config {
  recording: {
    resolution: string;
    audio_mode: string;
    output_dir: string;
  };
  storage: {
    max_gb: number;
  };
  app: {
    autostart: boolean;
    theme: string;
    event_filters: EventFilters;
  };
}

export interface RecordingStatus {
  recording: boolean;
  output_path?: string;
  elapsed_secs?: number;
}

export interface StorageUsage {
  used_gb: number;
  max_gb: number;
}

export const api = {
  getMatches: (): Promise<Match[]> => invoke("get_matches"),
  getEvents: (match_id: number): Promise<GameEvent[]> => invoke("get_events", { matchId: match_id }),
  getNetworkSamples: (match_id: number): Promise<NetworkSample[]> =>
    invoke("get_network_samples", { matchId: match_id }),
  getSettings: (): Promise<Config> => invoke("get_settings"),
  saveSettings: (cfg: Config): Promise<void> => invoke("save_settings", { cfg }),
  getRecordingStatus: (): Promise<RecordingStatus> => invoke("get_recording_status"),
  deleteRecording: (match_id: number): Promise<void> => invoke("delete_recording", { matchId: match_id }),
  openRecordingsFolder: (): Promise<void> => invoke("open_recordings_folder"),
  getStorageUsage: (): Promise<StorageUsage> => invoke("get_storage_usage"),
};

export function defaultEventFilters(): EventFilters {
  return {
    kill: true,
    death: true,
    assist: true,
    dragon: true,
    baron: true,
    herald: true,
    turret: false,
    inhibitor: true,
  };
}

export function formatDuration(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export function formatDate(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString("en-GB", {
    month:  "short",
    day:    "numeric",
    hour:   "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

export function eventColor(type: string): string {
  switch (type) {
    case "Kill":     return "var(--kill)";
    case "Death":    return "var(--death)";
    case "Assist":   return "var(--assist)";
    case "Dragon":   return "var(--dragon)";
    case "Baron":    return "var(--baron)";
    case "Herald":
    case "Turret":
    case "Inhibitor":return "var(--herald)";
    default:         return "var(--text-muted)";
  }
}

export function eventIcon(type: string): string {
  switch (type) {
    case "Kill":     return "🟡";
    case "Death":    return "🔴";
    case "Assist":   return "🟢";
    case "Dragon":   return "🟣";
    case "Baron":    return "🔵";
    case "Herald":   return "⚪";
    case "Turret":   return "⚪";
    case "Inhibitor":return "⚪";
    default:         return "·";
  }
}

export function eventLabel(event: GameEvent): string {
  const raw = JSON.parse(event.raw_data || "{}");
  switch (event.event_type) {
    case "Kill":
      return `You killed ${raw.VictimName || "enemy"}`;
    case "Death":
      return `Killed by ${raw.KillerName || "enemy"}`;
    case "Assist":
      return `Assist on ${raw.VictimName || "enemy"}`;
    case "Dragon":
      return `${raw.DragonType || "Dragon"} Dragon`;
    case "Baron":
      return "Baron Nashor";
    case "Herald":
      return "Rift Herald";
    case "Turret":
      return "Tower destroyed";
    case "Inhibitor":
      return `Inhibitor ${raw.InhibitorKilled || ""}`;
    default:
      return event.event_type;
  }
}

export function isEventVisible(event: GameEvent, filters: EventFilters): boolean {
  switch (event.event_type) {
    case "Kill": return filters.kill;
    case "Death": return filters.death;
    case "Assist": return filters.assist;
    case "Dragon": return filters.dragon;
    case "Baron": return filters.baron;
    case "Herald": return filters.herald;
    case "Turret": return filters.turret;
    case "Inhibitor": return filters.inhibitor;
    default: return true;
  }
}
