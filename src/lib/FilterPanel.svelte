<script lang="ts">
  /* FilterPanel.svelte — §5.1 dropdowns and §5.2 checkboxes. */
  import { FIELDS, CHECKS } from "./columns";
  import type { FieldId } from "./columns";
  import * as store from "./store.svelte";
  import Combo from "./Combo.svelte";
  import Icon from "./Icon.svelte";

  /* Measured: control boxes at x135/x637, 15px below the panel's top rule
   * with 48px rows and a 12px gap. */
  function toggle(id: FieldId, target: EventTarget | null): void {
    if (store.ui.menu?.id === id) {
      store.ui.menu = null;
      return;
    }
    if (!(target instanceof HTMLElement)) return;
    /* The popup is left-aligned to its control and drops 2px below it,
     * overlapping whatever is beneath (measured: the dropdown's lit bottom edge
     * is at y1097/1098 and the menu's lit top border at y1100). Position is
     * relative to the app frame, so it survives a resize. */
    const r = target.getBoundingClientRect();
    const host = target.closest(".app")?.getBoundingClientRect();
    if (!host) return;
    const field = FIELDS.find((f) => f.id === id);
    if (!field) return;
    store.ui.menu = {
      id,
      items: field.items,
      armed: 0,
      left: r.left - host.left,
      top: r.bottom - host.top + 2,
      width: r.width,
    };
  }
</script>

<div class="filtergrid">
  {#each FIELDS as field (field.id)}
    <div class="frow">
      <span class="flabel{field.wide ? ' wide' : ''}">{field.label}</span>
      {#if field.text}
        <input
          class="fentry dd-{field.width}"
          value={store.ui.discovery[field.id]}
          aria-label={field.label}
          placeholder={field.id === "name" ? "Contains..." : undefined}
          maxlength={field.id === "name" ? 128 : 64}
          spellcheck="false"
          oninput={(e) => {
            store.ui.discovery[field.id] = e.currentTarget.value;
            store.recompute();
          }}
          onkeydown={(e) => e.stopPropagation()}
        />
      {:else}
        <Combo
          width={field.width}
          value={store.ui.discovery[field.id]}
          open={store.ui.menu?.id === field.id}
          locked={field.locked}
          ontoggle={(event) => {
            if (!field.locked) toggle(field.id, event.currentTarget);
          }}
        />
      {/if}
    </div>
  {/each}
</div>

<div class="checks">
  {#each CHECKS as check (check.id)}
    <div
      class="check{store.ui.checks[check.id] ? ' on' : ''}{check.disabled ? ' disabled' : ''}"
      onclick={() => {
        if (check.disabled) return;
        store.ui.checks[check.id] = !store.ui.checks[check.id];
        store.ui.flash = null;
        /* These checkboxes filter the visible list, so a toggle recomputes the
         * view rather than only repainting the chrome. */
        store.recompute();
      }}
      role="presentation"
    >
      <span class="box"><Icon name="check" /></span>
      <span class="label">{check.label}</span>
    </div>
  {/each}
</div>
