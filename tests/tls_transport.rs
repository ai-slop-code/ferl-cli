//! TLS and proxy transport tests against a loopback HTTPS stub with a generated private
//! CA. These cover certificate-chain and hostname verification, explicit CA bundles,
//! CONNECT proxies, and no-proxy bypass (business spec §10.1). Certificates are created
//! at runtime with the local `openssl` tool; no key material is committed.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use common::{Harness, KEY};

const STUB: &str = r#"
import http.server, json, socketserver, ssl, sys
port, cert, key, log = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        with open(log, "a") as handle:
            handle.write(self.path + "\n")
        body = json.dumps({"id": "4a8ffcd5-3066-4519-a68e-060f956ab500", "username": "tls_probe", "role": "user", "enabled": True, "created_at": "2026-10-29T10:00:00Z", "updated_at": "2026-10-29T10:00:00Z"}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass
class Server(http.server.HTTPServer):
    def server_bind(self):
        # Skip the reverse-DNS lookup that HTTPServer performs on every bind.
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = port

server = Server(("127.0.0.1", port), Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(cert, key)
server.socket = context.wrap_socket(server.socket, server_side=True)
print("ready", flush=True)
server.serve_forever()
"#;

struct Pki {
    ca: PathBuf,
    cert: PathBuf,
    key: PathBuf,
    wrong_cert: PathBuf,
    wrong_key: PathBuf,
}

fn openssl(args: &[&str]) -> bool {
    Command::new("openssl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn make_pki(dir: &Path) -> Option<Pki> {
    let p = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let (ca_key, ca) = (p("ca.key"), p("ca.pem"));
    let ok = openssl(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        &ca_key,
        "-out",
        &ca,
        "-days",
        "2",
        "-subj",
        "/CN=ferl test CA",
        "-addext",
        "basicConstraints=critical,CA:TRUE",
        "-addext",
        "keyUsage=critical,keyCertSign,cRLSign",
    ]);
    if !ok {
        return None;
    }
    let make_leaf = |stem: &str, san: &str| -> Option<(String, String)> {
        let key = p(&format!("{stem}.key"));
        let csr = p(&format!("{stem}.csr"));
        let cert = p(&format!("{stem}.pem"));
        let ext = p(&format!("{stem}.ext"));
        std::fs::write(
            &ext,
            format!("subjectAltName={san}\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n"),
        )
        .ok()?;
        if !openssl(&[
            "req", "-newkey", "rsa:2048", "-nodes", "-keyout", &key, "-out", &csr, "-subj",
            "/CN=stub",
        ]) {
            return None;
        }
        if !openssl(&[
            "x509",
            "-req",
            "-in",
            &csr,
            "-CA",
            &ca,
            "-CAkey",
            &ca_key,
            "-CAcreateserial",
            "-out",
            &cert,
            "-days",
            "2",
            "-extfile",
            &ext,
        ]) {
            return None;
        }
        Some((cert, key))
    };
    let (cert, key) = make_leaf("server", "IP:127.0.0.1")?;
    let (wrong_cert, wrong_key) = make_leaf("wrong", "DNS:wrong.example.invalid")?;
    Some(Pki {
        ca: PathBuf::from(ca),
        cert: PathBuf::from(cert),
        key: PathBuf::from(key),
        wrong_cert: PathBuf::from(wrong_cert),
        wrong_key: PathBuf::from(wrong_key),
    })
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

struct Stub {
    port: u16,
    log: PathBuf,
    child: Child,
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_stub(dir: &Path, cert: &Path, key: &Path) -> Option<Stub> {
    let port = free_port();
    let script = dir.join("stub.py");
    std::fs::write(&script, STUB).ok()?;
    let log = dir.join("stub.log");
    let mut child = Command::new("python3")
        .arg(&script)
        .arg(port.to_string())
        .arg(cert)
        .arg(key)
        .arg(&log)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).ok()?;
    if line.trim() != "ready" {
        let _ = child.kill();
        return None;
    }
    Some(Stub { port, log, child })
}

fn served_requests(stub: &Stub) -> usize {
    std::fs::read_to_string(&stub.log)
        .map(|t| t.lines().count())
        .unwrap_or(0)
}

/// The TLS suites need python3 and an OpenSSL 3 command for their generated PKI. A
/// missing tool is a failed prerequisite, never a silent pass.
fn require_tools() {
    assert!(
        Command::new("python3").arg("--version").output().is_ok(),
        "python3 is required for the TLS stub"
    );
    let version = Command::new("openssl")
        .arg("version")
        .output()
        .expect("openssl is required to generate the test PKI");
    let text = String::from_utf8_lossy(&version.stdout).into_owned();
    let major: u32 = text
        .strip_prefix("OpenSSL ")
        .and_then(|rest| rest.split('.').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    assert!(
        major >= 3,
        "the test PKI needs OpenSSL 3 or newer, found: {text}"
    );
}

/// CONNECT proxy that records each tunnel request and pipes bytes to its target.
fn connect_proxy(count: Arc<Mutex<usize>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut client) = stream else { continue };
            let counter = count.clone();
            thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while head.len() < 8192 {
                    if client.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let Some(target) = text
                    .lines()
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .map(str::to_owned)
                else {
                    return;
                };
                *counter.lock().expect("lock") += 1;
                let Ok(mut upstream) = TcpStream::connect(&target) else {
                    return;
                };
                let _ = client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
                let mut from_client = client.try_clone().expect("clone");
                let mut to_upstream = upstream.try_clone().expect("clone");
                let forward = thread::spawn(move || {
                    let _ = std::io::copy(&mut from_client, &mut to_upstream);
                });
                let _ = std::io::copy(&mut upstream, &mut client);
                let _ = forward.join();
            });
        }
    });
    port
}

fn https_profile(harness: &Harness, name: &str, origin: &str, ca: Option<&Path>) -> common::Run {
    let mut args = vec![
        "profile",
        "add",
        name,
        "--server",
        origin,
        "--key-stdin",
        "--default",
    ];
    let ca_text;
    if let Some(path) = ca {
        ca_text = path.to_string_lossy().into_owned();
        args.push("--ca-bundle");
        args.push(&ca_text);
    }
    harness.run_with(&args, Some(format!("{KEY}\n").as_bytes()))
}

#[test]
fn https_certificate_and_hostname_verification_with_explicit_private_ca() {
    require_tools();
    let harness = Harness::new();
    let pki = make_pki(harness.dir.path()).expect("https certificate generation");
    let stub = start_stub(harness.dir.path(), &pki.cert, &pki.key).expect("stub");
    let origin = format!("https://127.0.0.1:{}", stub.port);

    let untrusted = https_profile(&harness, "untrusted", &origin, None);
    assert_eq!(
        untrusted.status, 0,
        "profile add is local: {}",
        untrusted.stderr
    );
    let refused = harness.run(&["profile", "check", "untrusted"]);
    assert_eq!(
        refused.status, 9,
        "an unknown private CA is a TLS failure: {}",
        refused.stderr
    );
    refused.assert_no_secret();
    assert_eq!(
        served_requests(&stub),
        0,
        "no request is sent after a certificate failure"
    );

    let with_ca = https_profile(&harness, "trusted", &origin, Some(&pki.ca));
    assert_eq!(with_ca.status, 0, "{}", with_ca.stderr);
    let check = harness.run(&["profile", "check", "trusted"]);
    assert_eq!(check.status, 0, "{}", check.stderr);
    assert_eq!(served_requests(&stub), 1);
}

#[test]
fn private_ca_profile_succeeds_and_reads_the_account_over_tls() {
    require_tools();
    let harness = Harness::new();
    let pki = make_pki(harness.dir.path()).expect("https certificate generation");
    let stub = start_stub(harness.dir.path(), &pki.cert, &pki.key).expect("stub");
    let origin = format!("https://127.0.0.1:{}", stub.port);
    assert_eq!(
        https_profile(&harness, "tls", &origin, Some(&pki.ca)).status,
        0
    );
    let check = harness.run(&["--output", "json", "profile", "check", "tls"]);
    check.assert_no_secret();
    assert_eq!(check.status, 0, "{}", check.stderr);
    assert_eq!(
        check.json_stdout()["data"]["account"]["username"],
        "tls_probe"
    );
    assert_eq!(served_requests(&stub), 1);
}

#[test]
fn hostname_mismatch_fails_verification_even_when_the_chain_is_trusted() {
    require_tools();
    let harness = Harness::new();
    let pki = make_pki(harness.dir.path()).expect("https certificate generation");
    let stub = start_stub(harness.dir.path(), &pki.wrong_cert, &pki.wrong_key).expect("stub");
    let origin = format!("https://127.0.0.1:{}", stub.port);
    assert_eq!(
        https_profile(&harness, "tls", &origin, Some(&pki.ca)).status,
        0
    );
    let run = harness.run(&["profile", "check", "tls"]);
    assert_eq!(run.status, 9, "{}", run.stderr);
    assert_eq!(served_requests(&stub), 0);
}

#[test]
fn https_requests_use_a_connect_proxy_and_no_proxy_bypasses_it() {
    require_tools();
    let harness = Harness::new();
    let pki = make_pki(harness.dir.path()).expect("https certificate generation");
    let stub = start_stub(harness.dir.path(), &pki.cert, &pki.key).expect("stub");
    let origin = format!("https://127.0.0.1:{}", stub.port);
    assert_eq!(
        https_profile(&harness, "tls", &origin, Some(&pki.ca)).status,
        0
    );

    let tunnels = Arc::new(Mutex::new(0usize));
    let proxy_port = connect_proxy(tunnels.clone());
    let proxy = format!("http://127.0.0.1:{proxy_port}");
    let via_proxy = run_with_env(
        &harness,
        &["--output", "json", "profile", "check", "tls"],
        &[("HTTPS_PROXY", proxy.as_str())],
    );
    assert_eq!(via_proxy.status, 0, "{}", via_proxy.stderr);
    assert_eq!(
        *tunnels.lock().expect("lock"),
        1,
        "the request used the CONNECT tunnel"
    );

    let bypassed = run_with_env(
        &harness,
        &["--output", "json", "profile", "check", "tls"],
        &[("HTTPS_PROXY", proxy.as_str()), ("NO_PROXY", "127.0.0.1")],
    );
    assert_eq!(bypassed.status, 0, "{}", bypassed.stderr);
    assert_eq!(
        *tunnels.lock().expect("lock"),
        1,
        "NO_PROXY must bypass the proxy"
    );
    assert_eq!(served_requests(&stub), 2);
}

fn run_with_env(harness: &Harness, args: &[&str], env: &[(&str, &str)]) -> common::Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ferl"));
    command.env_clear();
    command.env("PATH", std::env::var("PATH").unwrap_or_default());
    command.env("HOME", harness.dir.path().join("home"));
    for (name, value) in env {
        command.env(name, value);
    }
    command
        .arg("--config")
        .arg(&harness.config)
        .arg("--non-interactive")
        .args(args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = std::time::Instant::now();
    let output = command.output().expect("run");
    common::Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        elapsed: started.elapsed(),
    }
}
