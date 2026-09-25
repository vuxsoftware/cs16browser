<script lang="ts">
  /* The measured GoldSrc chrome is global; Svelte component styles are scoped,
   * so app.css must be imported at the layout level. */
  import "../app.css";

  let { children } = $props();
</script>

{@render children()}
