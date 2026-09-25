<script lang="ts">
  /* Grip.svelte — the bottom-right resize grip on the other dialogs: six 6x6
   * squares in a stepped triangle at a 12px pitch. */
  interface Props {
    onresize?: (event: PointerEvent) => void;
  }

  let { onresize }: Props = $props();
</script>

<div class="grip" onpointerdown={onresize} role="presentation">
  <i style="left:24px;top:0;background:#c0bfbd"></i>
  <i style="left:12px;top:12px"></i><i style="left:24px;top:12px"></i>
  <i style="left:0;top:24px"></i><i style="left:12px;top:24px"></i><i style="left:24px;top:24px"
  ></i>
</div>

<style>
  .grip {
    position: absolute;
    right: 2px;
    bottom: 3px;
    width: 30px;
    height: 30px;
    z-index: 3;
    cursor: nwse-resize;
  }
  i {
    position: absolute;
    width: 6px;
    height: 6px;
    background: #9a9a98;
  }
</style>
