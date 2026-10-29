//! Output contracts: stable JSON v1 envelopes, JSON Lines diagnostics on stderr,
//! and sanitized human text. Untrusted text is always rendered as data.

use std::io::{self, Write};

use serde::Serialize;
use serde_json::{Value, json};

use crate::error::CliError;

pub const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    Json,
}

/// Diagnostics for one command invocation. Human records go to stderr as prose;
/// JSON records go to stderr as JSON Lines (business spec §7.3).
#[derive(Debug, Clone)]
pub struct Reporter {
    pub mode: OutputMode,
    pub command: String,
    pub profile: Option<String>,
}

impl Reporter {
    pub fn new(mode: OutputMode, command: &str, profile: Option<&str>) -> Self {
        Reporter {
            mode,
            command: command.to_owned(),
            profile: profile.map(str::to_owned),
        }
    }

    pub fn warn(&self, code: &str, message: &str) {
        let mut stderr = io::stderr().lock();
        let _ = match self.mode {
            OutputMode::Json => write_json_line(
                &mut stderr,
                &json!({
                    "schema_version": SCHEMA_VERSION,
                    "type": "warning",
                    "command": self.command,
                    "profile": self.profile,
                    "code": code,
                    "message": message,
                }),
            ),
            OutputMode::Human => writeln!(stderr, "warning: {}", sanitize(message)),
        };
    }

    /// Notices that are informational in human mode only (success messages).
    pub fn notice(&self, message: &str) {
        if self.mode == OutputMode::Human {
            let mut stderr = io::stderr().lock();
            let _ = writeln!(stderr, "{}", sanitize(message));
        }
    }

    pub fn error(&self, err: &CliError) {
        let mut stderr = io::stderr().lock();
        match self.mode {
            OutputMode::Json => {
                let _ = write_json_line(&mut stderr, &error_record(self, err));
            }
            OutputMode::Human => {
                let _ = write_human_error(&mut stderr, err);
            }
        }
    }
}

pub fn error_record(reporter: &Reporter, err: &CliError) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "type": "error",
        "command": reporter.command,
        "profile": reporter.profile,
        "exit_code": err.exit_code(),
        "error": {
            "kind": err.kind.as_str(),
            "code": err.code,
            "message": &err.message,
        },
        "http_status": err.http_status,
        "request_id": err.request_id,
        "retry_after_seconds": err.retry_after,
        "mutation_outcome": err.outcome.as_str(),
        "partial": err.partial,
        "pages_fetched": err.pages_fetched,
    })
}

fn write_human_error(out: &mut impl Write, err: &CliError) -> io::Result<()> {
    writeln!(out, "ferl: error: {}", sanitize(&err.message))?;
    let mut details = Vec::new();
    if let Some(status) = err.http_status {
        details.push(format!("HTTP {status}"));
    }
    if err.code.starts_with("client.") {
        // Client-generated codes are internal identifiers; omit them from prose.
    } else {
        details.push(format!("server code {}", sanitize(&err.code)));
    }
    if let Some(id) = &err.request_id {
        details.push(format!("request ID {}", sanitize(id)));
    }
    if let Some(seconds) = err.retry_after {
        details.push(format!("retry after {seconds} second(s)"));
    }
    if !details.is_empty() {
        writeln!(out, "  {}", details.join(", "))?;
    }
    Ok(())
}

pub fn write_json_line(out: &mut impl Write, value: &Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    out.write_all(&line)
}

/// Writes one stdout result envelope. Propagates write failures so the caller
/// can classify them as local output failures.
pub fn write_result<D: Serialize, M: Serialize>(
    command: &str,
    profile: Option<&str>,
    data: &D,
    meta: &M,
) -> io::Result<()> {
    let envelope = json!({
        "schema_version": SCHEMA_VERSION,
        "command": command,
        "profile": profile,
        "data": serde_json::to_value(data).map_err(io::Error::other)?,
        "meta": serde_json::to_value(meta).map_err(io::Error::other)?,
    });
    let mut stdout = io::stdout().lock();
    write_json_line(&mut stdout, &envelope)?;
    stdout.flush()
}

/// Escapes terminal controls, bidirectional controls, and line breaks so that
/// untrusted text cannot alter the terminal. Printable Unicode is preserved.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if is_unsafe_for_terminal(ch) {
            out.push_str(&format!("\\u{{{:04x}}}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn is_unsafe_for_terminal(ch: char) -> bool {
    let code = ch as u32;
    code < 0x20
        || (0x7f..=0x9f).contains(&code)
        || matches!(code, 0x061c | 0x200e | 0x200f | 0x202a..=0x202e | 0x2066..=0x2069)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_controls_are_escaped_as_data() {
        let hostile = "ok\x1b]8;;https://evil\x07link\n\t\u{202e}x\u{9b}";
        let rendered = sanitize(hostile);
        assert!(!rendered.contains('\x1b'));
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\t'));
        assert!(!rendered.contains('\u{202e}'));
        assert!(!rendered.contains('\u{9b}'));
        assert!(rendered.contains("\\u{001b}"));
    }

    #[test]
    fn printable_unicode_is_preserved() {
        assert_eq!(sanitize("café — 東京 👍"), "café — 東京 👍");
    }
}
