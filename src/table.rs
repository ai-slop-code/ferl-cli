//! Human list tables: column selection, UTC date display, sanitized cells, and
//! grapheme-safe truncation for terminals (business spec §§7.1, 3.1).

use time::OffsetDateTime;
use time::UtcOffset;
use time::format_description::well_known::Rfc3339;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::api::Link;
use crate::error::CliError;
use crate::report::sanitize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    ShortCode,
    ShortUrl,
    DestinationUrl,
    Title,
    Tags,
    Enabled,
    Active,
    VisitCount,
    CreatedAt,
    UpdatedAt,
    OwnerId,
    Id,
}

pub const DEFAULT_COLUMNS: [Column; 5] = [
    Column::ShortUrl,
    Column::DestinationUrl,
    Column::Title,
    Column::VisitCount,
    Column::CreatedAt,
];

impl Column {
    pub fn name(self) -> &'static str {
        match self {
            Column::ShortCode => "short_code",
            Column::ShortUrl => "short_url",
            Column::DestinationUrl => "destination_url",
            Column::Title => "title",
            Column::Tags => "tags",
            Column::Enabled => "enabled",
            Column::Active => "active",
            Column::VisitCount => "visit_count",
            Column::CreatedAt => "created_at",
            Column::UpdatedAt => "updated_at",
            Column::OwnerId => "owner_id",
            Column::Id => "id",
        }
    }

    fn from_name(name: &str) -> Option<Column> {
        [
            Column::ShortCode,
            Column::ShortUrl,
            Column::DestinationUrl,
            Column::Title,
            Column::Tags,
            Column::Enabled,
            Column::Active,
            Column::VisitCount,
            Column::CreatedAt,
            Column::UpdatedAt,
            Column::OwnerId,
            Column::Id,
        ]
        .into_iter()
        .find(|c| c.name() == name)
    }
}

/// Parses `--columns` in the requested order. Duplicates and unknown names fail locally.
pub fn parse_columns(text: &str) -> Result<Vec<Column>, CliError> {
    let mut columns = Vec::new();
    for part in text.split(',') {
        let name = part.trim();
        let column = Column::from_name(name).ok_or_else(|| {
            CliError::usage(format!(
                "Unknown column {}. Accepted names: short_code, short_url, destination_url, title, tags, enabled, active, visit_count, created_at, updated_at, owner_id, id.",
                sanitize(name)
            ))
        })?;
        if columns.contains(&column) {
            return Err(CliError::usage(format!(
                "Column {} is listed more than once.",
                column.name()
            )));
        }
        columns.push(column);
    }
    if columns.is_empty() {
        return Err(CliError::usage("At least one column is required."));
    }
    Ok(columns)
}

const MISSING: &str = "—";

/// Formats an RFC 3339 UTC timestamp to second precision with a `Z` designation.
pub fn display_timestamp(text: &str) -> String {
    match OffsetDateTime::parse(text, &Rfc3339) {
        Ok(moment) => {
            let utc = moment.to_offset(UtcOffset::UTC);
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                utc.year(),
                u8::from(utc.month()),
                utc.day(),
                utc.hour(),
                utc.minute(),
                utc.second()
            )
        }
        Err(_) => sanitize(text),
    }
}

fn cell(link: &Link, column: Column) -> String {
    match column {
        Column::ShortCode => link.short_code.clone(),
        Column::ShortUrl => link.short_url.clone(),
        Column::DestinationUrl => link.destination_url.clone(),
        Column::Title => link.title.clone().unwrap_or_else(|| MISSING.to_owned()),
        Column::Tags => serde_json::to_string(&link.tags).unwrap_or_else(|_| "[]".to_owned()),
        Column::Enabled => link.enabled.to_string(),
        Column::Active => link.active.to_string(),
        Column::VisitCount => link.visit_count.to_string(),
        Column::CreatedAt => display_timestamp(&link.created_at),
        Column::UpdatedAt => display_timestamp(&link.updated_at),
        Column::OwnerId => link.owner_id.clone().unwrap_or_else(|| MISSING.to_owned()),
        Column::Id => link.id.clone(),
    }
}

/// Truncates to a display width at grapheme boundaries, adding an ellipsis.
pub fn truncate_to_width(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let budget = width - 1;
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let w = grapheme.width();
        if used + w > budget {
            break;
        }
        used += w;
        out.push_str(grapheme);
    }
    out.push('…');
    out
}

/// Two-pass table layout: measure every row first, then render. This keeps list
/// output streaming from a spool instead of holding every link in memory.
pub struct TableLayout {
    columns: Vec<Column>,
    widths: Vec<usize>,
    terminal_width: Option<usize>,
}

impl TableLayout {
    /// `terminal_width` is `Some` only for TTY output; piped output is never truncated.
    pub fn new(columns: &[Column], terminal_width: Option<usize>) -> Self {
        TableLayout {
            columns: columns.to_vec(),
            widths: columns.iter().map(|c| c.name().width()).collect(),
            terminal_width,
        }
    }

    pub fn observe(&mut self, link: &Link) {
        for (i, column) in self.columns.iter().enumerate() {
            let width = sanitize(&cell(link, *column)).width();
            self.widths[i] = self.widths[i].max(width);
        }
    }

    /// Applies the terminal width budget after all rows have been observed.
    pub fn finish(&mut self) {
        let Some(total) = self.terminal_width else {
            return;
        };
        let gaps = 2 * self.columns.len().saturating_sub(1);
        let available = total.saturating_sub(gaps).max(self.columns.len() * 4);
        while self.widths.iter().sum::<usize>() > available {
            let Some((index, _)) = self.widths.iter().enumerate().max_by_key(|(_, w)| **w) else {
                break;
            };
            if self.widths[index] <= 4 {
                break;
            }
            self.widths[index] -= 1;
        }
    }

    pub fn heading(&self) -> String {
        let names: Vec<String> = self.columns.iter().map(|c| c.name().to_owned()).collect();
        self.line(&names)
    }

    pub fn row(&self, link: &Link) -> String {
        let values: Vec<String> = self
            .columns
            .iter()
            .map(|c| sanitize(&cell(link, *c)))
            .collect();
        self.line(&values)
    }

    fn line(&self, values: &[String]) -> String {
        let mut line = String::new();
        for (i, value) in values.iter().enumerate() {
            let shown = if self.terminal_width.is_some() {
                truncate_to_width(value, self.widths[i])
            } else {
                value.clone()
            };
            line.push_str(&shown);
            if i + 1 < values.len() {
                let padding = self.widths[i].saturating_sub(shown.width());
                line.push_str(&" ".repeat(padding + 2));
            }
        }
        line.trim_end().to_owned()
    }
}

/// Renders a complete table in memory (used by tests).
#[cfg(test)]
pub fn render_table(columns: &[Column], links: &[Link], terminal_width: Option<usize>) -> String {
    let mut layout = TableLayout::new(columns, terminal_width);
    for link in links {
        layout.observe(link);
    }
    layout.finish();
    let mut out = layout.heading();
    out.push('\n');
    for link in links {
        out.push_str(&layout.row(link));
        out.push('\n');
    }
    out
}

/// Plain aligned table with headings for profile listings. Values are sanitized.
pub fn simple_table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut all: Vec<Vec<String>> = Vec::with_capacity(rows.len() + 1);
    all.push(headers.iter().map(|h| (*h).to_owned()).collect());
    all.extend(
        rows.into_iter()
            .map(|row| row.iter().map(|v| sanitize(v)).collect()),
    );
    let mut widths = vec![0usize; headers.len()];
    for row in &all {
        for (i, value) in row.iter().enumerate() {
            widths[i] = widths[i].max(value.width());
        }
    }
    let mut out = String::new();
    for row in &all {
        let mut line = String::new();
        for (i, value) in row.iter().enumerate() {
            line.push_str(value);
            if i + 1 < row.len() {
                line.push_str(&" ".repeat(widths[i].saturating_sub(value.width()) + 2));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(title: Option<&str>, visits: i64) -> Link {
        Link {
            id: "1f1bf5f6-075c-4cf1-a44b-05fca986b100".to_owned(),
            short_code: "a1b2c3d4".to_owned(),
            short_url: "https://s.example.com/a1b2c3d4".to_owned(),
            destination_url: "https://example.com/docs?edition=2#intro".to_owned(),
            title: title.map(str::to_owned),
            tags: vec!["work".to_owned()],
            owner_id: None,
            enabled: false,
            active: false,
            visit_count: visits,
            created_at: "2026-10-27T10:00:00.123456Z".to_owned(),
            updated_at: "2026-10-27T10:00:00.000000Z".to_owned(),
        }
    }

    #[test]
    fn column_parsing_keeps_order_and_rejects_duplicates_and_unknown_names() {
        assert_eq!(
            parse_columns("visit_count,short_url").expect("ok"),
            vec![Column::VisitCount, Column::ShortUrl]
        );
        assert!(parse_columns("short_url,short_url").is_err());
        assert!(parse_columns("secret").is_err());
        assert!(parse_columns("").is_err());
    }

    #[test]
    fn timestamps_render_in_utc_at_second_precision() {
        assert_eq!(
            display_timestamp("2026-10-27T10:00:00.123456Z"),
            "2026-10-27T10:00:00Z"
        );
        assert_eq!(
            display_timestamp("2026-10-27T12:00:00+02:00"),
            "2026-10-27T10:00:00Z"
        );
    }

    #[test]
    fn null_title_shows_placeholder_and_counts_stay_exact() {
        let table = render_table(
            &[Column::Title, Column::VisitCount],
            &[link(None, 9_007_199_254_740_993)],
            None,
        );
        assert!(table.contains("—"));
        assert!(table.contains("9007199254740993"));
    }

    #[test]
    fn hostile_titles_are_rendered_as_escaped_data() {
        let table = render_table(
            &[Column::Title],
            &[link(Some("\x1b]8;;x\x07 evil\n"), 0)],
            None,
        );
        assert!(!table.contains('\x1b'));
        assert_eq!(table.lines().count(), 2, "newline must not add table rows");
    }

    #[test]
    fn truncation_preserves_grapheme_clusters() {
        let text = "e\u{301}e\u{301}e\u{301}e\u{301}";
        let truncated = truncate_to_width(text, 3);
        assert!(truncated.ends_with('…'));
        assert!(truncated.width() <= 3);
        assert!(!truncated.contains('\u{301}') || truncated.starts_with("e\u{301}"));
    }
}
