<script lang="ts">
  /* Icon.svelte — stand-ins for Valve's 16x16 icon TGAs (gui-design.md §2.1.1):
   * same role, same measured ink box, our own art. Each svg carries its ink box
   * as its intrinsic width/height, so a flex cell can neither stretch it nor let
   * a viewBox-only svg claim the default 300x150.
   */
  type IconName =
    "lock" | "shield" | "bots" | "check" | "close" | "minimize" | "maximize" | "gear" | "info";

  interface Props {
    name: IconName;
  }

  let { name }: Props = $props();

  /* Measured ink boxes, width x height. */
  const BOX: Record<IconName, [number, number]> = {
    lock: [14, 18],
    shield: [14, 18],
    bots: [18, 18],
    check: [22, 15],
    close: [18, 18],
    minimize: [18, 18],
    maximize: [18, 18],
    gear: [20, 20],
    info: [20, 20],
  };

  const w = $derived(BOX[name][0]);
  const h = $derived(BOX[name][1]);
</script>

<svg class="glyph" width={w} height={h} viewBox="0 0 {w} {h}" aria-hidden="true">
  {#if name === "lock"}
    <g fill="currentColor">
      <path d="M2.5 8V5.5a4.5 4.5 0 0 1 9 0V8h-2.2V5.5a2.3 2.3 0 0 0-4.6 0V8Z" />
      <rect x="0.5" y="8" width="13" height="10" />
    </g>
  {:else if name === "shield"}
    <!-- Crenellated top (three points, two notches), tapering to a point. -->
    <path
      d="M0 1 3.5 3 7 0l3.5 3L14 1v8.5c0 4-3.2 6.8-7 8.5-3.8-1.7-7-4.5-7-8.5Z"
      fill="currentColor"
    />
  {:else if name === "bots"}
    <rect x="0" y="0" width="18" height="18" rx="3" fill="currentColor" />
    <path
      d="M5 3.5h5.2a3 3 0 0 1 2 5.3 3.2 3.2 0 0 1-1.6 5.7H5Zm2.4 2.2v2.3h2.6a1.15 1.15 0 0 0 0-2.3Zm0 4.4v2.3h3a1.15 1.15 0 0 0 0-2.3Z"
      fill="#4c5844"
    />
  {:else if name === "check"}
    <path d="M2 2 11 12 20 2" fill="none" stroke="currentColor" stroke-width="3.4" />
  {:else if name === "close"}
    <path
      d="M3 3l12 12M15 3 3 15"
      fill="none"
      stroke="currentColor"
      stroke-width="3.6"
      stroke-linecap="round"
    />
  {:else if name === "minimize"}
    <rect x="2" y="12" width="14" height="3.6" fill="currentColor" />
  {:else if name === "maximize"}
    <rect x="2" y="2" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.4" />
  {:else if name === "gear"}
    <!-- Eight square teeth around a ring. -->
    <g fill="currentColor">
      {#each [0, 45, 90, 135, 180, 225, 270, 315] as angle (angle)}
        <rect x="8.3" y="0.5" width="3.4" height="5" transform="rotate({angle} 10 10)" />
      {/each}
    </g>
    <circle cx="10" cy="10" r="5.4" fill="none" stroke="currentColor" stroke-width="3.6" />
  {:else if name === "info"}
    <circle cx="10" cy="10" r="8.5" fill="none" stroke="currentColor" stroke-width="2" />
    <circle cx="10" cy="5.5" r="1.3" fill="currentColor" />
    <path d="M10 9v6" stroke="currentColor" stroke-width="2.4" />
  {/if}
</svg>

<style>
  .glyph {
    display: block;
    flex: 0 0 auto;
  }
</style>
