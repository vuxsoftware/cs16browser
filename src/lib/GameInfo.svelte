<script lang="ts">
  /* GameInfo.svelte — the `Game Info` dialog, laid out on the reference capture
   * (831x879):
   *
   *   - label/value pairs at a 48px pitch from y80; labels dim and right-aligned
   *     to x243, values from x257
   *   - the address in a read-only sunken field, x255..774 / y127..174
   *   - the player table as a sunken well x47..782 / y455..734: a 36px header
   *     (Player Name | Score | Time, boundaries at x359 and x487), 34px rows, and
   *     a scrollbar whose up button sits in the header row
   *   - Join Game / Refresh / Close along the bottom (Dialog.svelte)
   */
  import Dialog from "./Dialog.svelte";
  import type { ServerRow } from "./bindings";

  interface Props {
    row: ServerRow;
    latencyMs: number;
    playerListLoading: boolean;
    playerListError: string | null;
    onjoin: () => void;
    onrefresh: () => void;
    onclose: () => void;
  }

  let { row, latencyMs, playerListLoading, playerListError, onjoin, onrefresh, onclose }: Props =
    $props();

  const PLAYER_ROW = 34;
  type PlayerSort = "name" | "score" | "time";
  let playerSort = $state<{ key: PlayerSort; asc: boolean }>({ key: "score", asc: false });
  const sortedPlayers = $derived(
    row.players_list
      .map((player, index) => ({ player, index }))
      .sort((a, b) => {
        let result: number;
        if (playerSort.key === "name")
          result = a.player.name.localeCompare(b.player.name, undefined, { sensitivity: "base" });
        else if (playerSort.key === "score") result = a.player.score - b.player.score;
        else {
          const left = a.player.duration_seconds;
          const right = b.player.duration_seconds;
          if (left === null || right === null)
            return left === right ? a.index - b.index : left === null ? 1 : -1;
          result = left - right;
        }
        return (playerSort.asc ? result : -result) || a.index - b.index;
      }),
  );

  function sortPlayers(key: PlayerSort): void {
    playerSort =
      playerSort.key === key ? { key, asc: !playerSort.asc } : { key, asc: key === "name" };
  }

  /* `8m 12s` — the reference's duration format. */
  function duration(seconds: number | null): string {
    if (seconds === null) return "";
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    return h > 0 ? `${h}h ${m}m ${s}s` : m > 0 ? `${m}m ${s}s` : `${s}s`;
  }

  /* The table's own scrollbar: native scrolling (wheel, keyboard) with drawn
   * chrome, driven exactly like the server list's ScrollBar.svelte. */
  let body = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let viewH = $state(0);
  /* Measured, not derived: the track is what the down button leaves. */
  let trackH = $state(0);
  const contentH = $derived(row.players_list.length * PLAYER_ROW + 5);
  const maxScroll = $derived(Math.max(1, contentH - viewH));
  const needed = $derived(contentH > viewH);
  const thumbH = $derived(needed ? Math.max(30, Math.round((trackH * viewH) / contentH)) : 0);
  const thumbTop = $derived(
    needed ? Math.round((Math.min(scrollTop, maxScroll) / maxScroll) * (trackH - thumbH)) : 0,
  );

  function nudge(rows: number): void {
    if (body) body.scrollTop += rows * PLAYER_ROW;
  }

  /* Thumb drag: its travel maps onto the scrollable range. Pointer capture
   * keeps the drag alive when the cursor leaves the thin track. */
  let dragging: { startY: number; startScroll: number } | null = null;

  function dragStart(event: PointerEvent): void {
    if (!needed || event.button !== 0) return;
    dragging = { startY: event.clientY, startScroll: body?.scrollTop ?? 0 };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    event.preventDefault();
    event.stopPropagation();
  }

  function dragMove(event: PointerEvent): void {
    if (!dragging || !body) return;
    const travel = Math.max(1, trackH - thumbH);
    body.scrollTop =
      dragging.startScroll + ((event.clientY - dragging.startY) / travel) * maxScroll;
  }

  function dragEnd(event: PointerEvent): void {
    if (!dragging) return;
    dragging = null;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
  }

  /* A click on the bare track pages toward it, as in the list. */
  function page(event: MouseEvent): void {
    if (event.target !== event.currentTarget) return;
    const rows = Math.max(1, Math.floor(viewH / PLAYER_ROW) - 1);
    nudge(event.offsetY < thumbTop ? -rows : rows);
  }
</script>

<Dialog
  title="Game Info -"
  width={831}
  height={879}
  buttons={[
    { label: "Join Game", onClick: onjoin, close: false },
    { label: "Refresh", onClick: onrefresh, close: false },
    { label: "Close" },
  ]}
  {onclose}
  showGrip={false}
>
  <div class="pairs">
    <div class="pair"><span class="label">Name:</span><span class="val">{row.hostname}</span></div>
    <div class="pair">
      <span class="label">IP Address:</span><span class="val well">{row.endpoint}</span>
    </div>
    <div class="pair"><span class="label">Game:</span><span class="val">{row.game}</span></div>
    <div class="pair"><span class="label">Map:</span><span class="val">{row.map}</span></div>
    <div class="pair">
      <span class="label">Players:</span><span class="val">{row.players} / {row.max_players}</span>
    </div>
    <div class="pair">
      <span class="label">Valve Anti-Cheat:</span><span class="val"
        >{row.secure ? "Secure" : "Not secure"}</span
      >
    </div>
    <div class="pair"><span class="label">Latency:</span><span class="val">{latencyMs}</span></div>
  </div>

  <div class="ptable">
    <div class="phead">
      <button
        class="pc name"
        class:sorted={playerSort.key === "name"}
        onclick={() => sortPlayers("name")}
        >Player Name{#if playerSort.key === "name"}<span class="caret"
            >{playerSort.asc ? "▲" : "▼"}</span
          >{/if}</button
      >
      <button
        class="pc score"
        class:sorted={playerSort.key === "score"}
        onclick={() => sortPlayers("score")}
        >Score{#if playerSort.key === "score"}<span class="caret">{playerSort.asc ? "▲" : "▼"}</span
          >{/if}</button
      >
      <button
        class="pc time"
        class:sorted={playerSort.key === "time"}
        onclick={() => sortPlayers("time")}
        >Time{#if playerSort.key === "time"}<span class="caret">{playerSort.asc ? "▲" : "▼"}</span
          >{/if}</button
      >
      <div class="pgutter">
        <div class="sbtn up" onclick={() => nudge(-1)} role="presentation"><span></span></div>
      </div>
    </div>
    <div class="pbody">
      <div
        class="prows"
        bind:this={body}
        bind:clientHeight={viewH}
        onscroll={() => {
          if (body) scrollTop = body.scrollTop;
        }}
      >
        <!-- Keep each player's original position as the key: GoldSrc reports
             index 0 for every player, so its index is not a safe key. -->
        {#each sortedPlayers as { player, index } (index)}
          <div class="prow">
            <div class="pc name">{player.name}</div>
            <div class="pc score">{player.score}</div>
            <div class="pc time">{duration(player.duration_seconds)}</div>
          </div>
        {:else}
          <div class="prow empty">
            {#if playerListLoading}
              Loading player list...
            {:else if playerListError}
              {playerListError}
            {:else if row.players > 0}
              Player list unavailable. The server reports {row.players} players.
            {:else}
              There are no players on the server.
            {/if}
          </div>
        {/each}
      </div>
      <div class="psbar">
        <div class="ptrack" bind:clientHeight={trackH} onclick={page} role="presentation">
          {#if needed}
            <div
              class="pthumb"
              style="top:{thumbTop}px; height:{thumbH}px"
              onpointerdown={dragStart}
              onpointermove={dragMove}
              onpointerup={dragEnd}
              onpointercancel={dragEnd}
              role="presentation"
            ></div>
          {/if}
        </div>
        <div class="sbtn down" onclick={() => nudge(1)} role="presentation"><span></span></div>
      </div>
    </div>
  </div>
</Dialog>

<style>
  .pairs {
    position: absolute;
    left: 0;
    right: 0;
    top: 80px;
  }
  .pair {
    display: flex;
    align-items: center;
    height: 48px;
    font-size: var(--fs-chrome);
  }
  .label {
    flex: 0 0 254px;
    padding-right: 11px;
    text-align: right;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .val {
    flex: 0 1 auto;
    min-width: 0;
    margin-right: 56px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Read-only field: sunken, panel-coloured, dim text inset 4px. */
  .val.well {
    flex: 0 0 520px;
    height: 48px;
    margin-left: 1px;
    display: flex;
    align-items: center;
    padding: 0 4px;
    color: var(--text-dim);
    border-top: 1px solid var(--inset);
    border-left: 1px solid var(--inset);
    border-bottom: 1px solid var(--bevel-light);
    border-right: 1px solid var(--bevel-light);
    user-select: text;
  }

  .ptable {
    position: absolute;
    left: 47px;
    top: 455px;
    width: 736px;
    height: 280px;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--inset);
    border-left: 1px solid var(--inset);
    border-bottom: 1px solid var(--bevel-light);
    border-right: 1px solid var(--bevel-light);
    background: var(--list-bg);
  }
  .phead {
    flex: 0 0 36px;
    display: flex;
    background: var(--panel-bg);
    border-bottom: 1px solid var(--inset);
    box-shadow: inset 0 1px 0 0 var(--bevel-light);
    font-size: var(--fs-row);
    color: var(--text-bright);
  }
  .phead .pc {
    display: flex;
    align-items: center;
    border-left: 1px solid var(--bevel-light);
    border-right: 1px solid var(--inset);
  }
  .phead button.pc {
    background: transparent;
    color: inherit;
    font: inherit;
    border-top: 0;
    border-bottom: 0;
    border-radius: 0;
    text-align: left;
    cursor: default;
  }
  .phead button.pc:focus-visible {
    outline: 2px solid var(--text-bright);
    outline-offset: -3px;
  }
  .caret {
    margin-left: 6px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .pc {
    padding-left: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pc.name {
    flex: 0 0 312px;
  }
  .pc.score {
    flex: 0 0 128px;
  }
  .pc.time {
    flex: 1 1 auto;
    min-width: 0;
  }
  .pgutter {
    flex: 0 0 37px;
    position: relative;
    background: var(--list-bg);
  }
  .pgutter .sbtn {
    top: 0;
    bottom: 0;
    border-bottom: none;
  }

  .pbody {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
  }
  .prows {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    padding-top: 5px;
    overflow-y: auto;
    scrollbar-width: none;
  }
  .prows::-webkit-scrollbar {
    display: none;
  }
  .prow {
    display: flex;
    align-items: center;
    height: 34px;
    font-size: var(--fs-chrome);
    color: var(--text);
  }
  .prow .pc.name {
    padding-left: 9px;
  }
  .prow .pc.score {
    padding-left: 12px;
  }
  .prow .pc.time {
    padding-left: 12px;
  }
  .prow.empty {
    padding-left: 9px;
    color: var(--text-dim);
  }

  .psbar {
    flex: 0 0 37px;
    display: flex;
    flex-direction: column;
  }
  .ptrack {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    width: 35px;
    background: var(--slider);
  }
  .pthumb {
    position: absolute;
    left: 0;
    width: 35px;
    background: var(--panel-bg);
    border-top: 1px solid var(--bevel-light);
    border-left: 1px solid var(--bevel-light);
    border-bottom: 1px solid var(--inset);
    border-right: 1px solid var(--inset);
  }
  .psbar .sbtn.down {
    position: relative;
    flex: 0 0 36px;
  }
</style>
