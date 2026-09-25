/* store.svelte.ts — the whole UI state, in runes.
 *
 * The rules here determine what the browser shows:
 *
 *   - every filter and view mode acts on the same broad, cached server list
 *   - `passesChecks` is applied to whatever we measured, because
 *     `Is not password protected` cannot be expressed to Steam's Web API at all
 *     (it has no password key), and the other three are only best-effort there
 *   - the Internet tab shows only rows the backend marked `verified`: the
 *     server answered A2S itself and passed every fake-server check, so a
 *     redirect server is never shown, not even briefly
 *   - the Anti-cheat dropdown is applied here, to what each server reported
 */

import { CHECKS, OPT_LATENCY, OPT_SECURE, ROW_H } from "./columns";
import type { CellKey, CheckDef, ColumnWidths, FieldId, SortKey } from "./columns";
import type { BanRow, Phase, ScanSummary, ServerRow } from "./bindings";

/** Non-reactive DOM handles: the scroller needs imperative access for
 * `ensureVisible`, and making that reactive would re-render on assignment. */
export const dom: { list: HTMLElement | null } = { list: null };

/* Favorites and History are endpoint sets, persisted client-side so they
 * survive a restart without a backend round-trip. History is recorded on
 * connect; favorites via the row context menu. */
const FAV_KEY = "cs16browser.favorites";
const HIST_KEY = "cs16browser.history";

function persist(key: string, set: Set<string>): void {
  try {
    localStorage.setItem(key, JSON.stringify([...set]));
  } catch {
    /* Persistence is best-effort: a denied quota must not break the view. */
  }
}

function revive(key: string): Set<string> {
  try {
    const raw = localStorage.getItem(key);
    return raw ? new Set(JSON.parse(raw) as string[]) : new Set();
  } catch {
    return new Set();
  }
}

export const favorites: Set<string> = revive(FAV_KEY);
export const history: Set<string> = revive(HIST_KEY);

export type Checks = Record<CheckDef["id"], boolean>;
export type Discovery = Record<FieldId, string>;

/** An open dropdown: where to draw its popup and what is armed. */
export interface OpenMenu {
  id: FieldId;
  items: readonly string[];
  armed: number;
  left: number;
  top: number;
  width: number;
}

export interface ModalState {
  title: string;
  body?: string;
  buttons: { label: string; accent?: boolean; close?: boolean; onClick?: () => void }[];
}

export interface UiState {
  rows: ServerRow[];
  index: Map<string, number>;
  order: number[];
  selected: number;
  sort: { key: SortKey; asc: boolean };
  discovery: Discovery;
  checks: Checks;
  phase: Phase;
  summary: ScanSummary | null;
  error: string | null;
  tab: number;
  scroll: number;
  viewport: number;
  listWidth: number;
  columnWidths: ColumnWidths;
  columnResizeKey: CellKey | null;
  bannedColumnWidths: ColumnWidths;
  filtersOpen: boolean;
  flash: string | null;
  menu: OpenMenu | null;
  modal: ModalState | null;
}

/* The filter panel is remembered across restarts. First-run defaults: every
 * checkbox off, Anti-cheat `Secure`, Latency `< 100>`.
 * Name and Map are case-insensitive substring filters. */
const FILTERS_KEY = "cs16browser.filters";

function reviveFilters(): { discovery: Discovery; checks: Checks } {
  const discovery: Discovery = { name: "", map: "", latency: "< 100", secure: "Secure" };
  const checks = Object.fromEntries(CHECKS.map((c) => [c.id, c.on])) as Checks;
  try {
    const raw = localStorage.getItem(FILTERS_KEY);
    const saved = raw
      ? (JSON.parse(raw) as { discovery?: Partial<Discovery>; checks?: Partial<Checks> })
      : {};
    /* Validate every value: a stale or hand-edited entry must not select an
     * option the dropdown no longer has. */
    const d = saved.discovery ?? {};
    if (typeof d.latency === "string" && OPT_LATENCY.includes(d.latency))
      discovery.latency = d.latency;
    if (typeof d.secure === "string" && OPT_SECURE.includes(d.secure)) discovery.secure = d.secure;
    if (typeof d.map === "string") discovery.map = d.map.slice(0, 64);
    if (typeof d.name === "string") discovery.name = d.name.slice(0, 128);
    for (const c of CHECKS) {
      const v = saved.checks?.[c.id];
      if (typeof v === "boolean") checks[c.id] = v;
    }
  } catch {
    /* Unreadable storage: first-run defaults. */
  }
  return { discovery, checks };
}

const filters = reviveFilters();

export const ui: UiState = $state({
  rows: [],
  index: new Map<string, number>(),
  order: [],
  selected: -1,
  sort: { key: "ping", asc: true },
  discovery: filters.discovery,
  checks: filters.checks,
  phase: "idle",
  summary: null,
  error: null,
  tab: 0,
  scroll: 0,
  viewport: 600,
  listWidth: 0,
  columnWidths: {},
  columnResizeKey: null,
  bannedColumnWidths: {},
  filtersOpen: true,
  flash: null,
  menu: null,
  modal: null,
});

/* Save the filter panel whenever any of it changes, from whichever control
 * changed it. Best-effort, like favorites: a denied quota must not break the
 * view. */
$effect.root(() => {
  $effect(() => {
    const snapshot = JSON.stringify({
      discovery: {
        name: ui.discovery.name,
        map: ui.discovery.map,
        latency: ui.discovery.latency,
        secure: ui.discovery.secure,
      },
      checks: { ...ui.checks },
    });
    try {
      localStorage.setItem(FILTERS_KEY, snapshot);
    } catch {
      /* best-effort */
    }
  });
});

/** The Anti-cheat dropdown, applied to what the server itself reported: the
 * Web API's `secure` key only narrows Steam's list, and "Not secure" cannot be
 * expressed to it at all. */
function passesSecure(row: ServerRow): boolean {
  if (ui.discovery.secure === "Secure") return row.secure;
  if (ui.discovery.secure === "Not secure") return !row.secure;
  return true;
}

/** The four discovery checkboxes, applied to whatever we measured. */
function passesChecks(row: ServerRow): boolean {
  if (ui.checks.no_full && row.max_players > 0 && row.players >= row.max_players) return false;
  if (ui.checks.no_empty && row.players === 0) return false;
  // A bot plugin can report its bots as humans, so its presence counts too.
  if (ui.checks.no_bots && (row.bots > 0 || row.bot_plugin)) return false;
  if (ui.checks.no_password && row.password) return false;
  return true;
}

/** Latency ceiling, from the `Latency` dropdown: `< 100` keeps ping_ms < 100. */
function latencyCap(): number | null {
  const m = /^<\s*(\d+)$/.exec(ui.discovery.latency);
  return m ? Number(m[1]) : null;
}

/** The `Map` entry: a case-insensitive substring of the map name. */
function mapMatches(row: ServerRow): boolean {
  const want = ui.discovery.map.trim().toLowerCase();
  return !want || row.map.toLowerCase().includes(want);
}

/** The Name entry matches any case-insensitive part of the server hostname. */
function nameMatches(row: ServerRow): boolean {
  const want = ui.discovery.name.trim().toLowerCase();
  return !want || row.hostname.toLowerCase().includes(want);
}

/** Tab indices, in `TABS` order. */
const FAVORITES_TAB = 1;
const HISTORY_TAB = 2;
const BANNED_TAB = 3;
const SKIPPED_OUTCOMES = new Set(["paced", "timeout", "bad-header", "refused", "err"]);

function passesView(row: ServerRow): boolean {
  /* Show terminal rows omitted from Internet by the scan pipeline. A plain
   * Steam listing is still awaiting its first query, so it is not a skip. */
  if (ui.tab === BANNED_TAB)
    return row.banned || row.fake || (!row.verified && SKIPPED_OUTCOMES.has(row.outcome));
  if (!passesChecks(row) || !passesSecure(row) || !mapMatches(row) || !nameMatches(row))
    return false;
  /* Favorites and History keep their unmeasured rows visible: the user put
   * them there deliberately, and the game shows favourites before pinging. */
  if (ui.tab !== 0) return tabHolds(row);
  /* A server appears only once the backend has fully verified it: it
   * answered A2S itself and passed every check, including the listing-wide
   * ones (redirect farms, cloned names). A Steam-listed row that has not been
   * verified yet is invisible, not pending. */
  if (!row.verified || row.fake) return false;
  const cap = latencyCap();
  return cap === null || (row.ping_ms !== null && row.ping_ms < cap);
}

function tabHolds(row: ServerRow): boolean {
  if (ui.tab === FAVORITES_TAB) return favorites.has(row.endpoint);
  if (ui.tab === HISTORY_TAB) return history.has(row.endpoint);
  return true;
}

function compare(a: ServerRow, b: ServerRow): number {
  switch (ui.sort.key) {
    case "players":
      return a.players - b.players;
    case "name":
      return a.hostname.toLowerCase().localeCompare(b.hostname.toLowerCase());
    case "game":
      return a.game.localeCompare(b.game);
    case "map":
      return a.map.localeCompare(b.map);
    case "ping": {
      /* Unmeasured rows sort last rather than pretending to be ping 0. */
      const ap = a.ping_ms ?? Number.POSITIVE_INFINITY;
      const bp = b.ping_ms ?? Number.POSITIVE_INFINITY;
      return ap - bp;
    }
  }
}

/** Recompute the visible order, coalesced by the app during scan events. */
export function recompute(): void {
  const out: number[] = [];
  for (let i = 0; i < ui.rows.length; i++) {
    const row = ui.rows[i];
    if (row && passesView(row)) out.push(i);
  }
  out.sort((x, y) => {
    const a = ui.rows[x];
    const b = ui.rows[y];
    if (!a || !b) return 0;
    const d = compare(a, b);
    return ui.sort.asc ? d : -d;
  });
  ui.order = out;
  if (ui.selected >= out.length) ui.selected = out.length - 1;
}

export function upsert(row: ServerRow): void {
  const at = ui.index.get(row.endpoint);
  if (at === undefined) {
    ui.index.set(row.endpoint, ui.rows.length);
    ui.rows.push(row);
  } else {
    const previous = ui.rows[at];
    /* A ban-list skip has no fresh analysis. Keep the saved verdict details
     * when its listing row replaces the ban-list row. */
    if (row.banned && previous.banned) {
      if (row.reasons.length === 0) row.reasons = previous.reasons;
      if (!row.hostname) row.hostname = previous.hostname;
    }
    ui.rows[at] = row;
  }
}

/** Bring in bans the current listing never surfaced: a server banned in an
 * earlier session (or one Steam has since delisted) still belongs on the
 * Banned tab, which is meant to show *every* ban, not just the ones still in
 * this sweep's answer. Existing rows keep their measurements and hostname,
 * while missing ban reasons are filled from the persisted entry. */
export function mergeBans(bans: BanRow[]): void {
  for (const b of bans) {
    const at = ui.index.get(b.endpoint);
    if (at !== undefined) {
      const row = ui.rows[at];
      if (row.banned && row.reasons.length === 0) row.reasons = b.reasons;
      if (row.banned && !row.hostname) row.hostname = b.hostname;
      continue;
    }
    upsert({
      endpoint: b.endpoint,
      hostname: b.hostname,
      map: "",
      gamedir: "",
      game: "",
      players: 0,
      max_players: 0,
      bots: 0,
      bot_plugin: null,
      ping_ms: null,
      secure: false,
      password: false,
      os: 0,
      live: false,
      outcome: "banned",
      banned: true,
      fake: true,
      verified: false,
      reasons: b.reasons,
      soft_reasons: [],
      country: null,
      version: "",
      players_list: [],
    });
  }
  recompute();
}

export function reset(): void {
  ui.rows = [];
  ui.index = new Map<string, number>();
  ui.order = [];
  ui.selected = -1;
}

/** Context menu: favourite or unfavourite a server. */
export function toggleFavorite(endpoint: string): void {
  if (!favorites.delete(endpoint)) favorites.add(endpoint);
  persist(FAV_KEY, favorites);
  recompute();
}

/** Connect: the visited endpoint enters History, most recent last. */
export function recordVisit(endpoint: string): void {
  history.delete(endpoint);
  history.add(endpoint);
  persist(HIST_KEY, history);
  recompute();
}

export function selectedRow(): ServerRow | null {
  const i = ui.order[ui.selected];
  return i == null ? null : (ui.rows[i] ?? null);
}

/** Move the selection, keeping it inside the viewport. */
export function moveSelection(delta: number): void {
  const n = ui.order.length;
  if (!n) return;
  let sel = ui.selected + delta;
  if (sel < 0) sel = 0;
  if (sel > n - 1) sel = n - 1;
  ui.selected = sel;
  ensureVisible(sel);
}

export function ensureVisible(i: number): void {
  const el = dom.list;
  if (!el) return;
  const top = i * ROW_H;
  if (top < el.scrollTop) {
    el.scrollTop = top;
  } else if (top + ROW_H > el.scrollTop + el.clientHeight) {
    el.scrollTop = top + ROW_H - el.clientHeight;
  }
}

/** `Counter-Strike; latency < 100; is not full; is not empty; has no password.`
 * — composed exactly as the reference's StatusLabel does: parts whose filter is
 * off are omitted rather than shown as disabled. */
export function statusLine(): string {
  const parts: string[] = [];
  parts.push("Counter-Strike");

  const name = ui.discovery.name.trim();
  if (name) parts.push(`name contains ${name}`);

  const map = ui.discovery.map.trim();
  if (map) parts.push(`map ${map}`);

  const latency = ui.discovery.latency;
  if (latency !== "<All>") parts.push(`latency ${latency}`);

  if (ui.discovery.secure === "Secure") parts.push("secure");
  else if (ui.discovery.secure === "Not secure") parts.push("not secure");

  if (ui.checks.no_full) parts.push("is not full");
  if (ui.checks.no_empty) parts.push("is not empty");
  if (ui.checks.no_bots) parts.push("has no bots or bot plugin");
  if (ui.checks.no_password) parts.push("has no password");

  return `${parts.join("; ")}.`;
}
