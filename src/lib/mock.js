/* mock.js — offline fixture + scripted event replay for the cs16browser UI.
 *
 * Loaded when `window.__TAURI__` is absent: it attaches one global,
 * `window.cs16Mock`, implementing the `Backend` interface declared in
 * `src/lib/cs16.ts`, so the frontend runs unchanged without a Rust build.
 *
 * Commands may answer synchronously — a fixture needs no I/O — and `cs16.ts`
 * normalises that at the boundary rather than making every caller await twice.
 * The event names and payload shapes mirror `src/lib/bindings.ts`, which
 * tauri-specta generates from the Rust DTOs.
 *
 * Everything here is deterministic — a seeded LCG drives every value, so the
 * fixture and the replay are identical on every load. Nothing calls
 * Math.random. No imports, no exports, no build step.
 */
(function () {
  "use strict";

  var root =
    typeof window !== "undefined" ? window : typeof globalThis !== "undefined" ? globalThis : this;

  /* ------------------------------------------------------------------ *
   * Seeded LCG — the single source of randomness in this file.
   * ------------------------------------------------------------------ */
  var seed = 0x2f6e2b1;
  function rnd() {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 4294967296;
  }
  function intBetween(lo, hi) {
    return lo + Math.floor(rnd() * (hi - lo + 1));
  }
  function pick(list) {
    return list[Math.floor(rnd() * list.length)];
  }

  /* A stable per-endpoint stream, so a row's player list never changes
   * between calls (and never perturbs the main stream). */
  function hash32(str) {
    var h = 2166136261;
    for (var i = 0; i < str.length; i++) {
      h ^= str.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return h >>> 0;
  }
  function seededRng(s0) {
    var s = s0 >>> 0 || 1;
    return function () {
      s = (s * 1664525 + 1013904223) >>> 0;
      return s / 4294967296;
    };
  }

  /* ------------------------------------------------------------------ *
   * Fixture vocabulary — real CS 1.6 server-list texture.
   * ------------------------------------------------------------------ */
  var HOSTNAMES = [
    "CS.IP-NET.RO # only_de.dust2",
    "Erdélyi Magyar D2 [NS/STEAM] | SynHosting.eu",
    "[PL] Serwer CS 1.6 tylko D2",
    "-=|RuS|=-> Public Dust2 24/7",
    "[GER] CSDM | Dust2 Only | FastDL",
    "#1 SniperWarz :: awp_map",
    "★ [FR] Only Mirage ★",
    "Zombie Plague 5.0 [SE]",
    "-=ES=- Servidor Publico #1",
    "CS-BG.NET [de_dust2 & cs_assault]",
    "|NORDIC| Dust2 24/7 HLstatsX",
    "-=DK=- CSDM 1.6",
    "[UA] UA-CS Public D2 #1",
    "CSManiaks.com | de_dust2 | HLTV",
    "^Turkish-Power^ de_dust2 24/7",
    "[CZ] OnlyDust2.cz #1 | Steam",
    "★ Sniper Elite ★ aim_map 24/7",
    "[NL] Dutch Only Dust2 | 128 tick",
    "Balkan-Force :: de_inferno ONLY",
    "[RO] Server Romanesc de_dust2 [NS]",
    "FragZone.eu | de_nuke | HLstatsX",
    "[SK] CS 1.6 ONLY D2 - VIP",
    "★ AWP LEGENDS ★ awp_map [STEAM]",
    "[IT] CS ITALIA - de_dust2 only",
    "DeathMatch.LT | cs_assault 24/7",
    "[HUN] Magyar Szerver | de_dust2",
    "RetroCS 1.6 - Public de_train",
    "[PT] Servidor Publico PT #2",
    "KGB-CS.RU :: de_aztec [NS]",
    "[SE] Nordic Dust2 - HLstatsX",
    "OldSchool CS 1.6 :: fy_iceworld",
    "[BR] CSBRASIL DUST2 24/7",
    "★ Zombie Escape ★ cs_office",
    "[FI] Finland Dust2 Only 24/7",
    "PlayHard.eu | de_dust2 | DeathMatch",
    "[GR] Greek Warriors :: de_cbble",
    "GunGame.NL | aim_map | 24/7",
    "[DE] CSDM Dust2 Only | FastDL",
    "CS1.6 Forever :: de_mirage",
    "[RU] -=Dust2-City=- Public",
    "CRAZY-CS.EU | de_dust2 #2",
    "[USA] Only Dust2 | East Coast",
    "★ ProTeam ★ de_cpl_mill",
    "[AT] Austria Dust2 24/7",
    "cs_assault FUN #1 [CSDM]",
    "de_italy.cz :: Only Italy | Steam",
    "[BY] Belarus CS 1.6 Public",
    "SteamPowered.eu | de_dust2",
    "★ CS 1.6 Classic ★ de_aztec",
    "[CH] Swiss Dust2 Only",
    "Public Server #4 | cs_office 24/7",
    "[BG] CS-BG DUST2 ONLY #2",
    "[LT] Baltic CS :: de_dust2",
    "HLDS.RO :: de_train [NS/STEAM]",
    "★ OnlyDust2 #7 ★ | HLstatsX",
    "[EE] Tallinn Public de_dust2",
    "NoBots-CS.de | de_dust2 | 30 slot",
    "[MX] Mexico CS 1.6 D2",
    "Veterans CS 1.6 :: de_inferno",
    "[KR] Asia Dust2 | Low Ping",
  ];

  var MAPS = [
    "de_dust2",
    "de_inferno",
    "de_aztec",
    "de_nuke",
    "cs_assault",
    "cs_office",
    "awp_map",
    "fy_iceworld",
    "de_train",
    "de_cbble",
    "cs_italy",
    "de_mirage",
    "de_cpl_mill",
    "aim_map",
  ];

  var GAME_ALTS = [
    { game: "Counter-Strike: Condition Zero", gamedir: "czero" },
    { game: "Half-Life", gamedir: "valve" },
    { game: "Team Fortress Classic", gamedir: "tfc" },
    { game: "Day of Defeat", gamedir: "dod" },
  ];

  var COUNTRIES = [
    "DE",
    "PL",
    "US",
    "RU",
    "BR",
    "SE",
    "NL",
    "FR",
    "CZ",
    "SK",
    "UA",
    "RO",
    "HU",
    "TR",
    "IT",
    "ES",
    "FI",
    "DK",
    "BG",
    "GR",
    "AT",
    "CH",
    "BE",
    "PT",
    "LT",
    "LV",
    "EE",
    "NO",
    "GB",
    "CA",
    "MX",
    "AR",
    "KR",
    "JP",
  ];

  var MAX_POOL = [16, 20, 22, 24, 26, 28, 32, 24, 32, 20];
  var PORTS = [27015, 27016, 27017, 27018, 27025];
  var IP_HEADS = [
    5, 31, 37, 46, 62, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 91, 93, 94, 95, 109, 176,
    178, 185, 188, 193, 194, 195, 212, 213,
  ];

  var FAKE_REASONS = [
    ["slots>32"],
    ["empty-hostname"],
    ["players>32"],
    ["server-farm"],
    ["foreign-ip"],
    ["name-churn"],
    ["slots>32", "server-farm"],
    ["players>32", "all-bots"],
    ["server-farm", "duplicate-name"],
    ["slots>32"],
  ];

  var BAN_REASONS = [
    ["slots>32"],
    ["foreign-ip"],
    ["slots>32", "players>32"],
    ["name-churn"],
    ["empty-map"],
  ];

  var NAME_WORDS = [
    "KabaL",
    "n00b",
    "SnipeR",
    "Ghost",
    "Viper",
    "Reaper",
    "Zeus",
    "K1LL3R",
    "Drake",
    "Frag",
    "Headshot",
    "Rambo",
    "Cobra",
    "Ninja",
    "Bandit",
    "Wolf",
    "Falcon",
    "Doom",
    "Blaze",
    "Spectre",
    "Phantom",
    "Titan",
    "Raptor",
    "Vandal",
    "Rogue",
    "Ace",
    "Joker",
    "Neo",
    "Tank",
    "Rush",
    "Camper",
    "NoScope",
    "Trigger",
    "Bullet",
    "Flash",
    "Smoke",
    "Deagle",
    "Colt",
  ];
  var CLAN_TAGS = [
    "xX_%s_Xx",
    "|%s|",
    "-=%s=-",
    "%s^",
    "[%s]",
    "%s_pl",
    "P|%s",
    "%s2000",
    "*%s*",
    "%s!",
  ];
  var ANON = ["Player", "PLAYER", "Guest", "anon", "cs player", "unnamed"];

  /* ------------------------------------------------------------------ *
   * Fixture: ~120 rows spanning every state the UI must render.
   * ------------------------------------------------------------------ */
  var SEGMENTS = [
    { from: 0, to: 59, live: true, outcome: "ok", banned: false, kind: "healthy" },
    { from: 60, to: 65, live: false, outcome: "listed", banned: false, kind: "plain" },
    { from: 66, to: 73, live: false, outcome: "timeout", banned: false, kind: "plain" },
    { from: 74, to: 76, live: false, outcome: "refused", banned: false, kind: "plain" },
    { from: 77, to: 78, live: false, outcome: "bad-header", banned: false, kind: "plain" },
    { from: 79, to: 80, live: false, outcome: "paced", banned: false, kind: "plain" },
    { from: 81, to: 85, live: false, outcome: "banned", banned: true, kind: "banned" },
    { from: 86, to: 95, live: true, outcome: "ok", banned: false, kind: "fake" },
    { from: 96, to: 119, live: true, outcome: "ok", banned: false, kind: "healthy" },
  ];

  function buildRoster(endpoint, count) {
    var rand = seededRng(hash32(endpoint));
    var list = [];
    for (var i = 0; i < count; i++) {
      var word = NAME_WORDS[Math.floor(rand() * NAME_WORDS.length)];
      var name =
        rand() < 0.14
          ? ANON[Math.floor(rand() * ANON.length)] + (100 + Math.floor(rand() * 900))
          : CLAN_TAGS[Math.floor(rand() * CLAN_TAGS.length)].replace("%s", word);
      list.push({
        index: i,
        name: name,
        score: Math.floor(rand() * 140),
        duration_seconds: 30 + Math.floor(rand() * 14400),
      });
    }
    return list;
  }

  function makeEndpoint(used) {
    for (;;) {
      var ep =
        pick(IP_HEADS) +
        "." +
        intBetween(1, 254) +
        "." +
        intBetween(1, 254) +
        "." +
        intBetween(1, 254) +
        ":" +
        pick(PORTS);
      if (!used.has(ep)) {
        used.add(ep);
        return ep;
      }
    }
  }

  function buildRow(i, seg, ctx, used) {
    var endpoint = makeEndpoint(used);

    var max = MAX_POOL[i % MAX_POOL.length];
    var players;
    if (i % 11 === 0) players = 0;
    else if (i % 7 === 0) players = max;
    else players = 1 + Math.floor(rnd() * (max - 1));

    var bots = 0;
    if (players > 0 && i % 3 === 0) bots = Math.min(players, 1 + Math.floor(rnd() * 4));

    var ping = null;
    if (seg.live) {
      if (i % 23 === 0) ping = 360 + intBetween(0, 220);
      else if (i % 9 === 0) ping = intBetween(5, 40);
      else ping = intBetween(12, 210);
    }

    var game = { game: "Counter-Strike", gamedir: "cstrike" };
    if (i % 17 === 0) game = GAME_ALTS[Math.floor(i / 17) % GAME_ALTS.length];

    var map = i % 5 !== 0 ? "de_dust2" : MAPS[1 + ((i * 7) % (MAPS.length - 1))];

    var os = i % 9 === 0 ? 0 : i % 37 === 3 ? 2 : 1;

    var reasons = [];
    if (seg.kind === "banned") reasons = BAN_REASONS[ctx.ban++ % BAN_REASONS.length];
    else if (seg.kind === "fake") reasons = FAKE_REASONS[ctx.fake++ % FAKE_REASONS.length];

    var soft = [];
    if (seg.live && ping > 350 && i % 23 === 0) soft.push("high-ping");
    if (seg.live && players > 16 && i % 19 === 0) soft.push("no-players-list");
    if (i % 37 === 0) soft.push("flapping");

    var base = HOSTNAMES[i % HOSTNAMES.length];
    var generation = Math.floor(i / HOSTNAMES.length);
    var hostname = generation > 0 ? base + " #" + (generation + 1) : base;
    if (i % 13 === 5) hostname += " [" + players + "/" + max + "]";
    if (reasons.indexOf("empty-name") >= 0) hostname = "";

    var row = {
      endpoint: endpoint,
      hostname: hostname,
      map: map,
      gamedir: game.gamedir,
      game: game.game,
      players: players,
      max_players: reasons.indexOf("player-count-faked") >= 0 ? 64 : max,
      bots: bots,
      // Some "humans" are a bot plugin reporting its bots as players.
      bot_plugin: seg.live && players > 0 && bots === 0 && i % 7 === 0 ? "YaPB 4.4.957" : null,
      ping_ms: ping,
      secure: i % 10 < 7,
      password: i % 8 === 3,
      os: os,
      live: seg.live,
      outcome: seg.outcome,
      banned: seg.banned,
      fake: reasons.length > 0,
      verified: seg.live && ping != null && reasons.length === 0,
      reasons: reasons.slice(),
      soft_reasons: soft,
      country: i % 29 === 0 ? null : COUNTRIES[(i * 5) % COUNTRIES.length],
      players_list: seg.live && players > 0 && players <= 16 ? buildRoster(endpoint, players) : [],
    };
    return row;
  }

  function buildRows() {
    var ctx = { fake: 0, ban: 0 };
    var used = new Set();
    var out = [];
    for (var s = 0; s < SEGMENTS.length; s++) {
      for (var i = SEGMENTS[s].from; i <= SEGMENTS[s].to; i++) {
        out.push(buildRow(i, SEGMENTS[s], ctx, used));
      }
    }
    return out;
  }

  var rows = buildRows();

  function rowCopy(row) {
    return {
      endpoint: row.endpoint,
      hostname: row.hostname,
      map: row.map,
      gamedir: row.gamedir,
      game: row.game,
      players: row.players,
      max_players: row.max_players,
      bots: row.bots,
      bot_plugin: row.bot_plugin,
      ping_ms: row.ping_ms,
      secure: row.secure,
      password: row.password,
      os: row.os,
      live: row.live,
      outcome: row.outcome,
      banned: row.banned,
      fake: row.fake,
      verified: row.verified,
      reasons: row.reasons.slice(),
      soft_reasons: row.soft_reasons.slice(),
      country: row.country,
      players_list: row.players_list.slice(),
    };
  }

  /* ------------------------------------------------------------------ *
   * Event bus + timer bookkeeping.
   * ------------------------------------------------------------------ */
  var handlers = [];

  function emit(event) {
    for (var i = 0; i < handlers.length; i++) {
      try {
        handlers[i](event);
      } catch {
        /* A broken listener must never stop the replay. */
      }
    }
  }

  var currentRun = null;
  var oneShots = [];

  function newRun() {
    return { live: true, timers: [] };
  }

  function after(run, ms, fn) {
    var id = setTimeout(function () {
      if (!run.live) return;
      fn();
    }, ms);
    run.timers.push(id);
    return id;
  }

  function every(run, ms, fn) {
    var id = setInterval(function () {
      if (!run.live) return;
      fn();
    }, ms);
    run.timers.push(id);
    return id;
  }

  function killRun(run) {
    if (!run) return;
    run.live = false;
    for (var i = 0; i < run.timers.length; i++) {
      clearTimeout(run.timers[i]);
      clearInterval(run.timers[i]);
    }
    run.timers.length = 0;
  }

  function killOneShots() {
    for (var i = 0; i < oneShots.length; i++) clearTimeout(oneShots[i]);
    oneShots = [];
  }

  function stopScan(emitDone = true) {
    var wasRunning = currentRun !== null;
    killRun(currentRun);
    currentRun = null;
    killOneShots();
    if (emitDone && wasRunning) emit({ kind: "phase", phase: "done" });
  }

  /* ------------------------------------------------------------------ *
   * Replay timing (~6s, one sweep of the fixture).
   * ------------------------------------------------------------------ */
  var ROW_STREAM_START_MS = 1400;
  var ROW_INTERVAL_MS = 40;
  var PROGRESS_EVERY = 10;
  var DONE_AT_MS = 5200;
  var SUMMARY_AT_MS = 5500;
  var RESOLVE_AT_MS = 6000;
  var RECHECKS = 2;
  var PACE_REASONS = [
    "rate limited, backing off 250ms",
    "master list throttled the burst, retrying",
    "A2S burst ceiling hit, pausing 180ms",
  ];

  function freshPing(row) {
    var base = row.ping_ms == null ? intBetween(12, 180) : row.ping_ms;
    var value = base + intBetween(-8, 8);
    if (value < 5) value = 5;
    if (value > 620) value = 620;
    return value;
  }

  function measure(row) {
    row.live = true;
    row.outcome = "ok";
    row.ping_ms = freshPing(row);
    row.verified = !row.fake;
  }

  function splitInto(list, parts) {
    var out = [];
    var size = Math.ceil(list.length / parts);
    for (var i = 0; i < list.length; i += size) out.push(list.slice(i, i + size));
    return out;
  }

  function buildSummary(paced) {
    var total = rows.length;
    var banned = 0;
    var hidden = 0;
    var live = 0;
    for (var i = 0; i < rows.length; i++) {
      var r = rows[i];
      if (r.banned) banned++;
      else if (r.fake) hidden++;
      if (r.live) live++;
    }
    return {
      shown: total - banned - hidden,
      banned: banned,
      hidden: hidden,
      paced: paced,
      listed: total - live,
      measured: live,
      rechecks: RECHECKS,
      total_before: total,
    };
  }

  function startScan(cfg) {
    /* `cfg` (ScanConfig) is accepted and ignored: the replay always sweeps the
     * whole fixture so the reviewer sees every state on one pass. */
    void cfg;
    stopScan(false);

    var run = newRun();
    currentRun = run;

    var total = rows.length;
    var pending = rows.filter(function (r) {
      return !r.live;
    });
    var measurable = rows.filter(function (r) {
      return r.live;
    });

    var paced = 0;

    emit({ kind: "phase", phase: "fetching" });
    after(run, 400, function () {
      emit({ kind: "phase", phase: "scanning" });
    });

    /* The cached listing lands in stages, so the table visibly fills in. */
    var chunks = splitInto(pending, 3);
    for (var c = 0; c < chunks.length; c++) {
      (function (chunk, index) {
        after(run, 600 + index * 300, function () {
          emit({ kind: "listed", rows: chunk.map(rowCopy) });
        });
      })(chunks[c], c);
    }

    /* Pacing notices land mid-sweep, exactly where the real scanner backs off. */
    var paceAt = [2400, 3300, 4100];
    for (var p = 0; p < paceAt.length; p++) {
      (function (ms, index) {
        after(run, ms, function () {
          var target = measurable[(index * 13 + 5) % measurable.length];
          paced++;
          emit({
            kind: "paced",
            endpoint: target.endpoint,
            reason: PACE_REASONS[index % PACE_REASONS.length],
          });
        });
      })(paceAt[p], p);
    }

    /* One `row` per measured server, with a `progress` tick every ten. */
    after(run, ROW_STREAM_START_MS, function () {
      var done = 0;
      var timer = every(run, ROW_INTERVAL_MS, function () {
        if (done >= measurable.length) {
          clearInterval(timer);
          return;
        }
        var row = measurable[done];
        done++;
        measure(row);
        emit({ kind: "row", done: done, total: total, row: rowCopy(row) });
        if (done % PROGRESS_EVERY === 0) {
          emit({ kind: "progress", done: done, total: total });
        }
      });
    });

    after(run, 4600, function () {
      emit({
        kind: "error",
        message: "recheck pass 1: 3 endpoints stopped answering (kept as <pending>)",
      });
    });

    after(run, DONE_AT_MS, function () {
      emit({ kind: "phase", phase: "done" });
    });

    after(run, SUMMARY_AT_MS, function () {
      emit({ kind: "summary", summary: buildSummary(paced) });
    });

    return new Promise(function (resolve) {
      after(run, RESOLVE_AT_MS, function () {
        resolve(undefined);
      });
    });
  }

  /* ------------------------------------------------------------------ *
   * Command surface — the shape of the Tauri commands, in JS.
   * ------------------------------------------------------------------ */
  function refreshServer(endpoint) {
    return new Promise(function (resolve) {
      var id = setTimeout(function () {
        oneShots = oneShots.filter(function (t) {
          return t !== id;
        });
        var row = null;
        for (var i = 0; i < rows.length; i++) {
          if (rows[i].endpoint === endpoint) row = rows[i];
        }
        if (!row) {
          resolve(null);
          return;
        }
        measure(row);
        row.players_list = buildRoster(row.endpoint, Math.min(32, row.players));
        emit({ kind: "refreshed", row: rowCopy(row) });
        resolve(rowCopy(row));
      }, 300);
      oneShots.push(id);
    });
  }

  function rescanAll() {
    return startScan({});
  }

  function serverPlayers(endpoint) {
    return new Promise(function (resolve, reject) {
      var id = setTimeout(function () {
        oneShots = oneShots.filter(function (t) {
          return t !== id;
        });
        var row = null;
        for (var i = 0; i < rows.length; i++) {
          if (rows[i].endpoint === endpoint) row = rows[i];
        }
        if (!row) {
          reject(new Error("unknown server"));
          return;
        }
        row.players_list = buildRoster(row.endpoint, Math.min(32, row.players));
        resolve(row.players_list.slice());
      }, 150);
      oneShots.push(id);
    });
  }

  function clearBans() {
    var cleared = 0;
    for (var i = 0; i < rows.length; i++) {
      var r = rows[i];
      if (!r.banned) continue;
      r.banned = false;
      /* A dropped ban is no longer an answer either — it goes back to pending. */
      if (r.outcome === "banned") {
        r.outcome = "listed";
        r.live = false;
        r.ping_ms = null;
      }
      cleared++;
    }
    return cleared;
  }

  var whitelist = new Set();
  function banServer(endpoint, hostname) {
    whitelist.delete(endpoint);
    var row = rows.find(function (r) {
      return r.endpoint === endpoint;
    });
    if (!row) throw new Error("unknown server");
    row.hostname = hostname || row.hostname;
    row.banned = true;
    row.fake = true;
    row.verified = false;
    row.outcome = "banned";
    row.reasons = ["manual"];
  }

  function whitelistServer(endpoint) {
    whitelist.add(endpoint);
    var row = rows.find(function (r) {
      return r.endpoint === endpoint;
    });
    if (!row) throw new Error("unknown server");
    row.banned = false;
    row.fake = false;
    row.reasons = [];
    if (row.outcome === "banned") row.outcome = "paced";
    row.verified = row.ping_ms !== null;
  }

  function cachedRows() {
    return rows.map(rowCopy);
  }

  /* The persisted ban list: every row this mock has ever marked banned,
   * mirroring the real backend's `bans` command (which reads the ban list on
   * disk, not just what the current listing happens to include). */
  function bans() {
    return rows
      .filter(function (r) {
        return r.banned;
      })
      .map(function (r) {
        return {
          endpoint: r.endpoint,
          hostname: r.hostname,
          banned_at: "2024-01-01 00:00",
          reasons: r.reasons || [],
          rechecks: 0,
        };
      });
  }

  function serverDetail(endpoint) {
    for (var i = 0; i < rows.length; i++) {
      if (rows[i].endpoint !== endpoint) continue;
      var row = rowCopy(rows[i]);
      var count = row.players > 32 ? 32 : row.players;
      row.players_list = count > 0 ? buildRoster(row.endpoint, count) : [];
      return row;
    }
    return null;
  }

  function connect(endpoint) {
    return new Promise(function (resolve) {
      var id = setTimeout(function () {
        oneShots = oneShots.filter(function (t) {
          return t !== id;
        });
        resolve("steam://connect/" + endpoint + "/");
      }, 250);
      oneShots.push(id);
    });
  }

  /* Settings: the key itself never comes back, only a masked hint. */
  var savedKey = null;
  var debugLogs = false;
  function settingsView() {
    return {
      api_key_hint: savedKey ? "••••••••" + savedKey.slice(-4) : null,
      api_key_source: savedKey ? "settings" : "none",
      api_key_url: "https://steamcommunity.com/dev/apikey",
      debug_logs: debugLogs,
    };
  }
  function getSettings() {
    return settingsView();
  }
  function setApiKey(key) {
    var k = String(key).trim();
    if (k && !/^[0-9a-fA-F]{32}$/.test(k)) {
      return Promise.reject(new Error("A Steam Web API key is 32 characters of 0-9 and A-F."));
    }
    savedKey = k ? k.toUpperCase() : null;
    return settingsView();
  }
  function setDebugLogs(enabled) {
    debugLogs = !!enabled;
    return settingsView();
  }

  function onEvent(fn) {
    if (typeof fn === "function") handlers.push(fn);
    return function off() {
      handlers = handlers.filter(function (h) {
        return h !== fn;
      });
    };
  }

  root.cs16Mock = {
    rows: rows,
    startScan: startScan,
    stopScan: stopScan,
    refreshServer: refreshServer,
    rescanAll: rescanAll,
    serverPlayers: serverPlayers,
    clearBans: clearBans,
    banServer: banServer,
    whitelistServer: whitelistServer,
    bans: bans,
    cachedRows: cachedRows,
    serverDetail: serverDetail,
    connect: connect,
    getSettings: getSettings,
    setApiKey: setApiKey,
    setDebugLogs: setDebugLogs,
    openApiKeyPage: function () {
      window.open("https://steamcommunity.com/dev/apikey", "_blank", "noopener");
    },
    openRepositoryPage: function () {
      window.open("https://github.com/vuxsoftware/cs16browser", "_blank", "noopener");
    },
    onEvent: onEvent,
  };
})();
