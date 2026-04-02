<script lang="ts">
  import { onMount } from "svelte";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { Window } from "@tauri-apps/api/window";
  import type { EventFilters, GameEvent, Match, NetworkSample } from "./api.js";
  import { api, defaultEventFilters, formatDuration, isEventVisible } from "./api.js";
  import NetworkDiagnostics, { type NetMetrics } from "./NetworkDiagnostics.svelte";
  import Timeline from "./Timeline.svelte";

  interface Props {
    match: Match | null;
    events: GameEvent[];
    networkSamples: NetworkSample[];
  }

  let { match, events, networkSamples }: Props = $props();

  let videoEl: HTMLVideoElement = $state(undefined!);
  let currentTime = $state(0);
  let duration = $state(0);
  let paused = $state(true);
  let speed = $state(1);
  let muted = $state(false);
  let eventFilters = $state<EventFilters>(defaultEventFilters());
  let filtersHydrated = $state(false);
  let showFilters = $state(false);
  let filterMenuEl: HTMLDivElement = $state(undefined!);
  let filterButtonEl: HTMLButtonElement = $state(undefined!);
  let netMetrics = $state<NetMetrics | null>(null);

  const SPEEDS = [0.5, 1, 1.5, 2];
  let mediaKey = $derived(match ? `${match.id}:${match.video_path}` : "empty");
  let visibleEvents = $derived(events.filter((event) => isEventVisible(event, eventFilters)));

  $effect(() => {
    mediaKey;
    currentTime = 0;
    duration = 0;
    paused = true;
  });

  function convertToAssetUrl(windowsPath: string): string {
    return convertFileSrc(windowsPath);
  }

  function togglePlay() {
    if (!videoEl) return;
    if (videoEl.paused) {
      videoEl.play();
    } else {
      videoEl.pause();
    }
  }

  function seekTo(t: number) {
    if (!videoEl) return;
    videoEl.currentTime = Math.max(0, Math.min(duration, t));
  }

  function cycleSpeed() {
    const idx = SPEEDS.indexOf(speed);
    speed = SPEEDS[(idx + 1) % SPEEDS.length];
    if (videoEl) videoEl.playbackRate = speed;
  }

  function skipBackward() {
    seekTo(currentTime - 5);
  }

  function skipForward() {
    seekTo(currentTime + 5);
  }

  function toggleMute() {
    muted = !muted;
    if (videoEl) videoEl.muted = muted;
  }

  function isTextEditingTarget(target: EventTarget | null): boolean {
    return target instanceof HTMLElement
      && (target instanceof HTMLInputElement
        || target instanceof HTMLTextAreaElement
        || target.isContentEditable);
  }

  function pausePlayback() {
    if (!videoEl) return;
    videoEl.pause();
  }

  function toggleFilters() {
    showFilters = !showFilters;
  }

  function closeFilters() {
    showFilters = false;
  }

  async function persistEventFilters(snapshot: EventFilters) {
    const cfg = await api.getSettings();
    cfg.app.event_filters = snapshot;
    await api.saveSettings(cfg);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (isTextEditingTarget(e.target)) return;
    switch (e.key) {
      case " ":
        e.preventDefault();
        togglePlay();
        break;
      case "ArrowLeft":
        e.preventDefault();
        seekTo(currentTime - 5);
        break;
      case "ArrowRight":
        e.preventDefault();
        seekTo(currentTime + 5);
        break;
      case "m":
        toggleMute();
        break;
      case "Escape":
        if (showFilters) {
          closeFilters();
        }
        break;
    }
  }

  onMount(() => {
    let unlistenClose = () => {};
    let unlistenGameStarted = () => {};

    function handleVisibilityChange() {
      if (document.hidden) {
        pausePlayback();
        closeFilters();
      }
    }

    function handlePointerDown(event: PointerEvent) {
      const target = event.target;
      if (!(target instanceof Node) || !showFilters) {
        return;
      }

      if (filterMenuEl?.contains(target) || filterButtonEl?.contains(target)) {
        return;
      }

      closeFilters();
    }

    document.addEventListener("keydown", handleKeydown);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    document.addEventListener("pointerdown", handlePointerDown);

    void (async () => {
      const cfg = await api.getSettings();
      eventFilters = cfg.app.event_filters ?? defaultEventFilters();
      filtersHydrated = true;
      unlistenClose = await Window.getCurrent().onCloseRequested(() => {
        pausePlayback();
      });
      unlistenGameStarted = await listen("game-started", () => {
        pausePlayback();
        closeFilters();
      });
    })();

    return () => {
      document.removeEventListener("keydown", handleKeydown);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      document.removeEventListener("pointerdown", handlePointerDown);
      unlistenClose();
      unlistenGameStarted();
    };
  });

  $effect(() => {
    const snapshot: EventFilters = {
      kill: eventFilters.kill,
      death: eventFilters.death,
      assist: eventFilters.assist,
      dragon: eventFilters.dragon,
      baron: eventFilters.baron,
      herald: eventFilters.herald,
      turret: eventFilters.turret,
      inhibitor: eventFilters.inhibitor,
    };

    if (!filtersHydrated) {
      return;
    }

    void persistEventFilters(snapshot);
  });

</script>

<div class="player">
  {#if !match}
    <div class="empty-state">
      <p>Select a recording to review</p>
    </div>
  {:else}
    {#key mediaKey}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={videoEl}
        bind:currentTime
        bind:duration
        bind:paused
        class="video"
        preload="metadata"
        src={convertToAssetUrl(match.video_path)}
        onclick={togglePlay}
      ></video>

      <div class="controls">
        <div class="left-controls">
          <button class="ctrl-btn" onclick={skipBackward} title="Back 5s (Left Arrow)">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <path d="M12 5V1L7 6l5 5V7c3.31 0 6 2.69 6 6s-2.69 6-6 6-6-2.69-6-6H4c0 4.42 3.58 8 8 8s8-3.58 8-8-3.58-8-8-8z"/>
              <text x="10" y="15" font-size="7" fill="currentColor" text-anchor="middle">5</text>
            </svg>
          </button>

          <button class="ctrl-btn play-btn" onclick={togglePlay} title="Play/Pause (Space)">
            {#if !paused}
              <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                <path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/>
              </svg>
            {:else}
              <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                <path d="M8 5v14l11-7z"/>
              </svg>
            {/if}
          </button>

          <button class="ctrl-btn" onclick={skipForward} title="Forward 5s (Right Arrow)">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <path d="M12 5V1l5 5-5 5V7c-3.31 0-6 2.69-6 6s2.69 6 6 6 6-2.69 6-6h2c0 4.42-3.58 8-8 8s-8-3.58-8-8 3.58-8 8-8z"/>
              <text x="14" y="15" font-size="7" fill="currentColor" text-anchor="middle">5</text>
            </svg>
          </button>

          <button class="ctrl-btn mute-btn" onclick={toggleMute} title="Toggle mute (M)">
            {#if muted}
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                <path d="M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z"/>
              </svg>
            {:else}
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"/>
              </svg>
            {/if}
          </button>

          <button class="speed-btn" onclick={cycleSpeed} title="Change playback speed">
            {speed}x
          </button>

          <div class="filter-menu-wrap">
            <button
              bind:this={filterButtonEl}
              class="ctrl-btn filter-btn"
              class:active={showFilters}
              type="button"
              title="Filter timeline events"
              aria-label="Filter timeline events"
              aria-expanded={showFilters}
              onclick={toggleFilters}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                <path d="M3 5h18l-7 8v5l-4 2v-7z"></path>
              </svg>
            </button>

            {#if showFilters}
              <div bind:this={filterMenuEl} class="filter-menu" aria-label="Timeline event filters">
                <label class="filter-chip">
                  <input type="checkbox" bind:checked={eventFilters.kill} />
                  <span>Kill</span>
                </label>
                <label class="filter-chip">
                  <input type="checkbox" bind:checked={eventFilters.death} />
                  <span>Death</span>
                </label>
                <label class="filter-chip">
                  <input type="checkbox" bind:checked={eventFilters.assist} />
                  <span>Assist</span>
                </label>
                <label class="filter-chip">
                  <input type="checkbox" bind:checked={eventFilters.dragon} />
                  <span>Dragon</span>
                </label>
                <label class="filter-chip">
                  <input type="checkbox" bind:checked={eventFilters.baron} />
                  <span>Baron</span>
                </label>
                <label class="filter-chip">
                  <input type="checkbox" bind:checked={eventFilters.herald} />
                  <span>Herald</span>
                </label>
                <label class="filter-chip">
                  <input type="checkbox" bind:checked={eventFilters.turret} />
                  <span>Tower</span>
                </label>
                <label class="filter-chip">
                  <input type="checkbox" bind:checked={eventFilters.inhibitor} />
                  <span>Inhib</span>
                </label>
              </div>
            {/if}
          </div>
        </div>

        <div class="track-column">
          <Timeline
            events={visibleEvents}
            {duration}
            {currentTime}
            onSeek={seekTo}
          />
        </div>

        <span class="time-display">
          {formatDuration(Math.round(currentTime))} / {formatDuration(Math.round(duration))}
        </span>

        {#if networkSamples.length > 0}
          <div class="ping-metrics">
            {#if netMetrics}
              <div class="ping-status-row">
                <span class="ping-status" class:degraded={netMetrics.statusLabel !== "Stable"}>{netMetrics.statusLabel}</span>
                <span class="ping-target" title={netMetrics.activeGroupKey ?? ""}>{netMetrics.targetLabel}</span>
              </div>
              <div class="ping-stats-row">
                <span>avg {netMetrics.avgMs == null ? "n/a" : `${netMetrics.avgMs.toFixed(0)} ms`}</span>
                <span>p95 {netMetrics.p95Ms == null ? "n/a" : `${netMetrics.p95Ms.toFixed(0)} ms`}</span>
                <span>max {netMetrics.maxMs == null ? "n/a" : `${netMetrics.maxMs.toFixed(0)} ms`}</span>
                <span>loss {netMetrics.lossPct.toFixed(1)}%</span>
                <span>spikes {netMetrics.spikeCount}</span>
              </div>
            {/if}
          </div>
          <div class="ping-track">
            <NetworkDiagnostics
              samples={networkSamples}
              {duration}
              {currentTime}
              onSeek={seekTo}
              onMetrics={(m) => { netMetrics = m; }}
            />
          </div>
        {/if}
      </div>
    {/key}
  {/if}
</div>

<style>
  .player {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: #000;
    min-height: 0;
  }

  .empty-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
    font-size: 14px;
    background: var(--bg-surface);
  }

  .video {
    flex: 1;
    min-height: 0;
    width: 100%;
    object-fit: contain;
    background: #000;
    cursor: pointer;
    display: block;
  }

  .controls {
    background: var(--bg-surface);
    border-top: 1px solid var(--border);
    padding: 6px 12px 7px 0;
    flex-shrink: 0;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    column-gap: 10px;
    row-gap: 4px;
    align-items: center;
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--bg-raised);
    color: var(--text-secondary);
    font-size: 11px;
    line-height: 1.2;
    cursor: pointer;
    user-select: none;
  }

  .filter-chip input {
    margin: 0;
    accent-color: var(--accent);
  }

  .track-column {
    min-width: 0;
    display: grid;
    gap: 4px;
  }

  .ping-metrics {
    grid-column: 1;
    grid-row: 2;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 3px;
    font-size: 10px;
    color: var(--text-muted);
    line-height: 1.3;
    padding-left: 4px;
  }

  .ping-status-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .ping-status {
    color: var(--win);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .ping-status.degraded {
    color: var(--loss);
  }

  .ping-target {
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ping-stats-row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .ping-track {
    grid-column: 2;
    grid-row: 2;
    min-width: 0;
  }

  .time-display {
    font-size: 12px;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    min-width: 90px;
    text-align: right;
  }

  .left-controls {
    grid-column: 1;
    grid-row: 1;
    display: flex;
    align-items: center;
    gap: 4px;
    position: relative;
  }

  .ctrl-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    color: var(--text-secondary);
    transition: color var(--transition), background var(--transition);
  }

  .ctrl-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .filter-btn.active {
    color: var(--accent);
    background: var(--bg-hover);
  }

  .filter-menu-wrap {
    position: relative;
  }

  .filter-menu {
    position: absolute;
    left: 0;
    bottom: calc(100% + 8px);
    width: 184px;
    display: grid;
    gap: 6px;
    padding: 10px;
    background: color-mix(in srgb, var(--bg-surface) 94%, black);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
    z-index: 10;
  }

  .play-btn {
    width: 32px;
    height: 32px;
    color: var(--text-primary);
    background: var(--bg-raised);
  }

  .play-btn:hover {
    background: var(--bg-hover);
  }

  .speed-btn {
    padding: 4px 8px;
    border-radius: var(--radius);
    background: var(--bg-raised);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 600;
    transition: color var(--transition), background var(--transition);
    min-width: 34px;
    text-align: center;
  }

  .speed-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }
</style>
