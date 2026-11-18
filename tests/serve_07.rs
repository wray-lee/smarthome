//! Serve panel black-box tests (ticket 07): QR login start/poll, API state
//! reset after success, telemetry, GH trait set, protected-device 403 with zero
//! upstream requests, and index smoke checks.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLEAR_ENV: &[&str] = &[
    "HOME_TUYA_LOGIN_URL",
    "HOME_TUYA_ENDPOINT",
    "HOME_TUYA_USER_CODE",
    "HOME_TUYA_POLL_MS",
    "HOME_TUYA_POLL_ATTEMPTS",
    "HOME_FOYER_URL",
    "HOME_GPSOAUTH_URL",
    "HOME_PASSWORD",
    "HOME_PASSWORD_FILE",
    "HOME_COOKIE_SECURE",
    "HOME_TUYA_REFRESH_POLL_MS",
    "HOME_GH_EMAIL",
    "HOME_HOST",
    "HOME_PORT",
];

fn clear_env(cmd: &mut Command) {
    for key in CLEAR_ENV {
        cmd.env_remove(key);
    }
}

fn http_full(port: u16, req: &str) -> (u16, Vec<(String, String)>, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect to serve");
    s.write_all(req.as_bytes()).expect("write request");
    let mut resp = String::new();
    s.read_to_string(&mut resp).expect("read response");
    let status: u16 = resp
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let mut headers = Vec::new();
    for line in resp.lines() {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let body_start = resp.find("\r\n\r\n").map(|p| p + 4).unwrap_or(resp.len());
    (status, headers, resp[body_start..].to_string())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn spawn_env(db: &Path, port: u16, envs: &[(String, String)]) -> Serve {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    clear_env(&mut cmd);
    cmd.arg("--db")
        .arg(db)
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--password")
        .arg("pw123456")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let child = cmd.spawn().expect("spawn home serve");
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

fn body_json(body: &str) -> Value {
    serde_json::from_str(body).expect("response body is JSON")
}

fn login_cookie(port: u16) -> String {
    let form = "password=pw123456";
    let req = format!(
        "POST /login HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{form}",
        form.len()
    );
    let (status, headers, _body) = http_full(port, &req);
    assert_eq!(status, 200);
    header(&headers, "set-cookie")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn post_form(port: u16, p: &str, form: &str, cookie: &str) -> (u16, Value) {
    let req = format!(
        "POST {p} HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n{form}",
        form.len()
    );
    let (status, _h, body) = http_full(port, &req);
    (status, body_json(&body))
}

fn get_json(port: u16, target: &str, cookie: &str) -> (u16, Value) {
    let req = format!(
        "GET {target} HTTP/1.1\r\nHost: x\r\nAccept: application/json\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    let (status, _h, body) = http_full(port, &req);
    (status, body_json(&body))
}

fn http_get(port: u16, target: &str) -> (u16, String, String) {
    let req = format!(
        "GET {target} HTTP/1.1\r\nHost: x\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    let (status, headers, body) = http_full(port, &req);
    let ct = header(&headers, "content-type").unwrap_or("").to_string();
    (status, ct, body)
}

fn temp_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("home.sqlite");
    (dir, db)
}

/// A port below the OS ephemeral range (Linux starts at 32768), handed out once
/// per test binary. `bind(0)` then drop let a parallel test's wiremock grab the
/// same port before `home serve` bound it, so the probe hit the wrong server.
fn free_port() -> u16 {
    static NEXT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
    let base = 20_000 + (std::process::id() % 400) as u16 * 25;
    loop {
        let port = base + NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

fn envelope(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(body.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

struct Serve(Child);
impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// --- fixtures ------------------------------------------------------------

const HOMES: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/homes.json");
const HOME_DEVICES: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/home-devices.json");
const SPEC_1: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/spec-1.json");
const DETAIL_FALSE: &str =
    include_str!("../.scratch/smarthome-v1/fixtures/tuya/detail-after-false.json");

fn tuya_home_id() -> String {
    let list: Value = serde_json::from_str(HOME_DEVICES).unwrap();
    list["result"][1]["id"].as_str().unwrap().to_string()
}

#[allow(dead_code)]
fn tuya_first_id() -> String {
    let list: Value = serde_json::from_str(HOME_DEVICES).unwrap();
    list["result"][0]["id"].as_str().unwrap().to_string()
}

const QR_TOKEN: &str = "fakeQrToken1";

fn qr_done(endpoint: &str) -> Value {
    json!({
        "t": now_ms(),
        "success": true,
        "result": {
            "access_token": "dummy-access-1",
            "endpoint": endpoint,
            "expire_time": 7200,
            "refresh_token": "dummy-refresh-1",
            "terminal_id": "dummy-terminal-1",
            "uid": "dummy-uid-1",
            "username": "dummy-user"
        }
    })
}

async fn mount_qr_login(server: &MockServer, endpoint: &str) {
    Mock::given(method("POST"))
        .and(path("/v1.0/m/life/home-assistant/qrcode/tokens"))
        .respond_with(envelope(json!({
            "t": 1794759258801i64, "success": true,
            "result": {"qrcode": QR_TOKEN}
        })))
        .mount(server)
        .await;
    let endpoint = endpoint.to_string();
    let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path(format!(
            "/v1.0/m/life/home-assistant/qrcode/tokens/{QR_TOKEN}"
        )))
        .respond_with(move |_req: &wiremock::Request| {
            let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                envelope(json!({"success": false, "code": 1001, "msg": "not scanned"}))
            } else {
                envelope(qr_done(&endpoint))
            }
        })
        .mount(server)
        .await;
}

async fn mount_tuya_devices(api: &MockServer) {
    let homes: Value = serde_json::from_str(HOMES).unwrap();
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/users/homes"))
        .respond_with(envelope(homes))
        .mount(api)
        .await;
    let devices: Value = serde_json::from_str(HOME_DEVICES).unwrap();
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/home/devices"))
        .respond_with(envelope(devices))
        .mount(api)
        .await;
}

async fn mount_tuya_detail_spec(api: &MockServer, id: &str) {
    let detail: Value = serde_json::from_str(DETAIL_FALSE).unwrap();
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/devices/detail"))
        .respond_with(envelope(detail))
        .mount(api)
        .await;
    let spec: Value = serde_json::from_str(SPEC_1).unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/v1.1/m/life/{id}/specifications")))
        .respond_with(envelope(spec))
        .mount(api)
        .await;
}

async fn mount_tuya_command(api: &MockServer, id: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/v1.1/m/thing/{id}/commands")))
        .respond_with(envelope(
            json!({"t": now_ms(), "success": true, "result": true}),
        ))
        .mount(api)
        .await;
}

async fn count_path(server: &MockServer, needle: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().contains(needle))
        .count()
}

// --- GH fixtures ---------------------------------------------------------

const GH_LAMP: &str = "00000000-0000-4000-8000-00000000h0me";
const GRAPH_PATH: &str = "/$rpc/google.internal.home.foyer.v1.StructuresService/GetHomeGraph";
const GET_PATH: &str = "/$rpc/google.internal.home.foyer.v1.HomeControlService/GetTraits";
const UPDATE_PATH: &str = "/$rpc/google.internal.home.foyer.v1.HomeControlService/UpdateTraits";

fn gh_graph() -> Value {
    json!([
        "<num>",
        [[
            "home-1",
            "home",
            [],
            [],
            null,
            [[
                "room-Den",
                null,
                "Den",
                ["OTHER"],
                [[[GH_LAMP, ["tuya-smart-cee31", format!("{GH_LAMP}_all")]]]]
            ]],
            [[
                [GH_LAMP, ["tuya-smart-cee31", "homePartner_all"]],
                null,
                null,
                "Lamp",
                null,
                "action.devices.types.LIGHT",
                [
                    "action.devices.traits.OnOff",
                    "action.devices.traits.Brightness"
                ]
            ]]
        ]]
    ])
}

fn gh_bool(v: bool) -> Value {
    json!([null, null, null, if v { 1 } else { 0 }])
}

fn gh_state() -> Value {
    json!([
        [GH_LAMP],
        [
            ["deviceStatus", [["online", [null, null, null, 1]]]],
            ["onOff", [["onOff", gh_bool(true)]]],
            ["brightness", [["brightness", [null, 40]]]]
        ]
    ])
}

fn gh_states() -> Value {
    json!([[gh_state()]])
}

async fn mount_foyer(foyer: &MockServer) {
    Mock::given(method("POST"))
        .and(path(GRAPH_PATH))
        .respond_with(envelope(gh_graph()))
        .mount(foyer)
        .await;
    Mock::given(method("POST"))
        .and(path(GET_PATH))
        .respond_with(envelope(gh_states()))
        .mount(foyer)
        .await;
    Mock::given(method("POST"))
        .and(path(UPDATE_PATH))
        .respond_with(envelope(gh_states()))
        .mount(foyer)
        .await;
}

fn seed_gh(db: &Path) {
    let store = home_store::Store::open(db).unwrap();
    let now = home_store::store::now_unix() as u64;
    let cred = json!({
        "master": "aas_et/dummy-master",
        "android_id": "dummy-android",
        "email": "user@example.com",
        "bearer": "dummy-bearer-fresh",
        "bearer_expires_at": now + 3600,
    });
    store
        .set_credential("google_home", &cred.to_string())
        .unwrap();
}

fn seed_tuya(db: &Path, endpoint: &str, offset: i64) {
    let store = home_store::Store::open(db).unwrap();
    let now = home_store::store::now_unix();
    let sess = json!({
        "access_token": "dummy-access-1",
        "refresh_token": "dummy-refresh-1",
        "endpoint": endpoint,
        "user_code": "DummyCode1",
        "terminal_id": "dummy-terminal-1",
        "uid": "dummy-uid-1",
        "expire_time": now + offset,
    });
    store.set_credential("tuya", &sess.to_string()).unwrap();
}

fn tuya_env(api: &MockServer) -> Vec<(String, String)> {
    vec![
        ("HOME_TUYA_LOGIN_URL".into(), api.uri()),
        ("HOME_TUYA_ENDPOINT".into(), api.uri()),
    ]
}

fn tuya_env_with_login(api: &MockServer, login: &MockServer) -> Vec<(String, String)> {
    vec![
        ("HOME_TUYA_LOGIN_URL".into(), login.uri()),
        ("HOME_TUYA_ENDPOINT".into(), api.uri()),
    ]
}

fn gh_env(foyer: &MockServer) -> Vec<(String, String)> {
    vec![(
        "HOME_FOYER_URL".into(),
        format!("{}/$rpc/google.internal.home.foyer.v1.", foyer.uri()),
    )]
}

// --- tests ---------------------------------------------------------------

/// QR login start returns token + SVG; poll goes pending -> authenticated and
/// persists the session; /api/status then reports ok (state reset).
#[tokio::test]
async fn qr_login_start_poll_persist_and_status_reset() {
    let (_dir, db) = temp_db();
    let login = MockServer::start().await;
    let api = MockServer::start().await;
    mount_qr_login(&login, &api.uri()).await;
    mount_tuya_devices(&api).await;
    // background refresh must not flap: session far from expiry
    mount_tuya_detail_spec(&api, &tuya_home_id()).await;

    let port = free_port();
    let mut envs = tuya_env_with_login(&api, &login);
    envs.push(("HOME_TUYA_REFRESH_POLL_MS".into(), "60000".into()));
    let _s = spawn_env(&db, port, &envs);
    let cookie = login_cookie(port);

    // status before login: tuya ok (no reauth), user code not configured
    let (st, sj) = get_json(port, "/api/status", &cookie);
    assert_eq!(st, 200, "{sj}");
    assert_eq!(sj["data"]["status"], "ok");
    assert_eq!(sj["data"]["tuya_user_code_configured"], false);

    // start with form user_code
    let (st, j) = post_form(
        port,
        "/api/tuya/login/start",
        "user_code=DummyCode1",
        &cookie,
    );
    assert_eq!(st, 200, "{j}");
    let token = j["data"]["token"].as_str().unwrap();
    assert_eq!(token, QR_TOKEN);
    let svg = j["data"]["qr_svg"].as_str().unwrap();
    assert!(svg.contains("<svg"), "{svg}");
    // no tokens leaked
    assert!(!svg.contains("dummy-access-1"));

    // first poll -> pending
    let (st, j) = post_form(
        port,
        "/api/tuya/login/poll",
        &format!("token={token}"),
        &cookie,
    );
    assert_eq!(st, 200, "{j}");
    assert_eq!(j["data"]["status"], "pending");

    // second poll -> authenticated, session persisted
    let (st, j) = post_form(
        port,
        "/api/tuya/login/poll",
        &format!("token={token}"),
        &cookie,
    );
    assert_eq!(st, 200, "{j}");
    assert_eq!(j["data"]["status"], "authenticated");
    assert_eq!(j["data"]["backend"], "tuya");
    assert_eq!(j["data"]["uid"], "dummy-uid-1");

    let store = home_store::Store::open(&db).unwrap();
    let raw = store
        .credential("tuya")
        .unwrap()
        .expect("tuya session after poll");
    let sess: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(sess["access_token"], "dummy-access-1");
    assert_eq!(sess["refresh_token"], "dummy-refresh-1");
    assert_eq!(sess["user_code"], "DummyCode1");

    // status after login: ok, code now configured, no reauth banner state
    let (st, sj) = get_json(port, "/api/status", &cookie);
    assert_eq!(st, 200, "{sj}");
    assert_eq!(sj["data"]["status"], "ok");
    assert_eq!(sj["data"]["tuya"]["status"], "ok");
    assert_eq!(sj["data"]["tuya_user_code_configured"], true);
}

/// Anonymous login start/poll are 401.
#[tokio::test]
async fn qr_login_unauthenticated_is_401() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);
    let (st, ct, body) = http_get(port, "/api/status");
    assert_eq!(st, 401);
    assert!(ct.contains("application/json"));
    let j: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(j["error"]["code"], "unauthorized");
    // POST start without cookie also 401
    let form = "user_code=DummyCode1";
    let req = format!(
        "POST /api/tuya/login/start HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{form}",
        form.len()
    );
    let (st, _h, body) = http_full(port, &req);
    assert_eq!(st, 401, "{body}");
    let j: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(j["error"]["code"], "unauthorized");
}

/// telemetry fields via /api/tuya/get
#[tokio::test]
async fn tuya_get_returns_telemetry_fields() {
    let (_dir, db) = temp_db();
    let api = MockServer::start().await;
    mount_tuya_devices(&api).await;
    let id = tuya_home_id();
    mount_tuya_detail_spec(&api, &id).await;
    seed_tuya(&db, &api.uri(), 7200);

    let port = free_port();
    let _s = spawn_env(&db, port, &tuya_env(&api));
    let cookie = login_cookie(port);

    let (st, j) = get_json(port, &format!("/api/tuya/get?device={id}"), &cookie);
    assert_eq!(st, 200, "{j}");
    let data = &j["data"];
    assert_eq!(data["id"], id);
    assert_eq!(data["backend"], "tuya");
    let tele = data["telemetry"].as_object().unwrap();
    for k in ["cur_power", "cur_current", "cur_voltage", "add_ele"] {
        assert!(tele.contains_key(k), "missing telemetry {k}: {tele:?}");
    }
    assert_eq!(tele["cur_voltage"]["value"], 103.5);
    assert_eq!(tele["add_ele"]["value"], 0.002);
}

/// GH trait set via /api/gh/set=onOff and /api/gh/set=brightness
#[tokio::test]
async fn gh_set_trait_via_api() {
    let (_dir, db) = temp_db();
    seed_gh(&db);
    let foyer = MockServer::start().await;
    mount_foyer(&foyer).await;

    let port = free_port();
    let _s = spawn_env(&db, port, &gh_env(&foyer));
    let cookie = login_cookie(port);

    // get first
    let (st, j) = get_json(port, &format!("/api/gh/get?device={GH_LAMP}"), &cookie);
    assert_eq!(st, 200, "{j}");
    assert_eq!(j["data"]["traits"]["onOff"]["onOff"], true);
    assert_eq!(j["data"]["traits"]["brightness"]["brightness"], 40);

    // set onOff false
    let form = format!("device={GH_LAMP}&trait=onOff&value=false");
    let (st, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(st, 200, "{j}");
    assert_eq!(j["data"]["changed"]["trait"], "onOff");
    assert_eq!(j["data"]["changed"]["value"], false);

    // set brightness 75
    let form = format!("device={GH_LAMP}&trait=brightness&value=75");
    let (st, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(st, 200, "{j}");
    assert_eq!(j["data"]["changed"]["trait"], "brightness");

    // Tuya on/off for completeness: use tuya device via same server? skip, GH covers trait ops
    assert_eq!(count_path(&foyer, UPDATE_PATH).await, 2);
}

/// Protected GH + Tuya writes 403 with zero upstream command requests;
/// reads still 200.
#[tokio::test]
async fn protected_devices_403_zero_requests_on_writes() {
    let (_dir, db) = temp_db();
    seed_gh(&db);
    let pid = tuya_home_id();
    home_store::Store::open(&db)
        .unwrap()
        .protect("google_home", GH_LAMP)
        .unwrap();
    home_store::Store::open(&db)
        .unwrap()
        .protect("tuya", &pid)
        .unwrap();

    let foyer = MockServer::start().await;
    mount_foyer(&foyer).await;
    let api = MockServer::start().await;
    mount_tuya_devices(&api).await;
    mount_tuya_detail_spec(&api, &pid).await;
    mount_tuya_command(&api, &pid).await;
    seed_tuya(&db, &api.uri(), 7200);

    let port = free_port();
    let mut envs = gh_env(&foyer);
    envs.extend(tuya_env(&api));
    envs.push(("HOME_TUYA_REFRESH_POLL_MS".into(), "60000".into()));
    let _s = spawn_env(&db, port, &envs);
    let cookie = login_cookie(port);

    // GH protected write -> 403, zero UpdateTraits
    let form = format!("device={GH_LAMP}&trait=onOff&value=false");
    let (st, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(st, 403, "{j}");
    assert_eq!(j["error"]["code"], "protected");
    assert_eq!(count_path(&foyer, UPDATE_PATH).await, 0);

    // Tuya protected on -> 403, zero commands
    let (st, j) = post_form(port, "/api/tuya/on", &format!("device={pid}"), &cookie);
    assert_eq!(st, 403, "{j}");
    assert_eq!(j["error"]["code"], "protected");
    assert_eq!(count_path(&api, "/commands").await, 0);

    // reads still work
    let (st, j) = get_json(port, &format!("/api/gh/get?device={GH_LAMP}"), &cookie);
    assert_eq!(st, 200, "{j}");
    let (st, j) = get_json(port, &format!("/api/tuya/get?device={pid}"), &cookie);
    assert_eq!(st, 200, "{j}");
    assert_eq!(j["data"]["telemetry"]["cur_voltage"]["value"], 103.5);
}

/// Smoke: index.html landmarks and interactive strings, no em dash.
#[test]
fn index_smoke_landmarks_and_controls() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);
    let cookie = login_cookie(port);
    // fetch index via cookie
    let req = format!(
        "GET / HTTP/1.1\r\nHost: x\r\nAccept: text/html\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    let (status, headers, body) = http_full(port, &req);
    assert_eq!(status, 200);
    let ct = header(&headers, "content-type").unwrap_or("");
    assert!(ct.contains("text/html"), "{ct}");
    for needle in [
        "<header",
        "<main",
        "aria-live=\"polite\"",
        "<dialog",
        "Sign out",
        "Refresh",
        "/api/devices",
        "/api/status",
        "/api/gh/get",
        "/api/gh/set",
        "/api/tuya/get",
        "/api/tuya/on",
        "/api/tuya/off",
        "/api/tuya/login/start",
        "/api/tuya/login/poll",
        "protected",
        "backend-banner",
        // Copy now lives in the inline dictionary; the function calls that
        // reach it stay in the markup.
        "t(\"status.loading\")",
        "t(\"state.loading\")",
        "t(\"telemetry.loading\")",
        "t(\"telemetry.none\")",
        "t(\"empty.devices\")",
        "t(\"syncing\")",
        "t(\"telemetry\")",
        "t(\"banner.tuyaSignIn\")",
        // A failed state read is not an offline Device: only a real offline
        // refusal clears the online flag, so a transient 500 keeps the switch.
        "if(off)dev._online=false;",
        "data-i18n=\"qr.userCodeLabel\"",
        "\"Loading devices...\"",
        "\"No devices found. Check that both backends are logged in.\"",
        "\"Sign in to Tuya (QR)\"",
        "\"User code (optional)\"",
    ] {
        assert!(body.contains(needle), "index missing {needle:?}");
    }
    assert!(
        !body.contains(char::from_u32(0x2014).unwrap()),
        "no em dash"
    );
    // Device state is served in the shape control.rs emits, so the panel must
    // write back the same trait/field names it reads.
    for needle in [
        "temperatureSetting.thermostatTemperatureSetpoint",
        "traits.color.colorTemperature",
    ] {
        assert!(body.contains(needle), "index writes {needle:?}");
    }
    // Product copy only: the header is the product name, with no metaphor and
    // no duplicate tag beside it.
    for needle in ["<h1>home</h1>", "<title>home</title>"] {
        assert!(body.contains(needle), "index header missing {needle:?}");
    }
    for gone in ["家里的配电箱", "home-tag", ">Breakers<"] {
        assert!(!body.contains(gone), "index still carries {gone:?}");
    }
    // Long read-only readings sit behind one native disclosure per row.
    for needle in [
        "createElement(\"details\")",
        "createElement(\"summary\")",
        "t(\"details\")",
        "\"Details\"",
    ] {
        assert!(body.contains(needle), "index lacks disclosure {needle:?}");
    }
    // The protected marker is drawn by the cover around the control, not above it.
    assert!(
        body.contains("cover.className=\"cover\"") && body.contains("g.className=\"guard\""),
        "protected cover markup missing"
    );
    // The list heading is visually hidden so it does not float under the status.
    assert!(
        body.contains("id=\"list-heading\" class=\"visually-hidden\""),
        "list heading must be visually hidden"
    );
}

/// Smoke: login page contract strings.
#[test]
fn login_page_smoke() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);
    let req = "GET /login HTTP/1.1\r\nHost: x\r\nAccept: text/html\r\nConnection: close\r\n\r\n";
    let (status, headers, body) = http_full(port, req);
    assert_eq!(status, 200);
    assert!(header(&headers, "content-type")
        .unwrap_or("")
        .contains("text/html"));
    for needle in [
        "action=\"/login\"",
        "<label for=\"pw\">Password</label>",
        "role=\"alert\"",
    ] {
        assert!(body.contains(needle), "login missing {needle:?}");
    }
}

/// Every answer carries the hardening headers, and a stale or forged session
/// id is 401 even for a page path.
#[test]
fn responses_carry_security_headers_and_only_live_sessions_pass() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);
    let cookie = login_cookie(port);
    assert_eq!(cookie.split('=').count(), 2, "one cookie pair: {cookie}");
    let id = cookie.split_once('=').map(|(_, v)| v).unwrap_or("");
    assert_eq!(id.len(), 64, "32 random bytes as hex: {id}");

    for target in ["/login", "/api/health", "/api/devices"] {
        let req = format!(
            "GET {target} HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
        );
        let (_, headers, _) = http_full(port, &req);
        for name in [
            "x-content-type-options",
            "x-frame-options",
            "content-security-policy",
            "referrer-policy",
            "cache-control",
        ] {
            assert!(header(&headers, name).is_some(), "{target} missing {name}");
        }
        assert_eq!(header(&headers, "x-frame-options"), Some("DENY"));
        assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    }

    // A forged id is refused, and so is the signed-out one.
    let mut forged = cookie.clone();
    forged.pop();
    let req = format!(
        "GET /api/health HTTP/1.1\r\nHost: x\r\nCookie: {forged}\r\nConnection: close\r\n\r\n"
    );
    let (status, _h, body) = http_full(port, &req);
    assert_eq!(status, 401, "{body}");
    let req = format!(
        "POST /logout HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(http_full(port, &req).0, 200);
    let req = format!(
        "GET /api/health HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(http_full(port, &req).0, 401, "logged-out id still works");
}

// --- shared failed-password rate limiting --------------------------------

/// POST /login without a body returns the status and headers.
fn post_login(port: u16, password: &str) -> (u16, Vec<(String, String)>, String) {
    let form = format!("password={password}");
    let req = format!(
        "POST /login HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{form}",
        form.len()
    );
    http_full(port, &req)
}

fn post_form_headers(
    port: u16,
    p: &str,
    form: &str,
    cookie: &str,
) -> (u16, Vec<(String, String)>, Value) {
    let req = format!(
        "POST {p} HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n{form}",
        form.len()
    );
    let (status, headers, body) = http_full(port, &req);
    (status, headers, body_json(&body))
}

/// Three wrong logins are 401; the fourth is 429 with Retry-After, and even
/// the right password stays 429 until the backoff expires.
#[test]
fn login_rate_limit_blocks_even_the_right_password_during_backoff() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);

    for _ in 0..3 {
        let (status, _h, _b) = post_login(port, "wrong");
        assert_eq!(status, 401);
    }
    let (status, headers, body) = post_login(port, "wrong");
    assert_eq!(status, 429, "{body}");
    assert!(header(&headers, "retry-after").is_some(), "Retry-After");
    assert!(body.contains("Too many failed attempts"), "{body}");

    // A right password during the backoff must not succeed.
    let (status, _h, body) = post_login(port, "pw123456");
    assert_eq!(status, 429, "{body}");

    // After the 2 s backoff, the right password works and resets the counter.
    std::thread::sleep(Duration::from_millis(2300));
    let (status, _h, _b) = post_login(port, "pw123456");
    assert_eq!(status, 200);
    // Reset: two more wrong ones are 401, not 429.
    for _ in 0..2 {
        let (status, _h, _b) = post_login(port, "wrong");
        assert_eq!(status, 401);
    }
}

/// Failures on a protected override share the counter with /login.
#[tokio::test]
async fn override_failures_rate_limit_the_login_too() {
    let (_dir, db) = temp_db();
    seed_gh(&db);
    home_store::Store::open(&db)
        .unwrap()
        .protect("google_home", GH_LAMP)
        .unwrap();
    let foyer = MockServer::start().await;
    mount_foyer(&foyer).await;
    let port = free_port();
    let _s = spawn_env(&db, port, &gh_env(&foyer));
    let cookie = login_cookie(port);

    for _ in 0..3 {
        let form = format!("device={GH_LAMP}&trait=onOff&value=false&password=nope");
        let (status, _h, j) = post_form_headers(port, "/api/gh/set", &form, &cookie);
        assert_eq!(status, 403, "{j}");
        assert_eq!(j["error"]["code"], "protected");
    }
    // The fourth failure through the override is 429 with the JSON code.
    let form = format!("device={GH_LAMP}&trait=onOff&value=false&password=nope");
    let (status, headers, j) = post_form_headers(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 429, "{j}");
    assert!(header(&headers, "retry-after").is_some(), "Retry-After");
    assert_eq!(j["error"]["code"], "rate_limited");
    assert_eq!(count_path(&foyer, UPDATE_PATH).await, 0);

    // The shared counter also blocks /login, even with the right password.
    let (status, _h, _b) = post_login(port, "pw123456");
    assert_eq!(status, 429);
    // And a protected override with the RIGHT password is still 429.
    let form = format!("device={GH_LAMP}&trait=onOff&value=false&password=pw123456");
    let (status, _h, j) = post_form_headers(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 429, "{j}");
    assert_eq!(count_path(&foyer, UPDATE_PATH).await, 0);
}

/// Backoff grows: after the 2 s block expires, the next failure blocks for
/// longer than 2 s.
#[test]
fn the_backoff_grows_exponentially_after_each_failure() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);

    for _ in 0..3 {
        assert_eq!(post_login(port, "wrong").0, 401);
    }
    assert_eq!(post_login(port, "wrong").0, 429);
    std::thread::sleep(Duration::from_millis(2300));
    // Backoff over; one more failure takes the count to 5 -> 4 s block.
    let (status, _h, body) = post_login(port, "wrong");
    assert_eq!(status, 401, "{body}");
    let (status, _h, _b) = post_login(port, "wrong");
    assert_eq!(status, 429);
    // 2.3 s is past the first block but not the 4 s one.
    std::thread::sleep(Duration::from_millis(2300));
    let (status, _h, body) = post_login(port, "pw123456");
    assert_eq!(status, 429, "still blocked for 4 s: {body}");
    std::thread::sleep(Duration::from_millis(2300));
    let (status, _h, _b) = post_login(port, "pw123456");
    assert_eq!(status, 200);
}

// --- GET /settings: same page, same policy as / ---------------------------

fn get_html(port: u16, target: &str, cookie: &str) -> (u16, String, String) {
    let req = format!(
        "GET {target} HTTP/1.1\r\nHost: x\r\nAccept: text/html\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    let (status, headers, body) = http_full(port, &req);
    let ct = header(&headers, "content-type").unwrap_or("").to_string();
    (status, ct, body)
}

/// Anonymous: HTML gets the login page, anything else gets the JSON 401.
#[test]
fn settings_is_the_login_page_for_anonymous_html_and_json_401_otherwise() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);

    let (status, ct, body) = get_html(port, "/settings", "");
    assert_eq!(status, 200, "{body}");
    assert!(ct.contains("text/html"), "content-type {ct}");
    assert!(body.contains("action=\"/login\""), "{body}");

    let (status, ct, body) = http_get(port, "/settings");
    assert_eq!(status, 401, "{body}");
    assert!(ct.contains("application/json"), "content-type {ct}");
    assert_eq!(body_json(&body)["error"]["code"], "unauthorized");
}

/// Signed in: /settings and / are byte-for-byte the same page, with the
/// hardening headers on the new path too.
#[test]
fn settings_serves_the_same_page_as_root_with_security_headers() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);
    let cookie = login_cookie(port);

    let (status, _ct, root) = get_html(port, "/", &cookie);
    assert_eq!(status, 200, "{root}");
    let req = format!(
        "GET /settings HTTP/1.1\r\nHost: x\r\nAccept: text/html\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    );
    let (status, headers, settings) = http_full(port, &req);
    assert_eq!(status, 200, "{settings}");
    assert_eq!(settings, root, "/settings body differs from /");
    assert_eq!(
        header(&headers, "content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(header(&headers, "x-content-type-options"), Some("nosniff"));
    assert_eq!(header(&headers, "x-frame-options"), Some("DENY"));
    assert_eq!(
        header(&headers, "content-security-policy"),
        Some("frame-ancestors 'none'; base-uri 'none'")
    );
    assert_eq!(header(&headers, "referrer-policy"), Some("no-referrer"));
    assert_eq!(header(&headers, "cache-control"), Some("no-store"));
}

// --- login failure pages carry a machine-readable code --------------------

/// The 401 page names the code, keeps the English text, and the 429 page
/// names its own code.
#[test]
fn login_failures_render_a_localizable_error_code() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn_env(&db, port, &[]);

    let (status, _h, body) = post_login(port, "wrong");
    assert_eq!(status, 401, "{body}");
    assert!(
        body.contains("<p role=\"alert\" data-code=\"wrong_password\">Wrong password</p>"),
        "{body}"
    );

    for _ in 0..2 {
        assert_eq!(post_login(port, "wrong").0, 401);
    }
    let (status, headers, body) = post_login(port, "wrong");
    assert_eq!(status, 429, "{body}");
    assert!(header(&headers, "retry-after").is_some(), "Retry-After");
    assert!(
        body.contains("<p role=\"alert\" data-code=\"rate_limited\">Too many failed attempts"),
        "{body}"
    );
}
