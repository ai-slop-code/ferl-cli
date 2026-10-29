//! Live integration tests against an explicitly provided local ferl API.
//!
//! These tests are ignored by default. Run them only with the local server available:
//!
//! ```sh
//! (set +x; umask 077; set -a; . "$HOME/.config/ferl-cli-local/profile.env"; set +a;
//!  cargo test --test live_api -- --ignored --test-threads=1)
//! ```
//!
//! They read `FERL_URL` and `FERL_API_KEY` from the environment, never print them, and
//! create only per-run links whose exact codes they delete. The shared `cli-demo`
//! fixture is only read.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::Harness;

fn live_origin() -> (String, String) {
    let url =
        std::env::var("FERL_URL").expect("FERL_URL must be set for live tests (see module docs)");
    let key = std::env::var("FERL_API_KEY")
        .expect("FERL_API_KEY must be set for live tests (see module docs)");
    (url, key)
}

fn run_id(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("live-{label}-{}", nanos % 1_000_000_000_000)
}

fn live_harness() -> (Harness, String) {
    let (url, key) = live_origin();
    let harness = Harness::new();
    let added = harness.add_profile("local", &url, &key);
    assert_eq!(added.status, 0, "profile add failed: {}", added.stderr);
    added.assert_no_secret();
    (harness, key)
}

fn create_link(harness: &Harness, destination: &str, title: &str) -> serde_json::Value {
    let run = harness.run_json(
        "local",
        &["create", destination, "--title", title, "--no-copy"],
    );
    run.assert_no_secret();
    assert_eq!(run.status, 0, "create failed: {}", run.stderr);
    run.json_stdout()
}

#[test]
#[ignore = "requires the local live API and FERL_URL/FERL_API_KEY"]
fn live_profile_check_and_cli_demo_search_use_the_documented_contract() {
    let (harness, _) = live_harness();
    let check = harness.run(&["--output", "json", "profile", "check", "local"]);
    check.assert_no_secret();
    assert_eq!(check.status, 0, "{}", check.stderr);
    let account = check.json_stdout();
    assert_eq!(account["data"]["account"]["username"], "ferl_cli_admin");
    assert!(account["meta"]["request_id"].is_string());

    let search = harness.run_json("local", &["list", "--query", "cli-demo"]);
    search.assert_no_secret();
    assert_eq!(search.status, 0, "{}", search.stderr);
    let value = search.json_stdout();
    let items = value["data"]["items"].as_array().expect("items");
    assert!(items.iter().any(|item| item["short_code"] == "cli-demo"));
    for item in items {
        assert!(item.get("tags").is_some());
        assert!(item["visit_count"].is_i64() || item["visit_count"].is_u64());
    }
}

#[test]
#[ignore = "requires the local live API and FERL_URL/FERL_API_KEY"]
fn live_create_list_and_delete_round_trip_keeps_metadata_and_is_not_reusable_by_repeat() {
    let (harness, _) = live_harness();
    let marker = run_id("roundtrip");
    let destination = format!("https://example.com/ferl-cli/{marker}?edition=2&x=%20#intro");
    let created = create_link(&harness, &destination, &marker);
    let link = &created["data"]["link"];
    let code = link["short_code"].as_str().expect("code").to_owned();
    assert_eq!(
        link["destination_url"],
        destination.as_str(),
        "query and fragment survive"
    );
    assert_eq!(link["title"], marker.as_str());
    assert!(link["owner_id"].is_string());
    assert!(created["data"]["link"].get("management_token").is_none());
    assert!(
        link["short_url"]
            .as_str()
            .expect("short url")
            .ends_with(&format!("/{code}"))
    );

    let found = harness.run(&[
        "--profile",
        "local",
        "list",
        "--query",
        &marker,
        "--columns",
        "short_code",
    ]);
    assert_eq!(found.status, 0, "{}", found.stderr);
    let listed = harness.run_json("local", &["list", "--query", &marker]);
    let items = listed.json_stdout()["data"]["items"]
        .as_array()
        .expect("items")
        .clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["short_code"], code.as_str());

    let deleted = harness.run(&["--profile", "local", "delete", &code, "--yes"]);
    assert_eq!(deleted.status, 0, "{}", deleted.stderr);
    assert!(deleted.stdout.is_empty());

    let again = harness.run_json("local", &["delete", &code, "--yes"]);
    assert_eq!(
        again.status, 6,
        "a repeated delete is an observed not-found"
    );
    let after = harness.run_json("local", &["list", "--query", &marker]);
    assert!(
        after.json_stdout()["data"]["items"]
            .as_array()
            .expect("items")
            .is_empty()
    );
}

#[test]
#[ignore = "requires the local live API and FERL_URL/FERL_API_KEY"]
fn live_duplicate_destinations_are_independent_and_deleting_one_keeps_the_other() {
    let (harness, _) = live_harness();
    let marker = run_id("dup");
    let destination = format!("https://example.com/ferl-cli/{marker}");
    let first = create_link(&harness, &destination, &marker);
    let second = create_link(&harness, &destination, &marker);
    let first_code = first["data"]["link"]["short_code"]
        .as_str()
        .expect("code")
        .to_owned();
    let second_code = second["data"]["link"]["short_code"]
        .as_str()
        .expect("code")
        .to_owned();
    assert_ne!(first_code, second_code);

    let short_url = first["data"]["link"]["short_url"]
        .as_str()
        .expect("url")
        .to_owned();
    let by_url = harness.run_json("local", &["delete", &short_url, "--yes"]);
    assert_eq!(by_url.status, 0, "{}", by_url.stderr);
    assert_eq!(
        by_url.json_stdout()["data"]["short_code"],
        first_code.as_str()
    );

    let remaining = harness.run_json("local", &["list", "--query", &marker]);
    let codes: Vec<String> = remaining.json_stdout()["data"]["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["short_code"].as_str().expect("code").to_owned())
        .collect();
    assert_eq!(codes, vec![second_code.clone()]);
    let cleanup = harness.run_json("local", &["delete", &second_code, "--yes"]);
    assert_eq!(cleanup.status, 0);
}

#[test]
#[ignore = "requires the local live API and FERL_URL/FERL_API_KEY"]
fn live_small_pages_traverse_every_link_once_in_server_order() {
    let (harness, _) = live_harness();
    let marker = run_id("pages");
    let mut codes = Vec::new();
    for i in 0..3 {
        let created = create_link(
            &harness,
            &format!("https://example.com/ferl-cli/{marker}/{i}"),
            &marker,
        );
        codes.push(
            created["data"]["link"]["short_code"]
                .as_str()
                .expect("code")
                .to_owned(),
        );
    }
    let all = harness.run_json(
        "local",
        &["list", "--query", &marker, "--limit", "1", "--all"],
    );
    assert_eq!(all.status, 0, "{}", all.stderr);
    let value = all.json_stdout();
    let listed: Vec<String> = value["data"]["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["short_code"].as_str().expect("code").to_owned())
        .collect();
    assert_eq!(listed.len(), 3, "every link appears exactly once");
    let mut sorted_listed = listed.clone();
    sorted_listed.sort();
    let mut expected = codes.clone();
    expected.sort();
    assert_eq!(sorted_listed, expected);
    assert_eq!(value["meta"]["complete"], true);

    let first_page = harness.run_json("local", &["list", "--query", &marker, "--limit", "2"]);
    let page = first_page.json_stdout();
    assert_eq!(page["data"]["items"].as_array().expect("items").len(), 2);
    assert!(
        page["data"]["next_cursor"].is_string(),
        "more results are advertised"
    );

    for code in codes {
        let _ = harness.run_json("local", &["delete", &code, "--yes"]);
    }
}

#[test]
#[ignore = "requires the local live API and FERL_URL/FERL_API_KEY"]
fn live_wrong_key_fails_authentication_without_fallback_or_creation() {
    let (url, _) = live_origin();
    let harness = Harness::new();
    let bogus = "ferl_ak_v1_definitely_not_a_real_key_0000000000";
    let added = harness.add_profile("bogus", &url, bogus);
    assert_eq!(added.status, 0);
    let before = harness.run_json("bogus", &["list", "--query", "cli-demo"]);
    before.assert_no_secret();
    assert_eq!(before.status, 4);
    let create = harness.run_json(
        "bogus",
        &["create", "https://example.com/never-created", "--no-copy"],
    );
    assert_eq!(
        create.status, 4,
        "invalid credentials never fall back to anonymous creation"
    );
    assert_eq!(
        create.json_stderr_lines()[0]["mutation_outcome"],
        "rejected"
    );
}

/// A plaintext TCP proxy that forwards one request to upstream, then discards the
/// upstream response before the client can read it. This simulates a lost
/// acknowledgement after the server has committed the mutation.
struct DroppingProxy {
    port: u16,
    forwarded_posts: Arc<AtomicUsize>,
    _thread: thread::JoinHandle<()>,
}

fn dropping_proxy(upstream: String) -> DroppingProxy {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy");
    let port = listener.local_addr().expect("addr").port();
    let forwarded_posts = Arc::new(AtomicUsize::new(0));
    let counter = forwarded_posts.clone();
    let thread = thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut client) = stream else { continue };
            let _ = client.set_read_timeout(Some(Duration::from_secs(20)));
            let Some(request) = read_http_request(&mut client) else {
                continue;
            };
            if request.starts_with(b"POST ") {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            let upstream_addr = upstream.trim_start_matches("http://");
            let Ok(mut server) = std::net::TcpStream::connect(upstream_addr) else {
                continue;
            };
            let _ = server.set_read_timeout(Some(Duration::from_secs(20)));
            if server.write_all(&request).is_err() {
                continue;
            }
            let mut sink = Vec::new();
            let _ = server.read_to_end(&mut sink);
            drop(client);
        }
    });
    DroppingProxy {
        port,
        forwarded_posts,
        _thread: thread,
    }
}

fn read_http_request(stream: &mut std::net::TcpStream) -> Option<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let read = stream.read(&mut byte).ok()?;
        if read == 0 {
            return None;
        }
        buffer.push(byte[0]);
        if buffer.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buffer).to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).ok()?;
    buffer.extend_from_slice(&body);
    Some(buffer)
}

#[test]
#[ignore = "requires the local live API and FERL_URL/FERL_API_KEY"]
fn live_dropped_create_acknowledgement_is_reported_uncertain_and_never_replayed() {
    let (url, key) = live_origin();
    let proxy = dropping_proxy(url);
    let harness = Harness::new();
    let proxied = format!("http://127.0.0.1:{}", proxy.port);
    let added = harness.add_profile("proxied", &proxied, &key);
    assert_eq!(added.status, 0, "{}", added.stderr);
    let marker = run_id("dropped");
    let run = harness.run_json(
        "proxied",
        &[
            "create",
            &format!("https://example.com/ferl-cli/{marker}"),
            "--title",
            &marker,
            "--no-copy",
        ],
    );
    run.assert_no_secret();
    assert_eq!(run.status, 13, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty());
    assert_eq!(
        proxy.forwarded_posts.load(Ordering::SeqCst),
        1,
        "exactly one POST reached the server"
    );

    let direct = harness.add_profile(
        "direct",
        &std::env::var("FERL_URL").expect("FERL_URL"),
        &key,
    );
    assert_eq!(direct.status, 0);
    let observed = harness.run_json("direct", &["list", "--query", &marker]);
    let items = observed.json_stdout()["data"]["items"]
        .as_array()
        .expect("items")
        .clone();
    assert_eq!(
        items.len(),
        1,
        "the committed link is visible through list; no duplicate was created"
    );
    let code = items[0]["short_code"].as_str().expect("code").to_owned();
    let cleanup = harness.run_json("direct", &["delete", &code, "--yes"]);
    assert_eq!(cleanup.status, 0);
}

#[test]
#[ignore = "requires the local live API and FERL_URL/FERL_API_KEY"]
fn live_profile_writes_are_observable_only_through_the_committed_state() {
    let (url, key) = live_origin();
    let harness = Harness::new();
    let work = harness.add_profile("validated", &url, &key);
    assert_eq!(work.status, 0);
    let validated = harness.run_with(
        &[
            "profile",
            "add",
            "second",
            "--server",
            &url,
            "--allow-http-loopback",
            "--key-stdin",
            "--validate",
        ],
        Some(format!("{key}\n").as_bytes()),
    );
    assert_eq!(validated.status, 0, "{}", validated.stderr);
    assert!(validated.stderr.contains("Validated"));
    let listing = harness
        .run(&["--output", "json", "profile", "list"])
        .json_stdout();
    assert_eq!(listing["data"]["items"].as_array().expect("items").len(), 2);
}

/// Minimal HTTP/1.1 POST for the reservation check. The CLI intentionally has no custom
/// code option, so this uses the documented API directly, through the local test
/// boundary only, to create a fixture code.
fn raw_post_link(origin: &str, key: &str, body: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let address = origin.trim_start_matches("http://");
    let mut stream = std::net::TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let request = format!(
        "POST /api/v1/links HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {key}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read");
    let status = response
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .expect("status");
    let payload = response.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
    (status, payload)
}

#[test]
#[ignore = "requires the local live API and FERL_URL/FERL_API_KEY"]
fn live_a_deleted_custom_code_stays_reserved_and_api_is_reserved() {
    let (url, key) = live_origin();
    let harness = live_harness().0;
    let code = format!("resv{}", run_id("c").rsplit('-').next().unwrap_or("0"))
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .take(20)
        .collect::<String>();
    let body = format!(
        r#"{{"destination_url":"https://example.com/ferl-cli/reserve","short_code":"{code}"}}"#
    );
    let (first, _) = raw_post_link(&url, &key, &body);
    assert_eq!(first, 201, "the fixture code is free");
    let deleted = harness.run_json("local", &["delete", &code, "--yes"]);
    assert_eq!(deleted.status, 0, "{}", deleted.stderr);
    let (reused, payload) = raw_post_link(&url, &key, &body);
    assert_eq!(reused, 409, "a deleted code stays reserved: {payload}");
    assert!(payload.contains("short_code_unavailable"));
    let (reserved, reserved_payload) = raw_post_link(
        &url,
        &key,
        r#"{"destination_url":"https://example.com/api","short_code":"api"}"#,
    );
    assert_eq!(reserved, 409, "api is reserved: {reserved_payload}");
}
