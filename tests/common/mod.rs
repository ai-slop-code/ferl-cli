//! Shared fixtures for integration tests: a recording loopback HTTP server with
//! scripted replies (including dropped acknowledgements), and a harness that runs the
//! real `ferl` binary with an isolated configuration and a clean environment.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const KEY: &str = "ferl_ak_v1_SENTINEL_0123456789abcdef";

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn query_pairs(&self) -> Vec<(String, String)> {
        let query = self.target.split_once('?').map(|(_, q)| q).unwrap_or("");
        url::form_urlencoded::parse(query.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    pub fn query_values(&self, key: &str) -> Vec<String> {
        self.query_pairs()
            .into_iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub enum Reply {
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// Read the request, then close without writing any response.
    Drop,
    /// Wait before answering, to exercise deadlines.
    Delay(Duration, Box<Reply>),
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Reply {
        Reply::Respond {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn with_header(self, name: &str, value: &str) -> Reply {
        match self {
            Reply::Respond {
                status,
                mut headers,
                body,
            } => {
                headers.push((name.into(), value.into()));
                Reply::Respond {
                    status,
                    headers,
                    body,
                }
            }
            other => other,
        }
    }
}

pub struct MockServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl MockServer {
    pub fn start<F>(handler: F) -> MockServer
    where
        F: Fn(&Request) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let recorded = requests.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let handler = handler.clone();
                let recorded = recorded.clone();
                thread::spawn(move || serve(stream, handler.as_ref(), &recorded));
            }
        });
        MockServer { addr, requests }
    }

    /// Origin without a trailing slash, e.g. `http://127.0.0.1:41234`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("lock").clone()
    }

    pub fn count(&self, method: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.method == method)
            .count()
    }
}

fn serve(
    mut stream: TcpStream,
    handler: &(dyn Fn(&Request) -> Reply + Send + Sync),
    recorded: &Mutex<Vec<Request>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    recorded.lock().expect("lock").push(request.clone());
    let reply = handler(&request);
    write_reply(&mut stream, reply);
}

fn write_reply(stream: &mut TcpStream, reply: Reply) {
    match reply {
        Reply::Drop => {}
        Reply::Delay(duration, inner) => {
            thread::sleep(duration);
            write_reply(stream, *inner);
        }
        Reply::Respond {
            status,
            headers,
            body,
        } => {
            let mut head = format!("HTTP/1.1 {} {}\r\n", status, reason(status));
            for (name, value) in &headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str(&format!(
                "Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            ));
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find(&buffer, b"\r\n\r\n") {
            break pos;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 4 * 1024 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let content_length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Some(Request {
        method,
        target,
        headers,
        body,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Captured output of one `ferl` invocation.
#[derive(Debug, Clone)]
pub struct Run {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
}

impl Run {
    pub fn assert_no_secret(&self) {
        assert!(!self.stdout.contains(KEY), "key leaked to stdout");
        assert!(!self.stderr.contains(KEY), "key leaked to stderr");
        assert!(
            !self.stdout.contains("SENTINEL"),
            "key fragment leaked to stdout"
        );
        assert!(
            !self.stderr.contains("SENTINEL"),
            "key fragment leaked to stderr"
        );
    }

    pub fn json_stdout(&self) -> serde_json::Value {
        serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.stdout))
    }

    pub fn json_stderr_lines(&self) -> Vec<serde_json::Value> {
        self.stderr
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("stderr line not JSON ({e}): {line}"))
            })
            .collect()
    }
}

/// An isolated configuration directory and an environment-scrubbed command runner.
pub struct Harness {
    pub dir: tempfile::TempDir,
    pub config: PathBuf,
}

impl Harness {
    pub fn new() -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("chmod");
        let config = dir.path().join("config.toml");
        Harness { dir, config }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn base(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ferl"));
        command.env_clear();
        command.env("PATH", std::env::var("PATH").unwrap_or_default());
        command.env("HOME", self.dir.path().join("home"));
        command.env("LANG", "C.UTF-8");
        command
    }

    /// Runs `ferl --config <test> --non-interactive <args>` with optional stdin.
    pub fn run_with(&self, args: &[&str], stdin: Option<&[u8]>) -> Run {
        let mut command = self.base();
        command
            .arg("--config")
            .arg(&self.config)
            .arg("--non-interactive")
            .args(args);
        command.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = command.spawn().expect("spawn ferl");
        if let Some(bytes) = stdin {
            let mut pipe = child.stdin.take().expect("stdin");
            let bytes = bytes.to_vec();
            thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            });
        }
        let output = child.wait_with_output().expect("wait");
        Run {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            elapsed: started.elapsed(),
        }
    }

    pub fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, None)
    }

    /// Adds a loopback profile through the public command, with the key on stdin.
    pub fn add_profile(&self, name: &str, server: &str, key: &str) -> Run {
        let input = format!("{key}\n");
        self.run_with(
            &[
                "profile",
                "add",
                name,
                "--server",
                server,
                "--allow-http-loopback",
                "--key-stdin",
                "--default",
            ],
            Some(input.as_bytes()),
        )
    }

    /// Runs with a profile and JSON output, a common automation form.
    pub fn run_json(&self, profile: &str, args: &[&str]) -> Run {
        let mut full = vec!["--profile", profile, "--output", "json"];
        full.extend_from_slice(args);
        self.run(&full)
    }

    pub fn write_private(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, contents).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        path
    }
}

pub fn link_json(code: &str, title: Option<&str>, visits: &str) -> String {
    let title = match title {
        Some(t) => format!("\"{t}\""),
        None => "null".to_owned(),
    };
    format!(
        r#"{{"id":"1f1bf5f6-075c-4cf1-a44b-05fca986b100","short_code":"{code}","short_url":"https://s.example.com/{code}","destination_url":"https://example.com/{code}?q=1#f","title":{title},"tags":["work"],"owner_id":"4a8ffcd5-3066-4519-a68e-060f956ab500","enabled":true,"active":true,"visit_count":{visits},"created_at":"2026-10-29T10:00:00.123456Z","updated_at":"2026-10-29T10:00:00.000000Z"}}"#
    )
}

pub fn error_json(code: &str, message: &str, request_id: &str) -> String {
    format!(r#"{{"error":{{"code":"{code}","message":"{message}"}},"request_id":"{request_id}"}}"#)
}

pub fn account_json(username: &str, role: &str) -> String {
    format!(
        r#"{{"id":"4a8ffcd5-3066-4519-a68e-060f956ab500","username":"{username}","role":"{role}","enabled":true,"created_at":"2026-10-29T10:00:00Z","updated_at":"2026-10-29T10:00:00Z"}}"#
    )
}

pub fn page_json(links: &[String], next: Option<&str>) -> String {
    let next = match next {
        Some(n) => format!("\"{n}\""),
        None => "null".to_owned(),
    };
    format!(r#"{{"items":[{}],"next_cursor":{next}}}"#, links.join(","))
}
