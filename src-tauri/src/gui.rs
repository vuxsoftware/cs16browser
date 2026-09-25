//! Tauri command surface and event bridge.
//!
//! Thin on purpose: every rule lives in [`crate::app`]. This module only
//! translates between the frontend's calls and that pipeline, and pushes
//! [`app::Event`]s to the webview.
//!
//! ## Why `emit` rather than a channel
//!
//! `Scanner::scan_with` is callback-driven by design (`scanner.rs` documents the
//! bounded-channel deadlock that motivated it). `AppHandle::emit` is
//! thread-safe and non-blocking, so it is a drop-in sink: the scan task never
//! waits on the UI, and a slow webview cannot stall a 20k-endpoint sweep.

use crate::app::{self, App, BanRow, Config, Event, Filters, PlayerRow, ServerRow};
use crate::client::webapi;
use crate::model::{Endpoint, Game};
use crate::settings::{self, Settings};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};
use tauri_specta::{collect_commands, collect_events, Builder, Event as _};

/// The generated bindings the Svelte frontend imports. Exported at startup
/// (debug and release alike): a Rust change to a DTO becomes a TypeScript
/// error on the next run rather than a silent `undefined`.
pub const BINDINGS_PATH: &str = "../src/lib/bindings.ts";

/// Shared state managed by Tauri.
pub struct Shared {
    app: Arc<App>,
    running: Arc<std::sync::atomic::AtomicBool>,
    logging: crate::logging::LoggingHandle,
}

impl Shared {
    fn new(app: App, logging: crate::logging::LoggingHandle) -> Self {
        Self {
            app: Arc::new(app),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            logging,
        }
    }

    fn app(&self) -> Arc<App> {
        Arc::clone(&self.app)
    }
}

/// Frontend's scan request. Field names are `camelCase` on the JS side.
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanConfig {
    /// `"cstrike" | "czero" | "valve" | …` (see `Game::parse`).
    #[serde(default)]
    pub game: Option<String>,
    #[serde(default)]
    pub fetch_rounds: Option<u32>,
    #[serde(default)]
    pub fetch_all: Option<bool>,
    #[serde(default)]
    pub concurrency: Option<u32>,
    #[serde(default)]
    pub query_timeout_ms: Option<u32>,
    /// Force a fresh list from Steam rather than the local cache.
    #[serde(default)]
    pub rediscover: Option<bool>,
    /// Discovery filters — the four the in-game dialog sends on the wire.
    #[serde(default)]
    pub filters: Option<Filters>,
    #[serde(default)]
    pub max_servers: Option<u32>,
    #[serde(default)]
    pub verbose: Option<bool>,
}

impl ScanConfig {
    /// Build a pipeline config, starting from the documented defaults so a
    /// partial request from the UI can never silently change tuning.
    fn to_config(&self) -> Config {
        let mut cfg = Config::default();
        if let Some(g) = &self.game {
            cfg.game = Game::parse(g);
        }
        if let Some(r) = self.fetch_rounds {
            cfg.fetch_rounds = r;
        }
        if let Some(a) = self.fetch_all {
            cfg.fetch_all = a;
        }
        if let Some(c) = self.concurrency {
            cfg.concurrency = c.max(1) as usize;
        }
        if let Some(t) = self.query_timeout_ms {
            cfg.query_timeout_ms = u64::from(t);
        }
        if let Some(r) = self.rediscover {
            cfg.rediscover = r;
        }
        if let Some(f) = &self.filters {
            cfg.filters = f.clone();
        }
        if let Some(m) = self.max_servers {
            cfg.max_servers = Some(m as usize);
        }
        if let Some(v) = self.verbose {
            cfg.verbose = v;
        }
        cfg
    }
}

/// `{ game: "cstrike", ... }` or plain invocation for defaults.
#[tauri::command]
#[specta::specta]
pub async fn start_scan(app: AppHandle, config: Option<ScanConfig>) -> Result<(), String> {
    let shared = app.state::<Shared>();
    // One sweep at a time: a second concurrent sweep would double the load on
    // every server, and the pacing gate would reject most of it anyway.
    if shared.running.swap(true, Ordering::SeqCst) {
        return Err("a scan is already running".into());
    }

    let pipeline = shared.app();
    let guard = RunningGuard(Arc::clone(&shared.running));

    // Update the config **in place**: rebuilding `App` would give this sweep a
    // fresh pacing gate and ban list, so a `refresh_server` issued from the UI
    // could re-query an endpoint the sweep had just hit — precisely what the
    // per-endpoint limits exist to prevent.
    pipeline.update_cfg(|cfg| {
        let mut next = config.map(|c| c.to_config()).unwrap_or_default();
        // Carry over what the UI cannot know about, or has no business
        // changing mid-session: resolved credentials and the session's
        // persisted-state switches.
        next.api_key = cfg.api_key.clone();
        next.no_history = cfg.no_history;
        // `Connect` and the view modes are UI state, not scan parameters.
        next.hide_fakes = cfg.hide_fakes;
        next.show_fakes = cfg.show_fakes;
        next.fake_only = cfg.fake_only;
        next.text_filter = cfg.text_filter.clone();
        *cfg = next;
    });

    // Reset before spawning: Stop can arrive while the worker is still queued.
    pipeline.reset_cancel();

    tauri::async_runtime::spawn(async move {
        let handle = app.clone();
        let mut sink = move |event: Event| {
            // Non-blocking: a webview that is busy simply misses the frame, and
            // the frontend re-syncs from the next event.
            let _ = event.emit(&handle);
        };
        let result = pipeline.scan_streaming(&mut sink).await;
        let _ = Event::Phase {
            phase: app::Phase::Done,
        }
        .emit(&app);
        if let Err(e) = result {
            let _ = Event::Error {
                message: format!("{e:#}"),
            }
            .emit(&app);
        }
        drop(guard);
    });

    Ok(())
}

/// Clears the running flag when the sweep ends, including on panic.
struct RunningGuard(Arc<std::sync::atomic::AtomicBool>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Ask the in-flight sweep to stop. Nothing is aborted mid-query; results that
/// already arrived are kept.
#[tauri::command]
#[specta::specta]
pub fn stop_scan(state: State<Shared>) {
    state.app.cancel();
}

/// Re-query one endpoint. `Err` carries the pacing refusal reason, so the UI
/// can say "paced" rather than showing the server as dead.
///
/// `interactive` marks a refresh the user asked for on this one server (see
/// `QueryGate::try_acquire_interactive`); a bulk refresh of many rows passes
/// `false` and keeps the sweep's limits.
#[tauri::command]
#[specta::specta]
pub async fn refresh_server(
    app: AppHandle,
    endpoint: String,
    interactive: bool,
) -> Result<(), String> {
    let pipeline = {
        let shared = app.state::<Shared>();
        shared.app()
    };
    let server = pipeline.refresh_one(&endpoint, interactive).await?;
    let _ = Event::Refreshed {
        row: app::row_from_scanned(&server),
    }
    .emit(&app);
    Ok(())
}

/// One server's player list, for Game Info: the sweep never asks for it,
/// and everything else in the row is already measured. `Err` carries the
/// pacing refusal or the query failure.
#[tauri::command]
#[specta::specta]
pub async fn server_players(app: AppHandle, endpoint: String) -> Result<Vec<PlayerRow>, String> {
    let pipeline = {
        let shared = app.state::<Shared>();
        shared.app()
    };
    pipeline.players_one(&endpoint).await
}

/// Drop bans, the recheck queue and pacing, then rescan.
#[tauri::command]
#[specta::specta]
pub async fn hard_refresh(app: AppHandle, config: Option<ScanConfig>) -> Result<String, String> {
    let msg = {
        let shared = app.state::<Shared>();
        // Refuse before clearing anything: the rescan below would be refused
        // too, leaving bans and pacing wiped for the sweep still in flight.
        if shared.running.load(Ordering::SeqCst) {
            return Err("a scan is already running".into());
        }
        shared.app().hard_refresh()
    };
    start_scan(app, config).await?;
    Ok(msg)
}

/// The ban list.
#[tauri::command]
#[specta::specta]
pub fn bans(state: State<Shared>) -> Vec<BanRow> {
    state.app.ban_rows()
}

/// The Banned tab's `Clear bans`: drop every ban without touching pacing.
///
/// `u32` because this crosses into the webview, where Specta refuses
/// pointer-width integers (JSON numbers are f64).
#[tauri::command]
#[specta::specta]
pub fn clear_bans(state: State<Shared>) -> u32 {
    state.app().clear_bans() as u32
}

#[tauri::command]
#[specta::specta]
pub fn ban_server(state: State<Shared>, endpoint: String, hostname: String) -> Result<(), String> {
    state.app().ban_server(&endpoint, &hostname)
}

#[tauri::command]
#[specta::specta]
pub fn whitelist_server(state: State<Shared>, endpoint: String) -> Result<(), String> {
    state.app().whitelist_server(&endpoint)
}

/// Rows from the local discovery cache, so the list paints before any network
/// work. Empty when the cache is missing or was built for another filter.
#[tauri::command]
#[specta::specta]
pub fn cached_rows(state: State<Shared>) -> Vec<ServerRow> {
    state.app.cached_rows()
}

/// Join a server through Steam: `steam://connect/<ip>:<port>/`.
///
/// Steam starts (or reuses) the game and connects it, exactly as joining
/// from the in-game browser does, so no install path has to be configured.
/// The endpoint is re-parsed rather than pasted into the URL, so nothing but
/// a plain `ip:port` can reach the URL handler. Returns the URL it opened.
#[tauri::command]
#[specta::specta]
pub fn connect(endpoint: String) -> Result<String, String> {
    let url = steam_connect_url(&endpoint)?;
    tauri_plugin_opener::open_url(&url, None::<&str>)
        .map_err(|e| format!("could not open {url}: {e}. Is Steam installed?"))?;
    Ok(url)
}

fn steam_connect_url(endpoint: &str) -> Result<String, String> {
    let ep = Endpoint::parse(endpoint.trim()).ok_or_else(|| format!("bad endpoint: {endpoint}"))?;
    Ok(format!("steam://connect/{ep}/"))
}

/// What the Settings dialog shows. The key itself never crosses back into
/// the webview — only a masked hint, so it cannot leak through the UI.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct SettingsView {
    /// `••••••••1A2B`, or `None` when no key is configured anywhere.
    pub api_key_hint: Option<String>,
    /// `"settings"` (a key is saved) or `"none"`.
    pub api_key_source: String,
    /// Where a key is minted.
    pub api_key_url: String,
    /// Whether `debug`-level entries are written to the log file.
    pub debug_logs: bool,
}

fn settings_view(app: &App, stored: &Settings) -> SettingsView {
    let (hint, source) = match app.api_key() {
        Some(k) => (Some(settings::mask_key(&k)), "settings"),
        None => (None, "none"),
    };
    SettingsView {
        api_key_hint: hint,
        api_key_source: source.into(),
        api_key_url: webapi::API_KEY_URL.into(),
        debug_logs: stored.debug_logs,
    }
}

/// The Settings dialog's current state.
#[tauri::command]
#[specta::specta]
pub fn get_settings(state: State<Shared>) -> SettingsView {
    let stored = Settings::load(&Settings::path());
    settings_view(&state.app, &stored)
}

/// Enable or disable `debug`-level entries in the log file. Takes effect
/// immediately (see `logging::LoggingHandle::set_debug`) and is persisted so
/// the next run starts with the same level.
#[tauri::command]
#[specta::specta]
pub fn set_debug_logs(state: State<Shared>, enabled: bool) -> Result<SettingsView, String> {
    let path = Settings::path();
    let mut stored = Settings::load(&path);
    stored.debug_logs = enabled;
    stored.save(&path)?;
    state.logging.set_debug(enabled);
    Ok(settings_view(&state.app, &stored))
}

/// Open Steam's API-key page in the default browser.
///
/// Takes no argument on purpose: the page can open exactly this one URL,
/// never an arbitrary one, so no URL permission is granted to the webview.
#[tauri::command]
#[specta::specta]
pub fn open_api_key_page() -> Result<(), String> {
    tauri_plugin_opener::open_url(webapi::API_KEY_URL, None::<&str>)
        .map_err(|e| format!("could not open {}: {e}", webapi::API_KEY_URL))
}

/// The project's GitHub repository, shown (and openable) from the About dialog.
pub const REPOSITORY_URL: &str = "https://github.com/vuxsoftware/cs16browser";

/// Open the GitHub repository in the default browser.
///
/// Same reasoning as [`open_api_key_page`]: one fixed URL, no arbitrary-URL
/// permission granted to the webview. A plain `<a target="_blank">` cannot do
/// this itself — the webview has no such permission either.
#[tauri::command]
#[specta::specta]
pub fn open_repository_page() -> Result<(), String> {
    tauri_plugin_opener::open_url(REPOSITORY_URL, None::<&str>)
        .map_err(|e| format!("could not open {REPOSITORY_URL}: {e}"))
}

/// Save (or, with an empty string, clear) the Steam Web API key.
///
/// Takes effect for the next sweep: `start_scan` carries `api_key` over from
/// the live config, which this updates in place.
#[tauri::command]
#[specta::specta]
pub fn set_api_key(state: State<Shared>, key: String) -> Result<SettingsView, String> {
    let key = settings::normalize_api_key(&key)?;
    let path = Settings::path();
    let mut stored = Settings::load(&path);
    stored.api_key = key.clone();
    stored.save(&path)?;
    state.app.update_cfg(|cfg| cfg.api_key = key);
    Ok(settings_view(&state.app, &stored))
}

/// The CSS size the chrome is laid out for.
///
/// The chrome copies the in-game browser's measured pixels, which need about
/// 1600x900 CSS px (checked by rendering it: every region fits with a
/// 13-row list). A smaller window keeps that layout and zooms the webview
/// down instead of squeezing it, which at 1024x576 overlapped the filter
/// panel onto a list with no rows left.
const LAYOUT_W: f64 = 1600.0;
const LAYOUT_H: f64 = 900.0;

/// Zoom that fits the layout into a window's logical size. Cap it at the
/// comfortable default-window scale so maximizing adds rows, not larger text.
fn layout_zoom(logical_w: f64, logical_h: f64) -> f64 {
    (logical_w / LAYOUT_W)
        .min(logical_h / LAYOUT_H)
        .clamp(0.3, 0.8)
}

/// Window events arrive on the event loop, while `inner_size` and `set_zoom`
/// wait for that loop. Keep one worker and one pending request so a resize drag
/// cannot spawn hundreds of blocked threads or apply obsolete zoom values.
struct ZoomFitter(SyncSender<()>);

impl ZoomFitter {
    fn new(app: &AppHandle, label: &str) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        if let Some(webview) = app.get_webview_window(label) {
            std::thread::spawn(move || {
                let mut last_zoom = None;
                let mut last_update = None::<Instant>;
                while receiver.recv().is_ok() {
                    // Webview zoom triggers a full layout. Limit it during a
                    // drag, then use the latest window size for the final fit.
                    if let Some(last) = last_update {
                        let remaining = Duration::from_millis(40).saturating_sub(last.elapsed());
                        if !remaining.is_zero() {
                            std::thread::sleep(remaining);
                        }
                    }
                    while receiver.try_recv().is_ok() {}
                    let (Ok(size), Ok(scale)) = (webview.inner_size(), webview.scale_factor())
                    else {
                        continue;
                    };
                    let logical = size.to_logical::<f64>(scale);
                    if logical.width < 1.0 || logical.height < 1.0 {
                        continue; // minimised
                    }
                    let zoom = layout_zoom(logical.width, logical.height);
                    if last_zoom == Some(zoom) {
                        continue;
                    }
                    if webview.set_zoom(zoom).is_ok() {
                        last_zoom = Some(zoom);
                        last_update = Some(Instant::now());
                    }
                }
            });
        }
        Self(sender)
    }

    fn request(&self) {
        // A full channel already represents the latest window size: the
        // worker reads the size when it handles the request.
        let _ = self.0.try_send(());
    }
}

/// Build the Tauri app and run it.
///
/// `tauri-specta` owns the command surface: `collect_commands!` is what Tauri
/// dispatches *and* what the TypeScript bindings describe, so a command whose
/// signature changes cannot keep compiling against a stale frontend contract.
pub fn run() {
    // Before anything else can log: `logging::init` installs the global
    // subscriber, and every module below (including `App::new`, which can
    // already touch the state file) is expected to be able to log.
    let stored_settings = Settings::load(&Settings::path());
    let logging = crate::logging::init(stored_settings.debug_logs);

    let builder = specta_builder();

    // Export on every build, not just debug: the bindings are a build artefact
    // the frontend imports, and a release build that silently kept an old
    // `bindings.ts` would be worse than a slightly slower startup.
    export_bindings(&builder);

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            // The key comes from the Settings dialog only.
            let config = Config {
                api_key: stored_settings.api_key.clone(),
                ..Config::default()
            };
            app.manage(Shared::new(App::new(config), logging));
            // Required for the typed event to resolve its name and payload.
            builder.mount_events(app);
            let zoom_fitter = ZoomFitter::new(app.handle(), "main");
            zoom_fitter.request();
            app.manage(zoom_fitter);
            Ok(())
        })
        // The close box (and Alt+F4) quits: closing the only window ends the
        // process, which is also what lets `tauri dev` return on its own.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. } =
                event
            {
                if let Some(fitter) = window.app_handle().try_state::<ZoomFitter>() {
                    fitter.request();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building cs16browser")
        .run(|app, event| {
            // A sweep only saves the pacing gate when it finishes. Quitting
            // mid-sweep would lose the record of the servers it just queried,
            // and a quick restart could query them again inside their limits.
            if let tauri::RunEvent::Exit = event {
                if let Some(shared) = app.try_state::<Shared>() {
                    shared.app.persist_gate();
                }
            }
        });
}

/// The command and event surface, shared by `run` and the bindings export.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            start_scan,
            stop_scan,
            refresh_server,
            server_players,
            hard_refresh,
            bans,
            clear_bans,
            ban_server,
            whitelist_server,
            cached_rows,
            connect,
            get_settings,
            set_api_key,
            set_debug_logs,
            open_api_key_page,
            open_repository_page,
        ])
        .events(collect_events![Event])
}

/// Write `bindings.ts` only when its content changed.
///
/// An unconditional write touches the file on every launch; under
/// `tauri dev` Vite hot-reloads the page for it, which remounts the app and
/// starts another sweep. Exporting to a scratch file and comparing keeps the
/// file (and its mtime) untouched unless a DTO really changed.
fn export_bindings(builder: &Builder<tauri::Wry>) {
    let target = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/lib/bindings.ts"
    ));
    let scratch =
        std::env::temp_dir().join(format!("cs16browser-bindings-{}.ts", std::process::id()));
    builder
        .export(specta_typescript::Typescript::default(), &scratch)
        .expect("failed to export the TypeScript bindings");
    let fresh = std::fs::read(&scratch).expect("failed to read the exported bindings");
    let _ = std::fs::remove_file(&scratch);
    if std::fs::read(target).ok().as_deref() != Some(fresh.as_slice()) {
        std::fs::write(target, fresh).expect("failed to write the TypeScript bindings");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_bindings_match_commands() {
        export_bindings(&specta_builder());
    }

    #[test]
    fn scan_config_defaults_do_not_change_tuning() {
        let cfg = ScanConfig {
            game: None,
            fetch_rounds: None,
            fetch_all: None,
            concurrency: None,
            query_timeout_ms: None,
            rediscover: None,
            filters: None,
            max_servers: None,
            verbose: None,
        }
        .to_config();
        let base = Config::default();
        assert_eq!(cfg.concurrency, base.concurrency);
        assert_eq!(cfg.query_timeout_ms, base.query_timeout_ms);
        assert_eq!(cfg.fetch_rounds, base.fetch_rounds);
        assert!(cfg.hide_fakes);
    }

    #[test]
    fn scan_config_overrides_are_applied() {
        let cfg = ScanConfig {
            game: Some("cstrike".into()),
            fetch_rounds: Some(9),
            fetch_all: Some(true),
            concurrency: Some(0), // clamped, never zero
            query_timeout_ms: Some(1500),
            rediscover: Some(true),
            filters: Some(Filters {
                secure: true,
                ..Default::default()
            }),
            max_servers: Some(50),
            verbose: Some(true),
        }
        .to_config();
        assert_eq!(cfg.game, Game::CS16);
        assert_eq!(cfg.fetch_rounds, 9);
        assert!(cfg.fetch_all);
        assert_eq!(cfg.concurrency, 1);
        assert_eq!(cfg.query_timeout_ms, 1500);
        assert!(cfg.rediscover);
        assert!(cfg.filters.secure);
        assert_eq!(cfg.max_servers, Some(50));
    }

    #[test]
    fn layout_zoom_fits_the_measured_layout() {
        assert!(
            (layout_zoom(1280.0, 720.0) - 0.8).abs() < 1e-9,
            "default window"
        );
        assert!(
            (layout_zoom(1024.0, 576.0) - 0.64).abs() < 1e-9,
            "minimum window"
        );
        assert_eq!(layout_zoom(1600.0, 900.0), 0.8);
        // Large windows gain rows, not bigger text.
        assert_eq!(layout_zoom(2560.0, 1440.0), 0.8);
        // The tighter axis wins: a tall narrow window still fits its width.
        assert!((layout_zoom(1200.0, 1200.0) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn connect_builds_a_steam_deeplink_from_a_parsed_endpoint() {
        assert_eq!(
            steam_connect_url(" 145.239.20.89:27015 ").unwrap(),
            "steam://connect/145.239.20.89:27015/"
        );
        // Nothing but ip:port may reach the URL handler.
        for bad in [
            "145.239.20.89:27015/+exec x",
            "evil.example:27015",
            "1.2.3.4",
        ] {
            assert!(steam_connect_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn game_names_round_trip_from_the_frontend() {
        for (input, want) in [
            ("cstrike", Game::CS16),
            ("czero", Game::CZ),
            ("valve", Game::HalfLife),
            ("tfc", Game::TFC),
            ("dod", Game::DOD),
        ] {
            assert_eq!(Game::parse(input), want, "{input}");
        }
    }
}
