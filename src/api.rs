//! Typed adapters for the four business operations used by release one, plus the
//! profile validation read. Responses are validated against the OpenAPI schemas
//! before any data is considered successful (business spec §§6, 7.2, 10.3).

use serde::Serialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::form_urlencoded::Serializer;
use uuid::Uuid;

use crate::error::{CliError, Kind, Outcome, kind_for_http_status};
use crate::input::is_short_code;
use crate::transport::{Failure, FailureClass, Response, Transport, Verb};

pub const MAX_CREATE_BODY_BYTES: usize = 65_536;
const MAX_DESTINATION_BYTES: usize = 8192;

const LINK_FIELDS: [&str; 12] = [
    "id",
    "short_code",
    "short_url",
    "destination_url",
    "title",
    "tags",
    "owner_id",
    "enabled",
    "active",
    "visit_count",
    "created_at",
    "updated_at",
];

/// The exact 12-field Link representation (business spec §7.2).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Link {
    pub id: String,
    pub short_code: String,
    pub short_url: String,
    pub destination_url: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub owner_id: Option<String>,
    pub enabled: bool,
    pub active: bool,
    pub visit_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub role: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

fn protocol(detail: &str) -> CliError {
    CliError::new(
        Kind::Protocol,
        "client.unexpected_response",
        format!("The server returned an unexpected response ({detail})."),
    )
}

fn require<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a Value, String> {
    object.get(field).ok_or_else(|| format!("missing {field}"))
}

fn text_field(object: &Map<String, Value>, field: &str) -> Result<String, String> {
    require(object, field)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{field} is not a string"))
}

fn bool_field(object: &Map<String, Value>, field: &str) -> Result<bool, String> {
    require(object, field)?
        .as_bool()
        .ok_or_else(|| format!("{field} is not a boolean"))
}

fn uuid_field(object: &Map<String, Value>, field: &str) -> Result<String, String> {
    let text = text_field(object, field)?;
    Uuid::parse_str(&text).map_err(|_| format!("{field} is not a UUID"))?;
    Ok(text)
}

fn timestamp_field(object: &Map<String, Value>, field: &str) -> Result<String, String> {
    let text = text_field(object, field)?;
    if !text.ends_with('Z') {
        return Err(format!("{field} is not a UTC timestamp"));
    }
    OffsetDateTime::parse(&text, &Rfc3339).map_err(|_| format!("{field} is not RFC 3339"))?;
    Ok(text)
}

fn http_url_field(object: &Map<String, Value>, field: &str) -> Result<String, String> {
    let text = text_field(object, field)?;
    let lower = text.to_ascii_lowercase();
    let valid_scheme = lower.starts_with("http://") || lower.starts_with("https://");
    if !valid_scheme
        || text.len() > MAX_DESTINATION_BYTES
        || text.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("{field} is not a safe absolute HTTP(S) URL"));
    }
    Ok(text)
}

impl Link {
    pub fn from_value(value: &Value) -> Result<Link, String> {
        let object = value.as_object().ok_or("link is not an object")?;
        for field in LINK_FIELDS {
            if !object.contains_key(field) {
                return Err(format!("link is missing {field}"));
            }
        }
        let short_code = text_field(object, "short_code")?;
        if !is_short_code(&short_code) {
            return Err("short_code is outside the public syntax".to_owned());
        }
        let title = match require(object, "title")? {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            _ => return Err("title is neither a string nor null".to_owned()),
        };
        let owner_id = match require(object, "owner_id")? {
            Value::Null => None,
            Value::String(s) => {
                Uuid::parse_str(s).map_err(|_| "owner_id is not a UUID".to_owned())?;
                Some(s.clone())
            }
            _ => return Err("owner_id is neither a UUID nor null".to_owned()),
        };
        let tags = require(object, "tags")?
            .as_array()
            .ok_or("tags is not an array")?
            .iter()
            .map(|t| t.as_str().map(str::to_owned).ok_or("a tag is not a string"))
            .collect::<Result<Vec<_>, _>>()?;
        let visit_count = require(object, "visit_count")?
            .as_i64()
            .filter(|n| *n >= 0)
            .ok_or("visit_count is not an exact non-negative 64-bit integer")?;
        Ok(Link {
            id: uuid_field(object, "id")?,
            short_code,
            short_url: http_url_field(object, "short_url")?,
            destination_url: http_url_field(object, "destination_url")?,
            title,
            tags,
            owner_id,
            enabled: bool_field(object, "enabled")?,
            active: bool_field(object, "active")?,
            visit_count,
            created_at: timestamp_field(object, "created_at")?,
            updated_at: timestamp_field(object, "updated_at")?,
        })
    }
}

impl Account {
    pub fn from_value(value: &Value) -> Result<Account, String> {
        let object = value.as_object().ok_or("account is not an object")?;
        let role = text_field(object, "role")?;
        if role != "admin" && role != "user" {
            return Err("role is not admin or user".to_owned());
        }
        Ok(Account {
            id: uuid_field(object, "id")?,
            username: text_field(object, "username")?,
            role,
            enabled: bool_field(object, "enabled")?,
            created_at: timestamp_field(object, "created_at")?,
            updated_at: timestamp_field(object, "updated_at")?,
        })
    }
}

/// A structured server error body (business spec §7.3). Only a safe, lowercase
/// code is retained; the message is sanitized before any display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerError {
    pub code: String,
    pub message: String,
    pub request_id: Option<String>,
}

pub fn parse_error_body(body: &[u8]) -> Option<ServerError> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let object = value.as_object()?;
    let error = object.get("error")?.as_object()?;
    let code = error.get("code")?.as_str()?;
    let message = error.get("message")?.as_str()?;
    let safe_code = !code.is_empty()
        && code.len() <= 64
        && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    if !safe_code {
        return None;
    }
    let request_id = object
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(ServerError {
        code: code.to_owned(),
        message: truncate_message(message),
        request_id,
    })
}

fn truncate_message(message: &str) -> String {
    const LIMIT: usize = 300;
    if message.chars().count() <= LIMIT {
        message.to_owned()
    } else {
        let head: String = message.chars().take(LIMIT).collect();
        format!("{head}…")
    }
}

/// Reconciles the body and header request IDs (business spec §7.3).
fn reconcile_request_id(header: Option<&str>, body: Option<&str>) -> Option<String> {
    match (header, body) {
        (Some(h), Some(b)) if h != b => {
            // Mismatch: the header is used; the caller records the protocol warning.
            Some(h.to_owned())
        }
        (Some(h), _) => Some(h.to_owned()),
        (None, Some(b)) => Some(b.to_owned()),
        (None, None) => None,
    }
}

fn request_id_of(response: &Response) -> Option<String> {
    let body = parse_error_body(&response.body).and_then(|e| e.request_id);
    reconcile_request_id(response.request_id.as_deref(), body.as_deref())
}

/// Maps a transport failure for a read operation.
fn read_failure(failure: Failure) -> CliError {
    match failure.class {
        FailureClass::Interrupted => CliError::interrupted(),
        FailureClass::Deadline => CliError::new(
            Kind::Transport,
            "client.deadline_exceeded",
            "The HTTP operation deadline elapsed before the request completed.",
        ),
        FailureClass::Protocol => {
            CliError::new(Kind::Protocol, "client.unexpected_response", failure.detail)
        }
        FailureClass::Tls => CliError::new(Kind::Transport, "client.tls_failure", failure.detail),
        FailureClass::Connect | FailureClass::Io | FailureClass::Proxy => {
            CliError::new(Kind::Transport, "client.transport_failure", failure.detail)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mutation {
    Create,
    Delete,
}

fn uncertain_message(mutation: Mutation) -> &'static str {
    match mutation {
        Mutation::Create => {
            "The link may have been created, but no acknowledgement was received. The CLI did not retry, so run ferl list --query with a recognizable part of the destination to check. Duplicate destinations create separate links."
        }
        Mutation::Delete => {
            "The code may have been deleted, but no acknowledgement was received. The CLI did not retry. A later 404 does not prove this deletion succeeded."
        }
    }
}

/// Maps a transport failure for a mutation. Only a provably unsent request is
/// `not_sent`; anything else is an uncertain outcome that must not be replayed.
fn mutation_failure(failure: Failure, mutation: Mutation) -> CliError {
    if failure.class == FailureClass::Interrupted {
        if !failure.possibly_sent {
            return CliError::interrupted().with_outcome(Outcome::NotSent);
        }
        return CliError::new(
            Kind::Uncertain,
            "client.mutation_outcome_unknown",
            format!(
                "The operation was interrupted. {}",
                uncertain_message(mutation)
            ),
        )
        .with_outcome(Outcome::Unknown);
    }
    if !failure.possibly_sent {
        return match failure.class {
            FailureClass::Deadline => CliError::new(
                Kind::Transport,
                "client.deadline_exceeded",
                "The HTTP operation deadline elapsed before the request was sent.",
            )
            .with_outcome(Outcome::NotSent),
            _ => CliError::new(Kind::Transport, "client.transport_failure", failure.detail)
                .with_outcome(Outcome::NotSent),
        };
    }
    CliError::new(
        Kind::Uncertain,
        "client.mutation_outcome_unknown",
        uncertain_message(mutation),
    )
    .with_outcome(Outcome::Unknown)
}

fn http_error_for_read(response: &Response) -> CliError {
    let status = response.status;
    let request_id = request_id_of(response);
    let mut error = match parse_error_body(&response.body) {
        Some(server) => CliError::new(kind_for_http_status(status), &server.code, server.message),
        None => CliError::new(
            kind_for_http_status(status),
            "client.unexpected_response",
            format!("The server returned HTTP {status} without a recognized error body."),
        ),
    };
    if status == 429 || status == 503 {
        error.retry_after = response.retry_after;
    }
    error.with_http(status, request_id)
}

fn is_structured_rejection(status: u16, code: &str) -> bool {
    match status {
        400 | 401 | 403 | 404 | 409 | 413 | 415 | 429 | 405 => true,
        503 => code == "service_unavailable" || code == "code_generation_unavailable",
        _ => false,
    }
}

fn http_error_for_mutation(response: &Response) -> CliError {
    let status = response.status;
    let request_id = request_id_of(response);
    match parse_error_body(&response.body) {
        Some(server) if is_structured_rejection(status, &server.code) => {
            let mut error =
                CliError::new(kind_for_http_status(status), &server.code, server.message)
                    .with_outcome(Outcome::Rejected)
                    .with_http(status, request_id);
            error.retry_after = response.retry_after;
            error
        }
        Some(server) => CliError::new(
            Kind::Uncertain,
            "client.mutation_outcome_unknown",
            format!(
                "The server reported {} (HTTP {status}). {}",
                server.code,
                uncertain_message(Mutation::Create)
            ),
        )
        .with_outcome(Outcome::Unknown)
        .with_http(status, request_id),
        None => CliError::new(
            Kind::Uncertain,
            "client.mutation_outcome_unknown",
            format!(
                "The server returned HTTP {status} without a recognized body. {}",
                uncertain_message(Mutation::Create)
            ),
        )
        .with_outcome(Outcome::Unknown)
        .with_http(status, request_id),
    }
}

fn parse_json_body(response: &Response) -> Result<Value, CliError> {
    serde_json::from_slice(&response.body).map_err(|_| protocol("malformed JSON"))
}

pub fn encode_query(pairs: &[(&str, String)]) -> String {
    let mut serializer = Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

/// Profile validation: `GET /api/v1/me` (business spec §6).
pub fn get_current_account(transport: &Transport) -> Result<(Account, Option<String>), CliError> {
    let response = transport
        .get_with_retries("/api/v1/me")
        .map_err(read_failure)?;
    let request_id = request_id_of(&response);
    if response.body_too_large || response.body_unreadable {
        return Err(protocol("the response body could not be read completely"));
    }
    match response.status {
        200 => {
            let value = parse_json_body(&response)?;
            let account = Account::from_value(&value).map_err(|d| protocol(&d))?;
            Ok((account, request_id))
        }
        _ => Err(http_error_for_read(&response)),
    }
}

/// Own-scope link listing, with the query fields the release supports.
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    pub search: Option<String>,
    pub limit: u16,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LinkPage {
    pub items: Vec<Link>,
    pub next_cursor: Option<String>,
    pub request_id: Option<String>,
}

pub fn list_links_page(transport: &Transport, query: &ListQuery) -> Result<LinkPage, CliError> {
    let mut pairs = vec![
        ("scope", "own".to_owned()),
        ("limit", query.limit.to_string()),
    ];
    if let Some(search) = &query.search {
        pairs.push(("q", search.clone()));
    }
    if let Some(cursor) = &query.cursor {
        pairs.push(("cursor", cursor.clone()));
    }
    let target = format!("/api/v1/links?{}", encode_query(&pairs));
    let response = transport.get_with_retries(&target).map_err(read_failure)?;
    if response.status != 200 {
        return Err(http_error_for_read(&response));
    }
    if response.body_too_large {
        return Err(CliError::new(
            Kind::Protocol,
            "client.page_too_large",
            "A page exceeded the 16 MiB per-response bound.",
        ));
    }
    if response.body_unreadable {
        return Err(CliError::new(
            Kind::Transport,
            "client.transport_failure",
            "The page body could not be read.",
        ));
    }
    let request_id = request_id_of(&response);
    let value = parse_json_body(&response)?;
    let object = value
        .as_object()
        .ok_or_else(|| protocol("collection is not an object"))?;
    if object.len() != 2 {
        return Err(protocol("collection has unexpected members"));
    }
    let items_value = object
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol("items missing"))?;
    let items = items_value
        .iter()
        .map(|item| Link::from_value(item).map_err(|d| protocol(&d)))
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = match object.get("next_cursor") {
        Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => {
            return Err(protocol(
                "next_cursor is neither a non-empty string nor null",
            ));
        }
    };
    Ok(LinkPage {
        items,
        next_cursor,
        request_id,
    })
}

#[derive(Debug, Serialize)]
struct CreateBody<'a> {
    destination_url: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<&'a [String]>,
}

#[derive(Debug, Clone)]
pub struct Created {
    pub link: Link,
    pub request_id: Option<String>,
}

/// Creates one account-owned link. Sent exactly once; never replayed.
pub fn create_link(
    transport: &Transport,
    destination: &str,
    title: Option<&str>,
    tags: &[String],
) -> Result<Created, CliError> {
    let body = CreateBody {
        destination_url: destination,
        title,
        tags: (!tags.is_empty()).then_some(tags),
    };
    let bytes = serde_json::to_vec(&body).map_err(|e| CliError::internal(e.to_string()))?;
    if bytes.len() > MAX_CREATE_BODY_BYTES {
        return Err(CliError::usage(
            "The creation request exceeds the 65536-byte body limit.",
        ));
    }
    let response = transport
        .send(Verb::Post, "/api/v1/links", Some(bytes))
        .map_err(|failure| mutation_failure(failure, Mutation::Create))?;
    if response.status != 201 {
        return Err(http_error_for_mutation(&response));
    }
    let request_id = request_id_of(&response);
    let committed = |detail: &str| protocol(detail).with_outcome(Outcome::Committed);
    if response.body_too_large || response.body_unreadable {
        return Err(committed(
            "the created-link body could not be read completely",
        ));
    }
    let value: Value =
        serde_json::from_slice(&response.body).map_err(|_| committed("malformed JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| committed("creation is not an object"))?;
    if object.contains_key("management_token") {
        return Err(committed(
            "an anonymous creation shape was returned to an account",
        ));
    }
    let link_value = object
        .get("link")
        .ok_or_else(|| committed("link is missing"))?;
    let link = Link::from_value(link_value).map_err(|d| committed(&d))?;
    if link.owner_id.is_none() {
        return Err(committed("an anonymous link was returned to an account"));
    }
    Ok(Created { link, request_id })
}

#[derive(Debug, Clone)]
pub struct Deleted {
    pub request_id: Option<String>,
}

/// Deletes exactly one short code with a bodyless request. Sent exactly once.
pub fn delete_link(transport: &Transport, code: &str) -> Result<Deleted, CliError> {
    let target = format!("/api/v1/links/{code}");
    let response = transport
        .send(Verb::Delete, &target, None)
        .map_err(|failure| mutation_failure(failure, Mutation::Delete))?;
    let request_id = request_id_of(&response);
    match response.status {
        204 => Ok(Deleted { request_id }),
        _ => Err(delete_status_error(&response)),
    }
}

fn delete_status_error(response: &Response) -> CliError {
    let status = response.status;
    let request_id = request_id_of(response);
    match parse_error_body(&response.body) {
        Some(server) if is_structured_rejection(status, &server.code) => {
            let mut error =
                CliError::new(kind_for_http_status(status), &server.code, server.message)
                    .with_outcome(Outcome::Rejected)
                    .with_http(status, request_id);
            error.retry_after = response.retry_after;
            error
        }
        _ => CliError::new(
            Kind::Uncertain,
            "client.mutation_outcome_unknown",
            format!(
                "The server returned HTTP {status} without a recognized rejection. {}",
                uncertain_message(Mutation::Delete)
            ),
        )
        .with_outcome(Outcome::Unknown)
        .with_http(status, request_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn link_json() -> Value {
        json!({
            "id": "1f1bf5f6-075c-4cf1-a44b-05fca986b100",
            "short_code": "a1b2c3d4",
            "short_url": "https://s.example.com/a1b2c3d4",
            "destination_url": "https://example.com/docs?edition=2#intro",
            "title": null,
            "tags": [],
            "owner_id": "4a8ffcd5-3066-4519-a68e-060f956ab500",
            "enabled": true,
            "active": true,
            "visit_count": 9007199254740993_i64,
            "created_at": "2026-10-29T10:00:00.000000Z",
            "updated_at": "2026-10-29T10:00:00.000000Z"
        })
    }

    #[test]
    fn link_keeps_exact_counts_above_2_pow_53_and_nulls() {
        let link = Link::from_value(&link_json()).expect("valid link");
        assert_eq!(link.visit_count, 9_007_199_254_740_993);
        assert!(link.title.is_none());
        let max = {
            let mut v = link_json();
            v["visit_count"] = json!(i64::MAX);
            v
        };
        assert_eq!(Link::from_value(&max).expect("max").visit_count, i64::MAX);
    }

    #[test]
    fn link_rejects_missing_fields_bad_counts_and_bad_timestamps() {
        let mut missing = link_json();
        missing.as_object_mut().expect("object").remove("title");
        assert!(Link::from_value(&missing).is_err());

        let mut float = link_json();
        float["visit_count"] = json!(1.5);
        assert!(Link::from_value(&float).is_err());

        let mut negative = link_json();
        negative["visit_count"] = json!(-1);
        assert!(Link::from_value(&negative).is_err());

        let mut overflow = link_json();
        overflow["visit_count"] = json!(9223372036854775808_u64);
        assert!(Link::from_value(&overflow).is_err());

        let mut offset = link_json();
        offset["created_at"] = json!("2026-10-29T10:00:00+02:00");
        assert!(Link::from_value(&offset).is_err());
    }

    #[test]
    fn account_representation_is_validated() {
        let account = json!({
            "id": "4a8ffcd5-3066-4519-a68e-060f956ab500",
            "username": "ferl_cli_admin",
            "role": "admin",
            "enabled": true,
            "created_at": "2026-10-29T10:00:00Z",
            "updated_at": "2026-10-29T10:00:00Z"
        });
        assert!(Account::from_value(&account).is_ok());
        let mut bad = account.clone();
        bad["role"] = json!("superuser");
        assert!(Account::from_value(&bad).is_err());
    }

    #[test]
    fn error_bodies_retain_only_safe_codes() {
        let good =
            br#"{"error":{"code":"short_code_unavailable","message":"Taken."},"request_id":"r1"}"#;
        let parsed = parse_error_body(good).expect("parsed");
        assert_eq!(parsed.code, "short_code_unavailable");
        assert_eq!(parsed.request_id.as_deref(), Some("r1"));
        let unsafe_code = br#"{"error":{"code":"Bad Code","message":"x"},"request_id":"r1"}"#;
        assert!(parse_error_body(unsafe_code).is_none());
        assert!(parse_error_body(b"<html>502</html>").is_none());
    }

    #[test]
    fn request_id_header_wins_on_mismatch() {
        assert_eq!(
            reconcile_request_id(Some("h"), Some("b")).as_deref(),
            Some("h")
        );
        assert_eq!(reconcile_request_id(None, Some("b")).as_deref(), Some("b"));
        assert_eq!(reconcile_request_id(None, None), None);
    }

    #[test]
    fn query_values_are_encoded_once() {
        let query = encode_query(&[
            ("q", "50%_off & a+b,\\ 東京".to_owned()),
            ("limit", "50".to_owned()),
        ]);
        assert!(query.contains("q=50%25_off+%26+a%2Bb%2C%5C+"));
        assert!(query.ends_with("&limit=50"));
    }

    #[test]
    fn unknown_mutation_responses_never_claim_rejection() {
        let response = Response {
            status: 500,
            request_id: None,
            retry_after: None,
            body: b"<html>".to_vec(),
            body_too_large: false,
            body_unreadable: false,
        };
        let error = http_error_for_mutation(&response);
        assert_eq!(error.kind, Kind::Uncertain);
        assert_eq!(error.outcome, Outcome::Unknown);
        assert_eq!(error.exit_code(), 13);
    }

    #[test]
    fn structured_conflict_is_a_definitive_rejection() {
        let response = Response {
            status: 409,
            request_id: Some("r9".to_owned()),
            retry_after: None,
            body: br#"{"error":{"code":"short_code_unavailable","message":"Taken."},"request_id":"r9"}"#.to_vec(),
            body_too_large: false,
            body_unreadable: false,
        };
        let error = http_error_for_mutation(&response);
        assert_eq!(error.outcome, Outcome::Rejected);
        assert_eq!(error.exit_code(), 7);
    }

    #[test]
    fn possible_dispatch_is_uncertain_and_pre_dispatch_is_not_sent() {
        let unsent = Failure {
            class: FailureClass::Connect,
            possibly_sent: false,
            retryable: true,
            detail: "refused".to_owned(),
        };
        assert_eq!(
            mutation_failure(unsent, Mutation::Create).outcome,
            Outcome::NotSent
        );
        let sent = Failure {
            class: FailureClass::Io,
            possibly_sent: true,
            retryable: true,
            detail: "reset".to_owned(),
        };
        let error = mutation_failure(sent, Mutation::Create);
        assert_eq!(error.outcome, Outcome::Unknown);
        assert_eq!(error.exit_code(), 13);
    }
}
