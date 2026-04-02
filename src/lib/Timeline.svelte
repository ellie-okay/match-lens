<script lang="ts">
  import type { GameEvent } from "./api.js";
  import { eventColor, eventIcon, eventLabel, formatDuration } from "./api.js";

  interface Props {
    events: GameEvent[];
    duration: number;        // video duration in seconds
    currentTime: number;     // current playback position in seconds
    onSeek: (t: number) => void;
  }

  let { events, duration, currentTime, onSeek }: Props = $props();
  const MARKER_SEEK_LEAD_SEC = 10;

  let tooltipEvent: GameEvent | null = $state(null);
  let tooltipX = $state(0);
  let tooltipY = $state(0);

  function markerLeft(ts: number): number {
    if (duration <= 0) return 0;
    return Math.min(100, Math.max(0, (ts / duration) * 100));
  }

  function handleMarkerClick(e: MouseEvent, event: GameEvent) {
    e.stopPropagation();
    onSeek(Math.max(0, event.timestamp_sec - MARKER_SEEK_LEAD_SEC));
  }

  function handleTrackClick(e: MouseEvent) {
    const track = e.currentTarget as HTMLDivElement;
    const rect = track.getBoundingClientRect();
    const ratio = rect.width > 0 ? (e.clientX - rect.left) / rect.width : 0;
    onSeek(Math.max(0, Math.min(duration, ratio * duration)));
  }

  function handleMarkerEnter(e: MouseEvent, event: GameEvent) {
    tooltipEvent = event;
    tooltipX = (e.target as HTMLElement).getBoundingClientRect().left;
    tooltipY = (e.target as HTMLElement).getBoundingClientRect().top;
  }

  function handleMarkerLeave() {
    tooltipEvent = null;
  }
</script>

<div class="timeline-track">
  <button
    class="track-hitbox"
    type="button"
    aria-label="Jump to a point in the timeline"
    onclick={handleTrackClick}
  ></button>

  {#if duration > 0}
    <div
      class="timeline-progress"
      style="width: {markerLeft(currentTime)}%;"
    ></div>
  {/if}

  {#each events as event (event.id)}
    <button
      type="button"
      class="marker"
      style="left: {markerLeft(event.timestamp_sec)}%; color: {eventColor(event.event_type)};"
      title={`${eventLabel(event)} (${formatDuration(Math.round(event.timestamp_sec))})`}
      onclick={(e) => handleMarkerClick(e, event)}
      onmouseenter={(e) => handleMarkerEnter(e, event)}
      onmouseleave={handleMarkerLeave}
      aria-label="{eventLabel(event)} at {formatDuration(Math.round(event.timestamp_sec))}"
    >
      {eventIcon(event.event_type)}
    </button>
  {/each}

  <!-- Playhead indicator -->
  {#if duration > 0}
    <div
      class="playhead"
      style="left: {markerLeft(currentTime)}%;"
    ></div>
  {/if}
</div>

{#if tooltipEvent}
  <div
    class="tooltip"
    style="left: {tooltipX}px; top: {tooltipY - 32}px;"
  >
    {eventIcon(tooltipEvent.event_type)}
    {eventLabel(tooltipEvent)}
    <span class="tooltip-time">{formatDuration(Math.round(tooltipEvent.timestamp_sec))}</span>
  </div>
{/if}

<style>
  .timeline-track {
    position: relative;
    height: 24px;
    background: var(--bg-raised);
    border-radius: var(--radius);
    margin: 2px 0;
    overflow: visible;
    cursor: pointer;
  }

  .track-hitbox {
    position: absolute;
    inset: 0;
    border: none;
    background: none;
    padding: 0;
    cursor: pointer;
    z-index: 0;
  }

  .timeline-progress {
    position: absolute;
    inset: 0 auto 0 0;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    border-radius: var(--radius);
    pointer-events: none;
    z-index: 0;
  }

  .marker {
    position: absolute;
    transform: translateX(-50%);
    top: 2px;
    font-size: 12px;
    line-height: 1;
    padding: 0;
    background: none;
    border: none;
    cursor: pointer;
    transition: transform var(--transition), opacity var(--transition);
    z-index: 2;
    filter: drop-shadow(0 0 3px currentColor);
  }

  .marker:hover {
    transform: translateX(-50%) scale(1.4);
    z-index: 3;
  }

  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--accent);
    transform: translateX(-50%);
    z-index: 1;
    pointer-events: none;
  }

  .tooltip {
    position: fixed;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 4px 10px;
    font-size: 12px;
    color: var(--text-primary);
    pointer-events: none;
    z-index: 1000;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 5px;
    box-shadow: 0 4px 12px rgba(0,0,0,0.5);
  }

  .tooltip-time {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
