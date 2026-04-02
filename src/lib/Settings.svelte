<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./api.js";
  import type { Config } from "./api.js";
  import { open } from "@tauri-apps/plugin-dialog";

  let config: Config | null = $state(null);
  let saving = $state(false);
  let saved = $state(false);
  let storageInfo: { used_gb: number; max_gb: number } | null = $state(null);

  onMount(async () => {
    config = await api.getSettings();
    storageInfo = await api.getStorageUsage();
  });

  async function save() {
    if (!config) return;
    saving = true;
    try {
      await api.saveSettings(config);
      saved = true;
      setTimeout(() => (saved = false), 2000);
    } finally {
      saving = false;
    }
  }

  async function browseOutputDir() {
    if (!config) return;
    const selected = await open({ directory: true, defaultPath: config.recording.output_dir });
    if (typeof selected === "string") {
      config.recording.output_dir = selected;
    }
  }

  function storagePercent(): number {
    if (!storageInfo || storageInfo.max_gb === 0) return 0;
    return Math.min(100, (storageInfo.used_gb / storageInfo.max_gb) * 100);
  }
</script>

<main class="settings">
  <h1>Settings</h1>

  {#if config}
    <section>
      <h2>Recording</h2>

      <div class="field">
        <label for="resolution">Resolution</label>
        <select id="resolution" bind:value={config.recording.resolution}>
          <option value="1080p">1080p (scaled)</option>
          <option value="native">Native (1440p / 4K)</option>
        </select>
      </div>

      <div class="field">
        <label for="audio_mode">Audio</label>
        <select id="audio_mode" bind:value={config.recording.audio_mode}>
          <option value="system">System loopback</option>
          <option value="off">Off</option>
        </select>
      </div>

      <div class="field">
        <label for="output_dir">Output folder</label>
        <div class="dir-row">
          <input
            id="output_dir"
            type="text"
            readonly
            value={config.recording.output_dir}
            class="dir-input"
          />
          <button class="browse-btn" onclick={browseOutputDir}>Browse</button>
        </div>
      </div>
    </section>

    <section>
      <h2>Storage</h2>

      {#if storageInfo}
        <div class="storage-bar-wrap">
          <div class="storage-bar">
            <div class="storage-fill" style="width: {storagePercent()}%;"></div>
          </div>
          <span class="storage-label">
            {storageInfo.used_gb.toFixed(1)} GB / {storageInfo.max_gb} GB
          </span>
        </div>
      {/if}

      <div class="field">
        <label for="max_gb">Limit: {config.storage.max_gb} GB</label>
        <input
          id="max_gb"
          type="range"
          min="10"
          max="500"
          step="5"
          bind:value={config.storage.max_gb}
          class="range-input"
        />
        <div class="range-labels"><span>10 GB</span><span>500 GB</span></div>
      </div>
    </section>

    <section>
      <h2>App</h2>

      <div class="field toggle-field">
        <label for="autostart">Start with Windows</label>
        <label class="toggle">
          <input id="autostart" type="checkbox" bind:checked={config.app.autostart} />
          <span class="toggle-track"></span>
        </label>
      </div>

      <div class="field">
        <button class="open-folder-btn" onclick={() => api.openRecordingsFolder()}>
          Open recordings folder
        </button>
      </div>
    </section>

    <div class="actions">
      <button class="save-btn" onclick={save} disabled={saving}>
        {saving ? "Saving…" : saved ? "Saved ✓" : "Save Settings"}
      </button>
    </div>
  {:else}
    <p class="loading">Loading…</p>
  {/if}
</main>

<style>
  .settings {
    padding: 24px;
    max-width: 420px;
    color: var(--text-primary);
  }

  h1 {
    font-size: 18px;
    font-weight: 600;
    margin-bottom: 20px;
    color: var(--accent);
  }

  section {
    margin-bottom: 24px;
  }

  h2 {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-secondary);
    margin-bottom: 12px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--border);
  }

  .field {
    margin-bottom: 14px;
  }

  label {
    display: block;
    font-size: 13px;
    color: var(--text-secondary);
    margin-bottom: 5px;
  }

  select, input[type="text"] {
    width: 100%;
  }

  .dir-row {
    display: flex;
    gap: 8px;
  }

  .dir-input {
    flex: 1;
    min-width: 0;
    cursor: default;
  }

  .browse-btn {
    padding: 6px 14px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 13px;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--transition);
  }

  .browse-btn:hover {
    background: var(--bg-hover);
  }

  .range-input {
    width: 100%;
    accent-color: var(--accent);
    background: transparent;
    border: none;
    padding: 6px 0;
    cursor: pointer;
  }

  .range-labels {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--text-muted);
    margin-top: -4px;
  }

  /* Storage bar */
  .storage-bar-wrap {
    margin-bottom: 14px;
  }

  .storage-bar {
    height: 6px;
    background: var(--bg-raised);
    border-radius: 3px;
    overflow: hidden;
    margin-bottom: 5px;
  }

  .storage-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  .storage-label {
    font-size: 12px;
    color: var(--text-secondary);
  }

  /* Toggle */
  .toggle-field {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .toggle-field label:first-child {
    margin-bottom: 0;
  }

  .toggle {
    position: relative;
    display: inline-block;
    width: 38px;
    height: 22px;
    cursor: pointer;
    margin: 0;
  }

  .toggle input {
    opacity: 0;
    width: 0;
    height: 0;
    position: absolute;
  }

  .toggle-track {
    position: absolute;
    inset: 0;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 11px;
    transition: background var(--transition), border-color var(--transition);
  }

  .toggle-track::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    background: var(--text-muted);
    border-radius: 50%;
    transition: transform var(--transition), background var(--transition);
  }

  .toggle input:checked + .toggle-track {
    background: var(--accent-dim);
    border-color: var(--accent);
  }

  .toggle input:checked + .toggle-track::after {
    transform: translateX(16px);
    background: var(--accent);
  }

  .hint {
    font-size: 11px;
    color: var(--text-muted);
    margin-top: 4px;
  }

  .open-folder-btn {
    padding: 7px 14px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-secondary);
    font-size: 13px;
    cursor: pointer;
    transition: color var(--transition), background var(--transition);
  }

  .open-folder-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .actions {
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }

  .save-btn {
    padding: 8px 20px;
    background: var(--accent-dim);
    border: 1px solid var(--accent);
    border-radius: var(--radius);
    color: var(--accent);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background var(--transition);
  }

  .save-btn:hover:not(:disabled) {
    background: rgba(200, 155, 60, 0.3);
  }

  .save-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .loading {
    color: var(--text-muted);
    font-style: italic;
  }
</style>
