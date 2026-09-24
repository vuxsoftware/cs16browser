<script lang="ts">
  /* ButtonBar.svelte — the anchored bottom row: `Change filters` toggle, the
   * filter summary, and the three actions. On the Banned tab the first
   * action is `Clear bans`. The Banned tab includes skipped rows as well as
   * bans; only actual bans enable that action. The row keeps its y in both
   * filter-panel states.
   *
   * Measured (relative to the frame): `Change filters` x16..231, summary text
   * from x239, then Quick refresh (210) · Refresh all (190) · Connect (150)
   * with 12px/10px gaps and Connect's right edge 8px inside the frame; the row
   * is y1179..1226, 19px above the frame's bottom rule. */
  import * as store from "./store.svelte";
  import { TABS } from "./columns";

  interface Props {
    summary: string;
    onquick: () => void;
    quickRefreshing: boolean;
    stopping: boolean;
    onrefresh: () => void;
    onstop: () => void;
    onconnect: () => void;
    onclearbans: () => void;
  }

  let {
    summary,
    onquick,
    quickRefreshing,
    stopping,
    onrefresh,
    onstop,
    onconnect,
    onclearbans,
  }: Props = $props();

  const ui = store.ui;
  /* While a sweep runs, the middle action is `Stop refresh` and `Quick refresh`
   * is dimmed (as in the reference); `Connect` needs a selected row. */
  const scanning = $derived(ui.phase === "scanning" || ui.phase === "fetching");
  const selected = $derived(store.selectedRow());
  const banned = $derived(TABS[ui.tab] === "Banned");
  const hasBans = $derived(ui.rows.some((row) => row.banned));
</script>

<div class="btnrow">
  <button
    class="vbtn btn-filters"
    class:pressed={ui.filtersOpen}
    onclick={() => {
      ui.filtersOpen = !ui.filtersOpen;
      ui.menu = null;
    }}>Change filters</button
  >
  <div class="statusrow">
    <span class="summary">{summary}</span>
    <span class="actions">
      {#if banned}
        <button class="vbtn b-quick" onclick={onclearbans} disabled={!hasBans}>Clear bans</button>
      {:else}
        <button
          class="vbtn b-quick"
          onclick={onquick}
          disabled={scanning || quickRefreshing || ui.order.length === 0}>Quick refresh</button
        >
      {/if}
      {#if scanning || quickRefreshing}
        <button class="vbtn b-all" onclick={onstop} disabled={stopping}>Stop refresh</button>
      {:else}
        <button class="vbtn b-all" onclick={onrefresh}>Refresh all</button>
      {/if}
      <button
        class="vbtn b-connect"
        onclick={onconnect}
        disabled={!selected || selected.banned || selected.fake}>Connect</button
      >
    </span>
  </div>
</div>

<style>
  .btnrow {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 18px;
    height: 48px;
  }
  /* `Change filters` is a ToggleButton (DialogServerBrowser.res names it
   * `Filter`): unpressed it is a raised box with a white label; pressed it
   * inverts its bevel and turns the label gold. */
  .btn-filters {
    position: absolute;
    left: 15px;
    top: 0;
    width: 216px;
  }
  .btn-filters.pressed {
    border-top-color: var(--inset);
    border-left-color: var(--inset);
    border-bottom-color: var(--bevel-light);
    border-right-color: var(--bevel-light);
    color: var(--accent);
  }
  .statusrow {
    position: absolute;
    left: 238px;
    right: 7px;
    top: 0;
    height: 48px;
    display: flex;
    align-items: center;
    overflow: hidden;
  }
  .summary {
    flex: 1 1 auto;
    min-width: 0;
    font-size: var(--fs-chrome);
    color: var(--text-dim);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .actions {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    height: 100%;
    margin-left: 16px;
  }
  .b-quick {
    width: 210px;
  }
  .b-all {
    width: 190px;
    margin-left: 12px;
  }
  .b-connect {
    width: 150px;
    margin-left: 10px;
  }
</style>
