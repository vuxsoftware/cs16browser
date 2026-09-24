<script lang="ts">
  /* Header.svelte — the list header strip.
   *
   * Boundaries carry a 1px dark + 1px lit pair (measured at x44/45, 84/85,
   * 116/117, 226/227, 1374/1375, 1598/1599, 1778/1779, 1904/1905) in the
   * original browser. The added flag column shifts later separators. No
   * column rules exist in the body. The labels are #ffffff and the same size as
   * the rows: `Players` measures 63px of ink, which is Tahoma 20px — the size
   * that also fits `Servers (316)`, the digits and the row text.
   */
  import {
    BANNED_COLUMNS,
    COLUMN_MIN_WIDTH,
    columnStyle,
    resizeBannedColumns,
    resolveColumnWidths,
  } from "./columns";
  import type { Column } from "./columns";
  import { ROW_H } from "./columns";
  import * as store from "./store.svelte";
  import Icon from "./Icon.svelte";

  interface Props {
    columns: readonly Column[];
  }
  let { columns }: Props = $props();

  const count = $derived(store.ui.order.length);
  const bannedLayout = $derived(columns === BANNED_COLUMNS);
  const widths = $derived(
    resolveColumnWidths(
      bannedLayout ? store.ui.bannedColumnWidths : store.ui.columnWidths,
      store.ui.listWidth,
      bannedLayout ? null : store.ui.columnResizeKey,
      columns,
    ),
  );
  let resizing: { key: Column["key"]; startX: number; startWidth: number; scale: number } | null =
    null;

  function setWidth(key: Column["key"], wanted: number): void {
    if (bannedLayout) {
      if (key === "name" || key === "endpoint") {
        store.ui.bannedColumnWidths = resizeBannedColumns(
          store.ui.bannedColumnWidths,
          store.ui.listWidth,
          key,
          wanted,
        );
      }
      return;
    }
    const minimum = COLUMN_MIN_WIDTH[key];
    const otherMinimums = columns.reduce(
      (sum, col) => sum + (col.key === key ? 0 : COLUMN_MIN_WIDTH[col.key]),
      0,
    );
    const maximum =
      store.ui.listWidth > 0 ? Math.max(minimum, store.ui.listWidth - otherMinimums) : Infinity;
    store.ui.columnResizeKey = key;
    store.ui.columnWidths[key] = Math.min(maximum, Math.max(minimum, wanted));
  }

  function resizeStart(event: PointerEvent, col: Column): void {
    if (event.button !== 0) return;
    const cell = (event.currentTarget as HTMLElement).parentElement;
    if (!cell) return;
    resizing = {
      key: col.key,
      startX: event.clientX,
      startWidth: cell.offsetWidth,
      scale: cell.getBoundingClientRect().width / cell.offsetWidth || 1,
    };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    event.preventDefault();
    event.stopPropagation();
  }

  function resizeMove(event: PointerEvent): void {
    if (!resizing) return;
    setWidth(
      resizing.key,
      Math.round(resizing.startWidth + (event.clientX - resizing.startX) / resizing.scale),
    );
  }

  function resizeEnd(event: PointerEvent): void {
    if (!resizing) return;
    resizing = null;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
  }

  function resizeKey(event: KeyboardEvent, col: Column): void {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    const cell = (event.currentTarget as HTMLElement).parentElement;
    if (!cell) return;
    const step = event.shiftKey ? 25 : 10;
    setWidth(col.key, cell.offsetWidth + (event.key === "ArrowRight" ? step : -step));
    event.preventDefault();
    event.stopPropagation();
  }

  function click(col: Column): void {
    if (!col.sort) return;
    if (store.ui.sort.key === col.sort) {
      store.ui.sort.asc = !store.ui.sort.asc;
    } else {
      /* First click picks the direction a reader wants: most-played on top, but
       * lowest ping on top. Ascending `players` would open on the empty
       * servers, which is useless. */
      store.ui.sort = { key: col.sort, asc: col.sort !== "players" };
    }
    store.recompute();
  }
</script>

<div class="header">
  <div class="hviewport">
    <div class="hcolumns">
      {#each columns as col (col.key)}
        {@const sorted = col.sort !== undefined && col.sort === store.ui.sort.key}
        <div
          class="hcell {col.align} {col.key}{sorted ? ' sorted' : ''}"
          style={columnStyle(col, widths)}
          onclick={() => click(col)}
          role="presentation"
        >
          {#if col.icon === "lock"}<Icon name="lock" />
          {:else if col.icon === "bots"}<Icon name="bots" />
          {:else if col.icon === "shield"}<Icon name="shield" />
          {:else if col.icon === "flag"}<span
              class="world-flag"
              title="Country flag"
              aria-label="Country flag">🌐</span
            >
          {:else if col.header === "servers"}Servers ({count})
          {:else}{col.label}{/if}
          {#if sorted}<span class="caret">{store.ui.sort.asc ? "▲" : "▼"}</span>{/if}
          {#if col.icon === undefined && !(bannedLayout && col.key === "reason")}
            <button
              class="resize-handle"
              type="button"
              aria-label={`Resize ${col.label ?? "Servers"} column`}
              onpointerdown={(event) => resizeStart(event, col)}
              onpointermove={resizeMove}
              onpointerup={resizeEnd}
              onpointercancel={resizeEnd}
              onkeydown={(event) => resizeKey(event, col)}
              onclick={(event) => event.stopPropagation()}
            ></button>
          {/if}
        </div>
      {/each}
    </div>
  </div>
  <!-- The scrollbar's up button sits in the header row, as in the reference;
       the rest of the bar (track, thumb, down button) is ScrollBar.svelte. -->
  <div class="hgutter">
    <div
      class="sbtn up"
      onclick={() => {
        if (store.dom.list) store.dom.list.scrollTop -= ROW_H;
      }}
      role="presentation"
    >
      <span></span>
    </div>
  </div>
</div>

<style>
  .hviewport {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    height: 100%;
  }
  .hcolumns {
    display: flex;
    width: 100%;
    height: 100%;
  }
  .hcell {
    position: relative;
  }
  .resize-handle {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 8px;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: col-resize;
    touch-action: none;
  }
  .resize-handle:focus-visible {
    outline: 2px solid var(--text-bright);
    outline-offset: -2px;
  }
</style>
