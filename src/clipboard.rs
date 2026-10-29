//! Clipboard copy through native helper executables. The URL is written to the
//! helper's stdin; no shell is used. Failure is reported to the caller, which keeps
//! the successful creation (business spec §9.3).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const CLIPBOARD_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helper {
    pub program: PathBuf,
    pub args: Vec<String>,
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Ordered helper candidates for this platform and session (business spec §9.3).
pub fn platform_helpers() -> Vec<Helper> {
    if cfg!(target_os = "macos") {
        let pbcopy = PathBuf::from("/usr/bin/pbcopy");
        return vec![Helper {
            program: pbcopy,
            args: Vec::new(),
        }];
    }
    let mut helpers = Vec::new();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
    if wayland {
        if let Some(program) = find_in_path("wl-copy") {
            helpers.push(Helper {
                program,
                args: Vec::new(),
            });
        }
    }
    if x11 {
        if let Some(program) = find_in_path("xclip") {
            helpers.push(Helper {
                program,
                args: vec!["-selection".to_owned(), "clipboard".to_owned()],
            });
        }
        if let Some(program) = find_in_path("xsel") {
            helpers.push(Helper {
                program,
                args: vec!["--clipboard".to_owned(), "--input".to_owned()],
            });
        }
    }
    helpers
}

/// Copies `text` using the first helper that succeeds. Returns a short reason on
/// failure; the reason never includes the copied text.
pub fn copy_with(helpers: &[Helper], text: &str) -> Result<(), String> {
    if helpers.is_empty() {
        return Err("no clipboard helper is available in this session".to_owned());
    }
    let mut last = String::from("the clipboard helper failed");
    for helper in helpers {
        match run_helper(helper, text.as_bytes()) {
            Ok(()) => return Ok(()),
            Err(reason) => last = reason,
        }
    }
    Err(last)
}

pub fn copy(text: &str) -> Result<(), String> {
    copy_with(&platform_helpers(), text)
}

fn run_helper(helper: &Helper, bytes: &[u8]) -> Result<(), String> {
    let mut child = Command::new(&helper.program)
        .args(&helper.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "the clipboard helper could not be started".to_owned())?;
    let started = Instant::now();
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(bytes).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("the clipboard helper closed its input".to_owned());
        }
    }
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err("the clipboard helper reported an error".to_owned())
                };
            }
            Ok(None) => {
                if started.elapsed() >= CLIPBOARD_BUDGET {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("the clipboard helper did not finish in time".to_owned());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return Err("the clipboard helper status is unavailable".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[test]
    fn helper_receives_exact_bytes_on_stdin_without_a_shell() {
        let dir = tempfile::tempdir().expect("dir");
        let out = dir.path().join("captured");
        let helper = script(dir.path(), "spy", &format!("cat > '{}'", out.display()));
        let text = "https://s.example.com/abc?x=1;rm -rf $HOME";
        copy_with(
            &[Helper {
                program: helper,
                args: vec![],
            }],
            text,
        )
        .expect("copied");
        assert_eq!(std::fs::read(&out).expect("captured"), text.as_bytes());
    }

    #[test]
    fn failing_helper_falls_through_to_the_next_candidate() {
        let dir = tempfile::tempdir().expect("dir");
        let bad = script(dir.path(), "bad", "exit 1");
        let good = script(dir.path(), "good", "cat > /dev/null");
        let helpers = [
            Helper {
                program: bad,
                args: vec![],
            },
            Helper {
                program: good,
                args: vec![],
            },
        ];
        assert!(copy_with(&helpers, "https://s.example.com/a").is_ok());
    }

    #[test]
    fn missing_helpers_are_a_reported_nonfatal_failure() {
        let dir = tempfile::tempdir().expect("dir");
        let missing = Helper {
            program: dir.path().join("does-not-exist"),
            args: vec![],
        };
        assert!(copy_with(&[missing], "https://s.example.com/a").is_err());
        assert!(copy_with(&[], "https://s.example.com/a").is_err());
    }

    #[test]
    fn slow_helpers_are_bounded_by_the_clipboard_budget() {
        let dir = tempfile::tempdir().expect("dir");
        let slow = script(dir.path(), "slow", "cat > /dev/null; sleep 10");
        let started = Instant::now();
        let result = copy_with(
            &[Helper {
                program: slow,
                args: vec![],
            }],
            "https://s.example.com/a",
        );
        assert!(result.is_err());
        assert!(started.elapsed() < CLIPBOARD_BUDGET + Duration::from_millis(1500));
    }
}
