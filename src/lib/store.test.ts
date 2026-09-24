import { beforeEach, describe, expect, it } from "vitest";
import type { ServerRow } from "./bindings";
import * as store from "./store.svelte";

const row = (endpoint: string, changes: Partial<ServerRow> = {}): ServerRow => ({
  endpoint,
  hostname: endpoint,
  map: "de_dust2",
  gamedir: "cstrike",
  game: "Counter-Strike",
  players: 4,
  max_players: 16,
  bots: 0,
  bot_plugin: null,
  ping_ms: 40,
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
  ...changes,
});

beforeEach(() => {
  store.reset();
  store.favorites.clear();
  store.history.clear();
  store.dom.list = null;
  store.ui.tab = 0;
  store.ui.discovery.name = "";
  store.ui.discovery.map = "";
  store.ui.discovery.latency = "< 100";
  store.ui.discovery.secure = "Secure";
  for (const key of Object.keys(store.ui.checks) as (keyof typeof store.ui.checks)[])
    store.ui.checks[key] = false;
  store.ui.sort = { key: "ping", asc: true };
});

describe("server list filters", () => {
  it("keeps verified servers, sorts by ping and replaces rows by endpoint", () => {
    store.upsert(row("slow", { ping_ms: 70 }));
    store.upsert(row("fast", { ping_ms: 15 }));
    store.upsert(row("slow", { ping_ms: 5 }));
    store.recompute();
    expect(store.ui.rows).toHaveLength(2);
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["slow", "fast"]);
    store.ui.sort = { key: "name", asc: false };
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["slow", "fast"]);
  });

  it("filters unverified, fake, latency, map, name and security", () => {
    store.upsert(row("good"));
    store.upsert(row("unverified", { verified: false }));
    store.upsert(row("fake", { fake: true }));
    store.upsert(row("slow", { ping_ms: 100 }));
    store.upsert(row("insecure", { secure: false }));
    store.upsert(row("offline", { live: false, outcome: "timeout" }));
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["good", "offline"]);
    store.ui.discovery.secure = "Not secure";
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["insecure"]);
    store.ui.discovery.secure = "<All>";
    store.ui.discovery.latency = "<All>";
    store.ui.discovery.map = "inferno";
    store.recompute();
    expect(store.ui.order).toHaveLength(0);
    store.ui.discovery.map = "DUST";
    store.ui.discovery.name = "GOOD";
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["good"]);
  });

  it("applies player and password controls", () => {
    store.upsert(row("full", { players: 16 }));
    store.upsert(row("empty", { players: 0 }));
    store.upsert(row("bots", { bots: 1 }));
    store.upsert(row("locked", { password: true }));
    store.upsert(row("normal"));
    for (const key of Object.keys(store.ui.checks) as (keyof typeof store.ui.checks)[])
      store.ui.checks[key] = true;
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["normal"]);
  });

  it("matches server names by case-insensitive substring and combines with map", () => {
    store.ui.discovery.latency = "<All>";
    store.upsert(row("one", { hostname: "[EU] Dust Arena", map: "de_dust2" }));
    store.upsert(row("two", { hostname: "Arena Classic", map: "de_inferno" }));
    store.upsert(row("three", { hostname: "Dust Lounge", map: "de_dust2" }));
    store.ui.discovery.name = "aReNa";
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["one", "two"]);
    store.ui.discovery.map = "dust";
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["one"]);
    expect(store.statusLine()).toContain("name contains aReNa");
  });

  it("keeps favorites and history visible and persists them", () => {
    store.upsert(row("saved", { verified: false, ping_ms: null }));
    store.toggleFavorite("saved");
    store.ui.tab = 1;
    store.recompute();
    expect(store.selectedRow()).toBeNull();
    expect(store.ui.order).toHaveLength(1);
    store.moveSelection(1);
    expect(store.selectedRow()?.endpoint).toBe("saved");
    expect(JSON.parse(localStorage.getItem("cs16browser.favorites")!)).toEqual(["saved"]);
    store.recordVisit("saved");
    store.ui.tab = 2;
    store.recompute();
    expect(store.ui.order).toHaveLength(1);
    store.toggleFavorite("saved");
    expect(store.favorites.has("saved")).toBe(false);
  });

  it("moves selection within bounds and scrolls a 28px row into view", () => {
    store.upsert(row("a"));
    store.upsert(row("b"));
    store.recompute();
    const list = document.createElement("div");
    Object.defineProperty(list, "clientHeight", { value: 28 });
    store.dom.list = list;
    store.moveSelection(-10);
    expect(store.ui.selected).toBe(0);
    store.moveSelection(10);
    expect(store.ui.selected).toBe(1);
    expect(list.scrollTop).toBe(28);
    store.ensureVisible(0);
    expect(list.scrollTop).toBe(0);
  });

  it("hides servers with bots or a bot plugin when asked", () => {
    store.upsert(row("humans"));
    store.upsert(row("bots", { bots: 3 }));
    store.upsert(row("hidden-bots", { bot_plugin: "YaPB 4.4.957" }));
    store.recompute();
    expect(store.ui.order).toHaveLength(3);
    store.ui.checks.no_bots = true;
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint)).toEqual(["humans"]);
    expect(store.statusLine()).toContain("has no bots or bot plugin");
  });

  it("lists every banned server on the Banned tab, whatever the filters", () => {
    store.upsert(row("good"));
    store.upsert(row("banned", { banned: true, verified: false, secure: false, ping_ms: null }));
    store.upsert(row("flagged", { fake: true, verified: false, reasons: ["server-farm"] }));
    store.upsert(row("paced", { outcome: "paced", verified: false, ping_ms: null }));
    store.upsert(row("timeout", { outcome: "timeout", verified: false, ping_ms: null }));
    store.upsert(row("pending", { outcome: "listed", verified: false, ping_ms: null }));
    store.upsert(row("reused", { outcome: "paced", verified: true }));
    store.ui.checks.no_empty = true;
    store.ui.tab = 3;
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint).sort()).toEqual([
      "banned",
      "flagged",
      "paced",
      "timeout",
    ]);
    store.ui.tab = 0;
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint).sort()).toEqual(["good", "reused"]);
  });

  it("merges persisted bans the current listing never surfaced, without touching rows already measured", () => {
    store.upsert(row("seen-and-banned", { banned: true, hostname: "Seen Farm" }));
    store.mergeBans([
      {
        endpoint: "seen-and-banned",
        hostname: "Stale name",
        reasons: ["stale"],
        banned_at: "x",
        rechecks: 0,
      },
      {
        endpoint: "delisted",
        hostname: "Delisted Farm",
        reasons: ["slots>32"],
        banned_at: "x",
        rechecks: 0,
      },
    ]);
    store.ui.tab = 3;
    store.recompute();
    expect(store.ui.order.map((i) => store.ui.rows[i].endpoint).sort()).toEqual([
      "delisted",
      "seen-and-banned",
    ]);
    // A row the sweep already measured is left alone: its own hostname wins.
    const seen = store.ui.rows[store.ui.index.get("seen-and-banned")!];
    expect(seen.hostname).toBe("Seen Farm");
    expect(seen.reasons).toEqual(["stale"]);
    const delisted = store.ui.rows[store.ui.index.get("delisted")!];
    expect(delisted.hostname).toBe("Delisted Farm");
    expect(delisted.reasons).toEqual(["slots>32"]);
    store.upsert(row("delisted", { banned: true, hostname: "Fresh name", reasons: [] }));
    const refreshed = store.ui.rows[store.ui.index.get("delisted")!];
    expect(refreshed.hostname).toBe("Fresh name");
    expect(refreshed.reasons).toEqual(["slots>32"]);
  });

  it("builds a status summary from enabled controls", () => {
    store.ui.discovery.map = "de_dust2";
    store.ui.discovery.secure = "Not secure";
    store.ui.checks.no_password = true;
    expect(store.statusLine()).toContain("map de_dust2; latency < 100; not secure");
    expect(store.statusLine()).toContain("has no password");
  });
});
