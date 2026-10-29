//! Server and public origins: explicit HTTP(S) origins without paths, queries,
//! fragments, or user information (business spec §9.1).

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use crate::error::CliError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    text: String,
    scheme: String,
    host: String,
    port: u16,
    loopback_ip: bool,
}

impl Origin {
    /// Canonical serialization, without a trailing slash.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Host without IPv6 brackets, as used for proxy bypass matching.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }

    /// True only for a literal loopback IP (127.0.0.0/8 or ::1). Names such as
    /// `localhost` are not accepted for plaintext development.
    pub fn is_loopback_literal(&self) -> bool {
        self.loopback_ip
    }
}

/// Parses an origin supplied as configuration. Accepts one trailing `/`, which is
/// normalized away; rejects any path prefix such as `/api/v1`.
pub fn parse_origin(raw: &str) -> Result<Origin, CliError> {
    let invalid = |why: &str| CliError::usage(format!("Invalid server origin: {why}."));
    if raw.is_empty() {
        return Err(invalid("a value is required"));
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid("whitespace and control characters are not allowed"));
    }
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    if trimmed.ends_with('/') {
        return Err(invalid("only one trailing slash is accepted"));
    }
    if trimmed.contains('@') {
        return Err(invalid("user information is not allowed"));
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        return Err(invalid("query and fragment components are not allowed"));
    }
    let url = Url::parse(trimmed).map_err(|_| invalid("it is not a valid absolute URL"))?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(invalid("the scheme must be http or https")),
    }
    if url.host().is_none() {
        return Err(invalid("a host is required"));
    }
    if url.path() != "/" {
        return Err(invalid("path prefixes such as /api/v1 are not allowed"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("user information is not allowed"));
    }
    let loopback_ip = match url.host() {
        Some(Host::Ipv4(addr)) => is_loopback_v4(addr),
        Some(Host::Ipv6(addr)) => is_loopback_v6(addr),
        _ => false,
    };
    let text = url.origin().ascii_serialization();
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = url.port_or_known_default().unwrap_or(443);
    Ok(Origin {
        text,
        scheme: url.scheme().to_owned(),
        host,
        port,
        loopback_ip,
    })
}

fn is_loopback_v4(addr: Ipv4Addr) -> bool {
    addr.is_loopback()
}

fn is_loopback_v6(addr: Ipv6Addr) -> bool {
    addr.is_loopback()
}

/// Parses an origin for the selected server and enforces the HTTP policy.
pub fn parse_server_origin(raw: &str, allow_http_loopback: bool) -> Result<Origin, CliError> {
    let origin = parse_origin(raw)?;
    if !origin.is_https() {
        if !origin.is_loopback_literal() {
            return Err(CliError::usage(
                "Plaintext HTTP is allowed only for a literal loopback address (127.0.0.0/8 or ::1). Use https for other servers.",
            ));
        }
        if !allow_http_loopback {
            return Err(CliError::usage(
                "Plaintext loopback HTTP requires --allow-http-loopback.",
            ));
        }
    }
    Ok(origin)
}

/// Compares an absolute URL's origin with a configured origin using URL standards.
pub fn url_has_origin(url: &Url, origin: &Origin) -> bool {
    url.origin().ascii_serialization() == origin.text()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_slash_is_normalized_and_case_and_default_port_canonicalize() {
        let origin = parse_origin("HTTPS://API.Example.com:443/").expect("valid");
        assert_eq!(origin.text(), "https://api.example.com");
        assert!(origin.is_https());
    }

    #[test]
    fn non_root_paths_queries_fragments_and_credentials_are_rejected() {
        for bad in [
            "https://api.example.com/api/v1",
            "https://api.example.com//",
            "https://api.example.com/?x=1",
            "https://api.example.com/#frag",
            "https://user@api.example.com",
            "https://user:pw@api.example.com",
            "ftp://api.example.com",
            "https://api.example.com/ ",
            "https://",
        ] {
            assert!(parse_origin(bad).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn plaintext_http_requires_a_literal_loopback_and_the_explicit_flag() {
        assert!(parse_server_origin("http://localhost:18080", true).is_err());
        assert!(parse_server_origin("http://10.0.0.5", true).is_err());
        assert!(parse_server_origin("http://127.0.0.1:18080", false).is_err());
        assert!(parse_server_origin("http://127.0.0.1:18080", true).is_ok());
        assert!(parse_server_origin("http://[::1]:18080", true).is_ok());
        assert!(parse_server_origin("https://api.example.com", false).is_ok());
    }

    #[test]
    fn url_origin_comparison_uses_canonical_origins() {
        let origin = parse_origin("https://s.example.com").expect("valid");
        let url = Url::parse("https://S.example.com:443/abc").expect("valid");
        assert!(url_has_origin(&url, &origin));
        let other = Url::parse("https://api.s.example.com/abc").expect("valid");
        assert!(!url_has_origin(&other, &origin));
    }
}
