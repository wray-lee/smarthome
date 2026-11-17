//! Serve panel tests: spawn `home serve` on an ephemeral port and assert the
//! static HTML is embedded, served with `text/html`, and free of em dashes.

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
    let mut lines = resp.lines();
    let status: u16 = lines
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

fn spawn_env(db: &Path, port: u16, extra: &[String], envs: &[(String, String)]) -> Serve {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    clear_env(&mut cmd);
    cmd.arg("--db")
        .arg(db)
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .args(extra)
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

fn login_cookie(port: u16, password: &str) -> (u16, Vec<(String, String)>, Value, String) {
    let form = format!("password={password}");
    let req = format!(
        "POST /login HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{form}",
        form.len()
    );
    let (status, headers, body) = http_full(port, &req);
    let cookie = header(&headers, "set-cookie")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let parsed = serde_json::from_str(&body).unwrap_or(Value::Null);
    (status, headers, parsed, cookie)
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

#[allow(dead_code)]
fn cli(db: &Path, args: &[&str], envs: &[(String, String)]) -> (i32, Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    clear_env(&mut cmd);
    cmd.arg("--db").arg(db).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("running home");
    let stdout = String::from_utf8(out.stdout).expect("stdout UTF-8");
    let stderr = String::from_utf8(out.stderr).expect("stderr UTF-8");
    let value: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|_| {
        panic!(
            "stdout is not JSON: {stdout:?}; stderr: {stderr:?}; code {:?}",
            out.status.code()
        )
    });
    (out.status.code().expect("exit code"), value)
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

fn seed_tuya(db: &Path, endpoint: &str, expire_offset: i64) {
    let store = home_store::Store::open(db).unwrap();
    let now = home_store::store::now_unix();
    let session = json!({
        "access_token": "dummy-access-1",
        "refresh_token": "dummy-refresh-1",
        "endpoint": endpoint,
        "user_code": "DummyCode1",
        "terminal_id": "dummy-terminal-1",
        "uid": "dummy-uid-1",
        "expire_time": now + expire_offset,
    });
    store.set_credential("tuya", &session.to_string()).unwrap();
}

fn tuya_session_json(db: &Path) -> Value {
    let store = home_store::Store::open(db).unwrap();
    let raw = store.credential("tuya").unwrap().expect("tuya credential");
    serde_json::from_str(&raw).unwrap()
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

const HOMES: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/homes.json");
const HOME_DEVICES: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/home-devices.json");

fn tuya_home_id() -> String {
    let list: Value = serde_json::from_str(HOME_DEVICES).unwrap();
    list["result"][1]["id"].as_str().unwrap().to_string()
}

fn tuya_first_id() -> String {
    let list: Value = serde_json::from_str(HOME_DEVICES).unwrap();
    list["result"][0]["id"].as_str().unwrap().to_string()
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

async fn mount_tuya_command(api: &MockServer, id: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/v1.1/m/thing/{id}/commands")))
        .respond_with(envelope(
            json!({"t": now_ms(), "success": true, "result": true}),
        ))
        .mount(api)
        .await;
}

async fn tuya_count(api: &MockServer, needle: &str) -> usize {
    api.received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().contains(needle))
        .count()
}

const GH_LAMP: &str = "00000000-0000-4000-8000-00000000h0me";
const GH_PLUG: &str = "00000000-0000-4000-8000-00000001ab";
const GH_LOCK: &str = "00000000-0000-4000-8000-00000000l0ck";

const GRAPH_PATH: &str = "/$rpc/google.internal.home.foyer.v1.StructuresService/GetHomeGraph";
const GET_PATH: &str = "/$rpc/google.internal.home.foyer.v1.HomeControlService/GetTraits";
const UPDATE_PATH: &str = "/$rpc/google.internal.home.foyer.v1.HomeControlService/UpdateTraits";

fn gh_device(id: &str, partner: &str, name: &str, kind: &str, traits: &[&str]) -> Value {
    json!([
        [id, ["tuya-smart-cee31", partner]],
        null,
        null,
        name,
        null,
        kind,
        traits
    ])
}

fn gh_room(name: &str, ids: &[&str]) -> Value {
    json!([
        format!("room-{name}"),
        null,
        name,
        ["OTHER"],
        ids.iter()
            .map(|id| json!([[id, ["tuya-smart-cee31", format!("{id}_all")]]]))
            .collect::<Vec<Value>>(),
    ])
}

fn gh_graph() -> Value {
    json!([
        "<num>",
        [[
            "home-1",
            "home",
            [],
            [],
            null,
            [
                gh_room("Den", &[GH_LAMP, GH_PLUG]),
                gh_room("Hall", &[GH_LOCK]),
            ],
            [
                gh_device(
                    GH_LAMP,
                    "homePartner_all",
                    "Lamp",
                    "action.devices.types.LIGHT",
                    &[
                        "action.devices.traits.OnOff",
                        "action.devices.traits.Brightness"
                    ],
                ),
                gh_device(
                    GH_PLUG,
                    "plugPartner_all",
                    "PlugX",
                    "action.devices.types.OUTLET",
                    &["action.devices.traits.OnOff"],
                ),
                gh_device(
                    GH_LOCK,
                    "lockPartnerId_all",
                    "Front Door",
                    "action.devices.types.LOCK",
                    &[
                        "action.devices.traits.OnOff",
                        "action.devices.traits.LockUnlock"
                    ],
                ),
            ]
        ]]
    ])
}

fn gh_bool(v: bool) -> Value {
    json!([null, null, null, if v { 1 } else { 0 }])
}

fn gh_state(id: &str, online: i64) -> Value {
    let mut traits = vec![json!([
        "deviceStatus",
        [["online", [null, null, null, online]]]
    ])];
    match id {
        GH_LAMP | GH_PLUG => traits.push(json!(["onOff", [["onOff", gh_bool(true)]]])),
        GH_LOCK => {
            traits.push(json!(["onOff", [["onOff", gh_bool(true)]]]));
            traits.push(json!(["lockUnlock", [["isLocked", gh_bool(true)]]]));
        }
        _ => {}
    }
    json!([[id], traits])
}

fn gh_states() -> Value {
    json!([[
        gh_state(GH_LAMP, 1),
        gh_state(GH_PLUG, 1),
        gh_state(GH_LOCK, 1)
    ]])
}

fn gh_states_offline() -> Value {
    json!([[
        gh_state(GH_LAMP, 0),
        gh_state(GH_PLUG, 1),
        gh_state(GH_LOCK, 1)
    ]])
}

async fn mount_foyer(foyer: &MockServer, get_answer: &Value) {
    Mock::given(method("POST"))
        .and(path(GRAPH_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(gh_graph().to_string()),
        )
        .mount(foyer)
        .await;
    Mock::given(method("POST"))
        .and(path(GET_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(get_answer.to_string()),
        )
        .mount(foyer)
        .await;
    Mock::given(method("POST"))
        .and(path(UPDATE_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(gh_states().to_string()),
        )
        .mount(foyer)
        .await;
}

async fn mount_auth_sync(auth: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Auth=dummy-bearer\n"))
        .mount(auth)
        .await;
}

async fn foyer_count(foyer: &MockServer, p: &str) -> usize {
    foyer
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == p)
        .count()
}

fn gh_env(foyer: &MockServer) -> Vec<(String, String)> {
    vec![(
        "HOME_FOYER_URL".into(),
        format!("{}/$rpc/google.internal.home.foyer.v1.", foyer.uri()),
    )]
}

fn tuya_env(api: &MockServer) -> Vec<(String, String)> {
    vec![
        ("HOME_TUYA_LOGIN_URL".into(), api.uri()),
        ("HOME_TUYA_ENDPOINT".into(), api.uri()),
    ]
}

// --- (a) startup refusal + password-file / secure cookie ---

#[test]
fn no_password_refuses_start_with_usage_exit_2() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    clear_env(&mut cmd);
    cmd.arg("--db")
        .arg(&db)
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = cmd.output().expect("run serve without password");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let value: Value = serde_json::from_str(stdout.trim()).expect("JSON");
    assert_eq!(out.status.code(), Some(2), "{value}");
    assert_eq!(value["error"]["code"], "usage");
}

#[test]
fn unreadable_password_file_is_usage() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let missing = PathBuf::from("/nonexistent/pw.txt");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    clear_env(&mut cmd);
    cmd.arg("--db")
        .arg(&db)
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--password-file")
        .arg(&missing)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = cmd.output().expect("run serve with missing password file");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let value: Value = serde_json::from_str(stdout.trim()).expect("JSON");
    assert_eq!(out.status.code(), Some(2), "{value}");
    assert_eq!(value["error"]["code"], "usage");
    let msg = value["error"]["message"].as_str().unwrap();
    assert!(msg.contains("/nonexistent/pw.txt"), "{msg}");
}

#[tokio::test]
async fn secret_file_login_sets_secure_strict_cookie() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let pwfile = PathBuf::from(format!("/tmp/pw-{port}.txt"));
    std::fs::write(&pwfile, "s3cr3t-pw\n").unwrap();
    let _s = spawn_env(
        &db,
        port,
        &[
            "--password-file".into(),
            pwfile.to_string_lossy().into_owned(),
        ],
        &[("HOME_COOKIE_SECURE".into(), "true".into())],
    );
    // Wrong password: 401.
    let (bad, _h, _b) = http_full(
        port,
        "POST /login HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 16\r\nConnection: close\r\n\r\npassword=wrong-pw",
    );
    assert_eq!(bad, 401);
    // Right password: 200 + cookie attributes.
    let (status, headers, _json, _cookie) = login_cookie(port, "s3cr3t-pw");
    assert_eq!(status, 200);
    let cookie = header(&headers, "set-cookie")
        .expect("Set-Cookie")
        .to_string();
    let cookie = cookie.as_str();
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
    assert!(cookie.contains("Secure"), "{cookie}");
    let _ = std::fs::remove_file(&pwfile);
}

// --- (b) unauthenticated 401 on every endpoint ---

#[tokio::test]
async fn unauthenticated_every_endpoint_returns_401() {
    let (_dir, db) = temp_db();
    let port = free_port();
    let _s = spawn(&db, port, "pw123456");
    let cases = [
        "GET /api/devices HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        "GET /api/status HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        "GET /api/health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        "GET /api/gh/get?device=x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        "POST /api/gh/set HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "GET /api/tuya/get?device=x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        "POST /api/tuya/on HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "POST /api/tuya/off HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "POST /logout HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "GET /nonexistent HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        "GET / HTTP/1.1\r\nHost: x\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
    ];
    for req in cases {
        let (status, _ct, body) = http(port, req);
        assert_eq!(status, 401, "{req}");
        let j: Value = serde_json::from_str(&body).expect("401 JSON");
        assert_eq!(j["error"]["code"], "unauthorized", "{req}: {j}");
    }
    // GET / with HTML accept is login page (200), not 401.
    let (status, ct, _body) = http(
        port,
        "GET / HTTP/1.1\r\nHost: x\r\nAccept: text/html\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status, 200);
    assert!(ct.contains("text/html"));
}

// --- (c) device list parity with CLI + guard + sensitive action + exit mapping ---

#[tokio::test]
async fn api_devices_matches_cli_device_lists() {
    let (_dir, db) = temp_db();
    seed_gh(&db);
    // Protect the second fixture device (fake id, never a real one).
    let protected_id = tuya_first_id();
    home_store::Store::open(&db)
        .unwrap()
        .protect("google_home", GH_PLUG)
        .unwrap();

    let foyer = MockServer::start().await;
    mount_foyer(&foyer, &gh_states()).await;
    let auth = MockServer::start().await;
    mount_auth_sync(&auth).await;
    let api = MockServer::start().await;
    mount_tuya_devices(&api).await;
    seed_tuya(&db, &api.uri(), 7200);

    let port = free_port();
    let mut envs = gh_env(&foyer);
    envs.extend(tuya_env(&api));
    let _s = spawn_env(&db, port, &["--password".into(), "pw123456".into()], &envs);
    let (ls, _h, _j, cookie) = login_cookie(port, "pw123456");
    assert_eq!(ls, 200);

    let (_code, cli_gh) = cli(&db, &["gh", "devices"], &gh_env(&foyer));
    let (_code, cli_tuya) = cli(&db, &["tuya", "devices"], &tuya_env(&api));

    let (status, api_j) = get_json(port, "/api/devices", &cookie);
    assert_eq!(status, 200);
    assert_eq!(api_j["ok"], true);
    assert_eq!(api_j["data"]["google_home"], cli_gh["data"]);
    assert_eq!(api_j["data"]["tuya"], cli_tuya["data"]);
    let protected = api_j["data"]["protected"].as_array().unwrap();
    assert!(protected
        .iter()
        .any(|p| p["backend"] == "google_home" && p["device_id"] == GH_PLUG));
    let _ = protected_id;
}

#[tokio::test]
async fn protected_writes_return_403_and_send_nothing() {
    let (_dir, db) = temp_db();
    seed_gh(&db);
    home_store::Store::open(&db)
        .unwrap()
        .protect("google_home", GH_PLUG)
        .unwrap();
    let pid = tuya_home_id();
    home_store::Store::open(&db)
        .unwrap()
        .protect("tuya", &pid)
        .unwrap();

    let foyer = MockServer::start().await;
    mount_foyer(&foyer, &gh_states()).await;
    let auth = MockServer::start().await;
    mount_auth_sync(&auth).await;
    let api = MockServer::start().await;
    mount_tuya_devices(&api).await;
    mount_tuya_command(&api, &pid).await;
    seed_tuya(&db, &api.uri(), 7200);

    let port = free_port();
    let mut envs = gh_env(&foyer);
    envs.extend(tuya_env(&api));
    let _s = spawn_env(&db, port, &["--password".into(), "pw123456".into()], &envs);
    let (_ls, _h, _j, cookie) = login_cookie(port, "pw123456");

    let form = format!("device={GH_PLUG}&trait=onOff&value=false");
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 403, "{j}");
    assert_eq!(j["error"]["code"], "protected");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 0);

    let form = format!("device={pid}");
    let (status, j) = post_form(port, "/api/tuya/on", &form, &cookie);
    assert_eq!(status, 403, "{j}");
    assert_eq!(j["error"]["code"], "protected");
    assert_eq!(tuya_count(&api, "/commands").await, 0);
}

#[tokio::test]
async fn sensitive_action_needs_confirm() {
    let (_dir, db) = temp_db();
    seed_gh(&db);
    let foyer = MockServer::start().await;
    mount_foyer(&foyer, &gh_states()).await;
    let auth = MockServer::start().await;
    mount_auth_sync(&auth).await;
    seed_tuya(&db, "https://apigw.tuyaus.com", 7200);

    let port = free_port();
    let _s = spawn_env(
        &db,
        port,
        &["--password".into(), "pw123456".into()],
        &gh_env(&foyer),
    );
    let (_ls, _h, _j, cookie) = login_cookie(port, "pw123456");

    let form = format!("device={GH_LOCK}&trait=lockUnlock.isLocked&value=false");
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 403, "{j}");
    assert_eq!(j["error"]["code"], "sensitive_action");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 0);

    let form = format!("device={GH_LOCK}&trait=lockUnlock.isLocked&value=false&confirm=true");
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 200, "{j}");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 1);
}

#[tokio::test]
async fn a_password_override_allows_one_write_on_a_protected_device() {
    let (_dir, db) = temp_db();
    seed_gh(&db);
    home_store::Store::open(&db)
        .unwrap()
        .protect("google_home", GH_PLUG)
        .unwrap();
    let pid = tuya_home_id();
    home_store::Store::open(&db)
        .unwrap()
        .protect("tuya", &pid)
        .unwrap();

    let foyer = MockServer::start().await;
    mount_foyer(&foyer, &gh_states()).await;
    let api = MockServer::start().await;
    mount_tuya_devices(&api).await;
    mount_tuya_command(&api, &pid).await;
    seed_tuya(&db, &api.uri(), 7200);

    let port = free_port();
    let mut envs = gh_env(&foyer);
    envs.extend(tuya_env(&api));
    let _s = spawn_env(&db, port, &["--password".into(), "pw123456".into()], &envs);
    let (_ls, _h, _j, cookie) = login_cookie(port, "pw123456");

    // GH: no password -> 403 protected, zero UpdateTraits.
    let form = format!("device={GH_PLUG}&trait=onOff&value=false");
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 403, "{j}");
    assert_eq!(j["error"]["code"], "protected");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 0);

    // Wrong password -> 403 protected, still zero writes.
    let form = format!("device={GH_PLUG}&trait=onOff&value=false&password=nope");
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 403, "{j}");
    assert_eq!(j["error"]["code"], "protected");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 0);

    // Right password -> 200 and one write; the protected list is unchanged.
    let form = format!("device={GH_PLUG}&trait=onOff&value=false&password=pw123456");
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 200, "{j}");
    assert_eq!(j["data"]["changed"]["trait"], "onOff");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 1);
    let protected = home_store::Store::open(&db)
        .unwrap()
        .protected_devices()
        .unwrap();
    assert_eq!(protected.len(), 2, "{protected:?}");

    // Still protected: the next call without a password is refused again.
    let form = format!("device={GH_PLUG}&trait=onOff&value=true");
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 403, "{j}");
    assert_eq!(j["error"]["code"], "protected");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 1);

    // Tuya: no password -> 403, zero commands; right password -> 200.
    let (status, j) = post_form(port, "/api/tuya/on", &format!("device={pid}"), &cookie);
    assert_eq!(status, 403, "{j}");
    assert_eq!(j["error"]["code"], "protected");
    assert_eq!(tuya_count(&api, "/commands").await, 0);

    let form = format!("device={pid}&password=pw123456");
    let (status, j) = post_form(port, "/api/tuya/on", &form, &cookie);
    assert_eq!(status, 200, "{j}");
    assert_eq!(tuya_count(&api, "/commands").await, 1);
}

#[tokio::test]
async fn a_protected_lock_needs_confirm_even_with_the_override_password() {
    let (_dir, db) = temp_db();
    seed_gh(&db);
    home_store::Store::open(&db)
        .unwrap()
        .protect("google_home", GH_LOCK)
        .unwrap();
    let foyer = MockServer::start().await;
    mount_foyer(&foyer, &gh_states()).await;
    let port = free_port();
    let _s = spawn_env(
        &db,
        port,
        &["--password".into(), "pw123456".into()],
        &gh_env(&foyer),
    );
    let (_ls, _h, _j, cookie) = login_cookie(port, "pw123456");

    let form = format!("device={GH_LOCK}&trait=lockUnlock.isLocked&value=false&password=pw123456");
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 403, "{j}");
    assert_eq!(j["error"]["code"], "sensitive_action");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 0);

    let form = format!(
        "device={GH_LOCK}&trait=lockUnlock.isLocked&value=false&password=pw123456&confirm=true"
    );
    let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
    assert_eq!(status, 200, "{j}");
    assert_eq!(foyer_count(&foyer, UPDATE_PATH).await, 1);
}

#[tokio::test]
async fn exit_to_http_status_mapping_for_usage_offline_and_reauth() {
    // Usage (400): unknown trait value for GH.
    {
        let (_dir, db) = temp_db();
        seed_gh(&db);
        let foyer = MockServer::start().await;
        mount_foyer(&foyer, &gh_states()).await;
        let port = free_port();
        let _s = spawn_env(
            &db,
            port,
            &["--password".into(), "pw123456".into()],
            &gh_env(&foyer),
        );
        let (_ls, _h, _j, cookie) = login_cookie(port, "pw123456");
        let form = format!("device={GH_LAMP}&trait=onOff&value=maybe");
        let (status, j) = post_form(port, "/api/gh/set", &form, &cookie);
        assert_eq!(status, 400, "{j}");
        assert_eq!(j["error"]["code"], "usage");
    }
    // Offline (503): lamp reported offline.
    {
        let (_dir, db) = temp_db();
        seed_gh(&db);
        let foyer = MockServer::start().await;
        mount_foyer(&foyer, &gh_states_offline()).await;
        let port = free_port();
        let _s = spawn_env(
            &db,
            port,
            &["--password".into(), "pw123456".into()],
            &gh_env(&foyer),
        );
        let (_ls, _h, _j, cookie) = login_cookie(port, "pw123456");
        let target = format!("/api/gh/get?device={GH_LAMP}");
        let (status, j) = get_json(port, &target, &cookie);
        assert_eq!(status, 503, "{j}");
        assert_eq!(j["error"]["code"], "device_offline");
    }
    // Reauth (424): no GH credential seeded.
    {
        let (_dir, db) = temp_db();
        let foyer = MockServer::start().await;
        mount_foyer(&foyer, &gh_states()).await;
        let port = free_port();
        let _s = spawn_env(
            &db,
            port,
            &["--password".into(), "pw123456".into()],
            &gh_env(&foyer),
        );
        let (_ls, _h, _j, cookie) = login_cookie(port, "pw123456");
        let (status, j) = get_json(port, "/api/gh/get?device=x", &cookie);
        assert_eq!(status, 424, "{j}");
        assert_eq!(j["error"]["code"], "reauth_required");
    }
}

// --- (d) background Tuya refresh with zero client requests ---

#[tokio::test]
async fn background_refresh_runs_with_zero_client_requests() {
    let (_dir, db) = temp_db();
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/token/dummy-refresh-1"))
        .respond_with(envelope(json!({
            "t": now_ms(),
            "success": true,
            "result": {
                "accessToken": "dummy-access-2",
                "refreshToken": "dummy-refresh-2",
                "expireTime": 7200,
                "uid": "dummy-uid-1",
            },
        })))
        .mount(&api)
        .await;
    seed_tuya(&db, &api.uri(), 2);

    let port = free_port();
    let mut envs = tuya_env(&api);
    envs.push(("HOME_TUYA_REFRESH_POLL_MS".into(), "50".into()));
    let _s = spawn_env(&db, port, &["--password".into(), "pw123456".into()], &envs);

    // Zero HTTP client requests from this test process. Wait for the fake
    // upstream to have seen the refresh and the DB to hold the new pair.
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut refreshed = false;
    while Instant::now() < deadline {
        if tuya_count(&api, "/v1.0/m/token/").await >= 1 {
            let sess = tuya_session_json(&db);
            if sess["refresh_token"] == "dummy-refresh-2" {
                assert_eq!(sess["access_token"], "dummy-access-2");
                refreshed = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(refreshed, "background refresh did not persist new tokens");
}

// --- (e) background refresh failure surfaces on /api/status ---

#[tokio::test]
async fn background_refresh_failure_surfaces_on_api_status() {
    let (_dir, db) = temp_db();
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/token/dummy-refresh-1"))
        .respond_with(envelope(json!({
            "success": false, "code": 1010, "msg": "token invalid",
        })))
        .mount(&api)
        .await;
    seed_tuya(&db, &api.uri(), 2);

    let port = free_port();
    let mut envs = tuya_env(&api);
    envs.push(("HOME_TUYA_REFRESH_POLL_MS".into(), "50".into()));
    let _s = spawn_env(&db, port, &["--password".into(), "pw123456".into()], &envs);

    // Wait for the background tick to fail once (zero client requests yet).
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if tuya_count(&api, "/v1.0/m/token/").await >= 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(tuya_count(&api, "/v1.0/m/token/").await >= 1);

    let (_ls, _h, _j, cookie) = login_cookie(port, "pw123456");
    let (status, j) = get_json(port, "/api/status", &cookie);
    assert_eq!(status, 200, "{j}");
    assert_eq!(j["data"]["status"], "degraded");
    assert_eq!(j["data"]["tuya"]["status"], "reauth_required");
    let msg = j["data"]["tuya"]["message"].as_str().unwrap_or("");
    assert!(!msg.is_empty(), "{j}");
}

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
