//! HTTP/1.1 transport over rustls for the business operations and profile check.
//!
//! One profile supplies every authenticated request's origin, key, and CA setting.
//! This client sends exactly one request per connection, follows no redirects,
//! stores no cookies or cache entries, and retries nothing itself; only GET
//! operations are retried by the caller (business spec §10). A small hand-written
//! client is used because the common HTTP URI type cannot carry the 87,387-byte
//! continuation cursors the release must support.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::cancel::{self, Stop};
use crate::error::CliError;
use crate::origin::Origin;

pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_READ_RETRIES: u8 = 2;
pub const MAX_SUCCESS_BODY_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ERROR_BODY_BYTES: u64 = 64 * 1024;
const MAX_HEAD_BYTES: usize = 64 * 1024;

const USER_AGENT: &str = concat!("ferl/", env!("CARGO_PKG_VERSION"));

/// Deadline for one command's HTTP work, shared across retries and pages.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    end: Instant,
}

impl Deadline {
    pub fn after(total: Duration) -> Self {
        Deadline {
            end: Instant::now() + total,
        }
    }

    pub fn remaining(&self) -> Option<Duration> {
        let now = Instant::now();
        (now < self.end).then(|| self.end - now)
    }

    pub fn instant(&self) -> Instant {
        self.end
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub connect: Duration,
    pub overall: Duration,
    pub read_retries: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            connect: DEFAULT_CONNECT_TIMEOUT,
            overall: DEFAULT_OPERATION_TIMEOUT,
            read_retries: DEFAULT_READ_RETRIES,
        }
    }
}

/// A complete HTTP response whose body is bounded.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub request_id: Option<String>,
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
    /// The body exceeded its size bound; it cannot be trusted as a valid success.
    pub body_too_large: bool,
    /// The body could not be read to its end after the status arrived.
    pub body_unreadable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Deadline,
    Connect,
    Tls,
    Proxy,
    Io,
    Protocol,
    Interrupted,
}

/// A transport-level failure. `possibly_sent` is true only once request bytes may
/// have reached the server; earlier failures are provably not sent.
#[derive(Debug, Clone)]
pub struct Failure {
    pub class: FailureClass,
    pub possibly_sent: bool,
    pub retryable: bool,
    pub detail: String,
}

impl Failure {
    fn new(
        class: FailureClass,
        possibly_sent: bool,
        retryable: bool,
        detail: impl Into<String>,
    ) -> Self {
        Failure {
            class,
            possibly_sent,
            retryable,
            detail: detail.into(),
        }
    }

    pub fn deadline(possibly_sent: bool) -> Self {
        Failure::new(
            FailureClass::Deadline,
            possibly_sent,
            false,
            "The HTTP operation deadline elapsed.",
        )
    }

    pub fn interrupted(possibly_sent: bool) -> Self {
        Failure::new(
            FailureClass::Interrupted,
            possibly_sent,
            false,
            "The operation was interrupted.",
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Get,
    Post,
    Delete,
}

trait Duplex: Read + Write + Send {}
impl<T: Read + Write + Send> Duplex for T {}
type Stream = Box<dyn Duplex>;

#[derive(Debug, Clone)]
struct ProxySpec {
    host: String,
    port: u16,
    https: bool,
    basic_auth: Option<String>,
}

struct Inner {
    host: String,
    port: u16,
    https: bool,
    host_header: String,
    authorization: String,
    tls: Option<Arc<ClientConfig>>,
    proxy: Option<ProxySpec>,
    connect: Duration,
}

pub struct Transport {
    inner: Arc<Inner>,
    limits: Limits,
    deadline: Deadline,
}

impl Transport {
    /// Builds the transport for one command. The key is held only in the
    /// `Authorization` value and is never logged or displayed.
    pub fn new(
        origin: &Origin,
        key: &str,
        ca_bundle: Option<&Path>,
        limits: Limits,
    ) -> Result<Transport, CliError> {
        let https = origin.is_https();
        let proxy = match proxy_for(origin, &|name| std::env::var(name).ok())? {
            Some(url) => Some(parse_proxy(&url)?),
            None => None,
        };
        let needs_tls = https || proxy.as_ref().is_some_and(|p| p.https);
        let tls = if needs_tls {
            Some(tls_client_config(ca_bundle)?)
        } else {
            None
        };
        let host_header = origin
            .text()
            .split_once("://")
            .map(|(_, rest)| rest.to_owned())
            .unwrap_or_default();
        Ok(Transport {
            inner: Arc::new(Inner {
                host: origin.host().to_owned(),
                port: origin.port(),
                https,
                host_header,
                authorization: format!("Bearer {key}"),
                tls,
                proxy,
                connect: limits.connect,
            }),
            limits,
            deadline: Deadline::after(limits.overall),
        })
    }

    /// Sends one request. `target` is an origin-relative path with an already-encoded
    /// query. Callers send each mutation exactly once; this method never retries.
    pub fn send(
        &self,
        verb: Verb,
        target: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Response, Failure> {
        let remaining = self
            .deadline
            .remaining()
            .ok_or_else(|| Failure::deadline(false))?;
        let dispatched = Arc::new(AtomicBool::new(false));
        let inner = self.inner.clone();
        let target = target.to_owned();
        let flag = dispatched.clone();
        let until = self.deadline.instant();
        let outcome = cancel::run_until(until, move || {
            exchange(&inner, verb, &target, body.as_deref(), remaining, &flag)
        });
        match outcome {
            Ok(result) => result,
            Err(Stop::Interrupted) => Err(Failure::interrupted(dispatched.load(Ordering::SeqCst))),
            Err(Stop::TimedOut) => Err(Failure::deadline(dispatched.load(Ordering::SeqCst))),
            Err(Stop::Failed) => Err(Failure::new(
                FailureClass::Protocol,
                dispatched.load(Ordering::SeqCst),
                false,
                "The request could not be completed.",
            )),
        }
    }

    /// Sends a read with bounded retries for transient failures and 502/503/504
    /// responses. Waits fit within the remaining command deadline.
    pub fn get_with_retries(&self, target: &str) -> Result<Response, Failure> {
        let mut attempt: u8 = 0;
        loop {
            let result = self.send(Verb::Get, target, None);
            let retry_wait = match &result {
                Ok(response) if matches!(response.status, 502..=504) => {
                    response.retry_after.map(Duration::from_secs)
                }
                Err(failure) if failure.retryable => None,
                _ => return result,
            };
            if attempt >= self.limits.read_retries {
                return result;
            }
            let wait = retry_wait.unwrap_or_else(|| backoff(attempt));
            match self.deadline.remaining() {
                Some(remaining) if remaining > wait => {}
                _ => return result,
            }
            if cancel::sleep(wait).is_err() {
                return Err(Failure::interrupted(false));
            }
            attempt += 1;
        }
    }
}

fn backoff(attempt: u8) -> Duration {
    let base = if attempt == 0 {
        Duration::from_millis(250)
    } else {
        Duration::from_secs(1)
    };
    let jitter_ms = u64::from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
            % 100,
    );
    base + Duration::from_millis(jitter_ms)
}

/// One request/response exchange on a fresh connection. Runs on a helper thread.
fn exchange(
    inner: &Inner,
    verb: Verb,
    target: &str,
    body: Option<&[u8]>,
    budget: Duration,
    dispatched: &AtomicBool,
) -> Result<Response, Failure> {
    let (stream, socket) = open(inner, budget)?;
    socket
        .set_read_timeout(Some(budget))
        .and_then(|_| socket.set_write_timeout(Some(budget)))
        .map_err(|e| io_failure(&e, false))?;
    let mut reader = BufReader::new(stream);
    let request = build_request(inner, verb, target, body);
    dispatched.store(true, Ordering::SeqCst);
    let writing = reader
        .get_mut()
        .write_all(&request)
        .and_then(|_| match body {
            Some(bytes) => reader.get_mut().write_all(bytes),
            None => Ok(()),
        })
        .and_then(|_| reader.get_mut().flush());
    if let Err(error) = writing {
        return Err(io_failure(&error, true));
    }
    read_response(&mut reader)
}

fn build_request(inner: &Inner, verb: Verb, target: &str, body: Option<&[u8]>) -> Vec<u8> {
    let method = match verb {
        Verb::Get => "GET",
        Verb::Post => "POST",
        Verb::Delete => "DELETE",
    };
    let mut head = format!(
        "{method} {target} HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nAccept: application/json\r\nUser-Agent: {USER_AGENT}\r\nAccept-Encoding: identity\r\nConnection: close\r\n",
        inner.host_header, inner.authorization
    );
    if let Some(bytes) = body {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            bytes.len()
        ));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

fn io_failure(error: &io::Error, possibly_sent: bool) -> Failure {
    let retryable = matches!(
        error.kind(),
        io::ErrorKind::TimedOut
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof
    );
    let class = if possibly_sent {
        FailureClass::Io
    } else {
        FailureClass::Connect
    };
    // Retryability describes the transient kind only. Only GET operations are retried,
    // and a GET is safe to repeat even when an earlier attempt may have reached the server.
    Failure::new(
        class,
        possibly_sent,
        retryable,
        "A network I/O error occurred.",
    )
}

/// Opens the connection, through a proxy tunnel when one is selected, and completes TLS.
fn open(inner: &Inner, budget: Duration) -> Result<(Stream, TcpStream), Failure> {
    let (host, port) = match &inner.proxy {
        Some(proxy) => (proxy.host.as_str(), proxy.port),
        None => (inner.host.as_str(), inner.port),
    };
    let tcp = connect_tcp(host, port, inner.connect.min(budget))?;
    let socket = tcp.try_clone().map_err(|e| io_failure(&e, false))?;
    let mut stream: Stream = Box::new(tcp);
    if let Some(proxy) = &inner.proxy {
        if proxy.https {
            let config = inner.tls.clone().ok_or_else(|| {
                Failure::new(FailureClass::Tls, false, false, "TLS is unavailable.")
            })?;
            stream = tls_wrap(stream, &proxy.host, config)?;
        }
        stream = connect_tunnel(stream, &inner.host, inner.port, proxy.basic_auth.as_deref())?;
    }
    if inner.https {
        let config = inner
            .tls
            .clone()
            .ok_or_else(|| Failure::new(FailureClass::Tls, false, false, "TLS is unavailable."))?;
        stream = tls_wrap(stream, &inner.host, config)?;
    }
    Ok((stream, socket))
}

fn connect_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, Failure> {
    let addresses: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|_| {
            Failure::new(
                FailureClass::Connect,
                false,
                false,
                "The server name could not be resolved.",
            )
        })?
        .collect();
    if addresses.is_empty() {
        return Err(Failure::new(
            FailureClass::Connect,
            false,
            false,
            "The server name could not be resolved.",
        ));
    }
    let mut last = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error),
        }
    }
    let error = last.unwrap_or_else(|| io::Error::other("no address"));
    Err(io_failure(&error, false))
}

fn tls_wrap(stream: Stream, host: &str, config: Arc<ClientConfig>) -> Result<Stream, Failure> {
    let name = ServerName::try_from(host.to_owned()).map_err(|_| {
        Failure::new(
            FailureClass::Tls,
            false,
            false,
            "The server name is not valid for TLS.",
        )
    })?;
    let connection = ClientConnection::new(config, name).map_err(|_| {
        Failure::new(
            FailureClass::Tls,
            false,
            false,
            "The TLS session could not be created.",
        )
    })?;
    let mut tls = StreamOwned::new(connection, stream);
    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock).map_err(|_| {
            Failure::new(
                FailureClass::Tls,
                false,
                false,
                "The TLS handshake or certificate check failed.",
            )
        })?;
    }
    Ok(Box::new(tls))
}

fn connect_tunnel(
    mut stream: Stream,
    host: &str,
    port: u16,
    basic: Option<&str>,
) -> Result<Stream, Failure> {
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(credentials) = basic {
        request.push_str(&format!("Proxy-Authorization: Basic {credentials}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|_| {
            Failure::new(
                FailureClass::Proxy,
                false,
                false,
                "The proxy could not be reached.",
            )
        })?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let read = stream.read(&mut byte).map_err(|_| {
            Failure::new(
                FailureClass::Proxy,
                false,
                false,
                "The proxy did not respond.",
            )
        })?;
        if read == 0 || head.len() > MAX_HEAD_BYTES {
            return Err(Failure::new(
                FailureClass::Proxy,
                false,
                false,
                "The proxy response was malformed.",
            ));
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let status_ok = head.starts_with(b"HTTP/1.1 2") || head.starts_with(b"HTTP/1.0 2");
    if !status_ok {
        return Err(Failure::new(
            FailureClass::Proxy,
            false,
            false,
            "The proxy refused the tunnel.",
        ));
    }
    Ok(stream)
}

fn read_response(reader: &mut BufReader<Stream>) -> Result<Response, Failure> {
    let mut head = Vec::new();
    loop {
        let mut line = Vec::new();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| io_failure(&e, true))?;
        if read == 0 {
            return Err(Failure::new(
                FailureClass::Io,
                true,
                true,
                "The connection closed before a response.",
            ));
        }
        head.extend_from_slice(&line);
        if head.len() > MAX_HEAD_BYTES {
            return Err(Failure::new(
                FailureClass::Protocol,
                true,
                false,
                "The response header is too large.",
            ));
        }
        if line == b"\r\n" || line == b"\n" {
            break;
        }
    }
    let mut headers = [httparse::EMPTY_HEADER; 96];
    let mut parsed = httparse::Response::new(&mut headers);
    match parsed.parse(&head) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => {
            return Err(Failure::new(
                FailureClass::Protocol,
                true,
                false,
                "The HTTP response was malformed.",
            ));
        }
    }
    let status = parsed.code.ok_or_else(|| {
        Failure::new(
            FailureClass::Protocol,
            true,
            false,
            "The HTTP status is missing.",
        )
    })?;
    let value_of = |name: &str| -> Option<String> {
        parsed
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .and_then(|h| std::str::from_utf8(h.value).ok())
            .map(|v| v.trim().to_owned())
    };
    let request_id = value_of("x-request-id");
    let retry_after = value_of("retry-after")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0);
    let content_length = value_of("content-length").and_then(|v| v.parse::<u64>().ok());
    let chunked =
        value_of("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let limit = if (200..300).contains(&status) {
        MAX_SUCCESS_BODY_BYTES
    } else {
        MAX_ERROR_BODY_BYTES
    };
    let mut response = Response {
        status,
        request_id,
        retry_after,
        body: Vec::new(),
        body_too_large: false,
        body_unreadable: false,
    };
    if matches!(status, 100..=199 | 204 | 304) {
        return Ok(response);
    }
    let outcome = if chunked {
        read_chunked(reader, limit)
    } else if let Some(length) = content_length {
        read_bounded(reader, length.min(limit + 1), limit)
    } else {
        read_bounded(reader, limit + 1, limit)
    };
    match outcome {
        Ok(bytes) => response.body = bytes,
        Err(BodyError::TooLarge) => response.body_too_large = true,
        Err(BodyError::Unreadable) => response.body_unreadable = true,
    }
    Ok(response)
}

enum BodyError {
    TooLarge,
    Unreadable,
}

fn read_bounded(reader: &mut impl Read, want: u64, limit: u64) -> Result<Vec<u8>, BodyError> {
    let mut bytes = Vec::new();
    reader
        .take(want)
        .read_to_end(&mut bytes)
        .map_err(|_| BodyError::Unreadable)?;
    if bytes.len() as u64 > limit {
        return Err(BodyError::TooLarge);
    }
    Ok(bytes)
}

fn read_chunked(reader: &mut BufReader<Stream>, limit: u64) -> Result<Vec<u8>, BodyError> {
    let mut bytes = Vec::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|_| BodyError::Unreadable)?;
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = u64::from_str_radix(size_text, 16).map_err(|_| BodyError::Unreadable)?;
        if size == 0 {
            loop {
                let mut trailer = String::new();
                let read = reader
                    .read_line(&mut trailer)
                    .map_err(|_| BodyError::Unreadable)?;
                if read == 0 || trailer == "\r\n" || trailer == "\n" {
                    break;
                }
            }
            return Ok(bytes);
        }
        if bytes.len() as u64 + size > limit {
            return Err(BodyError::TooLarge);
        }
        let start = bytes.len();
        bytes.resize(start + size as usize, 0);
        reader
            .read_exact(&mut bytes[start..])
            .map_err(|_| BodyError::Unreadable)?;
        let mut crlf = [0u8; 2];
        reader
            .read_exact(&mut crlf)
            .map_err(|_| BodyError::Unreadable)?;
    }
}

/// Builds the rustls client configuration: the Mozilla roots plus any private anchors
/// from an explicit PEM bundle (business spec §10.1). Nothing from the environment.
fn tls_client_config(ca_bundle: Option<&Path>) -> Result<Arc<ClientConfig>, CliError> {
    let mut roots = RootCertStore::empty();
    for der in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
        let _ = roots.add(der.clone());
    }
    if let Some(path) = ca_bundle {
        for cert in read_ca_bundle(path)? {
            roots.add(cert).map_err(|_| {
                CliError::config(format!(
                    "The CA bundle {} contains a certificate that cannot be used.",
                    path.display()
                ))
            })?;
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| CliError::internal("TLS protocol setup failed."))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

fn read_ca_bundle(path: &Path) -> Result<Vec<CertificateDer<'static>>, CliError> {
    let bytes = std::fs::read(path).map_err(|_| {
        CliError::config(format!(
            "The CA bundle {} cannot be read. Fix the profile's CA setting.",
            path.display()
        ))
    })?;
    let certificates: Result<Vec<CertificateDer<'static>>, _> =
        CertificateDer::pem_slice_iter(&bytes).collect();
    match certificates {
        Ok(list) if !list.is_empty() => Ok(list),
        _ => Err(CliError::config(format!(
            "The CA bundle {} must contain PEM certificates and nothing else.",
            path.display()
        ))),
    }
}

/// Checks a CA bundle's readability and PEM content without any network use.
pub fn validate_ca_bundle(path: &Path) -> Result<(), CliError> {
    tls_client_config(Some(path)).map(|_| ())
}

fn parse_proxy(text: &str) -> Result<ProxySpec, CliError> {
    let invalid =
        || CliError::config("The selected proxy setting is not a valid HTTP or HTTPS proxy URL.");
    let url = url::Url::parse(text).map_err(|_| invalid())?;
    let https = match url.scheme() {
        "https" => true,
        "http" => false,
        _ => return Err(invalid()),
    };
    let host = url
        .host_str()
        .ok_or_else(invalid)?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = url.port_or_known_default().ok_or_else(invalid)?;
    let basic_auth = if url.username().is_empty() {
        None
    } else {
        let user = percent_encoding::percent_decode_str(url.username()).decode_utf8_lossy();
        let pass =
            percent_encoding::percent_decode_str(url.password().unwrap_or("")).decode_utf8_lossy();
        Some(base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}")))
    };
    Ok(ProxySpec {
        host,
        port,
        https,
        basic_auth,
    })
}

/// Chooses the proxy for an origin from the environment (business spec §10.1).
///
/// The lowercase variable wins over its uppercase counterpart even when empty,
/// which disables that pair. Plaintext loopback development is always direct.
pub fn proxy_for(
    origin: &Origin,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<String>, CliError> {
    if !origin.is_https() {
        return Ok(None);
    }
    let pair = |lower: &str, upper: &str| lookup(lower).or_else(|| lookup(upper));
    if let Some(no_proxy) = pair("no_proxy", "NO_PROXY") {
        if no_proxy_matches(&no_proxy, origin.host(), origin.port()) {
            return Ok(None);
        }
    }
    let selected = pair("https_proxy", "HTTPS_PROXY")
        .filter(|v| !v.is_empty())
        .or_else(|| pair("all_proxy", "ALL_PROXY").filter(|v| !v.is_empty()));
    let Some(url) = selected else {
        return Ok(None);
    };
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(CliError::config(
            "The proxy setting must be an HTTP or HTTPS proxy URL; SOCKS and PAC are not supported.",
        ));
    }
    Ok(Some(url))
}

/// Matches exact hosts/IPs, `.suffix` domains, optional `:port`, and `*`.
pub fn no_proxy_matches(list: &str, host: &str, port: u16) -> bool {
    let host = host.to_ascii_lowercase();
    list.split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .any(|entry| {
            if entry == "*" {
                return true;
            }
            let (name, entry_port) = match entry.rsplit_once(':') {
                Some((name, p)) if !name.contains(':') || name.starts_with('[') => {
                    match p.parse::<u16>() {
                        Ok(parsed) => (name, Some(parsed)),
                        Err(_) => (entry, None),
                    }
                }
                _ => (entry, None),
            };
            if entry_port.is_some_and(|required| required != port) {
                return false;
            }
            let name = name
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_ascii_lowercase();
            if let Some(suffix) = name.strip_prefix('.') {
                host == suffix || host.ends_with(&format!(".{suffix}"))
            } else {
                host == name
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::parse_origin;

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn https_proxy_precedence_and_empty_lowercase_disable() {
        let origin = parse_origin("https://api.example.com").expect("origin");
        let env = lookup(&[
            ("HTTPS_PROXY", "http://upper:1"),
            ("https_proxy", "http://lower:2"),
        ]);
        assert_eq!(
            proxy_for(&origin, &env).expect("ok").as_deref(),
            Some("http://lower:2")
        );
        let env = lookup(&[("https_proxy", ""), ("HTTPS_PROXY", "http://upper:1")]);
        assert_eq!(proxy_for(&origin, &env).expect("ok"), None);
        let env = lookup(&[("ALL_PROXY", "http://all:3")]);
        assert_eq!(
            proxy_for(&origin, &env).expect("ok").as_deref(),
            Some("http://all:3")
        );
    }

    #[test]
    fn http_proxy_variables_are_ignored_and_loopback_is_direct() {
        let origin = parse_origin("https://api.example.com").expect("origin");
        let env = lookup(&[("http_proxy", "http://ignored:1")]);
        assert_eq!(proxy_for(&origin, &env).expect("ok"), None);
        let loopback = parse_origin("http://127.0.0.1:18080").expect("origin");
        let env = lookup(&[("https_proxy", "http://proxy:1")]);
        assert_eq!(proxy_for(&loopback, &env).expect("ok"), None);
    }

    #[test]
    fn unsupported_proxy_schemes_fail_locally() {
        let origin = parse_origin("https://api.example.com").expect("origin");
        let env = lookup(&[("https_proxy", "socks5://proxy:1080")]);
        assert_eq!(proxy_for(&origin, &env).expect_err("socks").exit_code(), 3);
    }

    #[test]
    fn proxy_urls_carry_host_port_and_decoded_basic_credentials() {
        let spec = parse_proxy("http://us%40er:p%3Ass@proxy.internal:3128").expect("proxy");
        assert_eq!(spec.host, "proxy.internal");
        assert_eq!(spec.port, 3128);
        assert!(!spec.https);
        assert_eq!(spec.basic_auth.as_deref(), Some("dXNAZXI6cDpzcw=="));
    }

    #[test]
    fn no_proxy_entries_match_exact_suffix_port_and_wildcard() {
        assert!(no_proxy_matches("api.example.com", "api.example.com", 443));
        assert!(no_proxy_matches(".example.com", "api.example.com", 443));
        assert!(no_proxy_matches(".example.com", "example.com", 443));
        assert!(!no_proxy_matches(".example.com", "badexample.com", 443));
        assert!(no_proxy_matches(
            "api.example.com:8443",
            "api.example.com",
            8443
        ));
        assert!(!no_proxy_matches(
            "api.example.com:8443",
            "api.example.com",
            443
        ));
        assert!(no_proxy_matches("*", "anything", 1));
        assert!(no_proxy_matches("10.0.0.5, [::1]", "::1", 443));
    }

    #[test]
    fn deadline_reports_elapsed_time() {
        let deadline = Deadline::after(Duration::from_millis(0));
        assert!(deadline.remaining().is_none());
        let later = Deadline::after(Duration::from_secs(10));
        assert!(later.remaining().is_some());
    }

    #[test]
    fn transient_io_kinds_are_retryable_for_reads_whether_or_not_sent() {
        let reset = io::Error::from(io::ErrorKind::ConnectionReset);
        assert!(io_failure(&reset, true).retryable);
        assert!(io_failure(&reset, false).retryable);
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(!io_failure(&denied, true).retryable);
    }

    #[test]
    fn backoff_stays_within_documented_bounds() {
        let first = backoff(0);
        assert!(first >= Duration::from_millis(250) && first < Duration::from_millis(350));
        let second = backoff(1);
        assert!(second >= Duration::from_secs(1) && second < Duration::from_millis(1100));
    }

    #[test]
    fn chunked_bodies_decode_and_respect_the_limit() {
        let wire = b"5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nX-T: 1\r\n\r\n".to_vec();
        let mut reader = BufReader::new(Box::new(io::Cursor::new(wire)) as Stream);
        let decoded = read_chunked(&mut reader, 1024).ok().expect("decoded");
        assert_eq!(decoded, b"hello world");
        let big = b"ff\r\n".to_vec();
        let mut reader = BufReader::new(Box::new(io::Cursor::new(big)) as Stream);
        assert!(matches!(
            read_chunked(&mut reader, 10),
            Err(BodyError::TooLarge)
        ));
    }

    #[test]
    fn ca_bundles_must_contain_only_certificates() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("empty.pem");
        std::fs::write(&path, "not a certificate").expect("write");
        assert_eq!(
            validate_ca_bundle(&path).expect_err("no certs").exit_code(),
            3
        );
        assert_eq!(
            validate_ca_bundle(&dir.path().join("missing.pem"))
                .expect_err("missing")
                .exit_code(),
            3
        );
    }

    #[test]
    fn default_roots_load_without_a_bundle() {
        assert!(tls_client_config(None).is_ok());
    }
}
