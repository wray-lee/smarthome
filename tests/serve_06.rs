//! Serve panel tests: spawn `home serve` on an ephemeral port and assert the
//! static HTML is embedded, served with `text/html`, and free of em dashes.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn temp_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("home.sqlite");
    (dir, db)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn http(port: u16, req: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect to serve");
    s.write_all(req.as_bytes()).expect("write request");
    let mut resp = String::new();
    s.read_to_string(&mut resp).expect("read response");
    let mut lines = resp.lines();
    let status: u16 = lines
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let mut ct = String::new();
    let mut body_start = resp.len();
    for line in resp.lines() {
        let l = line.to_ascii_lowercase();
        if l.starts_with("content-type:") {
            ct = line
                .split_once(':')
                .map(|x| x.1)
                .unwrap_or("")
                .trim()
                .to_string();
        }
        if line.is_empty() {
            break;
        }
    }
    if let Some(pos) = resp.find("\r\n\r\n") {
        body_start = pos + 4;
    }
    (status, ct, resp[body_start..].to_string())
}

struct Serve(Child);
impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(db: &Path, port: u16, password: &str) -> Serve {
    let child = Command::new(env!("CARGO_BIN_EXE_home"))
        .arg("--db")
        .arg(db)
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--password")
        .arg(password)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn home serve");
    let s = Serve(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return s;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("home serve did not listen on port {port}");
}

fn assert_panel(body: &str) {
    // Em dash U+2014 must not appear anywhere in the panel copy.
    assert!(
        !body.contains(char::from_u32(0x2014).unwrap()),
        "panel must contain no em dash"
    );
}

#[test]
fn login_page_is_html_with_form_and_no_em_dash() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn(&db, port, "pw123456");
    let (status, ct, body) = http(
        port,
        "GET /login HTTP/1.1\r\nHost: x\r\nAccept: text/html\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status, 200);
    assert!(ct.contains("text/html"), "Content-Type is {ct}");
    assert!(
        body.contains("action=\"/login\""),
        "login form posts to /login"
    );
    assert!(
        body.contains("<label for=\"pw\">Password</label>"),
        "label for pw"
    );
    assert!(
        body.contains("autocomplete=\"current-password\""),
        "autocomplete hint"
    );
    assert!(body.contains("role=\"alert\"") || body.contains("role=\"alert\""));
    assert_panel(&body);
    // 401 path: wrong password returns 401 with alert copy.
    let (status2, _ct2, body2) =
        http(port, "POST /login HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 13\r\nConnection: close\r\n\r\npassword=nope");
    assert_eq!(status2, 401);
    assert!(
        body2.contains("role=\"alert\""),
        "failed login shows alert: {body2}"
    );
}

#[test]
fn authenticated_index_has_landmarks_controls() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn(&db, port, "pw123456");
    // Unauthenticated API request: 401 JSON.
    let (s401, _c401, _b401) = http(
        port,
        "GET /api/devices HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(s401, 401);

    // Login as form POST to get the session cookie.
    let (status, _ct, _body) = http(
        port,
        "POST /login HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 17\r\nConnection: close\r\n\r\npassword=pw123456",
    );
    assert_eq!(status, 200);

    // Re-login and capture the cookie via a raw socket to read Set-Cookie.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(b"POST /login HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 17\r\nConnection: close\r\n\r\npassword=pw123456")
        .unwrap();
    let mut resp = String::new();
    s.read_to_string(&mut resp).unwrap();
    let cookie = resp
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .expect("Set-Cookie header")
        .split_once(':')
        .unwrap()
        .1
        .trim();
    let pair = cookie.split(';').next().unwrap().trim();

    let (status, ct, body) = http(
        port,
        &format!("GET / HTTP/1.1\r\nHost: x\r\nAccept: text/html\r\nCookie: {pair}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(status, 200);
    assert!(ct.contains("text/html"), "Content-Type is {ct}");
    assert!(body.contains("<header"), "header landmark");
    assert!(body.contains("<main"), "main landmark");
    assert!(body.contains("Sign out"), "sign out control");
    assert!(body.contains("Refresh"), "refresh control");
    assert!(body.contains("aria-live=\"polite\""), "live region");
    assert!(body.contains("<dialog"), "confirmation dialog");
    assert!(body.contains("/api/devices"), "devices poll endpoint");
    assert!(body.contains("/api/gh/get"), "state endpoint");
    assert!(body.contains("/api/tuya/get"), "tuya state endpoint");
    assert!(body.contains("/api/gh/set"), "gh set endpoint");
    assert!(body.contains("/api/tuya/on"), "tuya on endpoint");
    assert!(body.contains("protected"), "protected label");
    assert!(body.contains("backend-banner"), "reauth banner text");
    assert_panel(&body);
}
