//! SIGINT behaviour: a later-page interruption yields a valid partial list with exit 130,
//! and an interrupted unacknowledged creation is an uncertain outcome (exit 13).

mod common;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Harness, KEY, MockServer, Reply, link_json, page_json};

fn interrupt(child_id: u32) {
    let _ = Command::new("kill")
        .arg("-INT")
        .arg(child_id.to_string())
        .status();
}

fn spawn_with_config(harness: &Harness, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_ferl"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", harness.dir.path().join("home"))
        .arg("--config")
        .arg(&harness.config)
        .arg("--non-interactive")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn")
}

#[test]
fn interrupt_during_a_later_page_emits_valid_partial_output_and_exits_130() {
    let server = MockServer::start(|request| {
        if request.query_values("cursor").is_empty() {
            let links = vec![link_json("sig00001", None, "0")];
            Reply::json(200, &page_json(&links, Some("sig-next")))
        } else {
            Reply::Delay(
                Duration::from_secs(30),
                Box::new(Reply::json(200, &page_json(&[], None))),
            )
        }
    });
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let child = spawn_with_config(
        &harness,
        &["--profile", "work", "--output", "json", "list", "--all"],
    );
    let id = child.id();
    let started = Instant::now();
    while server.requests().len() < 2 && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    interrupt(id);
    let output = child.wait_with_output().expect("wait");
    assert_eq!(
        output.status.code(),
        Some(130),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "interruption must be prompt"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid partial JSON");
    assert_eq!(value["meta"]["stop_reason"], "cancelled");
    assert_eq!(value["meta"]["complete"], false);
    assert_eq!(value["data"]["items"].as_array().expect("items").len(), 1);
    assert_eq!(value["data"]["next_cursor"], "sig-next");
}

#[test]
fn interrupt_before_any_page_leaves_stdout_empty() {
    let server = MockServer::start(|_| {
        Reply::Delay(
            Duration::from_secs(30),
            Box::new(Reply::json(200, &page_json(&[], None))),
        )
    });
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let child = spawn_with_config(&harness, &["--profile", "work", "--output", "json", "list"]);
    let id = child.id();
    let started = Instant::now();
    while server.requests().is_empty() && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    interrupt(id);
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(130));
    assert!(output.stdout.is_empty());
}

#[test]
fn interrupt_after_a_create_was_sent_is_uncertain_and_not_replayed() {
    let server = MockServer::start(|_| {
        Reply::Delay(
            Duration::from_secs(30),
            Box::new(Reply::json(
                201,
                &format!(r#"{{"link":{}}}"#, link_json("sig00002", None, "0")),
            )),
        )
    });
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let child = spawn_with_config(
        &harness,
        &[
            "--profile",
            "work",
            "--output",
            "json",
            "create",
            "https://example.com/interrupted",
            "--no-copy",
        ],
    );
    let id = child.id();
    let started = Instant::now();
    while server.requests().is_empty() && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    interrupt(id);
    let output = child.wait_with_output().expect("wait");
    assert_eq!(
        output.status.code(),
        Some(13),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_eq!(server.count("POST"), 1);
}
