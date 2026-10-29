//! Resource bounds for `list --all` (business spec §8, acceptance AC10).
//!
//! These tests are deliberately expensive: they serve hundreds of MiB of synthetic
//! pages over loopback and run the real `ferl` binary under the platform's `time`
//! utility, so peak resident memory is that of the CLI child alone. They are ignored
//! by default. Run them explicitly, one at a time:
//!
//! ```sh
//! cargo test --locked --test list_resource -- --ignored --test-threads=1
//! ```
//!
//! They require `/usr/bin/time` (`-l` on macOS, `-v` on Linux).

mod common;

use std::fs::File;
use std::io::BufReader;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use common::{Harness, KEY, MockServer, Reply, page_json};
use serde::Deserialize;

const MIB: u64 = 1024 * 1024;
const LINKS_PER_PAGE: usize = 200;
const SPOOL_TOTAL_PAGES: usize = 500;
const OVER_BOUND_TOTAL_PAGES: usize = 700;
const STAGING_OBSERVED_PAGE: usize = 3;

fn link_body(page: usize, index: usize) -> String {
    let code = format!("p{page:06}-{index:03}");
    let title = "x".repeat(200);
    let path = "a".repeat(1500);
    let tags: Vec<String> = (0..10)
        .map(|t| format!("\"tag-{t:02}-{}\"", "b".repeat(30)))
        .collect();
    let visits = 1_000_000_000 + page * LINKS_PER_PAGE + index;
    format!(
        r#"{{"id":"1f1bf5f6-075c-4cf1-a44b-05fca986b100","short_code":"{code}","short_url":"https://s.example.com/{code}","destination_url":"https://example.com/{path}/{code}","title":"{title}","tags":[{}],"owner_id":"4a8ffcd5-3066-4519-a68e-060f956ab500","enabled":true,"active":true,"visit_count":{visits},"created_at":"2026-10-29T10:00:00.123456Z","updated_at":"2026-10-29T10:00:00.000000Z"}}"#,
        tags.join(",")
    )
}

fn page_reply(page: usize, total_pages: usize) -> String {
    let links: Vec<String> = (0..LINKS_PER_PAGE)
        .map(|index| link_body(page, index))
        .collect();
    let next = (page + 1 < total_pages).then(|| format!("c{:06}", page + 1));
    page_json(&links, next.as_deref())
}

fn requested_page(request: &common::Request) -> usize {
    request
        .query_values("cursor")
        .first()
        .map(|c| c.trim_start_matches('c').parse().expect("cursor page"))
        .unwrap_or(0)
}

#[derive(Deserialize)]
struct Envelope {
    schema_version: u32,
    command: String,
    data: Data,
    meta: Meta,
}

#[derive(Deserialize)]
struct Data {
    items: Vec<CodeOnly>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct CodeOnly {
    short_code: String,
}

#[derive(Deserialize)]
struct Meta {
    pages_fetched: u64,
    complete: bool,
    truncated: bool,
    stop_reason: String,
}

struct Measured {
    status: i32,
    peak_rss_bytes: u64,
    output_bytes: u64,
    stdout_path: PathBuf,
}

fn peak_rss_bytes(stderr: &str) -> u64 {
    for line in stderr.lines() {
        if cfg!(target_os = "macos") && line.contains("maximum resident set size") {
            // macOS `time -l` reports bytes.
            return line
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .expect("macOS RSS value");
        }
        if cfg!(target_os = "linux") && line.contains("Maximum resident set size (kbytes)") {
            return line
                .rsplit(' ')
                .next()
                .and_then(|n| n.parse::<u64>().ok())
                .expect("Linux RSS value")
                * 1024;
        }
    }
    panic!("no peak RSS line in time output:\n{stderr}");
}

fn spool_entries(tmp: &Path) -> Vec<String> {
    std::fs::read_dir(tmp)
        .expect("read tmp")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("ferl-spool-"))
        .collect()
}

/// Staging is inspected from inside the fixture while the CLI is still fetching pages.
fn staging_modes(tmp: &Path) -> Option<(u32, u32)> {
    let dir = std::fs::read_dir(tmp)
        .expect("read tmp")
        .map(|e| e.expect("entry").path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("ferl-spool-"))
        })?;
    let dir_mode = std::fs::metadata(&dir)
        .expect("dir meta")
        .permissions()
        .mode()
        & 0o777;
    let file = std::fs::read_dir(&dir)
        .expect("read spool dir")
        .next()?
        .expect("entry")
        .path();
    let file_mode = std::fs::metadata(&file)
        .expect("file meta")
        .permissions()
        .mode()
        & 0o777;
    Some((dir_mode, file_mode))
}

fn run_measured(harness: &Harness, tmp: &Path, format_args: &[&str]) -> Measured {
    let stdout_path = harness.path(&format!("out-{}.txt", format_args.join("-")));
    let stderr_path = harness.path(&format!("time-{}.txt", format_args.join("-")));
    let mut command = Command::new("/usr/bin/time");
    command.arg(if cfg!(target_os = "macos") {
        "-l"
    } else {
        "-v"
    });
    command
        .arg(env!("CARGO_BIN_EXE_ferl"))
        .arg("--config")
        .arg(&harness.config)
        .arg("--non-interactive")
        .args(["--timeout", "600"])
        .args(format_args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", harness.dir.path().join("home"))
        .env("LANG", "C.UTF-8")
        .env("TMPDIR", tmp)
        .stdin(Stdio::null())
        .stdout(File::create(&stdout_path).expect("stdout file"))
        .stderr(File::create(&stderr_path).expect("stderr file"));
    let status = command.status().expect("run time");
    let stderr = std::fs::read_to_string(&stderr_path).expect("stderr");
    let output_bytes = std::fs::metadata(&stdout_path).expect("output").len();
    Measured {
        status: status.code().unwrap_or(-1),
        peak_rss_bytes: peak_rss_bytes(&stderr),
        output_bytes,
        stdout_path,
    }
}

fn private_tmp(harness: &Harness) -> PathBuf {
    let tmp = harness.path("tmp");
    std::fs::create_dir(&tmp).expect("tmp");
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    tmp
}

fn start_fixture(
    total_pages: usize,
    tmp: PathBuf,
    staging: Arc<Mutex<Option<(u32, u32)>>>,
    wire_bytes: Arc<AtomicU64>,
) -> MockServer {
    MockServer::start(move |request| {
        assert_eq!(request.path(), "/api/v1/links");
        assert_eq!(request.query_values("scope"), ["own"]);
        let page = requested_page(request);
        assert!(page < total_pages, "requested beyond the fixture: {page}");
        if page == STAGING_OBSERVED_PAGE {
            *staging.lock().expect("lock") = staging_modes(&tmp);
        }
        let body = page_reply(page, total_pages).into_bytes();
        wire_bytes.fetch_add(body.len() as u64, Ordering::Relaxed);
        Reply::Respond {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body,
        }
    })
}

fn expected_code(index: usize) -> String {
    format!(
        "p{:06}-{:03}",
        index / LINKS_PER_PAGE,
        index % LINKS_PER_PAGE
    )
}

fn read_envelope(path: &Path) -> Envelope {
    let file = BufReader::new(File::open(path).expect("output"));
    serde_json::from_reader(file).expect("bounded output is valid JSON")
}

fn assert_ordered(items: &[CodeOnly]) {
    for (index, item) in items.iter().enumerate() {
        assert_eq!(item.short_code, expected_code(index), "order at {index}");
    }
}

struct Fixture {
    harness: Harness,
    tmp: PathBuf,
    staging: Arc<Mutex<Option<(u32, u32)>>>,
    wire: Arc<AtomicU64>,
    server: MockServer,
}

fn setup(total_pages: usize) -> Fixture {
    let harness = Harness::new();
    let tmp = private_tmp(&harness);
    let staging = Arc::new(Mutex::new(None));
    let wire = Arc::new(AtomicU64::new(0));
    let server = start_fixture(total_pages, tmp.clone(), staging.clone(), wire.clone());
    let added = harness.add_profile("bulk", &server.url(), KEY);
    assert_eq!(added.status, 0, "profile add failed");
    Fixture {
        harness,
        tmp,
        staging,
        wire,
        server,
    }
}

#[test]
#[ignore = "resource measurement; run explicitly with --ignored"]
fn fetch_all_200_mib_spool_keeps_cli_peak_memory_under_128_mib() {
    let Fixture {
        harness,
        tmp,
        staging,
        wire,
        server,
    } = setup(SPOOL_TOTAL_PAGES);
    let gets_before = server.count("GET");

    let json = run_measured(
        &harness,
        &tmp,
        &[
            "--output",
            "json",
            "list",
            "--all",
            "--limit",
            "200",
            "--max-pages",
            "1000",
        ],
    );
    assert_eq!(json.status, 0);
    assert_eq!(
        server.count("GET") - gets_before,
        SPOOL_TOTAL_PAGES,
        "one GET per page"
    );
    let envelope = read_envelope(&json.stdout_path);
    assert_eq!(envelope.schema_version, 1);
    assert_eq!(envelope.command, "list");
    assert_eq!(envelope.meta.pages_fetched, SPOOL_TOTAL_PAGES as u64);
    assert!(envelope.meta.complete && !envelope.meta.truncated);
    assert_eq!(envelope.meta.stop_reason, "end");
    assert_eq!(envelope.data.next_cursor, None);
    assert_eq!(
        envelope.data.items.len(),
        SPOOL_TOTAL_PAGES * LINKS_PER_PAGE
    );
    assert_ordered(&envelope.data.items);
    assert!(
        json.output_bytes >= 200 * MIB,
        "spool payload {} < 200 MiB",
        json.output_bytes
    );
    assert!(
        json.peak_rss_bytes < 128 * MIB,
        "peak RSS {} bytes is not below 128 MiB",
        json.peak_rss_bytes
    );
    let staged = staging.lock().expect("lock").expect("staging observed");
    assert_eq!(staged, (0o700, 0o600), "dir and file modes while staging");
    assert!(
        spool_entries(&tmp).is_empty(),
        "staging left behind after success"
    );
    eprintln!(
        "AC10 200 MiB JSON: exit {} output {} bytes, peak RSS {} bytes ({:.1} MiB), wire bodies {} bytes, staging dir/file modes {:o}/{:o}",
        json.status,
        json.output_bytes,
        json.peak_rss_bytes,
        json.peak_rss_bytes as f64 / MIB as f64,
        wire.load(Ordering::Relaxed),
        staged.0,
        staged.1
    );
    let _ = std::fs::remove_file(&json.stdout_path);

    let human = run_measured(
        &harness,
        &tmp,
        &["list", "--all", "--limit", "200", "--max-pages", "1000"],
    );
    assert_eq!(human.status, 0);
    let rows = std::fs::read_to_string(&human.stdout_path)
        .expect("human output")
        .lines()
        .count();
    assert_eq!(
        rows,
        SPOOL_TOTAL_PAGES * LINKS_PER_PAGE + 1,
        "heading plus one row per link"
    );
    assert!(
        human.peak_rss_bytes < 128 * MIB,
        "human peak RSS {} bytes is not below 128 MiB",
        human.peak_rss_bytes
    );
    assert!(
        spool_entries(&tmp).is_empty(),
        "staging left behind after human output"
    );
    eprintln!(
        "AC10 200 MiB human: exit {} output {} bytes, peak RSS {} bytes ({:.1} MiB)",
        human.status,
        human.output_bytes,
        human.peak_rss_bytes,
        human.peak_rss_bytes as f64 / MIB as f64
    );
}

#[test]
#[ignore = "resource measurement; run explicitly with --ignored"]
fn fetch_all_stops_at_the_256_mib_aggregate_bound_with_valid_partial_output() {
    let Fixture {
        harness,
        tmp,
        server,
        ..
    } = setup(OVER_BOUND_TOTAL_PAGES);

    let run = run_measured(
        &harness,
        &tmp,
        &[
            "--output",
            "json",
            "list",
            "--all",
            "--limit",
            "200",
            "--max-pages",
            "1000",
        ],
    );
    assert_eq!(run.status, 12, "size-limit stop is a partial result");
    let envelope = read_envelope(&run.stdout_path);
    assert!(!envelope.meta.complete && envelope.meta.truncated);
    assert_eq!(envelope.meta.stop_reason, "size_limit");

    let accepted = envelope.meta.pages_fetched as usize;
    assert!(accepted > 0 && accepted < OVER_BOUND_TOTAL_PAGES);
    assert_eq!(
        server.count("GET"),
        accepted + 1,
        "accepted pages plus one excluded probe"
    );
    assert_eq!(envelope.data.items.len(), accepted * LINKS_PER_PAGE);
    assert_ordered(&envelope.data.items);
    assert_eq!(
        envelope.data.next_cursor.as_deref(),
        Some(format!("c{accepted:06}").as_str()),
        "continuation requests the first excluded page"
    );

    let bound = 256 * MIB;
    let per_page = run.output_bytes / accepted as u64;
    assert!(
        run.output_bytes <= bound + 4096,
        "accepted {} exceeds the bound",
        run.output_bytes
    );
    assert!(
        run.output_bytes + per_page > bound,
        "one more page would have fit: output {} page {}",
        run.output_bytes,
        per_page
    );
    assert!(
        run.peak_rss_bytes < 128 * MIB,
        "peak RSS {} bytes is not below 128 MiB",
        run.peak_rss_bytes
    );
    assert!(
        spool_entries(&tmp).is_empty(),
        "staging left behind after size bound"
    );
    eprintln!(
        "AC10 256 MiB bound: exit {} accepted pages {} items {} output {} bytes, peak RSS {} bytes ({:.1} MiB)",
        run.status,
        accepted,
        envelope.data.items.len(),
        run.output_bytes,
        run.peak_rss_bytes,
        run.peak_rss_bytes as f64 / MIB as f64
    );
    let _ = std::fs::remove_file(&run.stdout_path);
}
