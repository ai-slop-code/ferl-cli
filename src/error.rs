//! Error classification shared by every command. The kind selects the exit code
//! (business spec §7.4) and the mutation outcome records what the server may have
//! done for writes (§10.3).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Usage,
    Configuration,
    Authentication,
    Authorization,
    NotFound,
    Conflict,
    RateLimit,
    Transport,
    Server,
    Protocol,
    Partial,
    Uncertain,
    LocalIo,
    Cancelled,
    Internal,
}

impl Kind {
    pub fn exit_code(self) -> u8 {
        match self {
            Kind::Internal => 1,
            Kind::Usage => 2,
            Kind::Configuration => 3,
            Kind::Authentication => 4,
            Kind::Authorization => 5,
            Kind::NotFound => 6,
            Kind::Conflict => 7,
            Kind::RateLimit => 8,
            Kind::Transport => 9,
            Kind::Server => 10,
            Kind::Protocol => 11,
            Kind::Partial => 12,
            Kind::Uncertain => 13,
            Kind::LocalIo => 14,
            Kind::Cancelled => 130,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Usage => "usage",
            Kind::Configuration => "configuration",
            Kind::Authentication => "authentication",
            Kind::Authorization => "authorization",
            Kind::NotFound => "not_found",
            Kind::Conflict => "conflict",
            Kind::RateLimit => "rate_limit",
            Kind::Transport => "transport",
            Kind::Server => "server",
            Kind::Protocol => "protocol",
            Kind::Partial => "partial",
            Kind::Uncertain => "uncertain",
            Kind::LocalIo => "local_io",
            Kind::Cancelled => "cancelled",
            Kind::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    NotApplicable,
    NotSent,
    Rejected,
    Committed,
    Unknown,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::NotApplicable => "not_applicable",
            Outcome::NotSent => "not_sent",
            Outcome::Rejected => "rejected",
            Outcome::Committed => "committed",
            Outcome::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliError {
    pub kind: Kind,
    pub code: String,
    pub message: String,
    pub http_status: Option<u16>,
    pub request_id: Option<String>,
    pub retry_after: Option<u64>,
    pub outcome: Outcome,
    pub partial: bool,
    pub pages_fetched: u64,
}

impl CliError {
    pub fn new(kind: Kind, code: &str, message: impl Into<String>) -> Self {
        CliError {
            kind,
            code: code.to_owned(),
            message: message.into(),
            http_status: None,
            request_id: None,
            retry_after: None,
            outcome: Outcome::NotApplicable,
            partial: false,
            pages_fetched: 0,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Kind::Usage, "client.invalid_arguments", message)
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(Kind::Configuration, "client.configuration", message)
    }

    pub fn local_io(message: impl Into<String>) -> Self {
        Self::new(Kind::LocalIo, "client.local_io", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Kind::Internal, "client.internal", message)
    }

    pub fn declined() -> Self {
        Self::new(
            Kind::Cancelled,
            "client.declined",
            "The operation was declined.",
        )
    }

    pub fn interrupted() -> Self {
        Self::new(
            Kind::Cancelled,
            "client.interrupted",
            "The operation was interrupted.",
        )
    }

    pub fn exit_code(&self) -> u8 {
        self.kind.exit_code()
    }

    pub fn with_outcome(mut self, outcome: Outcome) -> Self {
        self.outcome = outcome;
        self
    }

    pub fn with_http(mut self, status: u16, request_id: Option<String>) -> Self {
        self.http_status = Some(status);
        self.request_id = request_id;
        self
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

/// Maps a stable server error code (business spec §7.3) to the local exit kind.
pub fn kind_for_http_status(status: u16) -> Kind {
    match status {
        400 | 413 => Kind::Usage,
        401 => Kind::Authentication,
        403 => Kind::Authorization,
        404 => Kind::NotFound,
        409 => Kind::Conflict,
        429 => Kind::RateLimit,
        500 | 502 | 503 | 504 => Kind::Server,
        _ => Kind::Protocol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_documented_table() {
        let expected = [
            (Kind::Usage, 2),
            (Kind::Configuration, 3),
            (Kind::Authentication, 4),
            (Kind::Authorization, 5),
            (Kind::NotFound, 6),
            (Kind::Conflict, 7),
            (Kind::RateLimit, 8),
            (Kind::Transport, 9),
            (Kind::Server, 10),
            (Kind::Protocol, 11),
            (Kind::Partial, 12),
            (Kind::Uncertain, 13),
            (Kind::LocalIo, 14),
            (Kind::Cancelled, 130),
        ];
        for (kind, code) in expected {
            assert_eq!(kind.exit_code(), code, "{}", kind.as_str());
        }
    }

    #[test]
    fn http_statuses_map_to_their_documented_kinds() {
        assert_eq!(kind_for_http_status(401), Kind::Authentication);
        assert_eq!(kind_for_http_status(403), Kind::Authorization);
        assert_eq!(kind_for_http_status(404), Kind::NotFound);
        assert_eq!(kind_for_http_status(409), Kind::Conflict);
        assert_eq!(kind_for_http_status(429), Kind::RateLimit);
        assert_eq!(kind_for_http_status(503), Kind::Server);
        assert_eq!(kind_for_http_status(405), Kind::Protocol);
        assert_eq!(kind_for_http_status(415), Kind::Protocol);
    }
}
