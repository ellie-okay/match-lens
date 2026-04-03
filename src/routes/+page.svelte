<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { Window } from "@tauri-apps/api/window";
  import { api } from "$lib/api.js";
  import type { Match, GameEvent, NetworkSample } from "$lib/api.js";
  import Sidebar from "$lib/Sidebar.svelte";
  import VideoPlayer from "$lib/VideoPlayer.svelte";

  let matches: Match[] = $state([]);
  let selectedMatch: Match | null = $state(null);
  let events: GameEvent[] = $state([]);
  let networkSamples: NetworkSample[] = $state([]);
  let recordingStatus = $state<{ recording: boolean; elapsed_secs?: number }>({ recording: false });
  let loading = $state(true);
  let deleteError = $state<string | null>(null);
  let lastKnownLatestMatchId: number | null = $state(null);
  let sidebarCollapsed = $state(false);
  // When recording stops, track how many matches existed so we can detect when the new one lands.
  let matchCountAtRecordingStop = -1;

  onMount(() => {
    let unlisten = () => {};
    let unlistenFocus = () => {};
    const interval = setInterval(refreshRecordingStatus, 2000);

    void (async () => {
      await loadMatches();
      await refreshRecordingStatus();
      loading = false;

      unlisten = await listen<{ match_id: number }>("game-recorded", async (event) => {
        await loadMatches({ selectMatchId: event.payload.match_id, autoSelectLatestOnNewTop: true });
      });

      unlistenFocus = await Window.getCurrent().onFocusChanged(({ payload: focused }) => {
        if (focused) void loadMatches({ autoSelectLatestOnNewTop: true });
      });
    })();

    return () => {
      unlisten();
      unlistenFocus();
      clearInterval(interval);
    };
  });

  async function loadMatches(
    options: { selectMatchId?: number; autoSelectLatestOnNewTop?: boolean } = {}
  ) {
    const previousLatestMatchId = matches[0]?.id ?? lastKnownLatestMatchId;
    const previousSelectedId = selectedMatch?.id ?? null;
    const nextMatches = await api.getMatches();

    matches = nextMatches;
    lastKnownLatestMatchId = matches[0]?.id ?? null;

    if (options.selectMatchId != null) {
      const requested = matches.find((m) => m.id === options.selectMatchId);
      if (requested) {
        await selectMatch(requested);
        return;
      }
    }

    if (
      options.autoSelectLatestOnNewTop &&
      matches.length > 0 &&
      matches[0].id !== previousLatestMatchId
    ) {
      await selectMatch(matches[0]);
      return;
    }

    if (previousSelectedId != null) {
      const refreshedSelection = matches.find((m) => m.id === previousSelectedId);
      if (refreshedSelection) {
        selectedMatch = refreshedSelection;
        return;
      }
    }

    if (!selectedMatch && matches.length > 0) {
      await selectMatch(matches[0]);
    }
  }

  async function selectMatch(m: Match) {
    selectedMatch = m;
    const [nextEvents, nextNetworkSamples] = await Promise.all([
      api.getEvents(m.id),
      api.getNetworkSamples(m.id),
    ]);
    events = nextEvents;
    networkSamples = nextNetworkSamples;
  }

  async function refreshRecordingStatus() {
    const prevRecording = recordingStatus.recording;
    recordingStatus = await api.getRecordingStatus();

    if (prevRecording && !recordingStatus.recording) {
      // Recording just stopped — remember how many matches exist now so we can
      // detect when the newly-processed match lands in the database.
      matchCountAtRecordingStop = matches.length;
    }

    if (matchCountAtRecordingStop >= 0) {
      if (recordingStatus.recording) {
        // A new game started before the previous one finished processing — give up.
        matchCountAtRecordingStop = -1;
      } else {
        // Keep refreshing the list until the new match appears.
        await loadMatches({ autoSelectLatestOnNewTop: true });
        if (matches.length > matchCountAtRecordingStop) {
          matchCountAtRecordingStop = -1;
        }
      }
    }
  }

  async function handleDelete(m: Match) {
    deleteError = null;
    try {
      await api.deleteRecording(m.id);
      if (selectedMatch?.id === m.id) {
        selectedMatch = null;
        events = [];
        networkSamples = [];
      }
    } catch (e) {
      deleteError = String(e);
    } finally {
      await loadMatches();
    }
  }
</script>

<div class="app">
  {#if recordingStatus.recording}
    <div class="recording-banner">
      <span class="rec-dot"></span>
      Recording in progress - {Math.floor((recordingStatus.elapsed_secs ?? 0) / 60)}:{String(
        (recordingStatus.elapsed_secs ?? 0) % 60
      ).padStart(2, "0")}
    </div>
  {/if}

  <div class="layout">
    <Sidebar
      {matches}
      selectedId={selectedMatch?.id ?? null}
      collapsed={sidebarCollapsed}
      onSelect={selectMatch}
      onDelete={handleDelete}
    />

    <button
      class="sidebar-toggle"
      class:collapsed={sidebarCollapsed}
      type="button"
      onclick={() => { sidebarCollapsed = !sidebarCollapsed; }}
      aria-label={sidebarCollapsed ? "Expand recordings sidebar" : "Collapse recordings sidebar"}
      aria-expanded={!sidebarCollapsed}
      title={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M15.41 7.41 10.83 12l4.58 4.59L14 18l-6-6 6-6z"></path>
      </svg>
    </button>

    <div class="main">
      {#if deleteError}
        <div class="delete-error">Failed to delete: {deleteError}</div>
      {/if}

      <VideoPlayer match={selectedMatch} {events} networkSamples={networkSamples} />
    </div>
  </div>
</div>

<style>
  :global(body) {
    overflow: hidden;
  }

  .app {
    height: 100vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .recording-banner {
    background: rgba(192, 57, 43, 0.15);
    border-bottom: 1px solid rgba(192, 57, 43, 0.4);
    color: #e66;
    padding: 6px 16px;
    font-size: 12px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .rec-dot {
    width: 8px;
    height: 8px;
    background: #e44;
    border-radius: 50%;
    animation: pulse 1.2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }

  .layout {
    flex: 1;
    display: flex;
    min-height: 0;
    overflow: hidden;
    position: relative;
  }

  .sidebar-toggle {
    position: absolute;
    left: 12px;
    top: 8px;
    z-index: 10;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    color: var(--text-secondary);
    background: var(--bg-raised);
    flex-shrink: 0;
    transition: color var(--transition), background var(--transition), box-shadow var(--transition);
  }

  .sidebar-toggle:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .sidebar-toggle svg {
    width: 16px;
    height: 16px;
    fill: currentColor;
  }

  .sidebar-toggle.collapsed {
    transform: rotate(180deg);
    background: var(--bg-surface);
    border: 1px solid var(--border);
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.35);
  }

  .sidebar-toggle.collapsed:hover {
    background: var(--bg-hover);
  }

  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
    background: var(--bg-base);
  }

  .delete-error {
    margin: 6px 16px;
    padding: 6px 10px;
    background: rgba(192, 57, 43, 0.15);
    border: 1px solid rgba(192, 57, 43, 0.4);
    border-radius: var(--radius);
    color: var(--loss);
    font-size: 12px;
  }
</style>
