//! Debug logging.
//!
//! Off by default: only `info`/`warn`/`error` reach the log file. The
//! Settings checkbox flips a live [`reload`] handle, so turning debug
//! logging on (or off) takes effect immediately, without restarting the app.
//!
//! Each run writes its own file, named with the process's start time
//! (`cs16browser_<start time>.log`) so two overlapping runs never share a
//! file and `ls` sorts newest-last. Files older than [`RETENTION_DAYS`] are
//! deleted on startup — logs are a debugging aid, not a permanent record, so
//! nothing here needs to grow forever.
//!
//! Console output (`stderr`) stays at `info` regardless of the setting: it is
//! what `tauri dev` shows, and enabling *debug* logging is about the file,
//! not about flooding the terminal.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tracing_subscriber::{
    fmt, layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter, Layer, Registry,
};

/// Subdirectory of the discovery cache directory that holds log files.
pub const LOG_DIR: &str = "logs";

/// Log files older than this, by modification time, are deleted on startup.
const RETENTION_DAYS: u64 = 7;

/// Live handle to the file layer's filter, so the Settings toggle can change
/// the log level without tearing down and rebuilding the subscriber.
pub struct LoggingHandle {
    reload: reload::Handle<EnvFilter, Registry>,
    /// Keeps the non-blocking writer's background flush thread alive; must
    /// live as long as the app (see `tracing_appender::non_blocking` docs).
    _file_guard: tracing_appender::non_blocking::WorkerGuard,
    pub log_path: PathBuf,
}

impl LoggingHandle {
    /// Switch the file log between `info` (default) and `debug`.
    pub fn set_debug(&self, enabled: bool) {
        let level = if enabled { "debug" } else { "info" };
        if self.reload.reload(EnvFilter::new(level)).is_ok() {
            tracing::info!(enabled, "debug logging toggled");
        }
    }
}

fn log_dir() -> PathBuf {
    crate::store::store_dir().join(LOG_DIR)
}

/// Whether a `.log` file last modified at `modified` is old enough to prune,
/// relative to `now`. A non-`.log` file is never prunable — the log
/// directory is exclusively ours, but this keeps a stray file harmless.
fn is_prunable(path: &Path, modified: SystemTime, now: SystemTime) -> bool {
    if path.extension().and_then(|e| e.to_str()) != Some("log") {
        return false;
    }
    let cutoff = now
        .checked_sub(Duration::from_secs(RETENTION_DAYS * 24 * 60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    modified < cutoff
}

/// Delete `*.log` files whose modification time is older than
/// [`RETENTION_DAYS`]. Best-effort: an unreadable directory or a file that
/// cannot be removed (locked, permissions) is skipped rather than failing
/// startup.
fn prune_old_logs(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    let mut pruned = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if is_prunable(&path, modified, now) && fs::remove_file(&path).is_ok() {
            pruned += 1;
        }
    }
    if pruned > 0 {
        tracing::info!(
            pruned,
            "log retention: removed log file(s) older than {RETENTION_DAYS}d"
        );
    }
}

/// `cs16browser_<start time>.log`, so the app-start timestamp is visible
/// directly in a directory listing.
fn log_file_name(started: chrono::DateTime<chrono::Local>) -> String {
    format!("cs16browser_{}.log", started.format("%Y-%m-%d_%H-%M-%S"))
}

/// Initialise the global subscriber. Call exactly once, at startup, before
/// anything else logs — a second call panics (`tracing` refuses a second
/// global default).
///
/// `debug_enabled` is the persisted Settings value, so a user who turned
/// debug logging on keeps it on across restarts without an extra click.
pub fn init(debug_enabled: bool) -> LoggingHandle {
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);
    prune_old_logs(&dir);

    // Local time in the filename: it is what the user sees in their file
    // browser and matches the clock they would use to find "the run from
    // just now" among several.
    let file_name = log_file_name(chrono::Local::now());
    let log_path = dir.join(&file_name);

    let file_appender = tracing_appender::rolling::never(&dir, &file_name);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let initial = if debug_enabled { "debug" } else { "info" };
    let (reloadable_filter, reload_handle) = reload::Layer::new(EnvFilter::new(initial));

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_filter(reloadable_filter);

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    // `file_layer` must be added first: its `Filtered` wraps whatever
    // subscriber it was attached onto, and `LoggingHandle` stores the reload
    // handle typed against a bare `Registry` — adding `stderr_layer` first
    // would type it against `Layered<stderr, Registry>` instead and fail to
    // compile.
    tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .init();

    tracing::info!(
        log_path = %log_path.display(),
        debug_enabled,
        "logging started"
    );

    LoggingHandle {
        reload: reload_handle,
        _file_guard: guard,
        log_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_embeds_the_start_time() {
        use chrono::TimeZone;
        let started = chrono::Local
            .with_ymd_and_hms(2026, 9, 24, 21, 37, 52)
            .unwrap();
        assert_eq!(
            log_file_name(started),
            "cs16browser_2026-09-24_21-37-52.log"
        );
    }

    #[test]
    fn only_log_files_are_prunable() {
        let now = SystemTime::now();
        let ancient = now - Duration::from_secs((RETENTION_DAYS + 1) * 24 * 60 * 60);
        assert!(
            !is_prunable(Path::new("servers.json"), ancient, now),
            "a non-.log file must never be pruned, however old"
        );
    }

    #[test]
    fn a_log_file_is_prunable_only_once_past_retention() {
        let now = SystemTime::now();
        let just_inside = now - Duration::from_secs(RETENTION_DAYS * 24 * 60 * 60 - 60);
        let just_outside = now - Duration::from_secs(RETENTION_DAYS * 24 * 60 * 60 + 60);
        let path = Path::new("cs16browser_2026-01-01_00-00-00.log");
        assert!(
            !is_prunable(path, just_inside, now),
            "a log file just inside the retention window must be kept"
        );
        assert!(
            is_prunable(path, just_outside, now),
            "a log file past the retention window must be pruned"
        );
    }

    #[test]
    fn prune_old_logs_removes_only_expired_log_files() {
        let dir = std::env::temp_dir().join(format!("cs16-logs-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let fresh = dir.join("cs16browser_fresh.log");
        let stale = dir.join("cs16browser_stale.log");
        let other = dir.join("servers.json");
        for p in [&fresh, &stale, &other] {
            fs::write(p, b"x").unwrap();
        }

        let now = SystemTime::now();
        let old = now - Duration::from_secs((RETENTION_DAYS + 1) * 24 * 60 * 60);
        fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old)
            .unwrap();

        prune_old_logs(&dir);

        assert!(
            fresh.exists(),
            "a log file inside the retention window survives"
        );
        assert!(
            !stale.exists(),
            "a log file past the retention window is removed"
        );
        assert!(other.exists(), "a non-.log file is never touched");

        let _ = fs::remove_dir_all(&dir);
    }
}
