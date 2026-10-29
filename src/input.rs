//! Local input validation: destinations, titles, tags, search text, cursors, short
//! codes, and deletion targets. Server responses remain authoritative for
//! authorization, availability, and canonical normalization (business spec §9).

use url::Url;

use crate::error::CliError;
use crate::origin::{Origin, url_has_origin};

pub const MAX_DESTINATION_BYTES: usize = 8192;
pub const MAX_TITLE_SCALARS: usize = 200;
pub const MAX_TAG_INPUTS: usize = 20;
pub const MAX_TAG_INPUT_SCALARS: usize = 256;
pub const MAX_SEARCH_SCALARS: usize = 256;

fn scalar_count(text: &str) -> usize {
    text.chars().count()
}

fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// Validates a destination URL without fetching it. The caller sends the value
/// exactly as supplied, so query and fragment data remain intact.
pub fn validate_destination(raw: &str) -> Result<(), CliError> {
    if raw.is_empty() {
        return Err(CliError::usage("A destination URL is required."));
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(CliError::usage(
            "The destination URL must not contain whitespace or control characters.",
        ));
    }
    if raw.len() > MAX_DESTINATION_BYTES {
        return Err(CliError::usage("The destination URL exceeds 8192 bytes."));
    }
    let lower = raw.to_ascii_lowercase();
    let scheme_len = if lower.starts_with("https://") {
        8
    } else if lower.starts_with("http://") {
        7
    } else {
        return Err(CliError::usage(
            "The destination must be an absolute http or https URL.",
        ));
    };
    let authority_end = raw[scheme_len..]
        .find(['/', '?', '#'])
        .map_or(raw.len(), |i| scheme_len + i);
    let authority = &raw[scheme_len..authority_end];
    if authority.contains('@') {
        return Err(CliError::usage(
            "The destination URL must not contain user information.",
        ));
    }
    if authority.is_empty() {
        return Err(CliError::usage("The destination URL must include a host."));
    }
    let url = Url::parse(raw).map_err(|_| CliError::usage("The destination URL is malformed."))?;
    if url.host().is_none() {
        return Err(CliError::usage("The destination URL must include a host."));
    }
    if url.as_str().len() > MAX_DESTINATION_BYTES {
        return Err(CliError::usage(
            "The normalized destination URL exceeds 8192 bytes.",
        ));
    }
    Ok(())
}

pub fn validate_title(raw: &str) -> Result<(), CliError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::usage("A title must not be blank."));
    }
    if scalar_count(trimmed) > MAX_TITLE_SCALARS {
        return Err(CliError::usage(
            "A title may contain at most 200 characters.",
        ));
    }
    if has_control(trimmed) {
        return Err(CliError::usage(
            "A title must not contain control characters.",
        ));
    }
    Ok(())
}

/// Checks the unambiguous local tag rules. Full case-folding and NFC canonical
/// validation is left to the server (decision D-IMPL-05).
pub fn validate_tags(tags: &[String]) -> Result<(), CliError> {
    if tags.len() > MAX_TAG_INPUTS {
        return Err(CliError::usage("At most 20 --tag values may be supplied."));
    }
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return Err(CliError::usage("A tag must not be blank."));
        }
        if scalar_count(trimmed) > MAX_TAG_INPUT_SCALARS {
            return Err(CliError::usage(
                "A tag input may contain at most 256 characters.",
            ));
        }
        if has_control(trimmed) {
            return Err(CliError::usage(
                "A tag must not contain control characters.",
            ));
        }
    }
    Ok(())
}

pub fn normalize_search(raw: &str) -> Result<String, CliError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::usage("The search text must not be blank."));
    }
    if scalar_count(trimmed) > MAX_SEARCH_SCALARS {
        return Err(CliError::usage(
            "The search text may contain at most 256 characters.",
        ));
    }
    if has_control(trimmed) {
        return Err(CliError::usage(
            "The search text must not contain control characters.",
        ));
    }
    Ok(trimmed.to_owned())
}

pub fn validate_cursor(raw: &str) -> Result<(), CliError> {
    if raw.is_empty() {
        return Err(CliError::usage("A cursor must not be empty."));
    }
    if has_control(raw) || raw.chars().any(char::is_whitespace) {
        return Err(CliError::usage(
            "A cursor must not contain whitespace or control characters.",
        ));
    }
    Ok(())
}

/// `[a-z0-9-]{3,64}`: the exact public short-code syntax.
pub fn is_short_code(text: &str) -> bool {
    (3..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_uuid_shape(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// Resolves a deletion target to its exact short code (business spec §9.2).
///
/// Raw input is inspected before any URL parsing so that uppercase, percent-encoded,
/// dot-segment, and suffix aliases are rejected instead of normalized into a valid code.
pub fn parse_delete_target(raw: &str, public_origin: &Origin) -> Result<String, CliError> {
    let invalid = || {
        CliError::usage(
            "Specify one exact short code (for example a1b2c3d4) or its full short URL on the selected profile's public origin.",
        )
    };
    if raw.is_empty() || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid());
    }
    let lower = raw.to_ascii_lowercase();
    let is_url = lower.starts_with("https://") || lower.starts_with("http://");
    if !is_url {
        if is_uuid_shape(raw) {
            return Err(CliError::usage(
                "A link UUID is not a deletion target; use its short code.",
            ));
        }
        return if is_short_code(raw) {
            Ok(raw.to_owned())
        } else {
            Err(invalid())
        };
    }
    let scheme_len = if lower.starts_with("https://") { 8 } else { 7 };
    let rest = &raw[scheme_len..];
    let Some(path_start) = rest.find('/') else {
        return Err(invalid());
    };
    let authority = &rest[..path_start];
    if authority.is_empty() || authority.contains('@') || authority.contains('%') {
        return Err(invalid());
    }
    let path = &rest[path_start..];
    let Some(code) = path.strip_prefix('/') else {
        return Err(invalid());
    };
    if !is_short_code(code) {
        return Err(invalid());
    }
    let url = Url::parse(raw).map_err(|_| invalid())?;
    if !url_has_origin(&url, public_origin) {
        return Err(CliError::usage(format!(
            "This short URL is not on the selected profile's public origin ({}). Select the profile that owns it, or set its public origin.",
            public_origin.text()
        )));
    }
    Ok(code.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::parse_origin;

    fn public() -> Origin {
        parse_origin("https://s.example.com").expect("origin")
    }

    #[test]
    fn destinations_keep_query_and_fragment_and_reject_bad_shapes() {
        assert!(validate_destination("https://example.com/docs?edition=2#intro").is_ok());
        assert!(validate_destination("HTTP://localhost:8080/x").is_ok());
        assert!(validate_destination("http://[::1]:9/").is_ok());
        assert!(validate_destination("http://10.0.0.5/private").is_ok());
        assert!(validate_destination("https://bücher.example/%C3%BC").is_ok());
        for bad in [
            " https://example.com",
            "https://example.com ",
            "https://example.com/a b",
            "https://user@example.com",
            "https://@example.com",
            "ftp://example.com",
            "example.com",
            "https://",
            "https://exa mple.com",
            "https://example.com:99999",
        ] {
            assert!(
                validate_destination(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn destination_length_is_checked_in_utf8_bytes() {
        let at_limit = format!(
            "https://example.com/{}",
            "a".repeat(MAX_DESTINATION_BYTES - 20)
        );
        assert!(at_limit.len() <= MAX_DESTINATION_BYTES);
        assert!(validate_destination(&at_limit).is_ok());
        let over = format!("https://example.com/{}", "é".repeat(MAX_DESTINATION_BYTES));
        assert!(validate_destination(&over).is_err());
    }

    #[test]
    fn titles_trim_and_bound_by_unicode_scalar_values() {
        assert!(validate_title("  Team docs  ").is_ok());
        assert!(validate_title(&"é".repeat(200)).is_ok());
        assert!(validate_title(&"é".repeat(201)).is_err());
        assert!(validate_title("   ").is_err());
        assert!(validate_title("bad\u{7}title").is_err());
    }

    #[test]
    fn tag_input_limit_applies_before_deduplication() {
        let twenty: Vec<String> = (0..20).map(|_| "work".to_owned()).collect();
        assert!(validate_tags(&twenty).is_ok());
        let twenty_one: Vec<String> = (0..21).map(|_| "work".to_owned()).collect();
        assert!(validate_tags(&twenty_one).is_err());
        assert!(validate_tags(&["reference,internal".to_owned()]).is_ok());
        assert!(validate_tags(&["  ".to_owned()]).is_err());
    }

    #[test]
    fn search_text_is_trimmed_and_literal() {
        assert_eq!(
            normalize_search("  50%_off\\ & 東京 ").expect("ok"),
            "50%_off\\ & 東京"
        );
        assert!(normalize_search("   ").is_err());
        assert!(normalize_search(&"a".repeat(257)).is_err());
    }

    #[test]
    fn bare_codes_and_url_codes_resolve_to_exact_codes() {
        assert_eq!(
            parse_delete_target("a1b2c3d4", &public()).expect("code"),
            "a1b2c3d4"
        );
        assert_eq!(
            parse_delete_target("-abc", &public()).expect("hyphen"),
            "-abc"
        );
        assert_eq!(
            parse_delete_target("https://s.example.com/a1b2c3d4", &public()).expect("url"),
            "a1b2c3d4"
        );
        assert_eq!(
            parse_delete_target("HTTPS://S.EXAMPLE.COM:443/api", &public()).expect("api"),
            "api"
        );
    }

    #[test]
    fn delete_target_aliases_and_foreign_origins_fail_locally() {
        let bad = [
            "A1B2C3D4",
            "ab",
            "a1b2c3d4/",
            "https://s.example.com/a1b2c3d4/",
            "https://s.example.com/a1b2c3d4?",
            "https://s.example.com/a1b2c3d4#",
            "https://s.example.com/a1b2c3d4?x=1",
            "https://s.example.com/a%31b2c3d4",
            "https://s.example.com/A1b2c3d4",
            "https://s.example.com/a1b2c3d4/extra",
            "https://s.example.com/..",
            "https://s.example.com/./abc",
            "https://s.example.com\\a1b2c3d4",
            "https://user@s.example.com/a1b2c3d4",
            "https://other.example.com/a1b2c3d4",
            "https://s.example.com:444/a1b2c3d4",
            "https://s.example.com/",
            "1f1bf5f6-075c-4cf1-a44b-05fca986b100",
            "example.com/a1b2c3d4",
        ];
        for raw in bad {
            assert!(
                parse_delete_target(raw, &public()).is_err(),
                "{raw} must be rejected"
            );
        }
    }
}
