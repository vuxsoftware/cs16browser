import { afterEach, expect, it, vi } from "vitest";
import { mount, tick, unmount } from "svelte";
import type { ServerRow } from "./bindings";
import type { Backend } from "./cs16";

vi.mock("./mock.js", () => ({}));

const server: ServerRow = {
  endpoint: "8.8.8.8:27015",
  hostname: "Community server",
  map: "de_dust2",
  gamedir: "cstrike",
  game: "Counter-Strike",
  players: 4,
  max_players: 16,
  bots: 0,
  bot_plugin: null,
  ping_ms: 30,
  secure: true,
  password: false,
  os: 1,
  live: true,
  outcome: "ok",
  banned: false,
  fake: false,
  verified: true,
  reasons: [],
  soft_reasons: [],
  country: null,
  version: "",
  players_list: [],
};

afterEach(() => {
  delete window.cs16Mock;
  delete window.cs16Debug;
  document.body.innerHTML = "";
});

it("renders cached servers and responds to scan events and view controls", async () => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
      unobserve() {}
    },
  );
  let emit: ((event: Parameters<NonNullable<Window["cs16Debug"]>["feed"]>[0]) => void) | undefined;
  const backend = {
    mode: "mock",
    startScan: vi.fn(async () => undefined),
    stopScan: vi.fn(async () => undefined),
    refreshServer: vi.fn(async () => undefined),
    serverPlayers: vi.fn(async () => []),
    clearBans: vi.fn(async () => 0),
    banServer: vi.fn(async () => undefined),
    whitelistServer: vi.fn(async () => undefined),
    bans: vi.fn(async () => []),
    cachedRows: vi.fn(async () => [server]),
    connect: vi.fn(async () => "steam://connect/8.8.8.8:27015/"),
    getSettings: vi.fn(async () => ({
      api_key_source: "settings",
      api_key_hint: "••••1234",
      api_key_url: "https://steamcommunity.com/dev/apikey",
      debug_logs: false,
    })),
    setApiKey: vi.fn(async () => ({
      api_key_source: "settings",
      api_key_hint: "••••1234",
      api_key_url: "https://steamcommunity.com/dev/apikey",
      debug_logs: false,
    })),
    setDebugLogs: vi.fn(async () => ({
      api_key_source: "settings",
      api_key_hint: "••••1234",
      api_key_url: "https://steamcommunity.com/dev/apikey",
      debug_logs: false,
    })),
    openApiKeyPage: vi.fn(async () => undefined),
    openRepositoryPage: vi.fn(async () => undefined),
    onEvent: vi.fn((fn) => {
      emit = fn;
      return () => undefined;
    }),
  } as Backend;
  window.cs16Mock = backend;
  const App = (await import("./App.svelte")).default;
  const app = mount(App, { target: document.body });
  await tick();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await tick();
  expect(document.body.textContent).toContain("Community server");
  expect(backend.startScan).toHaveBeenCalled();
  emit?.({ kind: "phase", phase: "done" });
  emit?.({ kind: "row", row: { ...server, hostname: "Updated community" }, done: 1, total: 1 });
  await tick();
  expect(document.body.textContent).toContain("Updated community");
  const key = async (name: string) => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true }));
    await tick();
  };
  const click = async (label: string) => {
    const button = [...document.querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === label,
    );
    if (!button) throw new Error(`Missing button: ${label}`);
    button.click();
    await tick();
  };
  await key("Home");
  await key("End");
  (document.activeElement as HTMLElement).blur();
  document.querySelector<HTMLElement>(".row")?.click();
  await tick();
  await key("Enter");
  expect(backend.connect).toHaveBeenCalled();
  emit?.({ kind: "paced", endpoint: server.endpoint, reason: "5s gap" });
  emit?.({
    kind: "summary",
    summary: {
      shown: 1,
      banned: 0,
      hidden: 0,
      paced: 1,
      listed: 1,
      measured: 0,
      rechecks: 0,
      total_before: 1,
    },
  });
  await tick();
  const tab = async (label: string) => {
    const face = [...document.querySelectorAll<HTMLElement>(".tab")].find(
      (e) => e.textContent?.trim() === label,
    );
    if (!face) throw new Error(`Missing tab: ${label}`);
    face.click();
    await tick();
  };
  /* The Banned tab lists flagged and skipped rows whatever the filters say,
   * while Clear bans only applies to saved bans. */
  await tab("Banned");
  expect(document.body.textContent).toContain("No banned or skipped servers.");
  emit?.({
    kind: "listed",
    rows: [
      {
        ...server,
        endpoint: "6.6.6.5:27015",
        hostname: "Skipped timeout",
        outcome: "timeout",
        verified: false,
        ping_ms: null,
      },
    ],
  });
  window.cs16Debug?.store.recompute();
  await tick();
  expect(document.body.textContent).toContain("Skipped timeout");
  expect(
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent === "Clear bans",
    )?.disabled,
  ).toBe(true);
  emit?.({
    kind: "listed",
    rows: [
      {
        ...server,
        endpoint: "6.6.6.6:27015",
        hostname: "Banned redirect",
        banned: true,
        verified: false,
        secure: false,
        outcome: "banned",
      },
    ],
  });
  window.cs16Debug?.store.recompute();
  await tick();
  expect(document.body.textContent).toContain("Banned redirect");
  expect(document.body.textContent).not.toContain("Quick refresh");
  await click("Clear bans");
  expect(backend.clearBans).toHaveBeenCalled();
  await tab("Internet");
  emit?.({ kind: "listed", rows: [server] });
  window.cs16Debug?.store.recompute();
  await tick();
  await click("Refresh all");
  expect(backend.startScan).toHaveBeenLastCalledWith(
    expect.objectContaining({ rediscover: true, fetchAll: true }),
  );
  const secondServer = {
    ...server,
    endpoint: "8.8.4.4:27016",
    hostname: "Second community server",
  };
  emit?.({ kind: "listed", rows: [server, secondServer] });
  window.cs16Debug?.store.recompute();
  emit?.({ kind: "phase", phase: "done" });
  await tick();
  let finishQuick: () => void = () => undefined;
  vi.mocked(backend.refreshServer).mockImplementationOnce(
    () => new Promise<void>((resolve) => (finishQuick = resolve)),
  );
  await click("Quick refresh");
  expect(backend.refreshServer).toHaveBeenCalledWith(server.endpoint, false);
  expect(backend.refreshServer).toHaveBeenCalledWith(secondServer.endpoint, false);
  expect(document.body.textContent).toContain("Stop refresh");
  await click("Stop refresh");
  expect(document.body.textContent).toContain("Stopping refresh...");
  finishQuick();
  await vi.waitFor(() => expect(document.body.textContent).toContain("Refresh all"));
  const nameFilter = document.querySelector<HTMLInputElement>('input[aria-label="Name"]');
  if (!nameFilter) throw new Error("Missing Name filter");
  nameFilter.value = "sEcOnD";
  nameFilter.dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
  expect(document.querySelectorAll(".row")).toHaveLength(1);
  expect(document.body.textContent).toContain("Second community server");
  nameFilter.value = "";
  nameFilter.dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
  const settings = document.querySelector<HTMLButtonElement>('button[aria-label="Settings"]');
  settings?.click();
  await tick();
  expect(document.body.textContent).toContain("Steam Web API key");
  const apiKey = document.querySelector<HTMLInputElement>("#api-key");
  if (!apiKey) throw new Error("Missing API key input");
  apiKey.value = "0123456789abcdef0123456789abcdef";
  apiKey.dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
  await click("Save");
  expect(backend.setApiKey).toHaveBeenCalled();
  await click("Clear");
  await click("Save");
  expect(backend.setApiKey).toHaveBeenCalledWith("");
  await click("Get a Steam Web API key");
  expect(backend.openApiKeyPage).toHaveBeenCalled();
  await click("Close");

  const context = async (label: string) => {
    const row = document.querySelector<HTMLElement>(".row");
    if (!row) throw new Error("Missing server row");
    row.dispatchEvent(
      new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 20 }),
    );
    await tick();
    const item = [...document.querySelectorAll<HTMLElement>(".ctxmenu .item")].find(
      (e) => e.textContent?.trim() === label,
    );
    if (!item) throw new Error(`Missing context item: ${label}`);
    item.click();
    await tick();
  };
  /* Opening Game Info asks for the player list only: the sweep measured the
   * rest of the row moments ago. */
  const refresh = vi.mocked(backend.refreshServer);
  const refreshesBefore = refresh.mock.calls.length;
  let release: (players: ServerRow["players_list"]) => void = () => undefined;
  vi.mocked(backend.serverPlayers).mockImplementationOnce(
    () => new Promise((resolve) => (release = resolve)),
  );
  await context("View server info");
  expect(document.body.textContent).toContain("Loading player list");
  expect(backend.serverPlayers).toHaveBeenCalledWith(server.endpoint);
  expect(refresh.mock.calls.length).toBe(refreshesBefore);
  const listedBeforeDetail = window.cs16Debug?.state.rows.find(
    (row) => row.endpoint === server.endpoint,
  );
  release([
    { index: 0, name: "Alice", score: 12, duration_seconds: 492 },
    { index: 1, name: "Bob", score: 7, duration_seconds: 3840 },
    { index: 2, name: "Carol", score: 0, duration_seconds: null },
  ]);
  await vi.waitFor(() => expect(document.body.textContent).toContain("Alice"));
  const playerNames = () =>
    [...document.querySelectorAll<HTMLElement>(".prows .prow .name")].map((cell) =>
      cell.textContent?.trim(),
    );
  expect(playerNames()).toEqual(["Alice", "Bob", "Carol"]);
  document.querySelector<HTMLButtonElement>(".phead .score")?.click();
  await tick();
  expect(playerNames()).toEqual(["Carol", "Bob", "Alice"]);
  document.querySelector<HTMLButtonElement>(".phead .name")?.click();
  await tick();
  expect(playerNames()).toEqual(["Alice", "Bob", "Carol"]);
  document.querySelector<HTMLButtonElement>(".phead .time")?.click();
  await tick();
  expect(playerNames()).toEqual(["Bob", "Alice", "Carol"]);
  expect(document.querySelector(".dialog .grip")).toBeNull();
  /* The dialog's Refresh re-queries the whole server; its result updates the
   * dialog, not the list. */
  await click("Refresh");
  expect(refresh).toHaveBeenLastCalledWith(server.endpoint, true);
  emit?.({
    kind: "refreshed",
    row: {
      ...server,
      hostname: "Detail-only name",
      ping_ms: 490,
      players_list: [{ index: 0, name: "Dave", score: 1, duration_seconds: 60 }],
    },
  });
  await tick();
  expect(document.querySelector(".dialog")?.textContent).toContain("Detail-only name");
  expect(document.body.textContent).toContain("Dave");
  expect(window.cs16Debug?.state.rows.find((row) => row.endpoint === server.endpoint)).toEqual(
    listedBeforeDetail,
  );
  /* A refresh refused by pacing sends nothing; the dialog asks again once
   * the gap has passed instead of leaving the click without effect. */
  refresh.mockRejectedValueOnce(new Error("recently queried; retry in 1 s"));
  const before = refresh.mock.calls.length;
  await click("Refresh");
  await vi.waitFor(() => expect(refresh.mock.calls.length).toBe(before + 1));
  await vi.waitFor(() => expect(refresh.mock.calls.length).toBe(before + 2), {
    timeout: 2500,
  });
  await click("Join Game");
  await click("Close");
  emit?.({ kind: "refreshed", row: { ...server, hostname: "Late detail result", ping_ms: 999 } });
  await tick();
  expect(window.cs16Debug?.state.rows.find((row) => row.endpoint === server.endpoint)).toEqual(
    listedBeforeDetail,
  );
  await context("Refresh server");
  emit?.({ kind: "refreshed", row: { ...server, hostname: "List refresh result", ping_ms: 20 } });
  await tick();
  expect(
    window.cs16Debug?.state.rows.find((row) => row.endpoint === server.endpoint)?.hostname,
  ).toBe("List refresh result");
  await context("Add to favorites");
  expect(localStorage.getItem("cs16browser.favorites")).toContain(server.endpoint);
  await context("Remove from favorites");
  expect(window.cs16Debug?.store.favorites.has(server.endpoint)).toBe(false);

  const latency = [...document.querySelectorAll<HTMLElement>(".dropdown")].find((e) =>
    e.textContent?.includes("< 100"),
  );
  latency?.click();
  await tick();
  const all = [...document.querySelectorAll<HTMLElement>(".menu .item")].find(
    (e) => e.textContent?.trim() === "<All>",
  );
  all?.click();
  await tick();
  expect(window.cs16Debug?.state.discovery.latency).toBe("<All>");
  await key("Escape");
  await key("PageDown");
  await key("PageUp");
  await key("ArrowDown");
  await key("ArrowUp");
  document.querySelector<HTMLElement>(".header .hcell.players")?.click();
  document.querySelector<HTMLElement>(".header .hcell.players")?.click();
  document.querySelector<HTMLElement>(".header .hcell.latency")?.click();
  const playersHeader = document.querySelector<HTMLElement>(".header .hcell.players")!;
  Object.defineProperty(playersHeader, "offsetWidth", { value: 110 });
  playersHeader
    .querySelector<HTMLButtonElement>(".resize-handle")
    ?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
  await tick();
  expect(playersHeader.style.flex).toContain("120px");
  expect(document.querySelector<HTMLElement>(".row .cell.players")?.style.flex).toContain("120px");
  document.querySelector<HTMLElement>(".header .sbtn.up")?.click();
  document.querySelector<HTMLElement>(".sbar .sbtn.down")?.click();
  await tick();
  await click("Change filters");
  await click("Change filters");
  document.querySelector<HTMLElement>(".row")?.click();
  await tick();
  await click("Quick refresh");
  expect(backend.refreshServer).toHaveBeenCalled();
  emit?.({ kind: "phase", phase: "scanning" });
  await tick();
  await click("Stop refresh");
  expect(backend.stopScan).toHaveBeenCalled();
  emit?.({ kind: "phase", phase: "done" });
  await tick();
  await click("Refresh all");
  expect(backend.startScan).toHaveBeenCalledTimes(3);

  emit?.({ kind: "listed", rows: [server] });
  window.cs16Debug?.store.recompute();
  await tick();
  const row = document.querySelector<HTMLElement>(".row")!;
  row.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 20 }),
  );
  await tick();
  await key("ArrowDown");
  await key("ArrowUp");
  await key("Escape");

  await context("Add to banned");
  expect(backend.banServer).toHaveBeenCalledWith(server.endpoint, server.hostname);
  await vi.waitFor(() => expect(document.querySelectorAll(".row")).toHaveLength(0));
  await tab("Banned");
  const bannedRow = document.querySelector<HTMLElement>(".row")!;
  bannedRow.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 20 }),
  );
  await tick();
  const bannedMenu = [...document.querySelectorAll<HTMLElement>(".ctxmenu .item")].map((e) =>
    e.textContent?.trim(),
  );
  expect(bannedMenu).toContain("Add to whitelist");
  expect(bannedMenu).not.toContain("Connect to server");
  expect(bannedMenu).not.toContain("Add to banned");
  [...document.querySelectorAll<HTMLElement>(".ctxmenu .item")]
    .find((e) => e.textContent?.trim() === "Add to whitelist")
    ?.click();
  await vi.waitFor(() => expect(backend.whitelistServer).toHaveBeenCalledWith(server.endpoint));
  await tab("Internet");
  expect(document.body.textContent).toContain(server.hostname);

  latency?.click();
  await tick();
  await key("ArrowDown");
  await key("ArrowUp");
  await key("Enter");
  emit?.({ kind: "error", message: "Network unavailable" });
  await tick();
  expect(document.body.textContent).toContain("Network unavailable");
  await unmount(app);
});
