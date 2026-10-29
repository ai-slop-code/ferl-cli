//! Pseudo-terminal tests of interactive commands (business spec §§3.2, 7.4). A Python
//! driver runs the real `ferl` binary on a PTY, answers prompts in order, and reports
//! the exit status and transcript. The driver enforces its own deadline and kills the
//! child on any failure, so a stuck prompt cannot hang the test run. python3 is a
//! required prerequisite for these tests, not an optional skip.

mod common;

use std::process::Command;

use common::{Harness, KEY, MockServer, Reply, page_json};

const DRIVER: &str = r#"
import fcntl, json, os, pty, select, signal, struct, sys, termios, time

binary, argv_json, steps_json, home = sys.argv[1:5]
argv = json.loads(argv_json)
steps = json.loads(steps_json)
env = dict(PATH=os.environ.get("PATH", ""), HOME=home, LANG="C.UTF-8", TERM="dumb")
deadline = time.time() + 30
pid, fd = pty.fork()
if pid == 0:
    os.execve(binary, [binary] + argv, env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 100, 0, 0))

transcript = bytearray()
exited = None

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

def expect(text):
    while time.time() < deadline:
        if not text or text.encode() in transcript:
            return True
        read_for(0.1)
    return False

ok = True
for expected, answer in steps:
    if not expect(expected):
        ok = False
        break
    os.write(fd, answer.encode())
    read_for(0.2)

read_for(0.5)
code = None
while time.time() < deadline:
    done, status = os.waitpid(pid, os.WNOHANG)
    if done:
        code = os.waitstatus_to_exitcode(status)
        break
    read_for(0.1)
if code is None:
    ok = False
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    code = -9
print(json.dumps({"ok": ok, "exit": code, "transcript": transcript.decode("utf-8", "replace")}))
"#;

struct PtyRun {
    ok: bool,
    exit: i64,
    transcript: String,
}

fn require_python() {
    assert!(
        Command::new("python3").arg("--version").output().is_ok(),
        "python3 is required for the PTY driver"
    );
}

fn drive(harness: &Harness, argv: &[&str], steps: &[(&str, &str)]) -> PtyRun {
    require_python();
    let driver = harness.path("pty-driver.py");
    std::fs::write(&driver, DRIVER).expect("write driver");
    let config = harness.config.to_str().expect("utf-8 path");
    let mut full = vec!["--config", config];
    full.extend_from_slice(argv);
    let steps_json: Vec<[&str; 2]> = steps.iter().map(|(e, a)| [*e, *a]).collect();
    let output = Command::new("python3")
        .arg(&driver)
        .arg(env!("CARGO_BIN_EXE_ferl"))
        .arg(serde_json::to_string(&full).expect("argv json"))
        .arg(serde_json::to_string(&steps_json).expect("steps json"))
        .arg(harness.dir.path().join("home"))
        .output()
        .expect("run PTY driver");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let value: serde_json::Value = serde_json::from_str(text.lines().last().unwrap_or("{}"))
        .unwrap_or_else(|_| panic!("driver output: {text}"));
    PtyRun {
        ok: value["ok"].as_bool().unwrap_or(false),
        exit: value["exit"].as_i64().unwrap_or(-1),
        transcript: value["transcript"].as_str().unwrap_or("").to_owned(),
    }
}

#[test]
fn interactive_delete_default_no_sends_no_request() {
    let server = MockServer::start(|_| Reply::Respond {
        status: 204,
        headers: Vec::new(),
        body: Vec::new(),
    });
    let harness = Harness::new();
    assert_eq!(harness.add_profile("pty-del", &server.url(), KEY).status, 0);
    let run = drive(
        &harness,
        &["--profile", "pty-del", "delete", "a1b2c3d4"],
        &[("Permanently delete code a1b2c3d4", "\r")],
    );
    assert!(run.ok, "prompt did not appear: {}", run.transcript);
    assert_eq!(
        run.exit, 130,
        "a default answer declines: {}",
        run.transcript
    );
    assert!(
        run.transcript.contains("using profile pty-del at")
            && run.transcript.contains(&server.url()),
        "prompt names the profile and origin: {}",
        run.transcript
    );
    assert!(!run.transcript.contains(KEY), "the key is never echoed");
    assert!(
        server.requests().is_empty(),
        "declining sends no HTTP request"
    );
}

#[test]
fn interactive_delete_confirmation_sends_exactly_one_delete() {
    let server = MockServer::start(|_| Reply::Respond {
        status: 204,
        headers: Vec::new(),
        body: Vec::new(),
    });
    let harness = Harness::new();
    assert_eq!(harness.add_profile("pty-del", &server.url(), KEY).status, 0);
    let run = drive(
        &harness,
        &["--profile", "pty-del", "delete", "a1b2c3d4"],
        &[("Permanently delete code a1b2c3d4", "y\r")],
    );
    assert!(run.ok, "prompt did not appear: {}", run.transcript);
    assert_eq!(run.exit, 0, "transcript: {}", run.transcript);
    assert_eq!(
        server.count("DELETE"),
        1,
        "one DELETE after explicit confirmation"
    );
    assert_eq!(server.requests()[0].path(), "/api/v1/links/a1b2c3d4");
    assert!(!run.transcript.contains(KEY));
}

#[test]
fn wizard_cancellation_exits_130_and_commits_no_profile() {
    let server = MockServer::start(|_| Reply::json(200, &page_json(&[], None)));
    let harness = Harness::new();
    let run = drive(&harness, &["profile"], &[("Profile name", "\u{3}")]);
    assert_eq!(run.exit, 130, "transcript: {}", run.transcript);
    assert!(
        server.requests().is_empty(),
        "cancellation sends no request"
    );
    let listing = harness.run(&["--output", "json", "profile", "list"]);
    assert_eq!(
        listing.json_stdout()["data"]["items"],
        serde_json::json!([])
    );
    listing.assert_no_secret();
}

fn hostile_link(code: &str, title: serde_json::Value, visits: i64) -> serde_json::Value {
    serde_json::json!({
        "id": "1f1bf5f6-075c-4cf1-a44b-05fca986b100",
        "short_code": code,
        "short_url": format!("https://s.example.test/{code}"),
        "destination_url": "https://example.com/a/very/long/path?query=1#fragment",
        "title": title,
        "tags": ["work"],
        "owner_id": "4a8ffcd5-3066-4519-a68e-060f956ab500",
        "enabled": true,
        "active": true,
        "visit_count": visits,
        "created_at": "2026-10-29T10:00:00.123456Z",
        "updated_at": "2026-10-29T10:00:00.000000Z"
    })
}

#[test]
fn terminal_list_renders_hostile_titles_nulls_and_exact_counts_as_data() {
    let page = serde_json::json!({
        "items": [
            hostile_link("pty00001", serde_json::json!("\u{1b}]8;;http://evil.test\u{7}Click\u{1b}]8;;\u{7} \u{202e}rtl\nnext"), 3),
            hostile_link("pty00002", serde_json::Value::Null, 9_007_199_254_740_993),
        ],
        "next_cursor": null
    });
    let server = MockServer::start(move |_| Reply::json(200, &page.to_string()));
    let harness = Harness::new();
    assert_eq!(
        harness.add_profile("pty-list", &server.url(), KEY).status,
        0
    );
    let run = drive(&harness, &["--profile", "pty-list", "list"], &[("", "")]);
    assert_eq!(run.exit, 0, "transcript: {}", run.transcript);
    assert!(
        !run.transcript.contains('\u{1b}'),
        "terminal escapes are shown as data"
    );
    assert!(
        !run.transcript.contains('\u{202e}'),
        "bidi controls are shown as data"
    );
    assert!(
        run.transcript.contains("9007199254740993"),
        "exact count: {}",
        run.transcript
    );
    assert!(
        run.transcript.contains('—'),
        "null title placeholder: {}",
        run.transcript
    );
    for line in run.transcript.lines() {
        assert!(
            line.chars().count() <= 100,
            "row wider than the terminal: {line:?}"
        );
    }
    assert!(!run.transcript.contains(KEY));
}
