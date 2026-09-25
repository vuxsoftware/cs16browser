<script lang="ts">
  /* ServerList.svelte — the virtual scroller.
   *
   * Only rows intersecting the viewport are in the DOM, with a one-screen
   * overscan so scrolling never shows blanks: a 20 000-row list must not create
   * 20 000 nodes.
   *
   * The first row's box starts 7px below the body's top edge (measured: body top
   * 197, first row box 204), so the scroll content carries that offset.
   */
  import { ROW_H, COLUMNS } from "./columns";
  import type { Column } from "./columns";
  import * as store from "./store.svelte";
  import ServerRow from "./ServerRow.svelte";
  import ScrollBar from "./ScrollBar.svelte";

  interface Props {
    emptyMessage: string;
    columns?: readonly Column[];
    onactivate: () => void;
    onmenu: (event: MouseEvent, endpoint: string) => void;
  }

  let { emptyMessage, columns = COLUMNS, onactivate, onmenu }: Props = $props();

  let el = $state<HTMLDivElement | null>(null);
  const PAD = 7;

  const total = $derived(store.ui.order.length);
  const first = $derived(Math.max(0, Math.floor(store.ui.scroll / ROW_H) - 12));
  const last = $derived(
    Math.min(total, Math.ceil((store.ui.scroll + store.ui.viewport) / ROW_H) + 12),
  );
  /* `order` holds indices into `rows`: rows are stored once and ordered
   * separately, so the window has to map through it. */
  const visible = $derived(
    store.ui.order.slice(first, last).map((rowIndex, k) => ({ rowIndex, i: first + k })),
  );

  $effect(() => {
    store.dom.list = el;
    if (!el) return;
    store.ui.listWidth = el.clientWidth;
    const observer = new ResizeObserver(() => {
      store.ui.viewport = el?.clientHeight ?? 0;
      store.ui.listWidth = el?.clientWidth ?? 0;
    });
    observer.observe(el);
    return () => observer.disconnect();
  });
</script>

<!-- The scroller and the bar are siblings: the bar is drawn, not native, so
     it can hold its 36px of layout the way the reference does. -->
<div class="scroller">
  <div
    class="scroll"
    bind:this={el}
    onscroll={() => {
      if (el) store.ui.scroll = el.scrollTop;
    }}
    role="listbox"
    tabindex="-1"
    aria-label="Servers"
  >
    <div class="spacer" style="height:{PAD + total * ROW_H}px">
      {#each visible as v (store.ui.rows[v.rowIndex]?.endpoint ?? v.i)}
        {@const row = store.ui.rows[v.rowIndex]}
        {#if row}
          <ServerRow
            {row}
            {columns}
            top={v.i * ROW_H}
            selected={v.i === store.ui.selected}
            banned={row.banned}
            onselect={() => {
              store.ui.selected = v.i;
            }}
            {onactivate}
            {onmenu}
          />
        {/if}
      {/each}
    </div>
    {#if total === 0}
      <div class="empty">{emptyMessage}</div>
    {/if}
  </div>
  <ScrollBar />
</div>

<style>
  .scroller {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .scroll {
    flex: 1 1 auto;
    min-width: 0;
    position: relative;
    overflow-y: auto;
    overflow-x: hidden;
    background: var(--list-bg);
    padding-top: 7px;
    /* The vertical bar is drawn by ScrollBar.svelte. Columns are resolved
     * inside this width, so no horizontal scrollbar is needed. */
    scrollbar-width: none;
  }
  .scroll::-webkit-scrollbar {
    display: none;
  }
  .spacer {
    position: relative;
    width: 100%;
  }
  .empty {
    position: absolute;
    left: 0;
    right: 0;
    top: 40%;
    text-align: center;
    font-size: var(--fs-chrome);
    color: var(--text-dim);
    pointer-events: none;
  }
</style>
