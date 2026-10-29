//! Desktop clipboard smoke test (business spec §9.3, acceptance AC16). It needs a
//! graphical macOS session and overwrites the clipboard, so it is ignored by default
//! and runs only on disposable CI runners. Run it with:
//!
//! ```sh
//! cargo test --locked --test clipboard_desktop -- --ignored --test-threads=1
//! ```

#![cfg(target_os = "macos")]

mod common;

use std::process::Command;

use common::{Harness, KEY, MockServer, Reply, link_json};

const CODE: &str = "clip0001";

#[test]
#[ignore = "overwrites the desktop clipboard; run only on disposable macOS runners"]
fn explicit_copy_leaves_the_exact_returned_url_on_the_macos_clipboard() {
    let server = MockServer::start(|_| {
        Reply::json(
            201,
            &format!(r#"{{"link":{}}}"#, link_json(CODE, None, "0")),
        )
    });
    let harness = Harness::new();
    assert_eq!(harness.add_profile("clip", &server.url(), KEY).status, 0);
    let run = harness.run(&[
        "--profile",
        "clip",
        "https://example.com/clipboard",
        "--copy",
    ]);
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let expected = format!("https://s.example.com/{CODE}");
    assert_eq!(run.stdout, format!("{expected}\n"));

    let pasted = Command::new("/usr/bin/pbpaste")
        .output()
        .expect("pbpaste runs on macOS");
    assert_eq!(
        String::from_utf8_lossy(&pasted.stdout),
        expected,
        "the clipboard holds exactly the returned URL, with no trailing newline added"
    );
}
