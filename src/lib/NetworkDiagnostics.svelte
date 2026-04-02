<script lang="ts">
  import type { NetworkSample } from "./api.js";

  export type NetMetrics = {
    statusLabel: string;
    avgMs: number | null;
    p95Ms: number | null;
    maxMs: number | null;
    lossPct: number;
    spikeCount: number;
    targetLabel: string;
    activeGroupKey: string | null;
  };

  type TargetGroup = {
    key: string;
    label: string;
    samples: NetworkSample[];
    okSamples: NetworkSample[];
  };

  interface Props {
    samples: NetworkSample[];
    duration: number;
    currentTime: number;
    onSeek: (t: number) => void;
    onMetrics?: (metrics: NetMetrics) => void;
  }

  let { samples, duration, currentTime, onSeek, onMetrics }: Props = $props();

  function formatTargetLabel(target: string): string {
    if (target.startsWith("riot:")) {
      return `Riot route ${target.slice("riot:".length)}`;
    }

    if (target.startsWith("control:")) {
      return `Control ${target.slice("control:".length)}`;
    }

    return target;
  }

  function targetPriority(group: TargetGroup): number {
    if (group.key.startsWith("riot:") && group.okSamples.length >= 3) return 3;
    if (group.okSamples.length > 0) return 2;
    if (group.key.startsWith("riot:")) return 1;
    return 0;
  }

  let targetGroups = $derived.by(() => {
    const groups = new Map<string, NetworkSample[]>();

    for (const sample of samples) {
      const existing = groups.get(sample.target) ?? [];
      existing.push(sample);
      groups.set(sample.target, existing);
    }

    return Array.from(groups.entries())
      .map(([key, groupedSamples]) => ({
        key,
        label: formatTargetLabel(key),
        samples: groupedSamples,
        okSamples: groupedSamples.filter((sample) => sample.rtt_ms != null),
      }))
      .sort((left, right) =>
        targetPriority(right) - targetPriority(left)
        || right.okSamples.length - left.okSamples.length
        || right.samples.length - left.samples.length
        || left.key.localeCompare(right.key)
      );
  });

  let activeGroup = $derived(targetGroups[0] ?? null);
  let displaySamples = $derived(activeGroup?.samples ?? []);
  let okSamples = $derived(displaySamples.filter((sample) => sample.rtt_ms != null));
  let timeoutCount = $derived(displaySamples.filter((sample) => sample.timed_out || sample.rtt_ms == null).length);
  let avgMs = $derived(okSamples.length > 0
    ? okSamples.reduce((sum, sample) => sum + (sample.rtt_ms ?? 0), 0) / okSamples.length
    : null);
  let sortedRtts = $derived(
    okSamples
      .map((sample) => sample.rtt_ms ?? 0)
      .sort((a, b) => a - b)
  );
  let medianMs = $derived(sortedRtts.length > 0
    ? sortedRtts[Math.floor(sortedRtts.length / 2)]
    : null);
  let p95Ms = $derived(sortedRtts.length > 0
    ? sortedRtts[Math.min(sortedRtts.length - 1, Math.floor(sortedRtts.length * 0.95))]
    : null);
  let maxMs = $derived(sortedRtts.length > 0 ? sortedRtts[sortedRtts.length - 1] : null);
  let lossPct = $derived(displaySamples.length > 0 ? (timeoutCount / displaySamples.length) * 100 : 0);
  let spikeThresholdMs = $derived(
    Math.max(35, (medianMs ?? avgMs ?? 0) + 20, (medianMs ?? avgMs ?? 0) * 2.25)
  );
  let spikeCount = $derived(okSamples.filter((sample) => (sample.rtt_ms ?? 0) >= spikeThresholdMs).length);
  let graphMaxMs = $derived(Math.max(150, maxMs ?? 0, (p95Ms ?? 0) * 1.5));

  let graphPoints = $derived.by(() => {
    if (duration <= 0 || okSamples.length === 0 || graphMaxMs <= 0) {
      return "";
    }

    return okSamples
      .map((sample) => {
        const x = (sample.timestamp_sec / duration) * 100;
        const y = 100 - (((sample.rtt_ms ?? 0) / graphMaxMs) * 100);
        return `${Math.max(0, Math.min(100, x)).toFixed(2)},${Math.max(0, Math.min(100, y)).toFixed(2)}`;
      })
      .join(" ");
  });

  let statusLabel = $derived.by(() => {
    if (displaySamples.length === 0) return "No data";
    if (lossPct >= 5 || (maxMs ?? 0) >= 200) return "Severe";
    if (lossPct >= 1 || spikeCount >= 3 || (p95Ms ?? 0) >= 60 || (maxMs ?? 0) >= 70) {
      return "Degraded";
    }
    return "Stable";
  });

  function positionPct(timestampSec: number): number {
    if (duration <= 0) return 0;
    return Math.max(0, Math.min(100, (timestampSec / duration) * 100));
  }

  function handleTrackClick(event: MouseEvent) {
    const track = event.currentTarget as HTMLButtonElement;
    const rect = track.getBoundingClientRect();
    const ratio = rect.width > 0 ? (event.clientX - rect.left) / rect.width : 0;
    onSeek(Math.max(0, Math.min(duration, ratio * duration)));
  }

  $effect(() => {
    onMetrics?.({
      statusLabel,
      avgMs,
      p95Ms,
      maxMs,
      lossPct,
      spikeCount,
      targetLabel: activeGroup?.label ?? "Unknown target",
      activeGroupKey: activeGroup?.key ?? null,
    });
  });
</script>

{#if displaySamples.length > 0}
  <div class="net-graph" aria-label="Ping timeline graph">
    <button
      class="track-hitbox"
      type="button"
      aria-label="Jump to a point in the ping timeline"
      onclick={handleTrackClick}
    ></button>

    {#if graphPoints}
      <svg viewBox="0 0 100 100" preserveAspectRatio="none" class="graph-svg" aria-hidden="true">
        <polyline class="latency-line" points={graphPoints}></polyline>
      </svg>
    {/if}

    {#each displaySamples as sample (sample.id)}
      {#if sample.timed_out || sample.rtt_ms == null}
        <div
          class="loss-marker"
          style="left: {positionPct(sample.timestamp_sec)}%;"
          title={`Probe lost at ${sample.timestamp_sec.toFixed(0)}s`}
        ></div>
      {/if}
    {/each}

    {#if duration > 0}
      <div class="graph-playhead" style="left: {positionPct(currentTime)}%;"></div>
    {/if}
  </div>
{/if}

<style>
  .net-graph {
    position: relative;
    height: 34px;
    background: var(--bg-raised);
    border-radius: var(--radius);
    overflow: hidden;
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

  .graph-svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .latency-line {
    fill: none;
    stroke: color-mix(in srgb, var(--accent) 80%, white);
    stroke-width: 2.4;
    vector-effect: non-scaling-stroke;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .loss-marker {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: rgba(192, 57, 43, 0.85);
    transform: translateX(-50%);
    z-index: 1;
  }

  .graph-playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: rgba(255, 255, 255, 0.75);
    transform: translateX(-50%);
    pointer-events: none;
    z-index: 1;
  }

</style>
