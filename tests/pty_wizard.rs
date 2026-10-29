//! Pseudo-terminal tests of the interactive profile wizard (business spec §3.2). A
//! Python driver allocates a PTY so that stdin, stdout, and stderr are terminals. It
//! types answers, checks prompts, and confirms that the key is hidden and absent from
//! the captured transcript. Skipped with a message when python3 is unavailable.

mod common;

use std::process::Command;

use common::{Harness, KEY, MockServer, Reply, account_json};

const DRIVER: &str = r#"
import json, os, pty, select, sys, time

binary, config, scenario, key, url = sys.argv[1:6]
env = dict(PATH=os.environ.get("PATH", ""), HOME=os.environ.get("HOME", ""), LANG="C.UTF-8", TERM="dumb")
pid, fd = pty.fork()
if pid == 0:
    os.execve(binary, [binary, "--config", config, "profile"], env)

transcript = bytearray()

def read_for(seconds):
    end = time.time() + seconds
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if ready:
            try:
                chunk = os.read(fd, 4096)
            except OSError:
                return
            if not chunk:
                return
            transcript.extend(chunk)

def expect(text, timeout=15):
    end = time.time() + timeout
    while time.time() < end:
        if text.encode() in transcript:
            return True
        read_for(0.1)
    return False

def send(line):
    os.write(fd, line.encode())
    read_for(0.2)

ok = True
if scenario == "add":
    steps = [
        ("Profile name", "pty-work\r"),
        ("Server origin", url + "\r"),
        ("Allow plaintext HTTP", "y\r"),
        ("Public short-link origin", "\r"),
        ("CA bundle path", "\r"),
        ("API key", key + "\r"),
        ("Validate the key now", "\r"),
        ("Save this profile", "\r"),
    ]
    for prompt, answer in steps:
        if not expect(prompt):
            ok = False
            break
        send(answer)
elif scenario == "menu-inspect-exit":
    if expect("Profiles"):
        send("\x1b[B\x1b[B\r")
        ok = expect("pty-work")
        if ok:
            send("\x1b[B" * 6 + "\r")
    else:
        ok = False

read_for(0.5)
_, status = os.waitpid(pid, 0)
code = os.waitstatus_to_exitcode(status)
print(json.dumps({"ok": ok, "exit": code, "transcript": transcript.decode("utf-8", "replace")}))
"#;

fn require_python() {
    assert!(
        Command::new("python3").arg("--version").output().is_ok(),
        "python3 is required for the PTY driver"
    );
}

fn drive(harness: &Harness, scenario: &str, url: &str) -> (bool, i64, String) {
    let driver = harness.path("driver.py");
    std::fs::write(&driver, DRIVER).expect("write driver");
    let output = Command::new("python3")
        .arg(&driver)
        .arg(env!("CARGO_BIN_EXE_ferl"))
        .arg(&harness.config)
        .arg(scenario)
        .arg(KEY)
        .arg(url)
        .output()
        .expect("run driver");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let value: serde_json::Value = serde_json::from_str(text.lines().last().unwrap_or("{}"))
        .unwrap_or_else(|_| {
            panic!(
                "driver output: {text} stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        });
    (
        value["ok"].as_bool().unwrap_or(false),
        value["exit"].as_i64().unwrap_or(-1),
        value["transcript"].as_str().unwrap_or("").to_owned(),
    )
}

#[test]
fn wizard_adds_a_profile_with_hidden_key_entry_and_saves_it() {
    require_python();
    let server = MockServer::start(|_| Reply::json(200, &account_json("ferl_cli_admin", "admin")));
    let harness = Harness::new();
    let (ok, exit, transcript) = drive(&harness, "add", &server.url());
    assert!(
        ok,
        "a wizard prompt did not appear; transcript: {transcript}"
    );
    assert_eq!(exit, 0, "transcript: {transcript}");
    assert!(!transcript.contains(KEY), "the key must never be echoed");
    assert!(
        transcript.contains("Validated: account ferl_cli_admin"),
        "validation runs: {transcript}"
    );
    assert!(
        transcript.contains("Saved profile pty-work"),
        "{transcript}"
    );
    let listing = harness.run(&["--output", "json", "profile", "list"]);
    assert_eq!(
        listing.json_stdout()["data"]["items"][0]["name"],
        "pty-work"
    );
    listing.assert_no_secret();
    assert_eq!(server.requests().len(), 1, "exactly one validation read");
}

#[test]
fn wizard_menu_inspects_profiles_and_exits_cleanly_without_changes() {
    require_python();
    let server = MockServer::start(|_| Reply::json(200, &account_json("ferl_cli_admin", "admin")));
    let harness = Harness::new();
    let added = harness.add_profile("pty-work", &server.url(), KEY);
    assert_eq!(added.status, 0);
    let before = std::fs::read(harness.path("config.toml")).expect("read");
    let (ok, exit, transcript) = drive(&harness, "menu-inspect-exit", &server.url());
    assert!(
        ok,
        "menu did not show the profile; transcript: {transcript}"
    );
    assert_eq!(exit, 0, "transcript: {transcript}");
    assert!(!transcript.contains(KEY));
    assert_eq!(
        std::fs::read(harness.path("config.toml")).expect("read"),
        before,
        "inspection changes nothing"
    );
    assert_eq!(server.requests().len(), 0);
}
