/* columns.ts — the measured layout tables.
 *
 * One table drives both the header and the rows, so the two cannot drift apart.
 * Widths are border-box values in capture pixels, taken from the header's
 * separator pairs (1px #282e22 then 1px #889180 — the lit column sits one pixel
 * right of the dark one, so a cell's border-box width is `lit + 1`):
 *
 *   #889180 at x16/48/88/120/230/1378/1602/1782 and #282e22 one pixel left of
 *   each (x47/87/119/229/1377/1601/1781/1907), so a cell is a lit 1px left edge,
 *   its content, and a dark 1px right edge.
 *
 * The nine widths sum to 1892 — the width of the list's content box — and the
 * scrollbar occupies the remaining 36px of the 1928px body. The header stops at
 * the same edge the rows' content does; it has no scrollbar of its own.
 *
 * The Name column absorbs available space. Resizing it compresses the columns
 * to its right while the scrollbar stays at the list edge.
 */

export const ROW_H = 28;

/** Which field of `ServerRow` a cell renders. Not `keyof ServerRow`: the Name
 * column shows `hostname` and the Latency column shows `ping_ms`, while a few
 * columns (the password/bots/secure glyphs) show several fields at once.
 * `endpoint` and `reason` are Banned-tab-only: that tab has nothing to do
 * with players, map or latency — it shows who was banned, where, and why. */
export type CellKey =
  | "password"
  | "bots"
  | "secure"
  | "players"
  | "flag"
  | "name"
  | "game"
  | "map"
  | "latency"
  | "endpoint"
  | "reason";

/** Every sortable key is a field of `ServerRow`, so `sort.key` can index it. */
export type SortKey = "players" | "name" | "game" | "map" | "ping";

export interface Column {
  readonly key: CellKey;
  readonly w: number;
  readonly align: "center" | "left";
  /** Header glyph for the icon columns. */
  readonly icon?: "lock" | "bots" | "shield" | "flag";
  readonly label?: string;
  readonly header?: "servers";
  readonly sort?: SortKey;
  readonly grow?: boolean;
}

/** Border-box column widths, in order. */
export const COLUMNS: readonly Column[] = [
  { key: "password", w: 32, align: "center", icon: "lock" },
  { key: "bots", w: 40, align: "center", icon: "bots" },
  { key: "secure", w: 32, align: "center", icon: "shield" },
  { key: "players", w: 110, align: "left", label: "Players", sort: "players" },
  { key: "flag", w: 36, align: "center", icon: "flag" },
  { key: "name", w: 1112, align: "left", header: "servers", sort: "name", grow: true },
  { key: "game", w: 224, align: "left", label: "Game", sort: "game" },
  { key: "map", w: 180, align: "left", label: "Map", sort: "map" },
  /* Latency is *left*-aligned like every other column: the reference's ping ink
   * starts 8px into the cell and leaves ~98px empty to its right, and the header
   * word does the same. The column is wide because `Latency` is. */
  { key: "latency", w: 126, align: "left", label: "Latency", sort: "ping" },
];

/** The Banned tab's columns: who, where, and why. Servers fills the remaining
 * width; its divider changes IP : Port, and the IP : Port divider changes
 * Reason. The right edge is the list boundary, so it has no resize handle. */
export const BANNED_COLUMNS: readonly Column[] = [
  { key: "flag", w: 36, align: "center", icon: "flag" },
  { key: "name", w: 420, align: "left", header: "servers", grow: true },
  { key: "endpoint", w: 280, align: "left", label: "IP : Port" },
  { key: "reason", w: 500, align: "left", label: "Reason" },
];

export type ColumnWidths = Partial<Record<CellKey, number>>;

export const COLUMN_MIN_WIDTH: Readonly<Record<CellKey, number>> = {
  password: 32,
  bots: 40,
  secure: 32,
  players: 64,
  flag: 36,
  name: 120,
  game: 40,
  map: 40,
  latency: 80,
  endpoint: 100,
  reason: 100,
};

/** Both header and rows use the same resolved pixels, including while
 * dragging. `columns` defaults to the Internet/Favorites/History layout;
 * the Banned tab passes `BANNED_COLUMNS`. */
export function resolveColumnWidths(
  preferred: ColumnWidths,
  viewport: number,
  resized: CellKey | null,
  columns: readonly Column[] = COLUMNS,
): Record<CellKey, number> {
  const widths = Object.fromEntries(
    columns.map((col) => [col.key, preferred[col.key] ?? col.w]),
  ) as Record<CellKey, number>;
  const total = columns.reduce((sum, col) => sum + widths[col.key], 0);
  if (viewport <= 0) return widths;
  if (columns === BANNED_COLUMNS) {
    widths.endpoint = Math.max(COLUMN_MIN_WIDTH.endpoint, widths.endpoint);
    widths.reason = Math.max(COLUMN_MIN_WIDTH.reason, widths.reason);
    let overflow = Math.max(
      0,
      COLUMN_MIN_WIDTH.flag + COLUMN_MIN_WIDTH.name + widths.endpoint + widths.reason - viewport,
    );
    const reasonReduction = Math.min(overflow, widths.reason - COLUMN_MIN_WIDTH.reason);
    widths.reason -= reasonReduction;
    overflow -= reasonReduction;
    widths.endpoint -= Math.min(overflow, widths.endpoint - COLUMN_MIN_WIDTH.endpoint);
    widths.name = Math.max(
      COLUMN_MIN_WIDTH.name,
      viewport - widths.flag - widths.endpoint - widths.reason,
    );
    return widths;
  }
  if (total < viewport && preferred.name === undefined) widths.name += viewport - total;

  let overflow = Math.max(0, columns.reduce((sum, col) => sum + widths[col.key], 0) - viewport);
  const shrinkOrder: CellKey[] =
    resized === "name"
      ? ["game", "map", "latency", "name", "players"]
      : ["name", "game", "map", "latency", "players"];
  const priority =
    resized === "name"
      ? shrinkOrder
      : shrinkOrder.filter((key) => key !== resized).concat(resized ? [resized] : []);
  for (const key of priority) {
    const reduction = Math.min(overflow, Math.max(0, widths[key] - COLUMN_MIN_WIDTH[key]));
    widths[key] -= reduction;
    overflow -= reduction;
    if (overflow === 0) break;
  }
  return widths;
}

/** Resize a Banned separator while keeping the three columns inside the list.
 * The final column changes through the IP : Port separator. */
export function resizeBannedColumns(
  preferred: ColumnWidths,
  viewport: number,
  boundary: "name" | "endpoint",
  wanted: number,
): ColumnWidths {
  const current = resolveColumnWidths(preferred, viewport, null, BANNED_COLUMNS);
  if (boundary === "name") {
    const pair = current.name + current.endpoint;
    const name = Math.min(
      pair - COLUMN_MIN_WIDTH.endpoint,
      Math.max(COLUMN_MIN_WIDTH.name, wanted),
    );
    return { endpoint: pair - name, reason: current.reason };
  }
  const pair = current.endpoint + current.reason;
  const endpoint = Math.min(
    pair - COLUMN_MIN_WIDTH.reason,
    Math.max(COLUMN_MIN_WIDTH.endpoint, wanted),
  );
  return { endpoint, reason: pair - endpoint };
}

export function columnStyle(col: Column, widths: Record<CellKey, number>): string {
  return `flex:0 0 ${widths[col.key]}px`;
}

/** Tab labels, in the order the user chose: Internet, Favorites, History,
 * then Banned (flagged and skipped servers). */
export const TABS = ["Internet", "Favorites", "History", "Banned"] as const;

export interface CheckDef {
  readonly id: "no_full" | "no_empty" | "no_bots" | "no_password";
  readonly label: string;
  readonly on: boolean;
  readonly disabled?: boolean;
}

/** Checkbox labels. The reference's fourth box ("Has associated Steam
 * account") had no data source, so its slot holds a bots filter instead, which
 * every A2S reply does carry — plus the bot plugin a server's rules name, since
 * such a plugin can report its bots as humans. All four filter client-side
 * (`passesChecks`). */
export const CHECKS: readonly CheckDef[] = [
  { id: "no_full", label: "Server not full", on: false },
  { id: "no_empty", label: "Has users playing", on: false },
  { id: "no_bots", label: "Has no bots playing / plugin installed", on: false },
  { id: "no_password", label: "Is not password protected", on: false },
];

const OPT_MAP = ["<All>", "de_dust2", "de_inferno", "de_aztec", "cs_office", "cs_assault"];
export const OPT_LATENCY = ["<All>", "< 50", "< 100", "< 150", "< 250", "< 350", "< 600"];
export const OPT_SECURE = ["<All>", "Secure", "Not secure"];

/** The four local filter controls, keyed by the state field they write.
 * Name and Map are case-insensitive substring entries. */
export const FIELDS = [
  {
    id: "name",
    label: "Name",
    items: [] as string[],
    width: "game",
    wide: false,
    text: true,
    locked: false,
  },
  {
    id: "latency",
    label: "Latency",
    items: OPT_LATENCY,
    width: "small",
    wide: true,
    text: false,
    locked: false,
  },
  {
    id: "map",
    label: "Map",
    items: OPT_MAP,
    width: "game",
    wide: false,
    text: true,
    locked: false,
  },
  {
    id: "secure",
    label: "Anti-cheat",
    items: OPT_SECURE,
    width: "small",
    wide: true,
    text: false,
    locked: false,
  },
] as const;
export type FieldId = (typeof FIELDS)[number]["id"];

/** Empty-state strings; the tab ones are verbatim where the localization file
 * has them. */
export const EMPTY_TAB: Readonly<Record<string, string>> = {
  Favorites: "You currently have no favorite servers selected.",
  History: "No servers have been played recently.",
  Banned: "No banned or skipped servers.",
};

/** `ServerBrowser_PendingPing`, also used for paced and banned rows: nothing
 * was sent, and the server may be perfectly healthy. */
export const PENDING_PING = "<pending>";
