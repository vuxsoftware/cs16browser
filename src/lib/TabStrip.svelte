<script lang="ts">
  /* TabStrip.svelte — the tabs the user chose: Internet, Favorites,
   * History, Banned. Every face is a raised box (lit top/left, dark right) at a 145px
   * pitch from x15, label inset 12px. The active face is 4px taller (y92 vs
   * y96) and 1px wider, and it covers the frame's top rule (y143) so it merges
   * with the sheet below; inactive faces sit on that rule. */
  import { TABS } from "./columns";
  import * as store from "./store.svelte";

  const ui = store.ui;
</script>

<div class="tabs">
  {#each TABS as tab, i (tab)}
    <div
      class="tab{i === ui.tab ? ' active' : ''}"
      style="left:{i * 145}px"
      onclick={(e) => {
        e.stopPropagation();
        ui.tab = i;
        ui.selected = -1;
        store.recompute();
      }}
      role="presentation"
    >
      {tab}
    </div>
  {/each}
</div>

<style>
  .tabs {
    position: absolute;
    left: 15px;
    top: 92px;
    height: 52px;
    right: 16px;
    z-index: 3;
    pointer-events: none;
  }
  .tab {
    position: absolute;
    top: 4px;
    width: 143px;
    height: 47px;
    display: flex;
    align-items: center;
    padding-left: 12px;
    background: var(--panel-bg);
    border-top: 1px solid var(--bevel-light);
    border-left: 1px solid var(--bevel-light);
    border-right: 1px solid var(--inset);
    font-size: var(--fs-chrome);
    color: var(--text-bright);
    white-space: nowrap;
    pointer-events: auto;
  }
  /* Active: from y92 down through the frame rule at y143, so the rule is
   * interrupted under it. */
  .tab.active {
    top: 0;
    width: 144px;
    height: 52px;
    padding-left: 13px;
    padding-top: 3px;
    color: var(--accent);
  }
</style>
