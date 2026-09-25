<script lang="ts">
  /* ServerRow.svelte — one measured server row.
   *
   * Absolute positioning is required, not cosmetic: this is a virtual scroller,
   * so `top` is per-row and cannot live in a stylesheet (see the CSP note in
   * docs/gui-design.md §8.1 — inline styles are why tauri.conf.json allows
   * 'unsafe-inline' for style-src).
   */
  import {
    BANNED_COLUMNS,
    COLUMNS,
    PENDING_PING,
    columnStyle,
    resolveColumnWidths,
  } from "./columns";
  import type { Column } from "./columns";
  import * as store from "./store.svelte";
  import type { ServerRow } from "./bindings";
  import Icon from "./Icon.svelte";

  interface Props {
    row: ServerRow;
    top: number;
    selected: boolean;
    banned: boolean;
    columns?: readonly Column[];
    onselect: () => void;
    onactivate: () => void;
    onmenu: (event: MouseEvent, endpoint: string) => void;
  }

  let {
    row,
    top,
    selected,
    banned,
    columns = COLUMNS,
    onselect,
    onactivate,
    onmenu,
  }: Props = $props();
  const widths = $derived(
    resolveColumnWidths(
      columns === BANNED_COLUMNS ? store.ui.bannedColumnWidths : store.ui.columnWidths,
      store.ui.listWidth,
      columns === BANNED_COLUMNS ? null : store.ui.columnResizeKey,
      columns,
    ),
  );

  /** A banned row sent nothing; a paced row sent nothing; an unmeasured row has
   * no answer yet. All three show the pending token rather than a fake 0. */
  function latency(r: ServerRow): string {
    if (r.banned) return "banned";
    if (r.ping_ms !== null) return String(r.ping_ms);
    return PENDING_PING;
  }
</script>

<div
  class="row{selected ? ' selected' : ''}{banned ? ' banned' : ''}"
  style="top:{top}px"
  onclick={onselect}
  ondblclick={onactivate}
  oncontextmenu={(e) => {
    onselect();
    onmenu(e, row.endpoint);
  }}
  role="presentation"
>
  {#each columns as col (col.key)}
    <div class="cell {col.align} {col.key}" style={columnStyle(col, widths)}>
      {#if col.key === "password"}
        {#if row.password}<Icon name="lock" />{/if}
      {:else if col.key === "bots"}
        {row.bots > 0 ? row.bots : ""}
      {:else if col.key === "secure"}
        {#if row.secure}<Icon name="shield" />{/if}
      {:else if col.key === "players"}
        {row.players} / {row.max_players}
      {:else if col.key === "flag"}
        {#if row.country}
          <span
            class="fi fi-{row.country.toLowerCase()}"
            title={row.country}
            aria-label={row.country}
          ></span>
        {:else}
          <span class="world-flag" title="Unknown country" aria-label="Unknown country">🌐</span>
        {/if}
      {:else if col.key === "name"}
        {row.hostname}
      {:else if col.key === "game"}
        {row.game}
      {:else if col.key === "map"}
        {row.map}
      {:else if col.key === "latency"}
        <span class="latency">{latency(row)}</span>
      {:else if col.key === "endpoint"}
        {row.endpoint}
      {:else if col.key === "reason"}
        {row.reasons.length
          ? row.reasons.join(", ")
          : row.banned
            ? "banned"
            : row.fake
              ? "flagged"
              : row.outcome}
      {/if}
    </div>
  {/each}
</div>
