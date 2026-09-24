<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import { availableUpdate, cs16, installUpdate } from "./cs16";
  import { autoUpdateEnabled, setAutoUpdateEnabled } from "./updatePreference";
  import type { SettingsView } from "./bindings";
  import type { Update } from "@tauri-apps/plugin-updater";

  let { onsaved, onclose }: { onsaved: (view: SettingsView) => void; onclose: () => void } =
    $props();
  let view = $state<SettingsView | null>(null);
  let draft = $state("");
  let clearKey = $state(false);
  let autoUpdate = $state(autoUpdateEnabled());
  let debugLogs = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let busy = $state(false);
  let checking = $state(false);
  let update = $state<Update | null>(null);

  $effect(() => {
    cs16.getSettings().then(
      (v) => {
        view = v;
        debugLogs = v.debug_logs;
      },
      (err: unknown) => {
        error = String(err);
      },
    );
  });

  async function save(): Promise<void> {
    if (busy) return;
    busy = true;
    error = null;
    notice = null;
    try {
      if (clearKey || draft.trim()) {
        view = await cs16.setApiKey(clearKey ? "" : draft);
        draft = "";
        clearKey = false;
        onsaved(view);
      }
      if (view === null || debugLogs !== view.debug_logs) {
        view = await cs16.setDebugLogs(debugLogs);
        onsaved(view);
      }
      setAutoUpdateEnabled(autoUpdate);
      notice = "Settings saved.";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function checkUpdate(): Promise<void> {
    if (checking) return;
    checking = true;
    error = null;
    notice = null;
    update = null;
    try {
      update = await availableUpdate();
      notice =
        cs16.mode === "mock"
          ? "Update checks are available in the installed app."
          : update
            ? `Version ${update.version} is available.`
            : "You're up to date.";
    } catch (err) {
      error = `Update check failed: ${String(err)}`;
    } finally {
      checking = false;
    }
  }
</script>

<Dialog
  title="Settings"
  width={900}
  height={660}
  buttons={[{ label: "Save", onClick: () => void save(), close: false }, { label: "Close" }]}
  {onclose}
>
  <div class="settings-content">
    <div class="section-heading">Server discovery</div>
    <div class="setting-row">
      <label for="api-key">Steam Web API key</label>
      <div class="key-control">
        <input
          id="api-key"
          class="fentry"
          type="password"
          bind:value={draft}
          disabled={clearKey}
          placeholder={clearKey
            ? "Key will be removed on Save"
            : (view?.api_key_hint ?? "Paste a 32-character key")}
          autocomplete="off"
          spellcheck="false"
          onkeydown={(e) => e.stopPropagation()}
        />
        <button
          class="vbtn clear"
          onclick={() => {
            clearKey = !clearKey;
            draft = "";
          }}>{clearKey ? "Undo" : "Clear"}</button
        >
      </div>
    </div>
    <div class="setting-row key-link-row">
      <span></span>
      <button
        class="text-link"
        onclick={() =>
          cs16.openApiKeyPage().catch((err: unknown) => {
            error = String(err);
          })}>Get a Steam Web API key</button
      >
    </div>
    <div class="section-heading updates-heading">Updates</div>
    <div class="setting-row">
      <label class="checkbox-label"
        ><input type="checkbox" bind:checked={autoUpdate} /> Check for updates when the app starts</label
      >
    </div>
    <div class="setting-row">
      <button class="vbtn check" onclick={() => void checkUpdate()} disabled={checking}
        >{checking ? "Checking..." : "Check for updates"}</button
      >
      {#if update}<button
          class="vbtn install"
          onclick={() => {
            notice = "Downloading update...";
            void installUpdate(update!).catch((err: unknown) => {
              error = `Update failed: ${String(err)}`;
            });
          }}>Install version {update.version}</button
        >{/if}
    </div>
    <div class="section-heading updates-heading">Diagnostics</div>
    <div class="setting-row">
      <label class="checkbox-label"
        ><input type="checkbox" bind:checked={debugLogs} /> Enable debug logs</label
      >
    </div>
    <div class="msg" class:error={!!error} role="status">{error ?? notice ?? ""}</div>
  </div>
</Dialog>

<style>
  .settings-content {
    position: absolute;
    top: 86px;
    left: 54px;
    right: 54px;
    bottom: 100px;
    font-size: 20px;
  }
  .section-heading {
    color: var(--text-bright);
    font-size: 23px;
    border-bottom: 1px solid var(--inset);
    padding-bottom: 8px;
    margin-bottom: 10px;
  }
  .setting-row {
    display: flex;
    align-items: center;
    min-height: 48px;
    gap: 18px;
  }
  .setting-row > label:not(.checkbox-label),
  .key-link-row > span {
    flex: 0 0 230px;
    color: var(--text-dim);
  }
  .key-control {
    display: flex;
    gap: 10px;
    flex: 1;
  }
  .key-control input {
    min-width: 0;
    flex: 1;
    height: 43px;
  }
  .vbtn.clear {
    width: 95px;
    height: 43px;
  }
  .text-link {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
  }
  .updates-heading {
    margin-top: 14px;
  }
  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 12px;
    cursor: pointer;
  }
  .checkbox-label input {
    width: 22px;
    height: 22px;
    accent-color: var(--accent);
  }
  .vbtn.check {
    width: 230px;
  }
  .vbtn.install {
    width: 230px;
  }
  .msg {
    margin-top: 12px;
    color: var(--text);
  }
  .msg.error {
    color: var(--accent);
  }
</style>
