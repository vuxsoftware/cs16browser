//! Integration test for the Steam Web API client's HTTP contract.
//!
//! `WebApi::http_base` lets the test point the client at a local server, so the
//! real request/response path is exercised (URL, query parameters, JSON
//! parsing) rather than just the deserializer.

use cs16browser_lib::client::webapi::WebApi;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

/// Realistic `IGameServersService/GetServerList` payload for appid 10.
const SERVER_LIST_JSON: &str = r#"{"response":{"servers":[
 {"addr":"203.0.113.7:27015","gameport":27015,"specport":null,
  "steamid":"90071996900000007","name":"Example CS 1.6","appid":10,
  "gamedir":"cstrike","version":"1.1.2.7/Stdio","product":"cstrike",
  "region":255,"players":12,"max_players":32,"bots":0,"map":"de_dust2",
  "secure":true,"dedicated":true,"os":"l","gametype":"public"},
 {"addr":"198.51.100.9:27016","gameport":27016,"name":"Sparse",
  "players":0,"map":"cs_italy"}
]}}"#;

/// Serve one HTTP response, returning (base_url, captured_request_line).
fn serve_once(status: &str, body: &str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();

    let status = status.to_string();
    let body = body.to_string();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let line = request.lines().next().unwrap_or_default().to_string();
        let _ = tx.send(line);

        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });

    (format!("http://127.0.0.1:{port}"), rx)
}

#[test]
fn sends_filter_and_key_as_query_parameters() {
    let (base, rx) = serve_once("200 OK", SERVER_LIST_JSON);
    let servers = WebApi::new("TESTKEY123")
        .with_base(&base)
        .fetch_server_list("\\appid\\10\\gamedir\\cstrike", 5000)
        .expect("request must succeed");

    // The response must be parsed from the documented envelope.
    assert_eq!(servers.len(), 2);
    assert_eq!(
        servers[0].endpoint().unwrap().to_string(),
        "203.0.113.7:27015"
    );
    assert_eq!(servers[0].name.as_deref(), Some("Example CS 1.6"));
    assert_eq!(servers[0].appid, Some(10));
    assert_eq!(servers[0].players, Some(12));

    // The request must carry the key/filter/limit the API requires, and hit
    // the IGameServersService path.
    let line = rx
        .recv()
        .expect("server thread must report the request line");
    assert!(
        line.starts_with("GET /IGameServersService/GetServerList/v1/"),
        "unexpected request line: {line}"
    );
    assert!(line.contains("key=TESTKEY123"), "key missing: {line}");
    assert!(line.contains("limit=5000"), "limit missing: {line}");

    // The filter is `\key\value`; URL-encoded, the backslashes become %5C.
    let encoded = line
        .split(' ')
        .nth(1)
        .unwrap_or_default()
        .replace("%5C", "\\");
    assert!(
        encoded.contains("filter=\\appid\\10\\gamedir\\cstrike"),
        "filter missing or mis-encoded: {line}"
    );
}

#[test]
fn limit_is_clamped_to_the_measured_cap() {
    let (base, rx) = serve_once("200 OK", r#"{"response":{"servers":[]}}"#);
    WebApi::new("K")
        .with_base(&base)
        .fetch_server_list("\\appid\\10", 999_999)
        .expect("request must succeed");
    let line = rx.recv().unwrap();
    // Steam's docs say 20k, but a live fetch returned exactly 10000, so that is
    // the cap we clamp to.
    assert!(
        line.contains("limit=10000"),
        "limit must clamp to the measured cap: {line}"
    );
    assert!(
        !line.contains("limit=999999"),
        "must not forward an over-cap limit: {line}"
    );
}

#[test]
fn rejects_bad_key_with_actionable_message() {
    let (base, _rx) = serve_once("403 Forbidden", "<html>Access is denied.</html>");
    let err = WebApi::new("BAD")
        .with_base(&base)
        .fetch_server_list("\\appid\\10", 10)
        .expect_err("403 must surface as an error");

    let msg = format!("{err:#}");
    assert!(msg.contains("403"), "must report the status: {msg}");
    assert!(
        msg.contains("apikey"),
        "must point at where to get a key: {msg}"
    );
}

#[test]
fn surfaces_non_json_bodies_instead_of_panicking() {
    let (base, _rx) = serve_once("200 OK", "<html>not json</html>");
    let err = WebApi::new("K")
        .with_base(&base)
        .fetch_server_list("\\appid\\10", 10)
        .expect_err("non-JSON body must error");
    assert!(
        format!("{err:#}").contains("GetServerList JSON"),
        "error should name the endpoint: {err:#}"
    );
}

/// The API key travels in the query string, so transport errors that quote the
/// URL would print it verbatim into logs. It must never appear in an error.
#[test]
fn api_key_is_never_exposed_in_errors() {
    const SECRET: &str = "DEADBEEF0123456789ABCDEFSECRETKEY";
    // Port 1 is reserved and never listening: guarantees a transport error.
    let err = WebApi::new(SECRET)
        .with_base("http://127.0.0.1:1")
        .fetch_server_list("\\appid\\10", 10)
        .expect_err("connection to a dead port must fail");

    let msg = format!("{err:#}");
    assert!(
        !msg.contains(SECRET),
        "the API key leaked into the error message: {msg}"
    );
    assert!(msg.contains("request failed"), "unexpected error: {msg}");
}

/// A 5xx is a Steam-side outage, not a client problem, and should say so.
#[test]
fn server_error_reports_an_outage() {
    let (base, _rx) = serve_once("502 Bad Gateway", "<html>bad gateway</html>");
    let err = WebApi::new("K")
        .with_base(&base)
        .fetch_server_list("\\appid\\10", 10)
        .expect_err("502 must error");
    let msg = format!("{err:#}");
    assert!(msg.contains("502"), "must name the status: {msg}");
    assert!(
        msg.to_lowercase().contains("unavailable") || msg.to_lowercase().contains("outage"),
        "must explain it is a Steam-side outage: {msg}"
    );
}

/// Multi-round fetching must deduplicate by `ip:port`.
///
/// The API returns a different *sample* each call, so the same server is
/// routinely seen in several rounds. Without dedup the scan would waste its
/// whole budget re-querying the same machines.
#[test]
fn multi_round_fetch_deduplicates_by_endpoint() {
    // Round 1 and round 2 overlap heavily, and both repeat an endpoint
    // internally to prove intra-round dedup as well.
    let round1 = r#"{"response":{"servers":[
        {"addr":"203.0.113.1:27015","name":"A"},
        {"addr":"203.0.113.2:27015","name":"B"},
        {"addr":"203.0.113.1:27015","name":"A-duplicate"}
    ]}}"#;
    let round2 = r#"{"response":{"servers":[
        {"addr":"203.0.113.2:27015","name":"B-again"},
        {"addr":"203.0.113.3:27015","name":"C"}
    ]}}"#;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        // Serve the rounds in order, then keep accepting so a stray retry does
        // not fail the test with a connection error.
        let bodies = [round1, round2, round2, round2];
        for body in bodies {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.flush();
            // Give the client a moment to read before the socket is dropped.
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });

    let rows = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&rows);
    let stats = WebApi::new("K")
        .with_base(format!("http://127.0.0.1:{port}"))
        .fetch_all_streaming("\\appid\\10", 10_000, 4, 0.0, |fresh, _st| {
            sink.lock().unwrap().extend(fresh.iter().cloned());
        })
        .expect("both rounds must succeed");
    let rows = rows.lock().unwrap().clone();

    // Three distinct endpoints from duplicate-heavy rows over several rounds.
    let endpoints: Vec<String> = rows.iter().map(|r| r.addr.clone()).collect();
    assert_eq!(
        endpoints.len(),
        3,
        "duplicate endpoints must collapse: {endpoints:?}"
    );
    assert_eq!(stats.total, 3);

    let unique: std::collections::HashSet<&String> = endpoints.iter().collect();
    assert_eq!(unique.len(), 3, "no repeated ip:port may survive");

    // With `min_new_ratio = 0.0` the loop only stops once a round contributes
    // nothing new, which needs one more call than there are distinct batches.
    assert!(stats.converged(), "must stop because it ran dry: {stats:?}");
    assert_eq!(stats.rounds, 3, "stats: {stats:?}");
    // A repeats within round 1, B repeats across rounds 1->2, and round 3
    // repeats all of round 2. 1 + 1 + 2.
    assert_eq!(stats.duplicates_skipped, 4, "stats: {stats:?}");

    // First occurrence wins, so metadata is not clobbered by a later sample.
    let a = rows.iter().find(|r| r.addr == "203.0.113.1:27015").unwrap();
    assert_eq!(a.name.as_deref(), Some("A"));
    let b = rows.iter().find(|r| r.addr == "203.0.113.2:27015").unwrap();
    assert_eq!(
        b.name.as_deref(),
        Some("B"),
        "later sample must not overwrite"
    );
}

/// A round that adds nothing must stop the loop even if rounds remain.
#[test]
fn converged_fetch_stops_early() {
    let same = r#"{"response":{"servers":[{"addr":"203.0.113.9:27015","name":"X"}]}}"#;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        // Serve the identical body for every request.
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{same}",
                    same.len()
                )
                .as_bytes(),
            );
        }
    });

    let stats = WebApi::new("K")
        .with_base(format!("http://127.0.0.1:{port}"))
        .fetch_all_streaming("\\appid\\10", 10_000, 20, 0.0, |_, _| {})
        .expect("fetch must succeed");

    assert_eq!(stats.total, 1);
    assert_eq!(
        stats.rounds, 2,
        "must stop after the first no-new-data round"
    );
    assert!(stats.converged());
}

/// Rows whose `addr` cannot be parsed are dropped and counted, not silently
/// turned into a bogus endpoint.
#[test]
fn unparseable_rows_are_dropped_and_counted() {
    let body = r#"{"response":{"servers":[
        {"addr":"not-an-endpoint"},
        {"addr":"203.0.113.5:27015"}
    ]}}"#;
    let (base, _rx) = serve_once("200 OK", body);

    let stats = WebApi::new("K")
        .with_base(&base)
        .fetch_all_streaming("\\appid\\10", 10_000, 1, 0.0, |_, _| {})
        .expect("fetch must succeed");

    assert_eq!(stats.total, 1);
    assert_eq!(stats.unparseable, 1);
}

/// Regression: discovery must happen **once**. A second run reads the cache and
/// must not call Steam at all, or the metered API gets spammed by ordinary use.
///
/// This drives the real request path against a stub and counts how many HTTP
/// requests arrive.
#[test]
fn cached_discovery_avoids_a_second_fetch() {
    use cs16browser_lib::store::ServerStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const BODY: &str = r#"{"response":{"servers":[
        {"addr":"203.0.113.1:27015","name":"Cached A","appid":10,"gamedir":"cstrike",
         "map":"de_dust2","players":4,"max_players":32},
        {"addr":"203.0.113.2:27015","name":"Cached B","appid":10,"gamedir":"cstrike",
         "map":"de_dust2","players":7,"max_players":32}
    ]}}"#;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_srv = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            hits_srv.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                    BODY.len()
                )
                .as_bytes(),
            );
        }
    });

    // First fetch: populates the cache via the real HTTP path.
    let api = WebApi::new("K").with_base(format!("http://127.0.0.1:{port}"));
    let rows = api
        .fetch_server_list("\\appid\\10\\gamedir\\cstrike", 10_000)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(hits.load(Ordering::SeqCst), 1, "one fetch, one request");

    // Persist, then reload: the reloaded cache is what a later run reads.
    let dir = std::env::temp_dir().join(format!("cs16cache_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("servers.json");
    ServerStore::new("\\appid\\10\\gamedir\\cstrike", "cstrike", rows.clone())
        .save(&path)
        .unwrap();

    let hits_before = hits.load(Ordering::SeqCst);
    let cached = ServerStore::load(&path).expect("cache must load");
    assert!(cached.matches("\\appid\\10\\gamedir\\cstrike", "cstrike"));
    assert_eq!(cached.len(), 2);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        hits_before,
        "reading the cache must not touch the network"
    );

    // A different query must NOT be served from this cache, or the user would
    // silently get the wrong game's servers.
    assert!(
        !cached.matches("\\appid\\10\\gamedir\\czero", "czero"),
        "a different gamedir must invalidate the cache"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
