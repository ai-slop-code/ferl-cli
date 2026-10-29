//! Own-scope link listing with explicit continuation and bounded fetch-all (business
//! spec §8). Accepted items are staged in a private spool file so that memory use
//! does not grow with the number of pages.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;

use serde::Serialize;

use crate::api::{Link, ListQuery, list_links_page};
use crate::cancel;
use crate::error::{CliError, Kind, Outcome};
use crate::transport::Transport;

/// Aggregate encoded result bound for fetch-all (business spec §8).
pub const MAX_AGGREGATE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    End,
    Page,
    PageLimit,
    Timeout,
    Error,
    SizeLimit,
    Cancelled,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            StopReason::End => "end",
            StopReason::Page => "page",
            StopReason::PageLimit => "page_limit",
            StopReason::Timeout => "timeout",
            StopReason::Error => "error",
            StopReason::SizeLimit => "size_limit",
            StopReason::Cancelled => "cancelled",
        }
    }
}

/// Accepted items written as one JSON object per line in a staging file inside a
/// private (0700) temporary directory. Both are removed when the spool is dropped.
pub struct Spool {
    file: tempfile::NamedTempFile,
    _dir: tempfile::TempDir,
    pub items: usize,
    /// Encoded payload bytes staged so far; this is the aggregate bound's measure.
    pub bytes: u64,
}

impl Spool {
    pub fn new() -> Result<Spool, CliError> {
        let staging_error = |e: std::io::Error| {
            CliError::local_io(format!("Could not create the list staging file: {e}."))
        };
        let dir = tempfile::Builder::new()
            .prefix("ferl-spool-")
            .tempdir()
            .map_err(staging_error)?;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(staging_error)?;
        let file = tempfile::Builder::new()
            .prefix("staged-")
            .tempfile_in(dir.path())
            .map_err(staging_error)?;
        Ok(Spool {
            file,
            _dir: dir,
            items: 0,
            bytes: 0,
        })
    }

    /// Encodes one page as JSON lines before any bound is applied to it.
    fn encode(links: &[Link]) -> Result<Vec<u8>, CliError> {
        let mut lines = Vec::new();
        for link in links {
            serde_json::to_writer(&mut lines, link)
                .map_err(|e| CliError::internal(e.to_string()))?;
            lines.push(b'\n');
        }
        Ok(lines)
    }

    fn append(&mut self, lines: &[u8], items: usize) -> Result<(), CliError> {
        self.file
            .write_all(lines)
            .and_then(|_| self.file.flush())
            .map_err(|e| CliError::local_io(format!("Could not stage list results: {e}.")))?;
        self.items += items;
        self.bytes += lines.len() as u64;
        Ok(())
    }

    /// Streams staged links in server order.
    pub fn for_each(
        &self,
        mut visit: impl FnMut(Link) -> Result<(), CliError>,
    ) -> Result<(), CliError> {
        let file: File = self
            .file
            .reopen()
            .map_err(|e| CliError::local_io(format!("Could not read the staging file: {e}.")))?;
        for line in BufReader::new(file).lines() {
            let line = line
                .map_err(|e| CliError::local_io(format!("Could not read staged results: {e}.")))?;
            let value: serde_json::Value =
                serde_json::from_str(&line).map_err(|e| CliError::internal(e.to_string()))?;
            let link = Link::from_value(&value).map_err(CliError::internal)?;
            visit(link)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ListRequest {
    pub search: Option<String>,
    pub limit: u16,
    pub start_cursor: Option<String>,
    pub fetch_all: bool,
    pub max_pages: u32,
}

pub struct ListOutcome {
    pub spool: Spool,
    pub pages_fetched: u64,
    pub next_cursor: Option<String>,
    pub complete: bool,
    pub truncated: bool,
    pub stop_reason: StopReason,
    pub request_id: Option<String>,
    /// Set when the list stopped early; carries the underlying code and status.
    pub partial_error: Option<CliError>,
}

fn stop_reason_for(error: &CliError) -> StopReason {
    if error.kind == Kind::Cancelled {
        StopReason::Cancelled
    } else if error.code == "client.deadline_exceeded" {
        StopReason::Timeout
    } else {
        StopReason::Error
    }
}

fn as_partial(error: CliError, pages: u64) -> CliError {
    let mut partial = error.clone();
    partial.kind = if error.kind == Kind::Cancelled {
        Kind::Cancelled
    } else {
        Kind::Partial
    };
    partial.partial = true;
    partial.pages_fetched = pages;
    partial.outcome = Outcome::NotApplicable;
    partial
}

/// Walks pages in server order. The first failure returns an error with no output.
/// A later failure returns a partial outcome with the accepted pages retained.
pub fn run(transport: &Transport, request: &ListRequest) -> Result<ListOutcome, CliError> {
    let mut spool = Spool::new()?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut cursor = request.start_cursor.clone();
    if let Some(start) = &cursor {
        seen.insert(start.clone());
    }
    let mut pages: u64 = 0;
    let mut request_id: Option<String> = None;
    let mut next_cursor: Option<String> = None;

    loop {
        if cancel::is_interrupted() {
            return finish_interrupted(
                spool,
                pages,
                request_id,
                next_cursor,
                CliError::interrupted(),
            );
        }
        let query = ListQuery {
            search: request.search.clone(),
            limit: request.limit,
            cursor: cursor.clone(),
        };
        let page = match list_links_page(transport, &query) {
            Ok(page) => page,
            Err(error) => {
                if pages == 0 {
                    return Err(error);
                }
                let reason = stop_reason_for(&error);
                return finish_partial(spool, pages, request_id, next_cursor, reason, error);
            }
        };

        let encoded = Spool::encode(&page.items)?;
        if request.fetch_all
            && spool.bytes + encoded.len() as u64 > MAX_AGGREGATE_BYTES as u64
            && pages > 0
        {
            // The excluded page is not accepted; the continuation is the cursor that requested it.
            let error = CliError::new(
                Kind::Partial,
                "client.size_limit",
                "The fetch-all result reached its 256 MiB aggregate bound.",
            );
            return finish_partial(
                spool,
                pages,
                request_id,
                next_cursor,
                StopReason::SizeLimit,
                error,
            );
        }

        spool.append(&encoded, page.items.len())?;
        pages += 1;
        request_id = page.request_id.clone();
        next_cursor = page.next_cursor.clone();

        let Some(next) = page.next_cursor else {
            return Ok(ListOutcome {
                spool,
                pages_fetched: pages,
                next_cursor: None,
                complete: true,
                truncated: false,
                stop_reason: StopReason::End,
                request_id,
                partial_error: None,
            });
        };

        if !request.fetch_all {
            return Ok(ListOutcome {
                spool,
                pages_fetched: pages,
                next_cursor: Some(next),
                complete: false,
                truncated: false,
                stop_reason: StopReason::Page,
                request_id,
                partial_error: None,
            });
        }

        if seen.contains(&next) {
            let error = CliError::new(
                Kind::Protocol,
                "client.repeated_cursor",
                "The server returned a continuation that was already requested; the list was stopped to avoid a loop.",
            );
            return finish_partial(
                spool,
                pages,
                request_id,
                next_cursor,
                StopReason::Error,
                error,
            );
        }

        if pages >= u64::from(request.max_pages) {
            let error = CliError::new(
                Kind::Partial,
                "client.page_limit",
                "The fetch-all reached the --max-pages bound before the last page.",
            );
            return finish_partial(
                spool,
                pages,
                request_id,
                Some(next),
                StopReason::PageLimit,
                error,
            );
        }

        seen.insert(next.clone());
        cursor = Some(next);
    }
}

fn finish_partial(
    spool: Spool,
    pages: u64,
    request_id: Option<String>,
    next_cursor: Option<String>,
    reason: StopReason,
    error: CliError,
) -> Result<ListOutcome, CliError> {
    let mut error = as_partial(error, pages);
    if error.request_id.is_none() {
        error.request_id = request_id.clone();
    }
    Ok(ListOutcome {
        spool,
        pages_fetched: pages,
        next_cursor,
        complete: false,
        truncated: true,
        stop_reason: reason,
        request_id,
        partial_error: Some(error),
    })
}

fn finish_interrupted(
    spool: Spool,
    pages: u64,
    request_id: Option<String>,
    next_cursor: Option<String>,
    error: CliError,
) -> Result<ListOutcome, CliError> {
    if pages == 0 {
        return Err(error);
    }
    finish_partial(
        spool,
        pages,
        request_id,
        next_cursor,
        StopReason::Cancelled,
        error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reasons_map_from_error_kinds_and_codes() {
        let deadline = CliError::new(Kind::Transport, "client.deadline_exceeded", "x");
        assert_eq!(stop_reason_for(&deadline), StopReason::Timeout);
        assert_eq!(
            stop_reason_for(&CliError::interrupted()),
            StopReason::Cancelled
        );
        let other = CliError::new(Kind::Server, "internal_error", "x");
        assert_eq!(stop_reason_for(&other), StopReason::Error);
    }

    #[test]
    fn partial_errors_keep_underlying_status_while_reporting_partial_exit() {
        let underlying = CliError::new(Kind::Server, "storage_unavailable", "later page failed")
            .with_http(503, Some("req-1".to_owned()));
        let partial = as_partial(underlying, 2);
        assert_eq!(partial.kind, Kind::Partial);
        assert_eq!(partial.exit_code(), 12);
        assert_eq!(partial.code, "storage_unavailable");
        assert_eq!(partial.http_status, Some(503));
        assert_eq!(partial.pages_fetched, 2);
    }

    #[test]
    fn interruption_after_accepted_pages_is_cancelled_partial() {
        let spool = Spool::new().expect("spool");
        let outcome = finish_interrupted(
            spool,
            1,
            None,
            Some("c".to_owned()),
            CliError::interrupted(),
        )
        .expect("partial");
        assert_eq!(outcome.stop_reason, StopReason::Cancelled);
        assert_eq!(outcome.partial_error.expect("error").exit_code(), 130);
    }

    #[test]
    fn spool_round_trips_links_in_order() {
        let mut spool = Spool::new().expect("spool");
        let link = |code: &str| {
            Link::from_value(&serde_json::json!({
                "id": "1f1bf5f6-075c-4cf1-a44b-05fca986b100",
                "short_code": code,
                "short_url": format!("https://s.example.com/{code}"),
                "destination_url": "https://example.com/",
                "title": null,
                "tags": [],
                "owner_id": "4a8ffcd5-3066-4519-a68e-060f956ab500",
                "enabled": true,
                "active": true,
                "visit_count": 9223372036854775807_i64,
                "created_at": "2026-10-29T10:00:00Z",
                "updated_at": "2026-10-29T10:00:00Z"
            }))
            .expect("link")
        };
        for batch in [vec![link("aaa"), link("bbb")], vec![link("ccc")]] {
            let encoded = Spool::encode(&batch).expect("encode");
            spool.append(&encoded, batch.len()).expect("append");
        }
        let mut codes = Vec::new();
        spool
            .for_each(|l| {
                codes.push(l.short_code);
                Ok(())
            })
            .expect("read");
        assert_eq!(codes, ["aaa", "bbb", "ccc"]);
        assert_eq!(spool.items, 3);
        assert_eq!(
            spool.bytes as usize,
            Spool::encode(&[link("aaa"), link("bbb"), link("ccc")])
                .expect("encode")
                .len()
        );
    }

    #[test]
    fn spool_staging_is_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let spool = Spool::new().expect("spool");
        let dir = spool.file.path().parent().expect("parent").to_path_buf();
        let mode =
            |p: &std::path::Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(spool.file.path()), 0o600);
        let file = spool.file.path().to_path_buf();
        drop(spool);
        assert!(!file.exists());
        assert!(!dir.exists());
    }
}
