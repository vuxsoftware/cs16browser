<script lang="ts">
  /* ScrollBar.svelte — the list's own scrollbar.
   *
   * Not the native one: Chromium's overlay scrollbars have no layout width and
   * fade out when idle, so the reference's persistent bar with its arrow buttons
   * cannot be reproduced by styling `::-webkit-scrollbar`. The scroller keeps
   * native scrolling (wheel, keyboard, `scrollTop`) and this draws the chrome.
   *
   * Measured off the reference: the up button lives in the header row (see
   * Header.svelte); below it a 35px track (#5a6a50) holds a raised,
   * panel-coloured thumb, and a 36px down button closes the bar. 2px of list
   * fill separate the bar from the frame.
   */
  import { ROW_H } from "./columns";
  import * as store from "./store.svelte";

  const ARROW = 36;
  const MIN_THUMB = 30;

  /** Self-measured, because the track is what remains under the two arrows. */
  let selfH = $state(0);
  let dragging: { startY: number; startScroll: number } | null = null;

  const content = $derived(store.ui.order.length * ROW_H + 7);
  const viewport = $derived(store.ui.viewport);
  const maxScroll = $derived(Math.max(1, content - viewport));
  const trackH = $derived(Math.max(0, selfH - ARROW));
  const thumbH = $derived(
    content <= viewport ? trackH : Math.max(MIN_THUMB, Math.round((trackH * viewport) / content)),
  );
  const thumbTop = $derived(
    content <= viewport ? 0 : Math.round((store.ui.scroll / maxScroll) * (trackH - thumbH)),
  );
  const needed = $derived(content > viewport);

  const scroller = (): HTMLElement | null => store.dom.list;

  function nudge(rows: number): void {
    const el = scroller();
    if (el) el.scrollTop += rows * ROW_H;
  }

  function onPointerDown(event: PointerEvent): void {
    if (!needed) return;
    dragging = { startY: event.clientY, startScroll: store.ui.scroll };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function onPointerMove(event: PointerEvent): void {
    if (!dragging) return;
    const el = scroller();
    if (!el) return;
    /* The thumb's travel maps onto the scrollable range; clamping happens in
     * the element, so a drag past either end simply parks there. */
    const travel = Math.max(1, trackH - thumbH);
    const delta = ((event.clientY - dragging.startY) / travel) * maxScroll;
    el.scrollTop = dragging.startScroll + delta;
  }

  function onPointerUp(event: PointerEvent): void {
    if (!dragging) return;
    dragging = null;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
  }
</script>

<div class="sbar" bind:clientHeight={selfH} role="presentation">
  <div
    class="track"
    onclick={(e) => {
      if (e.target !== e.currentTarget) return;
      const page = Math.max(1, Math.floor(store.ui.viewport / ROW_H) - 1);
      nudge(e.offsetY < thumbTop ? -page : page);
    }}
    role="presentation"
  >
    {#if needed}
      <div
        class="thumb"
        style="top:{thumbTop}px; height:{thumbH}px"
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={onPointerUp}
        role="presentation"
      ></div>
    {/if}
  </div>
  <div class="sbtn down" onclick={() => nudge(1)} role="presentation">
    <span></span>
  </div>
</div>

<style>
  .sbar {
    position: relative;
    flex: 0 0 37px;
    height: 100%;
    background: var(--list-bg);
    display: flex;
    flex-direction: column;
  }
  .track {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    width: 35px;
    background: var(--slider);
  }
  .thumb {
    position: absolute;
    left: 0;
    width: 35px;
    background: var(--panel-bg);
    border-top: 1px solid var(--bevel-light);
    border-left: 1px solid var(--bevel-light);
    border-bottom: 1px solid var(--inset);
    border-right: 1px solid var(--inset);
  }
  .sbtn.down {
    position: relative;
    flex: 0 0 36px;
    height: 36px;
  }
</style>
