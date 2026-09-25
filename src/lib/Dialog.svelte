<script lang="ts">
  /* Dialog.svelte — the VGUI child frame the reference's `Game Info` window is
   * drawn in: the same caption strip as the main window (glyph, caption, boxed
   * close), a panel-coloured body and bevelled buttons along the bottom right.
   * It is dragged by its caption, like the real window.
   *
   * Children are positioned in the dialog's own coordinates, so a body can copy
   * the reference's measurements directly (Game Info does).
   *
   * Measured off Game Info (831x879): buttons 160x48 with ~30px gaps, the last
   * one's right edge 48px in from the frame and their bottom 32px up from it.
   */
  import type { Snippet } from "svelte";
  import type { ModalState } from "./store.svelte";
  import TitleBar from "./TitleBar.svelte";
  import Grip from "./Grip.svelte";

  interface Props {
    title: string;
    width: number;
    height: number;
    buttons: ModalState["buttons"];
    onclose: () => void;
    showGrip?: boolean;
    children: Snippet;
  }

  let { title, width, height, buttons, onclose, showGrip = true, children }: Props = $props();

  /* null = centred; set once the user drags the caption. */
  let pos = $state<{ x: number; y: number } | null>(null);
  let box = $state<HTMLDivElement | null>(null);

  function dragStart(event: PointerEvent): void {
    if (!box || event.button !== 0) return;
    const startX = event.clientX;
    const startY = event.clientY;
    const originX = box.offsetLeft;
    const originY = box.offsetTop;
    const move = (e: PointerEvent): void => {
      pos = { x: originX + e.clientX - startX, y: Math.max(0, originY + e.clientY - startY) };
    };
    const up = (): void => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    event.preventDefault();
  }

  const placement = $derived(
    pos
      ? `left:${pos.x}px; top:${pos.y}px;`
      : `left:calc(50% - ${width / 2}px); top:max(0px, calc(50% - ${height / 2}px));`,
  );
</script>

<!-- The main window stays visible but inert behind the dialog, as it does
     behind a VGUI modal frame. -->
<div class="dialog-back" role="presentation">
  <div
    class="dialog"
    bind:this={box}
    style="{placement} width:{width}px; height:{height}px"
    role="dialog"
    aria-label={title}
  >
    <TitleBar caption={title} {onclose} ondragstart={dragStart} />
    {@render children()}
    <div class="dfoot">
      {#each buttons as b (b.label)}
        <button
          class="vbtn"
          onclick={() => {
            if (b.close !== false) onclose();
            b.onClick?.();
          }}>{b.label}</button
        >
      {/each}
    </div>
    {#if showGrip}<Grip />{/if}
  </div>
</div>

<style>
  .dialog-back {
    position: absolute;
    inset: 0;
    z-index: 60;
  }
  .dialog {
    position: absolute;
    background: var(--panel-bg);
    border-top: 1px solid var(--bevel-light);
    border-left: 1px solid var(--bevel-light);
    border-right: 1px solid var(--inset);
    border-bottom: 1px solid var(--inset);
    max-width: 100%;
    max-height: 100%;
  }
  .dfoot {
    position: absolute;
    right: 47px;
    bottom: 31px;
    display: flex;
    gap: 30px;
  }
  .dfoot .vbtn {
    width: 160px;
  }
</style>
