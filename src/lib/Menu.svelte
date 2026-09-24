<script lang="ts">
  /* Menu.svelte — the open dropdown's popup.
   *
   * Measured: panel background with a lit 1px border, items 42px tall with a
   * 2px gap, dim text inset ~4px from the left, and the armed item a full-width
   * SelectionBG band with white text. It overlaps whatever is beneath, which is
   * why it is absolutely positioned and drawn last. */
  interface Props {
    items: readonly string[];
    armed: number;
    left: number;
    top: number;
    width: number;
    onarm: (index: number) => void;
    onpick: (index: number) => void;
  }

  let { items, armed, left, top, width, onarm, onpick }: Props = $props();
</script>

<!-- Measured: the popup is 10px wider than its combo (x637..870 under a
     x637..860 control). -->
<div class="menu" style="left:{left}px; top:{top}px; min-width:{width + 10}px">
  {#each items as item, i (item)}
    <div
      class="item{i === armed ? ' armed' : ''}"
      onclick={(e) => {
        e.stopPropagation();
        onpick(i);
      }}
      onmouseenter={() => onarm(i)}
      role="presentation"
    >
      {item}
    </div>
  {/each}
</div>
