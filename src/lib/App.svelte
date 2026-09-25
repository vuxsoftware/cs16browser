<script lang="ts">
  /* App.svelte — orchestration only: the scan event pump, the keyboard map and
   * the layout. Every visual region is a component under `lib/`.
   *
   * Keyboard: the arrows, Page Up/Down and Home/End move the selection, Enter
   * connects, Escape closes whatever is open.
   */
  import { availableUpdate, cs16, installUpdate, nativeWindow } from "$lib/cs16";
  import { version } from "../../package.json";
  import type { Event as ScanEvent, ScanConfig, ServerRow } from "$lib/bindings";
  import * as store from "$lib/store.svelte";
  import { TABS, EMPTY_TAB, ROW_H, COLUMNS, BANNED_COLUMNS } from "$lib/columns";
  import TitleBar from "$lib/TitleBar.svelte";
  import TabStrip from "$lib/TabStrip.svelte";
  import Header from "$lib/Header.svelte";
  import ServerList from "$lib/ServerList.svelte";
  import FilterPanel from "$lib/FilterPanel.svelte";
  import ButtonBar from "$lib/ButtonBar.svelte";
  import ContextMenu from "$lib/ContextMenu.svelte";
  import GameInfo from "$lib/GameInfo.svelte";
  import Menu from "$lib/Menu.svelte";
  import Modal from "$lib/Modal.svelte";
  import Settings from "$lib/Settings.svelte";
  import About from "$lib/About.svelte";
  import { autoUpdateEnabled } from "$lib/updatePreference";
  import { onMount, tick } from "svelte";

  let flashTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    /* The native window starts hidden. Wait for Svelte to finish mounting the
     * chrome before revealing it; timers still run while the window is hidden,
     * unlike animation frames on some webviews. */
    let mounted = true;
    void tick().then(() => {
      setTimeout(() => {
        if (mounted) nativeWindow.show();
      }, 0);
    });
    return () => {
      mounted = false;
    };
  });

  const ui = store.ui;

  /* Right-click row menu: position, armed item, and its anchor row. */
  interface CtxState {
    left: number;
    top: number;
    armed: number;
    endpoint: string;
    tab: number;
  }
  let ctx = $state<CtxState | null>(null);

  /* The title bar's gear. */
  let settingsOpen = $state(false);
  let aboutOpen = $state(false);
  let quickRefreshing = $state(false);
  let quickStop = false;
  let stopping = $state(false);

  /* The status line under the frame. Sticky error: an error explains why the
   * list is empty, so it outlives the transient phase messages and is cleared
   * only when the next scan starts. (The filter summary lives in the button
   * row, as in the reference.) */
  const status = $derived(
    ui.error ??
      ui.flash ??
      (ui.phase === "fetching"
        ? "Getting new server list..."
        : ui.phase === "scanning"
          ? "Refreshing server list..."
          : ""),
  );

  const columns = $derived(TABS[ui.tab] === "Banned" ? BANNED_COLUMNS : COLUMNS);

  const emptyMessage = $derived(
    ui.tab !== 0
      ? (EMPTY_TAB[TABS[ui.tab] ?? ""] ??
          "There are no internet games visible that pass your filter settings.")
      : ui.phase === "fetching" || ui.phase === "scanning"
        ? "Getting new server list..."
        : "No internet games responded to the query.",
  );

  function flash(message: string): void {
    ui.flash = message;
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => {
      ui.flash = null;
    }, 6000);
  }

  /** Fetch the same broad list regardless of the controls. Every filter in the
   * panel acts on rows already held by the UI, including after a restart. */
  function scanConfig(rediscover: boolean): ScanConfig {
    ui.error = null;
    return {
      /* Always Counter-Strike; the Game field is locked (see columns.ts). */
      game: "cstrike",
      rediscover,
      fetchAll: true,
      filters: {
        secure: false,
        noFull: false,
        noEmpty: false,
        noPassword: false,
      },
    };
  }

  async function rescan(rediscover: boolean): Promise<void> {
    store.reset();
    try {
      await cs16.startScan(scanConfig(rediscover));
    } catch (err) {
      ui.error = String(err);
    }
  }

  /** Dropdown picks change only the visible order. */
  function pickDiscovery(id: keyof typeof ui.discovery, pick: string | undefined): void {
    ui.menu = null;
    if (pick === undefined) return;
    ui.discovery[id] = pick;
    store.recompute();
  }

  function connectSelected(): void {
    const row = store.selectedRow();
    if (!row) return;
    connectRow(row.endpoint);
  }

  function connectRow(endpoint: string): void {
    const row = ui.rows[ui.index.get(endpoint) ?? -1];
    if (!row) return;
    if (row.banned || row.fake) {
      ui.modal = {
        title: "Connect",
        body: row.banned
          ? "This server is banned and is not re-queried. Clear bans first if you trust it."
          : "This server was flagged by the scan and is hidden from Internet.",
        buttons: [{ label: "OK", accent: true }],
      };
      return;
    }
    store.recordVisit(endpoint);
    cs16.connect(endpoint).then(
      (url) => flash(`joining via Steam: ${url}`),
      (err: unknown) => {
        ui.modal = {
          title: "Connect",
          body: String(err),
          buttons: [{ label: "OK", accent: true }],
        };
      },
    );
  }

  async function quickRefresh(): Promise<void> {
    if (quickRefreshing) return;
    const endpoints = ui.order.map((index) => ui.rows[index].endpoint);
    if (!endpoints.length) return;
    quickRefreshing = true;
    quickStop = false;
    stopping = false;
    let next = 0;
    let refreshed = 0;
    let paced = 0;
    flash(`Refreshing ${endpoints.length} visible servers...`);
    try {
      await Promise.all(
        Array.from({ length: Math.min(16, endpoints.length) }, async () => {
          while (!quickStop && next < endpoints.length) {
            const endpoint = endpoints[next++];
            try {
              await cs16.refreshServer(endpoint, false);
              refreshed++;
            } catch {
              paced++;
            }
          }
        }),
      );
      flash(
        quickStop
          ? `Quick refresh stopped: ${refreshed} queried, ${paced} skipped or failed.`
          : `Quick refresh: ${refreshed} queried, ${paced} skipped or failed.`,
      );
    } finally {
      quickRefreshing = false;
      stopping = false;
    }
  }

  function stopRefresh(): void {
    if (stopping) return;
    stopping = true;
    flash("Stopping refresh...");
    if (quickRefreshing) quickStop = true;
    if (ui.phase === "fetching" || ui.phase === "scanning") {
      void cs16.stopScan().catch((err: unknown) => {
        stopping = false;
        ui.error = `Could not stop refresh: ${String(err)}`;
      });
    }
  }

  function clearBans(): void {
    void cs16.clearBans().then((n) => {
      /* Bans are a hard filter, so the list must be recomputed. */
      for (const row of ui.rows) {
        if (!row.banned) continue;
        row.banned = false;
        if (row.outcome === "banned") {
          row.outcome = "listed";
          row.live = false;
          row.ping_ms = null;
          row.fake = false;
          row.reasons = [];
        }
      }
      flash(`cleared ${n} ban(s)`);
      store.recompute();
    });
  }

  /* ------------------------------------------------------- context menu */
  function rowMenu(event: MouseEvent, endpoint: string): void {
    event.preventDefault();
    const host = (event.currentTarget as HTMLElement).closest(".app")?.getBoundingClientRect();
    if (!host) return;
    ctx = {
      left: event.clientX - host.left,
      top: event.clientY - host.top,
      armed: 0,
      endpoint,
      tab: ui.tab,
    };
  }

  function ctxItems(): { label: string; disabled?: boolean }[] {
    const row = ctx ? ui.rows[ui.index.get(ctx.endpoint) ?? -1] : null;
    const bannedTab = ctx?.tab === 3;
    return [
      ...(!bannedTab
        ? [{ label: "Connect to server", disabled: !row || row.banned || row.fake }]
        : []),
      { label: "View server info", disabled: !row },
      { label: "Refresh server", disabled: !row || row.banned },
      {
        label:
          row && store.favorites.has(row.endpoint) ? "Remove from favorites" : "Add to favorites",
        disabled: !row,
      },
      bannedTab
        ? { label: "Add to whitelist", disabled: !row }
        : { label: "Add to banned", disabled: !row },
    ];
  }

  function ctxPick(i: number): void {
    if (!ctx) return;
    const endpoint = ctx.endpoint;
    const action = ctxItems()[i]?.label;
    ctx = null;
    if (action === "Connect to server") connectRow(endpoint);
    else if (action === "View server info") void showInfo(endpoint);
    else if (action === "Refresh server") {
      cs16.refreshServer(endpoint, true).catch((err: unknown) => flash(`paced: ${String(err)}`));
    } else if (action === "Add to favorites" || action === "Remove from favorites") {
      store.toggleFavorite(endpoint);
    } else if (action === "Add to banned") {
      const row = ui.rows[ui.index.get(endpoint) ?? -1];
      if (!row) return;
      void cs16.banServer(endpoint, row.hostname).then(
        () => {
          row.banned = true;
          row.fake = true;
          row.verified = false;
          row.outcome = "banned";
          row.reasons = ["manual"];
          store.recompute();
          flash("Server added to banned.");
        },
        (err: unknown) => {
          ui.error = String(err);
        },
      );
    } else if (action === "Add to whitelist") {
      const row = ui.rows[ui.index.get(endpoint) ?? -1];
      if (!row) return;
      void cs16.whitelistServer(endpoint).then(
        () => {
          row.banned = false;
          row.fake = false;
          row.reasons = [];
          if (row.outcome === "banned") row.outcome = "paced";
          row.verified = row.ping_ms !== null;
          store.recompute();
          flash("Server added to whitelist.");
        },
        (err: unknown) => {
          ui.error = String(err);
        },
      );
    }
  }

  /* ------------------------------------------------------- game info */
  let infoRow = $state<string | null>(null);
  let infoServerRow = $state<ServerRow | null>(null);
  let playerListLoading = $state(false);
  let playerListError = $state<string | null>(null);
  let infoRequest = 0;
  const detailRequests = new Map<
    string,
    { request: number; timer?: ReturnType<typeof setTimeout> }[]
  >();

  function removeDetailRequest(endpoint: string, request: number): void {
    const waiting = detailRequests.get(endpoint);
    if (!waiting) return;
    const index = waiting.findIndex((entry) => entry.request === request);
    if (index < 0) return;
    clearTimeout(waiting[index].timer);
    waiting.splice(index, 1);
    if (!waiting.length) detailRequests.delete(endpoint);
  }

  /** Open (or refresh) Game Info. Opening shows the row as the sweep measured
   * it and asks only for the player list, which the sweep never fetches; the
   * dialog's Refresh, or a row that was never measured, re-queries the server. */
  async function showInfo(endpoint: string, requery = false): Promise<void> {
    const request = ++infoRequest;
    if (infoRow !== endpoint) {
      const listed = ui.rows[ui.index.get(endpoint) ?? -1];
      if (!listed) return;
      infoServerRow = { ...listed, players_list: [...listed.players_list] };
    }
    infoRow = endpoint;
    playerListLoading = true;
    playerListError = null;
    if (!requery && infoServerRow?.live) {
      await showPlayers(endpoint, request);
      return;
    }
    const waiting = detailRequests.get(endpoint) ?? [];
    waiting.push({ request });
    detailRequests.set(endpoint, waiting);
    try {
      await cs16.refreshServer(endpoint, true);
      /* Command completion and event delivery use separate IPC paths. Keep the
       * request until its event arrives, including if the dialog was closed. */
      const entry = detailRequests.get(endpoint)?.find((item) => item.request === request);
      if (entry)
        entry.timer = setTimeout(() => {
          removeDetailRequest(endpoint, request);
          if (request === infoRequest && infoRow === endpoint) {
            playerListLoading = false;
            playerListError = "Player list unavailable: refresh result did not arrive.";
          }
        }, 30_000);
    } catch (err) {
      removeDetailRequest(endpoint, request);
      if (request !== infoRequest) return;
      if (retryWhenPaced(err, () => void showInfo(endpoint, requery), request, endpoint)) return;
      playerListLoading = false;
      playerListError = `Player list unavailable: ${String(err)}`;
    }
  }

  async function showPlayers(endpoint: string, request: number): Promise<void> {
    try {
      const players = await cs16.serverPlayers(endpoint);
      if (request !== infoRequest || infoRow !== endpoint || !infoServerRow) return;
      infoServerRow = { ...infoServerRow, players_list: players };
      playerListLoading = false;
    } catch (err) {
      if (request !== infoRequest) return;
      if (retryWhenPaced(err, () => void showInfo(endpoint), request, endpoint)) return;
      playerListLoading = false;
      playerListError = `Player list unavailable: ${String(err)}`;
    }
  }

  /** Queried moments ago (by the sweep or a previous click): nothing was sent.
   * Keep showing "Loading" and ask again once the gap has passed, rather than
   * leaving the dialog with no visible effect. */
  function retryWhenPaced(
    err: unknown,
    again: () => void,
    request: number,
    endpoint: string,
  ): boolean {
    const wait = /recently queried; retry in (\d+) s/.exec(String(err));
    if (!wait) return false;
    setTimeout(
      () => {
        if (request === infoRequest && infoRow === endpoint) again();
      },
      Number(wait[1]) * 1000,
    );
    return true;
  }

  /* ------------------------------------------------------------- events */
  let pendingFrame = false;

  /** Coalesce the event storm into one recompute per frame: a 20k sweep emits
   * ~20k events, and recomputing per event is O(n² log n). */
  function schedule(): void {
    if (pendingFrame) return;
    pendingFrame = true;
    requestAnimationFrame(() => {
      pendingFrame = false;
      store.recompute();
    });
  }

  function onEvent(event: ScanEvent): void {
    switch (event.kind) {
      case "phase":
        ui.phase = event.phase;
        if (event.phase === "done" && !quickRefreshing) stopping = false;
        break;
      case "listed":
        for (const row of event.rows) store.upsert(row);
        schedule();
        break;
      case "row":
        store.upsert(event.row);
        schedule();
        break;
      case "refreshed": {
        const waiting = detailRequests.get(event.row.endpoint);
        const detail = waiting?.[0];
        if (detail) {
          removeDetailRequest(event.row.endpoint, detail.request);
          if (infoRow === event.row.endpoint) {
            infoServerRow = event.row;
            playerListLoading = false;
            playerListError = null;
          }
          break;
        }
        store.upsert(event.row);
        schedule();
        break;
      }
      case "paced":
        /* A refusal sent no packet, so it is not a timeout — it is a notice. */
        if (!ui.error) flash(`paced: ${event.endpoint} (${event.reason})`);
        break;
      case "summary":
        ui.summary = event.summary;
        break;
      case "error":
        ui.error = event.message;
        break;
    }
  }

  function keydown(event: KeyboardEvent): void {
    if (ctx) {
      const n = 4;
      if (event.key === "ArrowDown") {
        ctx.armed = (ctx.armed + 1) % n;
        event.preventDefault();
        return;
      }
      if (event.key === "ArrowUp") {
        ctx.armed = (ctx.armed + n - 1) % n;
        event.preventDefault();
        return;
      }
      if (event.key === "Enter") {
        const a = ctx.armed;
        ctxPick(a);
        event.preventDefault();
        return;
      }
      if (event.key === "Escape") {
        ctx = null;
        event.preventDefault();
        return;
      }
    }
    const menu = ui.menu;
    if (menu) {
      const n = menu.items.length;
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        menu.armed = (menu.armed + (event.key === "ArrowDown" ? 1 : n - 1)) % n;
        event.preventDefault();
        return;
      }
      if (event.key === "Enter") {
        pickDiscovery(menu.id, menu.items[menu.armed]);
        event.preventDefault();
        return;
      }
      if (event.key === "Escape") {
        ui.menu = null;
        event.preventDefault();
        return;
      }
    }
    if (settingsOpen) {
      if (event.key === "Escape") {
        settingsOpen = false;
        event.preventDefault();
      }
      return;
    }
    if (ui.modal || infoRow) {
      if (event.key === "Escape") {
        ui.modal = null;
        infoRow = null;
        infoServerRow = null;
        infoRequest++;
        event.preventDefault();
      }
      return;
    }

    const page = Math.floor((store.dom.list?.clientHeight ?? 600) / ROW_H);
    switch (event.key) {
      case "ArrowDown":
        store.moveSelection(1);
        event.preventDefault();
        break;
      case "ArrowUp":
        store.moveSelection(-1);
        event.preventDefault();
        break;
      case "PageDown":
        store.moveSelection(page);
        event.preventDefault();
        break;
      case "PageUp":
        store.moveSelection(-page);
        event.preventDefault();
        break;
      case "Home":
        ui.selected = 0;
        store.ensureVisible(0);
        event.preventDefault();
        break;
      case "End":
        ui.selected = ui.order.length - 1;
        store.ensureVisible(ui.selected);
        event.preventDefault();
        break;
      case "Enter":
        connectSelected();
        event.preventDefault();
        break;
      case "Escape":
        ctx = null;
        ui.menu = null;
        ui.modal = null;
        break;
      default:
        break;
    }
  }

  $effect(() => {
    /* A denied capability would otherwise leave the window empty and silent, so
     * a listener that cannot be registered is reported like any other failure. */
    const off = cs16.onEvent(onEvent, (message) => {
      ui.error = message;
    });
    /* No key, no servers: take the user straight to where the key goes. */
    cs16.getSettings().then(
      (view) => {
        if (view.api_key_source === "none") settingsOpen = true;
      },
      () => undefined,
    );
    if (autoUpdateEnabled())
      void availableUpdate()
        .then((update) => {
          if (!update) return;
          ui.modal = {
            title: "Update available",
            body: `Version ${update.version} is ready. Install the signed update and restart now?`,
            buttons: [
              { label: "Later" },
              {
                label: "Install",
                accent: true,
                onClick: () => {
                  flash("Downloading update...");
                  void installUpdate(update).catch((err: unknown) => {
                    ui.error = `update failed: ${String(err)}`;
                  });
                },
              },
            ],
          };
        })
        .catch((err: unknown) => {
          // An unavailable release feed must not prevent the server browser opening.
          console.warn("update check failed", err);
        });
    /* The Banned tab must show every server ever banned, not just the ones
     * this sweep's listing happens to still include (a server can be delisted
     * by Steam, or banned in an earlier session, and stay banned). Fetched
     * independently of the scan pipeline below. */
    cs16
      .bans()
      .then((rows) => store.mergeBans(rows))
      .catch(() => undefined);

    /* Subscribe **before** the command that starts emitting: the scan streams
     * rows as it goes, and a listener attached afterwards would miss the first
     * round (and, on a warm cache, every row). */
    cs16
      .cachedRows()
      .then((rows) => {
        if (rows.length) {
          for (const row of rows) store.upsert(row);
          store.recompute();
        }
        return cs16.startScan(scanConfig(false));
      })
      .catch((err: unknown) => {
        /* The page mounted while a sweep was already running (a reload, or a
         * dev hot-update): that sweep keeps streaming to the listener above,
         * so this is attaching to it, not a failure. */
        if (err instanceof Error && err.message === "a scan is already running") {
          ui.phase = "scanning";
          return;
        }
        ui.error = `error: ${String(err)}`;
      });
    return off;
  });

  /* The seam the visual/scale harness uses: it pushes synthetic backend events
   * into the real render path (e.g. a 20 000-row sweep) without a Rust build.
   * It exposes no state that changes behaviour. */
  $effect(() => {
    window.cs16Debug = { feed: onEvent, state: ui, store };
  });
</script>

<svelte:window onkeydown={keydown} />
<svelte:head><title>cs16browser v{version}</title></svelte:head>

<div
  class="app"
  class:collapsed={!ui.filtersOpen}
  onclick={() => (ui.menu = null)}
  role="presentation"
>
  <!-- The close box quits the app. -->
  <TitleBar
    caption={`cs16browser v${version}`}
    onclose={() => nativeWindow.close()}
    onminimize={() => nativeWindow.minimize()}
    onmaximize={() => nativeWindow.toggleMaximize()}
    onsettings={() => {
      ctx = null;
      ui.menu = null;
      settingsOpen = true;
    }}
    onabout={() => {
      ctx = null;
      ui.menu = null;
      aboutOpen = true;
    }}
  />
  <TabStrip />
  <div class="frame"></div>

  <div class="listwrap">
    <Header {columns} />
    <div class="body">
      <ServerList {columns} {emptyMessage} onactivate={connectSelected} onmenu={rowMenu} />
    </div>
  </div>

  <div class="bottom" onclick={(e) => e.stopPropagation()} role="presentation">
    {#if ui.filtersOpen}
      <FilterPanel />
    {/if}
    <ButtonBar
      summary={store.statusLine()}
      onclearbans={clearBans}
      onquick={() => void quickRefresh()}
      {quickRefreshing}
      onrefresh={() => void rescan(true)}
      onstop={stopRefresh}
      {stopping}
      onconnect={connectSelected}
    />
  </div>

  <div class="bottomstrip" class:is-error={!!ui.error} title={status}>
    <span>{status}</span>
    <span class="disclaimer">Unofficial. Not affiliated with or endorsed by Valve Corporation.</span
    >
  </div>
  <div
    class="resize-hotspot"
    role="presentation"
    onpointerdown={() => nativeWindow.startResize()}
  ></div>

  {#if ctx}
    <ContextMenu
      left={ctx.left}
      top={ctx.top}
      items={ctxItems()}
      armed={ctx.armed}
      onarm={(i) => {
        if (ctx) ctx.armed = i;
      }}
      onpick={ctxPick}
      ondismiss={() => (ctx = null)}
    />
  {/if}

  {#if ui.menu}
    <Menu
      items={ui.menu.items}
      armed={ui.menu.armed}
      left={ui.menu.left}
      top={ui.menu.top}
      width={ui.menu.width}
      onarm={(i) => {
        if (ui.menu) ui.menu.armed = i;
      }}
      onpick={(i) => {
        const menu = ui.menu;
        if (!menu) return;
        pickDiscovery(menu.id, menu.items[i]);
      }}
    />
  {/if}

  {#if aboutOpen}
    <About onclose={() => (aboutOpen = false)} />
  {:else if settingsOpen}
    <Settings
      onclose={() => (settingsOpen = false)}
      onsaved={(view) => {
        /* A sweep that failed for want of a key can run now. */
        if (ui.error && view.api_key_source !== "none") void rescan(false);
      }}
    />
  {:else if infoServerRow}
    <GameInfo
      row={infoServerRow}
      latencyMs={infoServerRow.ping_ms ?? 0}
      {playerListLoading}
      {playerListError}
      onjoin={() => {
        if (infoRow) connectRow(infoRow);
      }}
      onrefresh={() => {
        if (infoRow) void showInfo(infoRow, true);
      }}
      onclose={() => {
        infoRequest++;
        infoRow = null;
        infoServerRow = null;
      }}
    />
  {:else if ui.modal}
    <Modal title={ui.modal.title} buttons={ui.modal.buttons} onclose={() => (ui.modal = null)}>
      <div class="mtext">{ui.modal.body}</div>
    </Modal>
  {/if}
</div>

<style>
  /* Below the frame; text centred on y1283. */
  .bottomstrip {
    position: absolute;
    left: 21px;
    right: 21px;
    bottom: 11px;
    height: 44px;
    display: flex;
    align-items: center;
    font-size: var(--fs-chrome);
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    justify-content: space-between;
    gap: 24px;
  }
  .bottomstrip > span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .disclaimer {
    flex: 0 0 auto;
    font-size: 16px;
    color: var(--text-dim);
  }
  .resize-hotspot {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 14px;
    height: 14px;
    cursor: nwse-resize;
  }
  .bottomstrip.is-error {
    color: var(--accent);
  }
</style>
