//! Business commands: shorten (create), list, and delete. Each validates input locally,
//! selects one profile, reads the key only when a request is about to be sent, and
//! keeps stdout, stderr, and exit semantics as specified (business spec §§7–10).

use std::cell::RefCell;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use dialoguer::Confirm;
use serde_json::{Value, json};

use crate::api::{self, Link};
use crate::cli::{CopyChoice, CursorInput, DeleteOpts, ListOpts, ShortenOpts};
use crate::clipboard;
use crate::config::Store;
use crate::config::{ProfileTarget, resolve_profile, select_profile_name};
use crate::error::{CliError, Outcome};
use crate::input;
use crate::list::{self, ListOutcome, ListRequest, StopReason};
use crate::report::{OutputMode, Reporter, SCHEMA_VERSION, write_result};
use crate::secret::KeySource;
use crate::storage::write_private_file_atomic;
use crate::table::{DEFAULT_COLUMNS, TableLayout};
use crate::transport::{Limits, Transport};

/// Everything a command needs from the invocation.
pub struct Env {
    pub store: Store,
    pub reporter: Reporter,
    pub output: OutputMode,
    pub interactive: bool,
    pub limits: Limits,
    pub profile_flag: Option<String>,
    pub env_profile: Option<String>,
    pub key: Option<KeySource>,
    /// The profile name selected for this invocation, once known.
    pub selected: RefCell<Option<String>>,
}

impl Env {
    pub fn json(&self) -> bool {
        self.output == OutputMode::Json
    }

    fn profile_name(&self) -> Option<String> {
        self.selected.borrow().clone()
    }
}

/// Selects the profile and resolves its origins. The key is not read yet.
fn select_target(env: &Env) -> Result<ProfileTarget, CliError> {
    let (snapshot, _lock) = env.store.snapshot(false)?;
    let name = select_profile_name(
        &snapshot,
        env.profile_flag.as_deref(),
        env.env_profile.as_deref(),
    )?;
    *env.selected.borrow_mut() = Some(name.clone());
    resolve_profile(&snapshot, &name, env.key.as_ref())
}

fn transport_for(
    target: &ProfileTarget,
    key: &crate::secret::ApiKey,
    limits: Limits,
) -> Result<Transport, CliError> {
    Transport::new(
        &target.server,
        key.expose(),
        target.ca_bundle.as_deref(),
        limits,
    )
}

fn output_failure(error: io::Error, outcome: Outcome) -> CliError {
    CliError::local_io(format!("Could not write the command output: {error}."))
        .with_outcome(outcome)
}

pub fn shorten(env: &Env, opts: &ShortenOpts) -> Result<(), CliError> {
    input::validate_destination(&opts.destination)?;
    if let Some(title) = &opts.title {
        input::validate_title(title)?;
    }
    input::validate_tags(&opts.tags)?;

    let target = select_target(env)?;
    let key = target.read_key()?;
    let transport = transport_for(&target, &key, env.limits)?;
    let created = api::create_link(
        &transport,
        &opts.destination,
        opts.title.as_deref(),
        &opts.tags,
    )?;
    let short_url = created.link.short_url.clone();

    let copy_requested = match opts.copy {
        CopyChoice::Always => true,
        CopyChoice::Never => false,
        CopyChoice::Auto => env.interactive,
    };

    if env.json() {
        let clipboard_status = if copy_requested {
            copy_status(env, &short_url)
        } else {
            "skipped"
        };
        write_result(
            "create",
            env.profile_name().as_deref(),
            &json!({ "link": created.link }),
            &json!({ "request_id": created.request_id, "clipboard": clipboard_status }),
        )
        .map_err(|e| output_failure(e, Outcome::Committed))?;
        return Ok(());
    }

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{short_url}")
        .and_then(|_| stdout.flush())
        .map_err(|e| output_failure(e, Outcome::Committed))?;
    drop(stdout);

    if copy_requested {
        match clipboard::copy(&short_url) {
            Ok(()) => env.reporter.notice("Copied short URL to clipboard."),
            Err(reason) => env.reporter.warn(
                "client.clipboard_unavailable",
                &format!("The short URL was not copied: {reason}."),
            ),
        }
    }
    Ok(())
}

/// Copies and reports the clipboard status for JSON output without any extra request.
fn copy_status(env: &Env, short_url: &str) -> &'static str {
    match clipboard::copy(short_url) {
        Ok(()) => "copied",
        Err(reason) => {
            env.reporter.warn(
                "client.clipboard_unavailable",
                &format!("The short URL was not copied: {reason}."),
            );
            "failed"
        }
    }
}

fn read_cursor_file(path: &Path) -> Result<String, CliError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CliError::local_io(format!("Could not read the cursor file: {e}.")))?;
    let value: Value = serde_json::from_str(text.trim())
        .map_err(|_| CliError::usage("The cursor file must contain a JSON string or null."))?;
    match value {
        Value::String(token) => {
            input::validate_cursor(&token)?;
            Ok(token)
        }
        Value::Null => Err(CliError::usage(
            "The cursor file contains null, meaning there is no continuation. No page was fetched.",
        )),
        _ => Err(CliError::usage(
            "The cursor file must contain a JSON string or null.",
        )),
    }
}

fn terminal_width() -> Option<usize> {
    rustix::termios::tcgetwinsize(io::stdout())
        .ok()
        .map(|size| usize::from(size.ws_col))
        .filter(|w| *w >= 20)
        .or(Some(120))
}

pub fn list(env: &Env, opts: &ListOpts) -> Result<(), CliError> {
    let search = match &opts.query {
        Some(text) => Some(input::normalize_search(text)?),
        None => None,
    };
    let start_cursor = match &opts.cursor {
        None => None,
        Some(CursorInput::Literal(token)) => {
            input::validate_cursor(token)?;
            Some(token.clone())
        }
        Some(CursorInput::File(path)) => Some(read_cursor_file(path)?),
    };

    let target = select_target(env)?;
    let key = target.read_key()?;
    let transport = transport_for(&target, &key, env.limits)?;
    let request = ListRequest {
        search,
        limit: opts.limit,
        start_cursor,
        fetch_all: opts.all,
        max_pages: opts.max_pages,
    };
    let mut outcome = list::run(&transport, &request)?;

    if env.json() {
        write_list_json(env, &outcome)?;
    } else {
        write_list_human(env, opts, &mut outcome)?;
    }

    if let Some(path) = &opts.save_cursor {
        let token = outcome.next_cursor.clone();
        let text = serde_json::to_string(&token).map_err(|e| CliError::internal(e.to_string()))?;
        write_private_file_atomic(path, format!("{text}\n").as_bytes())
            .map_err(|e| CliError::local_io(e.message))?;
    }

    if let Some(error) = outcome.partial_error.take() {
        return Err(error);
    }
    if outcome.stop_reason == StopReason::Page {
        env.reporter.notice(
            "More results are available. Use --save-cursor PATH to keep the continuation, or --output json to receive next_cursor.",
        );
    }
    Ok(())
}

fn list_meta(outcome: &ListOutcome) -> Value {
    json!({
        "pages_fetched": outcome.pages_fetched,
        "complete": outcome.complete,
        "truncated": outcome.truncated,
        "stop_reason": outcome.stop_reason.as_str(),
        "request_id": outcome.request_id,
    })
}

/// Streams the Result envelope with items read back from the spool.
fn write_list_json(env: &Env, outcome: &ListOutcome) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let profile = serde_json::to_string(&env.profile_name())
        .map_err(|e| CliError::internal(e.to_string()))?;
    let next = serde_json::to_string(&outcome.next_cursor)
        .map_err(|e| CliError::internal(e.to_string()))?;
    let meta = serde_json::to_string(&list_meta(outcome))
        .map_err(|e| CliError::internal(e.to_string()))?;
    let write = |out: &mut BufWriter<io::StdoutLock<'_>>, text: &str| -> Result<(), CliError> {
        out.write_all(text.as_bytes())
            .map_err(|e| output_failure(e, Outcome::NotApplicable))
    };
    write(
        &mut out,
        &format!(
            "{{\"schema_version\":{SCHEMA_VERSION},\"command\":\"list\",\"profile\":{profile},\"data\":{{\"items\":["
        ),
    )?;
    let mut first = true;
    outcome.spool.for_each(|link: Link| {
        let line = serde_json::to_string(&link).map_err(|e| CliError::internal(e.to_string()))?;
        if !first {
            write(&mut out, ",")?;
        }
        first = false;
        write(&mut out, &line)
    })?;
    write(
        &mut out,
        &format!("],\"next_cursor\":{next}}},\"meta\":{meta}}}\n"),
    )?;
    out.flush()
        .map_err(|e| output_failure(e, Outcome::NotApplicable))
}

fn write_list_human(env: &Env, opts: &ListOpts, outcome: &mut ListOutcome) -> Result<(), CliError> {
    let columns = opts
        .columns
        .clone()
        .unwrap_or_else(|| DEFAULT_COLUMNS.to_vec());
    let width = if rustix::termios::isatty(io::stdout()) {
        terminal_width()
    } else {
        None
    };
    let mut layout = TableLayout::new(&columns, width);
    outcome.spool.for_each(|link| {
        layout.observe(&link);
        Ok(())
    })?;
    layout.finish();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let write_line =
        |out: &mut BufWriter<io::StdoutLock<'_>>, line: &str| -> Result<(), CliError> {
            writeln!(out, "{line}").map_err(|e| output_failure(e, Outcome::NotApplicable))
        };
    write_line(&mut out, &layout.heading())?;
    outcome
        .spool
        .for_each(|link| write_line(&mut out, &layout.row(&link)))?;
    out.flush()
        .map_err(|e| output_failure(e, Outcome::NotApplicable))?;
    let count = outcome.spool.items;
    env.reporter.notice(&format!(
        "{count} {} shown.",
        if count == 1 { "link" } else { "links" }
    ));
    if count == 0 {
        env.reporter.notice("0 links");
    }
    Ok(())
}

fn confirm_delete(env: &Env, code: &str, target: &ProfileTarget) -> Result<(), CliError> {
    let prompt = format!(
        "Permanently delete code {code} using profile {} at {}? This also removes its visits; the code cannot be reused.",
        target.name,
        target.server.text()
    );
    match Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact_opt()
    {
        Ok(Some(true)) => Ok(()),
        Ok(_) => Err(CliError::declined()),
        Err(_) => {
            let _ = env;
            Err(CliError::interrupted())
        }
    }
}

pub fn delete(env: &Env, opts: &DeleteOpts) -> Result<(), CliError> {
    let target = select_target(env)?;
    let code = input::parse_delete_target(&opts.target, &target.public)?;
    if !opts.yes {
        if !env.interactive {
            return Err(CliError::usage(
                "Deletion requires --yes in non-interactive, JSON, or redirected use. No request was sent.",
            ));
        }
        confirm_delete(env, &code, &target)?;
    }
    let key = target.read_key()?;
    let transport = transport_for(&target, &key, env.limits)?;
    let deleted = api::delete_link(&transport, &code)?;

    if env.json() {
        write_result(
            "delete",
            env.profile_name().as_deref(),
            &json!({ "short_code": code, "deleted": true }),
            &json!({ "request_id": deleted.request_id }),
        )
        .map_err(|e| output_failure(e, Outcome::Committed))?;
    } else {
        env.reporter.notice(&format!("Deleted {code}."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_cursor_files_accept_strings_and_reject_null_and_other_types() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("c.json");
        std::fs::write(&path, "\"abc-123\"\n").expect("write");
        assert_eq!(read_cursor_file(&path).expect("string"), "abc-123");
        std::fs::write(&path, "null").expect("write");
        assert_eq!(read_cursor_file(&path).expect_err("null").exit_code(), 2);
        std::fs::write(&path, "{\"a\":1}").expect("write");
        assert_eq!(read_cursor_file(&path).expect_err("object").exit_code(), 2);
        std::fs::write(&path, "\"\"").expect("write");
        assert_eq!(read_cursor_file(&path).expect_err("empty").exit_code(), 2);
    }

    #[test]
    fn default_columns_are_the_documented_five() {
        let names: Vec<&str> = DEFAULT_COLUMNS.iter().map(|c| c.name()).collect();
        assert_eq!(
            names,
            [
                "short_url",
                "destination_url",
                "title",
                "visit_count",
                "created_at"
            ]
        );
    }

    #[test]
    fn list_stop_reason_strings_are_stable() {
        assert_eq!(StopReason::PageLimit.as_str(), "page_limit");
        assert_eq!(StopReason::Cancelled.as_str(), "cancelled");
    }
}
