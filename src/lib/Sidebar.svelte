<script lang="ts">
  import type { Match } from "./api.js";
  import { formatDuration, formatDate } from "./api.js";

  interface Props {
    matches: Match[];
    selectedId: number | null;
    collapsed?: boolean;
    onSelect: (m: Match) => void;
    onDelete: (m: Match) => void;
  }

  let {
    matches,
    selectedId,
    collapsed = false,
    onSelect,
    onDelete
  }: Props = $props();

  function handleDeleteClick(event: MouseEvent, match: Match) {
    event.stopPropagation();
    onDelete(match);
  }
</script>

<aside class="sidebar" class:collapsed>
  <div class="sidebar-inner">
  <div class="sidebar-header">
    <span class="title">Recordings</span>
    <span class="count" title={`${matches.length} recordings`}>{matches.length}</span>
  </div>

  <ul class="match-list" role="listbox">
    {#each matches as match (match.id)}
      <li
        class="match-item"
        class:selected={match.id === selectedId}
        role="option"
        aria-selected={match.id === selectedId}
        tabindex="0"
        onclick={() => onSelect(match)}
        onkeydown={(e) => e.key === "Enter" && onSelect(match)}
      >
        <div class="match-champion">{match.champion}</div>
        <div class="match-meta">
          <span class="result-badge" class:win={match.result === "Win"} class:loss={match.result === "Loss"}>
            {match.result}
          </span>
          <span class="kda">{match.kills}/{match.deaths}/{match.assists}</span>
        </div>
        <div class="match-info">
          <span class="duration">{formatDuration(match.duration_sec)}</span>
          <span class="date">{formatDate(match.recorded_at)}</span>
        </div>
        {#if match.summoner_name}
          <div class="match-account">{match.summoner_name}</div>
        {/if}
        <div class="match-actions">
          <button
            class="delete-btn"
            type="button"
            title={`Delete ${match.champion} recording`}
            aria-label={`Delete ${match.champion} recording`}
            onclick={(event) => handleDeleteClick(event, match)}
          >
            Delete
          </button>
        </div>
      </li>
    {/each}

    {#if matches.length === 0}
      <li class="empty">No recordings yet</li>
    {/if}
  </ul>
  </div>
</aside>

<style>
  .sidebar {
    width: 200px;
    flex-shrink: 0;
    overflow: hidden;
    transition: width var(--transition);
  }

  .sidebar.collapsed {
    width: 0;
  }

  .sidebar-inner {
    width: 200px;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border-right: 1px solid var(--border);
    transition: transform var(--transition);
  }

  .sidebar.collapsed .sidebar-inner {
    transform: translateX(-100%);
  }

  .sidebar-header {
    padding: 14px 12px 10px 48px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .title {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-secondary);
    flex: 1;
  }

  .count {
    font-size: 11px;
    color: var(--text-muted);
    background: var(--bg-raised);
    border-radius: 10px;
    padding: 1px 7px;
  }

  .match-list {
    list-style: none;
    overflow-y: auto;
    flex: 1;
    padding: 4px 0;
  }

  .match-item {
    padding: 10px 12px;
    cursor: pointer;
    border-bottom: 1px solid var(--border);
    transition: background var(--transition);
    outline: none;
  }

  .match-item:hover,
  .match-item:focus-visible {
    background: var(--bg-hover);
  }

  .match-item.selected {
    background: var(--bg-raised);
    border-left: 2px solid var(--accent);
    padding-left: 10px;
  }

  .match-champion {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .match-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 3px;
  }

  .result-badge {
    font-size: 10px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 3px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .result-badge.win  { background: rgba(60, 179, 113, 0.2); color: var(--win); }
  .result-badge.loss { background: rgba(192, 57, 43, 0.2);  color: var(--loss); }

  .kda {
    font-size: 12px;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .match-info {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--text-muted);
  }

  .match-account {
    font-size: 10px;
    color: var(--text-muted);
    margin-top: 2px;
    opacity: 0.7;
  }

  .match-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 8px;
  }

  .delete-btn {
    font-size: 11px;
    line-height: 1;
    padding: 4px 7px;
    border-radius: var(--radius);
    color: var(--text-muted);
    background: var(--bg-raised);
    transition: color var(--transition), background var(--transition);
  }

  .delete-btn:hover {
    color: var(--loss);
    background: rgba(192, 57, 43, 0.15);
  }

  .empty {
    padding: 24px 12px;
    text-align: center;
    color: var(--text-muted);
    font-size: 12px;
    font-style: italic;
  }
</style>
