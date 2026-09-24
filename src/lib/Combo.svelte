<script lang="ts">
  /* Combo.svelte — the measured dropdown: inset well, dim value, ▼ at the right,
   * and while its menu is open the control itself turns gold because the armed
   * item's band covers it (visible in the reference capture). */
  interface Props {
    width: "game" | "small";
    value: string;
    open: boolean;
    /** Shown but fixed: no arrow, never opens. */
    locked?: boolean;
    ontoggle: (event: MouseEvent) => void;
  }

  let { width, value, open, locked = false, ontoggle }: Props = $props();
</script>

<div
  class="dropdown {width === 'game' ? 'dd-game' : 'dd-small'}{open ? ' open' : ''}{locked
    ? ' locked'
    : ''}"
  onclick={ontoggle}
  role="presentation"
>
  <span class="value">{value}</span>
  {#if !open && !locked}<span class="arrow"></span>{/if}
</div>

<style>
  /* A disabled VGUI combo: same well, disabled text colour, no arrow. */
  .locked .value {
    color: var(--text-disabled);
  }
</style>
