<script lang="ts">
  /* Modal.svelte — the ban list and the hard-refresh / connect notices (§7.1),
   * in the same VGUI dialog frame as Game Info. `well` puts the body in a
   * sunken list well (the ban table); otherwise it is plain panel text. */
  import type { Snippet } from "svelte";
  import type { ModalState } from "./store.svelte";
  import Dialog from "./Dialog.svelte";

  interface Props {
    title: string;
    buttons: ModalState["buttons"];
    onclose: () => void;
    well?: boolean;
    children: Snippet;
  }

  let { title, buttons, onclose, well = false, children }: Props = $props();
</script>

<Dialog {title} {buttons} {onclose} width={well ? 1100 : 780} height={well ? 720 : 360}>
  <div class="dbody" class:well>{@render children()}</div>
</Dialog>

<style>
  .dbody {
    position: absolute;
    left: 32px;
    right: 32px;
    top: 76px;
    bottom: 104px;
    overflow: auto;
  }
  .dbody.well {
    left: 47px;
    right: 47px;
    background: var(--list-bg);
    border-top: 1px solid var(--inset);
    border-left: 1px solid var(--inset);
    border-bottom: 1px solid var(--bevel-light);
    border-right: 1px solid var(--bevel-light);
  }
</style>
