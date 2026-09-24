<script lang="ts">
  /* ContextMenu.svelte — the right-click row menu. App.svelte supplies actions
   * for the active tab. Dim items are disabled; the armed item is a
   * SelectionBG band with white text. */
  interface Props {
    left: number;
    top: number;
    items: { label: string; disabled?: boolean }[];
    armed: number;
    onarm: (index: number) => void;
    onpick: (index: number) => void;
    ondismiss: () => void;
  }

  let { left, top, items, armed, onarm, onpick, ondismiss }: Props = $props();
</script>

<svelte:window
  onpointerdown={(e) => {
    const t = e.target;
    if (!(t instanceof Element) || !t.closest(".ctxmenu")) ondismiss();
  }}
/>

<div
  class="ctxmenu"
  style="left:{left}px; top:{top}px"
  onpointerdown={(e) => e.stopPropagation()}
  oncontextmenu={(e) => e.preventDefault()}
  role="presentation"
>
  {#each items as item, i (item.label)}
    <div
      class="item{i === armed ? ' armed' : ''}{item.disabled ? ' disabled' : ''}"
      onclick={() => {
        if (!item.disabled) onpick(i);
      }}
      onmouseenter={() => onarm(i)}
      role="presentation"
    >
      {item.label}
    </div>
  {/each}
</div>

<style>
  .ctxmenu {
    position: absolute;
    z-index: 50;
    background: var(--panel-bg);
    border-top: 1px solid var(--bevel-light);
    border-left: 1px solid var(--bevel-light);
    border-bottom: 1px solid var(--inset);
    border-right: 1px solid var(--inset);
    padding: 1px;
    min-width: 282px;
  }
  .item {
    height: 44px;
    display: flex;
    align-items: center;
    padding: 0 5px;
    font-size: var(--fs-chrome);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .item.armed {
    background: var(--selection-bg);
    color: var(--text-bright);
  }
  .item.disabled {
    color: var(--text-disabled);
  }
</style>
