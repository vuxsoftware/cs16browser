<script lang="ts">
  /* Route entry — App.svelte holds the whole orchestration (scan event pump,
   * keyboard map, layout). Mounted by SvelteKit; SSR is off (`+layout.ts`),
   * so the mount target always exists. */
  import App from "$lib/App.svelte";
</script>

<App />
