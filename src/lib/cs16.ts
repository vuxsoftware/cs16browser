/* cs16.ts — the ONLY module that knows how the backend is reached.
 *
 * The command surface is not hand-written: `bindings.ts` is generated from the
 * Rust side by tauri-specta, so a command whose signature changes fails the
 * TypeScript build instead of silently passing `undefined`.
 *
 *   Tauri present -> `commands` / `events` from the generated bindings.
 *   Tauri absent  -> `mock.js`, the scripted offline replay used to review the
 *                    chrome in a plain browser.
 *
 * Both implement the same `Backend` interface, so nothing above this module
 * learns which one is live.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { commands, events } from "./bindings";
import type { BanRow, Event, PlayerRow, ScanConfig, ServerRow, SettingsView } from "./bindings";
/* The mock is a classic script that assigns `window.cs16Mock`; importing it for
 * its side effect is what makes the standalone review build work. Vite resolves
 * it statically, so a missing file is a build error, not a runtime surprise. */
import "./mock.js";

export type Unlisten = () => void;

/** The generated bindings report a Rust `Err` as a tagged union rather than by
 * throwing.
 *
 * Unwrapping it here — once, at the backend boundary — is what keeps every
 * caller honest: `refreshServer` rejecting with the pacing reason is exactly how
 * the UI distinguishes "paced" from "dead", and a caller that ignored the union
 * would silently treat a refusal as success. */
async function unwrap<T>(
  result: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>,
): Promise<T> {
  const outcome = await result;
  if (outcome.status === "error") throw new Error(outcome.error);
  return outcome.data;
}

/** What the UI actually uses. Narrowing the generated surface here is what lets
 * the mock implement exactly the same thing. */
export interface Backend {
  readonly mode: "tauri" | "mock";
  startScan(config: ScanConfig | null): Promise<void>;
  /** Ask the in-flight sweep to stop; arrived results are kept. */
  stopScan(): Promise<void>;
  /** `interactive`: the user asked for this one server (Game Info, the row
   * menu), which is paced by a short gap only; a bulk refresh passes `false`
   * and keeps the sweep's limits. */
  refreshServer(endpoint: string, interactive: boolean): Promise<void>;
  /** One server's player list, for Game Info; rejects with the pacing reason
   * or the query failure. The rest of the row is not re-queried. */
  serverPlayers(endpoint: string): Promise<PlayerRow[]>;
  clearBans(): Promise<number>;
  banServer(endpoint: string, hostname: string): Promise<void>;
  whitelistServer(endpoint: string): Promise<void>;
  /** The full, persisted ban list — every server ever banned, not just those
   * Steam still lists this sweep. */
  bans(): Promise<BanRow[]>;
  cachedRows(): Promise<ServerRow[]>;
  /** Opens `steam://connect/<endpoint>/`; resolves to that URL. */
  connect(endpoint: string): Promise<string>;
  getSettings(): Promise<SettingsView>;
  /** Save the Web API key (empty clears it); rejects with a readable reason. */
  setApiKey(key: string): Promise<SettingsView>;
  /** Enable or disable `debug`-level entries in the log file; takes effect
   * immediately, without a restart. */
  setDebugLogs(enabled: boolean): Promise<SettingsView>;
  /** Open Steam's API-key page in the default browser. */
  openApiKeyPage(): Promise<void>;
  /** Open the GitHub repository in the default browser. */
  openRepositoryPage(): Promise<void>;
  /** `onError` reports a listener that could not be registered at all — a
   * denied capability, not a missing event. */
  onEvent(fn: (event: Event) => void, onError?: (message: string) => void): Unlisten;
}

const tauri: Backend = {
  mode: "tauri",
  startScan: async (config) => {
    await unwrap(commands.startScan(config));
  },
  stopScan: () => commands.stopScan(),
  refreshServer: async (endpoint, interactive) => {
    await unwrap(commands.refreshServer(endpoint, interactive));
  },
  serverPlayers: (endpoint) => unwrap(commands.serverPlayers(endpoint)),
  clearBans: () => commands.clearBans(),
  banServer: async (endpoint, hostname) => {
    await unwrap(commands.banServer(endpoint, hostname));
  },
  whitelistServer: async (endpoint) => {
    await unwrap(commands.whitelistServer(endpoint));
  },
  bans: () => commands.bans(),
  cachedRows: () => commands.cachedRows(),
  connect: (endpoint) => unwrap(commands.connect(endpoint)),
  getSettings: () => commands.getSettings(),
  setApiKey: (key) => unwrap(commands.setApiKey(key)),
  setDebugLogs: (enabled) => unwrap(commands.setDebugLogs(enabled)),
  openApiKeyPage: async () => {
    await unwrap(commands.openApiKeyPage());
  },
  openRepositoryPage: async () => {
    await unwrap(commands.openRepositoryPage());
  },
  onEvent(fn, onError) {
    /* One event name, one listener. `events.scan` carries the generated payload
     * type, so the discriminated union narrows inside the handler.
     *
     * A rejected `listen` must not be swallowed: Tauri denies IPC commands by
     * default, and without `core:event:allow-listen` in the capability file this
     * promise rejects, the scan never reports a phase, and the window sits empty
     * while the backend scans. Reporting it turns a wordless blank list into a
     * named cause. */
    let unlisten: Unlisten | null = null;
    let dead = false;
    events.scan
      .listen(({ payload }) => fn(payload))
      .then((u) => {
        if (dead) u();
        else unlisten = u;
      })
      .catch((err: unknown) => onError?.(`scan events unavailable: ${String(err)}`));
    return () => {
      dead = true;
      unlisten?.();
    };
  },
};

const inTauri = typeof window !== "undefined" && "__TAURI__" in window;
const mock = typeof window !== "undefined" ? window.cs16Mock : undefined;

if (!inTauri && !mock) {
  throw new Error("cs16.ts: neither a Tauri runtime nor mock.js is present");
}

/** `mock.js` is a classic script written before this interface existed: several
 * of its commands answer synchronously (a fixture needs no I/O). Wrapping each
 * result here keeps the interface promise-shaped for every caller, so no
 * component has to know which backend it is on. */
function asAsync<T>(value: T | Promise<T>): Promise<T> {
  return Promise.resolve(value);
}

function adaptMock(source: Backend): Backend {
  return {
    mode: "mock",
    startScan: (config) => asAsync(source.startScan(config)),
    stopScan: () => asAsync(source.stopScan()),
    refreshServer: (endpoint, interactive) => asAsync(source.refreshServer(endpoint, interactive)),
    serverPlayers: (endpoint) => asAsync(source.serverPlayers(endpoint)),
    clearBans: () => asAsync(source.clearBans()),
    banServer: (endpoint, hostname) => asAsync(source.banServer(endpoint, hostname)),
    whitelistServer: (endpoint) => asAsync(source.whitelistServer(endpoint)),
    bans: () => asAsync(source.bans()),
    cachedRows: () => asAsync(source.cachedRows()),
    connect: (endpoint) => asAsync(source.connect(endpoint)),
    getSettings: () => asAsync(source.getSettings()),
    setApiKey: (key) => asAsync(source.setApiKey(key)),
    setDebugLogs: (enabled) => asAsync(source.setDebugLogs(enabled)),
    openApiKeyPage: () => asAsync(source.openApiKeyPage()),
    openRepositoryPage: () => asAsync(source.openRepositoryPage()),
    onEvent: (fn, onError) => source.onEvent(fn, onError),
  };
}

export const cs16: Backend = inTauri || !mock ? tauri : adaptMock(mock);

/** A signed release is checked once on launch. Installation is explicit so a
 * server scan cannot be interrupted without the user's choice. */
export async function availableUpdate(): Promise<Update | null> {
  return inTauri ? check() : null;
}

export async function installUpdate(update: Update): Promise<void> {
  await update.downloadAndInstall();
  await relaunch();
}

/** Window chrome actions. The window is frameless, so the close, minimize and maximize
 * boxes and the resize grip are drawn by the page and drive the native window
 * from here. Close quits the app (closing the only window ends the process);
 * in the plain-browser review build there is no native window, so these are
 * no-ops. */
export const nativeWindow = {
  show(): void {
    if (inTauri) void getCurrentWindow().show();
  },
  close(): void {
    if (inTauri) void getCurrentWindow().close();
  },
  minimize(): void {
    if (inTauri) void getCurrentWindow().minimize();
  },
  toggleMaximize(): void {
    if (inTauri) void getCurrentWindow().toggleMaximize();
  },
  startResize(): void {
    if (inTauri) void getCurrentWindow().startResizeDragging("SouthEast");
  },
};
