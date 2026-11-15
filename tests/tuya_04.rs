//! Black-box tests for ticket 04: the Tuya QR login, the device list and the
//! Local Key backup. The `home` binary is run against a fake Tuya upstream
//! (wiremock) and a temporary SQLite file, and only its stdout JSON, exit
//! code and the database are asserted.
//!
//! No test contains a real Device ID, token or Local Key. The wire shapes are
//! the redacted ticket-01 fixtures plus synthetic values (`fake-key-a`,
//! `dummy-access-1`, ...). The CLI wiring (`home tuya login|devices|keys`) is
//! in `src/main.rs`; these tests run the real binary against the fake upstream.

use std::path::{Path, PathBuf};
use std::process::Command;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// A `MockServer` that answers every request with a redirect, so a test can
/// point one at another server and prove where the hop did or did not land.
async fn redirecting_to(location: &str) -> MockServer {
    let server = MockServer::start().await;
    let place = location.to_string();
    Mock::given(wiremock::matchers::any())
        .respond_with(move |_req: &Request| {
            ResponseTemplate::new(307).insert_header("location", place.clone())
        })
        .mount(&server)
        .await;
    server
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn home(db: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    cmd.arg("--db").arg(db);
    // Every Tuya override is cleared, so an ambient variable in the developer
    // shell cannot point a test at a real host.
    for key in [
        "HOME_TUYA_LOGIN_URL",
        "HOME_TUYA_ENDPOINT",
        "HOME_TUYA_USER_CODE",
        "HOME_TUYA_POLL_MS",
        "HOME_TUYA_POLL_ATTEMPTS",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

/// Run the binary and demand exactly one JSON document on stdout.
fn run_full(cmd: &mut Command) -> (i32, serde_json::Value, String, String) {
    let out = cmd.output().expect("running the home binary");
    let stdout = String::from_utf8(out.stdout).expect("stdout is UTF-8");
    let stderr = String::from_utf8(out.stderr).expect("stderr is UTF-8");
    let mut documents =
        serde_json::Deserializer::from_str(&stdout).into_iter::<serde_json::Value>();
    let value = documents
        .next()
        .unwrap_or_else(|| panic!("stdout is not JSON: {stdout:?}; stderr: {stderr:?}"))
        .unwrap_or_else(|e| {
            panic!("stdout is not one JSON document ({e}): {stdout:?}; stderr: {stderr:?}")
        });
    let end = documents.byte_offset();
    assert!(
        documents.next().is_none(),
        "stdout holds more than one JSON document: {stdout:?}"
    );
    assert!(
        stdout[end..].trim().is_empty(),
        "stdout holds trailing non-JSON bytes: {stdout:?}"
    );
    (
        out.status.code().expect("an exit code"),
        value,
        stdout,
        stderr,
    )
}

fn run(cmd: &mut Command) -> (i32, serde_json::Value) {
    let (code, value, _, _) = run_full(cmd);
    (code, value)
}

fn temp_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("home.sqlite");
    (dir, db)
}

/// Point the binary at the fake login host and the fake regional endpoint,
/// with a fast poll so a login test takes milliseconds.
fn point_at_tuya(cmd: &mut Command, login: &MockServer, api: &MockServer) {
    cmd.env("HOME_TUYA_LOGIN_URL", login.uri());
    cmd.env("HOME_TUYA_ENDPOINT", api.uri());
    cmd.env("HOME_TUYA_POLL_MS", "0");
    cmd.env("HOME_TUYA_POLL_ATTEMPTS", "3");
}

fn envelope(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(body.to_string())
}

// ---------------------------------------------------------------------------
// Fixtures: the redacted ticket-01 shapes, synthetic credentials
// ---------------------------------------------------------------------------

const HOMES: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/homes.json");
const HOME_DEVICES: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/home-devices.json");

/// The QR token the fake login hands out; also the value in the QR text.
const QR_TOKEN: &str = "fakeQrToken1";

fn qr_started() -> serde_json::Value {
    serde_json::json!({"t": 1794759258801i64, "success": true,
        "result": {"qrcode": QR_TOKEN}})
}

fn qr_pending() -> serde_json::Value {
    serde_json::json!({"success": false, "code": 1001, "msg": "not scanned"})
}

/// The login result, with the fake regional endpoint in it.
///
/// `t` is the current millisecond time, as a real server sends it, so the
/// session is not born expired and the next command does not refresh first.
fn qr_done(endpoint: &str) -> serde_json::Value {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    serde_json::json!({
        "t": now_ms,
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

/// Mount the two login mocks: a start that succeeds, and a poll that is
/// pending once and then succeeds. Each `Mock::given` gets its own server
/// path, so the two calls are told apart by method.
async fn mount_login(server: &MockServer, endpoint: &str) {
    Mock::given(method("POST"))
        .and(path("/v1.0/m/life/home-assistant/qrcode/tokens"))
        .respond_with(envelope(qr_started()))
        .mount(server)
        .await;
    let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let endpoint = endpoint.to_string();
    Mock::given(method("GET"))
        .and(path(format!(
            "/v1.0/m/life/home-assistant/qrcode/tokens/{QR_TOKEN}"
        )))
        .respond_with(move |_req: &Request| {
            let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                envelope(qr_pending())
            } else {
                envelope(qr_done(&endpoint))
            }
        })
        .mount(server)
        .await;
}

/// The home list and the device list, answered unsealed (the fake upstream is
/// allowed to answer either way; one test below covers a sealed answer).
async fn mount_plain_devices(api: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/users/homes"))
        .respond_with(envelope(
            serde_json::from_str::<serde_json::Value>(HOMES).unwrap(),
        ))
        .mount(api)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/home/devices"))
        .respond_with(envelope(
            serde_json::from_str::<serde_json::Value>(HOME_DEVICES).unwrap(),
        ))
        .mount(api)
        .await;
}

/// The same device list with a real `local_key`, so the save path has
/// something to save. Keys are synthetic (`fake-key-a`, `fake-key-b`).
fn devices_with_keys(keys: &[(&str, &str)]) -> serde_json::Value {
    let mut list: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
    for device in list["result"].as_array_mut().unwrap() {
        let id = device["id"].as_str().unwrap().to_string();
        let key = keys
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, v)| *v)
            .unwrap_or("");
        device["local_key"] = serde_json::Value::String(key.to_string());
    }
    list
}

/// The two fake Device IDs, read off the ticket-01 fixture instead of
/// spelled out here (`<redacted>` in the source, the fake IDs in the data).
fn fixture_device_ids() -> Vec<String> {
    let list: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
    list["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_string())
        .collect()
}

/// The first fake Device ID in the fixture (the one `devices_with_keys`
/// keys first in the tests below).
fn device_one() -> String {
    fixture_device_ids()[0].clone()
}

/// The second fake Device ID in the fixture.
fn device_two() -> String {
    fixture_device_ids()[1].clone()
}

fn login(db: &Path, login: &MockServer, api: &MockServer) {
    let mut cmd = home(db);
    point_at_tuya(&mut cmd, login, api);
    cmd.args(["tuya", "login", "--user-code", "DummyCode1"]);
    let (code, json, _, stderr) = run_full(&mut cmd);
    assert_eq!(code, 0, "{json}; stderr: {stderr}");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["backend"], "tuya");
}

fn credential(db: &Path) -> serde_json::Value {
    let conn = rusqlite::Connection::open(db).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT json FROM credentials WHERE backend = 'tuya'",
            [],
            |r| r.get(0),
        )
        .expect("the Tuya credential is stored");
    serde_json::from_str(&raw).unwrap()
}

// ---------------------------------------------------------------------------
// Login: QR text on stderr, poll, tokens and endpoint in SQLite
// ---------------------------------------------------------------------------

#[tokio::test]
async fn login_renders_the_qr_on_stderr_and_stores_the_session() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api = MockServer::start().await;
    let endpoint = api.uri();
    mount_login(&login_server, &endpoint).await;
    mount_plain_devices(&api).await;

    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "login", "--user-code", "Dummy Code-1"]);
    let (code, json, stdout, stderr) = run_full(&mut cmd);
    assert_eq!(code, 0, "{json}; stderr: {stderr}");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["backend"], "tuya");
    // The data carries no token, only where and until when.
    let data = json["data"].as_object().expect("an object");
    for key in ["access_token", "refresh_token"] {
        assert!(!data.contains_key(key), "{key} must not be printed");
    }
    assert_eq!(json["data"]["endpoint"], endpoint);
    assert!(!stdout.contains("dummy-refresh-1"), "{stdout}");
    assert!(!stdout.contains("dummy-access-1"), "{stdout}");

    // The QR itself went to stderr, as UTF-8 block characters, with the SDK
    // payload in it. The token is part of the payload the user scans, so it
    // is expected there (and only there).
    assert!(
        stderr.contains('\u{2588}') || stderr.contains('\u{2580}') || stderr.contains('\u{2584}'),
        "the QR must be rendered on stderr: {stderr:?}"
    );
    // The block art is the QR itself; the payload is in it, not printed as
    // text, so the assertion is on the rendering only.

    // Tokens, endpoint, user code and terminal id are in the one SQLite file.
    let cred = credential(&db);
    assert_eq!(cred["access_token"], "dummy-access-1");
    assert_eq!(cred["refresh_token"], "dummy-refresh-1");
    assert_eq!(cred["endpoint"], endpoint);
    assert_eq!(cred["user_code"], "DummyCode1");
    assert_eq!(cred["terminal_id"], "dummy-terminal-1");
    assert_eq!(cred["uid"], "dummy-uid-1");
    assert!(cred["expire_time"].as_i64().unwrap() > 0);

    // The start carried the client id, the normalised user code and the
    // schema; the poll carried the client id and the same code.
    let starts: Vec<Request> = login_server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST")
        .collect();
    assert_eq!(starts.len(), 1);
    let query = starts[0].url.query().unwrap_or("");
    assert!(query.contains("clientid=HA_3y9q4ak7g4ephrvke"), "{query}");
    assert!(query.contains("usercode=DummyCode1"), "{query}");
    assert!(query.contains("schema=haauthorize"), "{query}");
    let polls: Vec<Request> = login_server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "GET")
        .collect();
    assert_eq!(polls.len(), 2, "one pending poll, then one that succeeds");
    assert!(polls[0]
        .url
        .query()
        .unwrap_or("")
        .contains("usercode=DummyCode1"));
    // No API request before the login completed; right after it, exactly one
    // homes call and one devices call per home (the fixture has one home),
    // which is the Local Key save the sign-in does on its own.
    let api_requests = api.received_requests().await.unwrap();
    let paths: Vec<&str> = api_requests.iter().map(|r| r.url.path()).collect();
    assert_eq!(
        paths,
        ["/v1.0/m/life/users/homes", "/v1.0/m/life/ha/home/devices"],
        "one device list after the sign-in, nothing before it"
    );
    assert!(
        !stderr.contains("fetching Local Keys failed"),
        "the key fetch must have succeeded: {stderr}"
    );
}

#[tokio::test]
async fn a_login_that_is_never_scanned_times_out_without_storing_anything() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1.0/m/life/home-assistant/qrcode/tokens"))
        .respond_with(envelope(qr_started()))
        .mount(&login_server)
        .await;
    Mock::given(method("GET"))
        .respond_with(envelope(qr_pending()))
        .mount(&login_server)
        .await;

    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.env("HOME_TUYA_POLL_ATTEMPTS", "2");
    cmd.args(["tuya", "login", "--user-code", "DummyCode1"]);
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 7, "{json}");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["backend"], "tuya");
    assert!(!stdout.contains("dummy-refresh-1"));
    let polls = login_server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "GET")
        .count();
    assert_eq!(polls, 2, "exactly the configured number of polls");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM credentials WHERE backend = 'tuya'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn login_without_a_user_code_is_a_usage_error() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api = MockServer::start().await;
    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "login"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
    assert!(login_server.received_requests().await.unwrap().is_empty());

    // `HOME_TUYA_USER_CODE` stands in when the flag is absent.
    mount_login(&login_server, &api.uri()).await;
    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.env("HOME_TUYA_USER_CODE", "DummyCode1");
    cmd.args(["tuya", "login"]);
    assert_eq!(run(&mut cmd).0, 0);
}

#[tokio::test]
async fn login_alone_saves_every_local_key() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api = MockServer::start().await;
    mount_login(&login_server, &api.uri()).await;
    let (one, two) = (device_one(), device_two());
    mount_devices_with(
        &api,
        vec![(one.clone(), "fake-key-a"), (two.clone(), "fake-key-t-1")],
    )
    .await;

    // Only the login runs: no `home tuya devices` command in between.
    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "login", "--user-code", "DummyCode1"]);
    let (code, json, stdout, stderr) = run_full(&mut cmd);
    assert_eq!(code, 0, "{json}; stderr: {stderr}");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["backend"], "tuya");
    assert!(
        !stdout.contains("fake-key-a") && !stdout.contains("fake-key-t-1"),
        "a key reached stdout: {stdout}"
    );

    let mut keys: Vec<(String, String)> = key_rows(&db).into_iter().map(|r| (r.0, r.1)).collect();
    keys.sort();
    let mut expected = vec![
        (one, "fake-key-a".to_string()),
        (two, "fake-key-t-1".to_string()),
    ];
    expected.sort();
    assert_eq!(keys, expected, "the sign-in must save every Local Key");
}

/// A device list that fails after a successful sign-in is reported, not
/// fatal: exit 0, the session stored, no keys, one URL-free note on stderr.
#[tokio::test]
async fn a_failed_device_list_leaves_the_login_signed_in() {
    for status in [404, 500] {
        let (_dir, db) = temp_db();
        let login_server = MockServer::start().await;
        let api = MockServer::start().await;
        mount_login(&login_server, &api.uri()).await;
        Mock::given(method("GET"))
            .and(path("/v1.0/m/life/users/homes"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&api)
            .await;

        let mut cmd = home(&db);
        point_at_tuya(&mut cmd, &login_server, &api);
        cmd.args(["tuya", "login", "--user-code", "DummyCode1"]);
        let (code, json, stdout, stderr) = run_full(&mut cmd);
        assert_eq!(code, 0, "{status}: {json}; stderr: {stderr}");
        assert_eq!(json["ok"], true);
        assert_eq!(json["data"]["backend"], "tuya", "{status}");
        // The session was saved before the device list was attempted.
        assert_eq!(credential(&db)["refresh_token"], "dummy-refresh-1");
        assert_eq!(credential(&db)["access_token"], "dummy-access-1");
        assert!(key_rows(&db).is_empty(), "{status}: no key could be saved");
        // The stderr note says what failed, without a token, URL or path.
        assert!(
            stderr.contains("signed in, but fetching Local Keys failed"),
            "{status}: {stderr}"
        );
        assert!(
            stderr.contains(&format!("HTTP {status}")),
            "{status}: {stderr}"
        );
        // stdout's `endpoint` is the fake host by contract; the tokens never
        // appear on either stream, and stderr carries no URL or path.
        for secret in ["dummy-access-1", "dummy-refresh-1"] {
            assert!(
                !stderr.contains(secret),
                "{status}: `{secret}` leaked to stderr: {stderr}"
            );
            assert!(!stdout.contains(secret), "{status}: `{secret}` in stdout");
        }
        let host = api.uri();
        for secret in [host.as_str(), "/v1.0/m/"] {
            assert!(
                !stderr.contains(secret),
                "{status}: `{secret}` leaked to stderr: {stderr}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Devices: metadata only, signed headers, Local Keys saved
// ---------------------------------------------------------------------------

#[tokio::test]
async fn devices_lists_id_name_online_category_and_backend_with_signed_headers() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api = MockServer::start().await;
    mount_login(&login_server, &api.uri()).await;
    login(&db, &login_server, &api);
    // The prompt mount is spent; a fresh server carries only the device list,
    // so no stray route can answer the homes call.
    let api = MockServer::start().await;
    mount_plain_devices(&api).await;

    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "devices"]);
    let (code, json, stdout, stderr) = run_full(&mut cmd);
    assert_eq!(code, 0, "{json}; stderr: {stderr}");
    assert_eq!(json["ok"], true);
    let devices = json["data"].as_array().expect("a device array");
    assert_eq!(devices.len(), 2);
    let mut ids: Vec<String> = devices
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort_unstable();
    let mut expected = fixture_device_ids();
    expected.sort_unstable();
    assert_eq!(ids, expected);
    for device in devices {
        assert!(device["name"].as_str().is_some_and(|n| !n.is_empty()));
        assert_eq!(device["online"], true);
        assert_eq!(device["category"], "cz");
        assert_eq!(device["backend"], "tuya");
        // Metadata only: the key never reaches stdout.
        assert!(
            device.get("local_key").is_none(),
            "devices must not print a local_key"
        );
    }
    // The redacted fixture's `<redacted>` keys are not empty strings, so the
    // save path accepted them; either way stdout has no key at all.
    assert!(!stdout.contains("local_key"));

    // The homes call was signed with the five X-* headers and the access
    // token from the stored session.
    let homes: Vec<Request> = api
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/v1.0/m/life/users/homes")
        .collect();
    assert_eq!(homes.len(), 1);
    let headers = &homes[0].headers;
    for name in [
        "x-appkey",
        "x-requestid",
        "x-sid",
        "x-time",
        "x-token",
        "x-sign",
    ] {
        assert!(headers.contains_key(name), "missing {name}: {headers:?}");
    }
    assert_eq!(headers["x-appkey"], "HA_3y9q4ak7g4ephrvke");
    assert_eq!(headers["x-token"], "dummy-access-1");
    assert_eq!(headers["x-sid"], "", "an empty sid is still sent");
    assert_eq!(headers["x-sign"].to_str().unwrap().len(), 64);
    assert!(headers["x-requestid"]
        .to_str()
        .unwrap()
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c == '-'));
    assert!(headers["x-time"]
        .to_str()
        .unwrap()
        .chars()
        .all(|c| c.is_ascii_digit()));

    // The device call carried the sealed `encdata` query with the home id.
    let device_calls: Vec<Request> = api
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/v1.0/m/life/ha/home/devices")
        .collect();
    assert_eq!(device_calls.len(), 1);
    let query = device_calls[0].url.query().unwrap_or("");
    assert!(query.starts_with("encdata="), "{query}");
    assert!(
        !query.contains("homeId"),
        "the home id travels sealed, not in the clear: {query}"
    );
}

#[tokio::test]
async fn devices_without_a_stored_session_is_reauth_exit_5() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api = MockServer::start().await;
    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "devices"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 5, "{json}");
    assert_eq!(json["error"]["code"], "reauth_required");
    assert_eq!(json["error"]["backend"], "tuya");
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn sign_invalid_and_1010_are_reauth_exit_5() {
    for body in [
        serde_json::json!({"success": false, "code": 1010, "msg": "token invalid"}),
        serde_json::json!({"success": false, "code": -9999999, "msg": "sign invalid"}),
    ] {
        let (_dir, db) = temp_db();
        let login_server = MockServer::start().await;
        let api_login = MockServer::start().await;
        mount_login(&login_server, &api_login.uri()).await;
        login(&db, &login_server, &api_login);
        let api = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(envelope(body.clone()))
            .mount(&api)
            .await;
        let mut cmd = home(&db);
        point_at_tuya(&mut cmd, &login_server, &api);
        cmd.args(["tuya", "devices"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 5, "{body}: {json}");
        assert_eq!(json["error"]["code"], "reauth_required", "{body}");
        assert_eq!(json["error"]["backend"], "tuya");
    }
}

#[tokio::test]
async fn another_upstream_error_is_exit_7() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api_login = MockServer::start().await;
    mount_login(&login_server, &api_login.uri()).await;
    login(&db, &login_server, &api_login);
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(envelope(
            serde_json::json!({"success": false, "code": 2008, "msg": "device not found"}),
        ))
        .mount(&api)
        .await;
    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "devices"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 7, "{json}");
    assert_eq!(json["error"]["code"], "upstream");
    assert_eq!(json["error"]["backend"], "tuya");
}

// ---------------------------------------------------------------------------
// Local Keys: first save, unchanged second sighting, change keeps old value
// ---------------------------------------------------------------------------

/// Serve `home-devices` with these keys for the two fake Devices.
async fn mount_devices_with(api: &MockServer, keys: Vec<(String, &'static str)>) {
    let homes: serde_json::Value = serde_json::from_str(HOMES).unwrap();
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/users/homes"))
        .respond_with(envelope(homes))
        .mount(api)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/home/devices"))
        .respond_with(move |_req: &Request| {
            let keys: Vec<(&str, &str)> =
                keys.iter().map(|(id, key)| (id.as_str(), *key)).collect();
            envelope(devices_with_keys(&keys))
        })
        .mount(api)
        .await;
}

fn key_rows(db: &Path) -> Vec<(String, String, i64, i64)> {
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT device_id, local_key, first_seen_at, changed_at
             FROM tuya_local_keys ORDER BY device_id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn history_rows(db: &Path, device: &str) -> Vec<(String, i64)> {
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT old_key, changed_at FROM tuya_local_key_history
             WHERE device_id = ?1 ORDER BY id",
        )
        .unwrap();
    let rows = stmt
        .query_map([device], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn devices(db: &Path, login: &MockServer, api: &MockServer) -> (i32, serde_json::Value) {
    let mut cmd = home(db);
    point_at_tuya(&mut cmd, login, api);
    cmd.args(["tuya", "devices"]);
    run(&mut cmd)
}

/// A few milliseconds past the next whole second, so `changed_at` (unix
/// seconds) is certain to move between two checks.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(1100));
}

#[tokio::test]
async fn a_local_key_is_saved_then_kept_then_changed_with_history() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api_login = MockServer::start().await;
    mount_login(&login_server, &api_login.uri()).await;
    login(&db, &login_server, &api_login);
    let (one, two) = (device_one(), device_two());

    // First sighting.
    let api = MockServer::start().await;
    mount_devices_with(
        &api,
        vec![(one.clone(), "fake-key-a"), (two.clone(), "fake-key-t-1")],
    )
    .await;
    let (code, json) = devices(&db, &login_server, &api);
    assert_eq!(code, 0, "{json}");
    let first = key_rows(&db);
    assert_eq!(first.len(), 2);
    let row1 = first.iter().find(|r| r.0 == one).unwrap();
    assert_eq!(row1.1, "fake-key-a");
    assert_eq!(row1.2, row1.3, "first save: both times are the same");
    assert!(row1.2 > 0);
    assert!(history_rows(&db, &one).is_empty());

    // Second sighting, same keys: no new rows, no history, changed_at moves.
    tick();
    let api = MockServer::start().await;
    mount_devices_with(
        &api,
        vec![(one.clone(), "fake-key-a"), (two.clone(), "fake-key-t-1")],
    )
    .await;
    assert_eq!(devices(&db, &login_server, &api).0, 0);
    let second = key_rows(&db);
    assert_eq!(second.len(), 2);
    let row2 = second.iter().find(|r| r.0 == one).unwrap();
    assert_eq!(row2.1, "fake-key-a");
    assert_eq!(row2.2, row1.2, "first_seen_at stays");
    assert!(row2.3 > row1.3, "changed_at moves on a re-observation");
    assert!(history_rows(&db, &one).is_empty());

    // A changed key: the new one current, the old one in history with a time.
    tick();
    let api = MockServer::start().await;
    mount_devices_with(
        &api,
        vec![(one.clone(), "fake-key-b"), (two.clone(), "fake-key-t-1")],
    )
    .await;
    assert_eq!(devices(&db, &login_server, &api).0, 0);
    let third = key_rows(&db);
    let row3 = third.iter().find(|r| r.0 == one).unwrap();
    assert_eq!(row3.1, "fake-key-b");
    assert_eq!(row3.2, row1.2, "first_seen_at survives a change");
    assert!(row3.3 >= row2.3);
    let history = history_rows(&db, &one);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].0, "fake-key-a");
    assert!(history[0].1 >= row2.3);
    assert!(history_rows(&db, &two).is_empty());
}

#[tokio::test]
async fn a_device_without_a_key_saves_nothing() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api_login = MockServer::start().await;
    mount_login(&login_server, &api_login.uri()).await;
    login(&db, &login_server, &api_login);
    let (one, two) = (device_one(), device_two());
    let api = MockServer::start().await;
    mount_devices_with(&api, vec![(one.clone(), "fake-key-a"), (two.clone(), "")]).await;
    let (code, json) = devices(&db, &login_server, &api);
    assert_eq!(code, 0, "{json}");
    let rows = key_rows(&db);
    assert_eq!(rows.len(), 1, "only the Device that reported a key");
    assert_eq!(rows[0].0, one);
}

#[tokio::test]
async fn keys_prints_every_saved_local_key_as_json() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api_login = MockServer::start().await;
    mount_login(&login_server, &api_login.uri()).await;
    login(&db, &login_server, &api_login);
    let (one, two) = (device_one(), device_two());
    let api = MockServer::start().await;
    mount_devices_with(
        &api,
        vec![(one.clone(), "fake-key-a"), (two.clone(), "fake-key-t-1")],
    )
    .await;
    assert_eq!(devices(&db, &login_server, &api).0, 0);

    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "keys"]);
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    let rows = json["data"].as_array().expect("a key array");
    assert_eq!(rows.len(), 2);
    // Stable Device-ID order, so the export is diffable.
    let mut ids: Vec<&str> = rows
        .iter()
        .map(|r| r["device_id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    let mut expected = vec![one.as_str(), two.as_str()];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    let by_id = |id: &str| {
        rows.iter()
            .find(|r| r["device_id"] == id)
            .unwrap_or_else(|| panic!("no row for {id}"))
            .clone()
    };
    assert_eq!(by_id(&one)["local_key"], "fake-key-a");
    assert_eq!(by_id(&two)["local_key"], "fake-key-t-1");
    for row in rows {
        assert!(row["first_seen_at"].as_i64().unwrap() > 0);
        assert!(row["changed_at"].as_i64().unwrap() > 0);
    }
    // This is the one command that prints keys, and it does so plainly.
    assert!(stdout.contains("fake-key-a"));
    // No token ever.
    assert!(!stdout.contains("dummy-access-1"));
    assert!(!stdout.contains("dummy-refresh-1"));
}

// ---------------------------------------------------------------------------
// Sealed answers: the real upstream AES-GCM-seals `result`
// ---------------------------------------------------------------------------

/// Seal an answer the way Tuya does, under the secret derived from the
/// request's own `X-requestId` and the session's refresh token, and wrap it in
/// the envelope. Written out here independently of the crate, so it proves
/// the client's `open` works against a real seal rather than the plain JSON
/// every other test answers with.
fn sealed_answer(req: &Request, plain: serde_json::Value, refresh_token: &str) -> ResponseTemplate {
    use aes_gcm::aead::{Aead, KeyInit};
    use base64::Engine;
    let rid = req.headers["x-requestid"].to_str().unwrap().to_string();
    let hash_key = {
        use md5::Digest;
        let mut md5 = md5::Md5::new();
        md5.update(rid.as_bytes());
        md5.update(refresh_token.as_bytes());
        md5.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let secret = {
        use hmac::{Hmac, Mac};
        let mut mac =
            <Hmac<sha2::Sha256> as hmac::digest::KeyInit>::new_from_slice(rid.as_bytes()).unwrap();
        mac.update(hash_key.as_bytes());
        let hex: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        hex[..16].to_string()
    };
    let nonce = b"ABCDEFGHJKMN";
    let cipher = aes_gcm::Aes128Gcm::new_from_slice(secret.as_bytes()).unwrap();
    let nonce_arr = <aes_gcm::Nonce<aes_gcm::aead::consts::U12>>::try_from(&nonce[..]).unwrap();
    let sealed = cipher
        .encrypt(&nonce_arr, plain.to_string().as_bytes())
        .unwrap();
    let b64 = base64::engine::general_purpose::STANDARD;
    let wire = format!("{}{}", b64.encode(nonce), b64.encode(sealed));
    envelope(serde_json::json!({
        "t": 1794759259078i64,
        "sign": "00000000000000000000000000000000",
        "tid": "dummy-tid",
        "success": true,
        "result": wire,
    }))
}

#[tokio::test]
async fn a_sealed_device_list_opens_and_lists_the_same_devices() {
    let (_dir, db) = temp_db();
    let login_server = MockServer::start().await;
    let api_login = MockServer::start().await;
    mount_login(&login_server, &api_login.uri()).await;
    login(&db, &login_server, &api_login);

    let api = MockServer::start().await;
    let homes: serde_json::Value = serde_json::from_str(HOMES).unwrap();
    let devices: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
    let sealed_homes = homes["result"].clone();
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/users/homes"))
        .respond_with(move |req: &Request| {
            sealed_answer(req, sealed_homes.clone(), "dummy-refresh-1")
        })
        .mount(&api)
        .await;
    let sealed_devices = devices["result"].clone();
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/home/devices"))
        .respond_with(move |req: &Request| {
            sealed_answer(req, sealed_devices.clone(), "dummy-refresh-1")
        })
        .mount(&api)
        .await;

    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "devices"]);
    let (code, json, stdout, stderr) = run_full(&mut cmd);
    assert_eq!(code, 0, "{json}; stderr: {stderr}");
    let list = json["data"].as_array().expect("a device array");
    assert_eq!(list.len(), 2);
    // The names come out of the sealed fixture, not out of a constant here.
    let names: Vec<&str> = list.iter().map(|d| d["name"].as_str().unwrap()).collect();
    let fixture_names: Vec<String> = devices["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    for name in &fixture_names {
        assert!(names.contains(&name.as_str()), "missing {name}: {names:?}");
    }
    assert!(!stdout.contains("local_key"));
}

// ---------------------------------------------------------------------------
// A failed refresh must not print the refresh token
// ---------------------------------------------------------------------------

/// The refresh path puts the refresh token in the URL's path, and reqwest
/// appends the URL to a `send` error. A session that has to refresh against a
/// closed loopback port must fail as `upstream` with no token in sight.
#[tokio::test]
async fn a_failed_refresh_reports_upstream_without_the_refresh_token_or_token_path() {
    let (_dir, db) = temp_db();
    expired_session(&db);

    // A port nothing listens on: bind, note it, drop it, so a connect fails
    // at once instead of hanging.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let dead = format!("http://127.0.0.1:{port}");

    let login_server = MockServer::start().await;
    let mut cmd = home(&db);
    cmd.env("HOME_TUYA_LOGIN_URL", login_server.uri());
    cmd.env("HOME_TUYA_ENDPOINT", &dead);
    cmd.env("HOME_TUYA_POLL_MS", "0");
    cmd.args(["tuya", "devices"]);
    let (code, json, stdout, stderr) = run_full(&mut cmd);
    assert_eq!(code, 7, "{json}; stderr: {stderr}");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "upstream");
    assert_eq!(json["error"]["backend"], "tuya");
    let message = json["error"]["message"].as_str().unwrap();
    // The error still says what happened, with the connect cause appended.
    assert!(
        message.contains("error sending request"),
        "the failure must be described: {message}"
    );
    assert!(
        message.contains("Connection refused"),
        "the cause must be described: {message}"
    );
    // No URL at all, so no path, which carries the refresh token.
    assert!(
        !message.contains("/v1.0/m/token/"),
        "the token path leaked: {message}"
    );
    assert!(!stdout.contains("dummy-refresh-old"), "{stdout}");
    assert!(!stdout.contains("/v1.0/m/token/"), "{stdout}");
    // The session was not rewritten: the old pair is still what is stored.
    assert_eq!(stored_refresh_token(&db), "dummy-refresh-old");
    assert_eq!(credential(&db)["access_token"], "dummy-access-old");
}

// ---------------------------------------------------------------------------
// Redirects: same origin is followed, another origin is refused
// ---------------------------------------------------------------------------

/// The fake upstream is reached over loopback HTTP, which the client allows
/// only for that base; a 307 to the same origin is followed, so the login
/// flow keeps working through one.
#[tokio::test]
async fn a_redirect_within_the_same_origin_is_followed() {
    let (_dir, db) = temp_db();
    expired_session(&db);

    // The token refresh lands on `/moved`, a same-origin 307 to the real
    // token answer; only a followed redirect reaches `dummy-access-new`.
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/token/dummy-refresh-old"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/v1.0/m/token/moved", api.uri())),
        )
        .mount(&api)
        .await;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Mock::given(method("GET"))
        .and(path("/v1.0/m/token/moved"))
        .respond_with(envelope(serde_json::json!({
            "t": now_ms,
            "success": true,
            "result": {
                "accessToken": "dummy-access-new",
                "refreshToken": "dummy-refresh-new",
                "expireTime": 7200,
                "uid": "dummy-uid-1",
            },
        })))
        .mount(&api)
        .await;
    let homes: serde_json::Value = serde_json::from_str(HOMES).unwrap();
    let devices: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/users/homes"))
        .respond_with(envelope(homes))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/home/devices"))
        .respond_with(envelope(devices))
        .mount(&api)
        .await;

    let login_server = MockServer::start().await;
    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "devices"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    let requests = api.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .any(|r| r.url.path() == "/v1.0/m/token/moved"),
        "the redirect was not followed"
    );
    assert_eq!(stored_refresh_token(&db), "dummy-refresh-new");
}

/// A 307 to another host would re-send the `X-token` header and, on the
/// refresh path, hand the token-bearing URL to a stranger. The hop is
/// refused and the second server sees nothing at all.
#[tokio::test]
async fn a_redirect_to_another_host_is_refused_and_the_other_host_is_not_contacted() {
    let (_dir, db) = temp_db();
    expired_session(&db);

    let elsewhere = MockServer::start().await;
    let first = redirecting_to(&format!("{}/v1.0/m/token/stolen", elsewhere.uri())).await;

    let login_server = MockServer::start().await;
    let mut cmd = home(&db);
    cmd.env("HOME_TUYA_LOGIN_URL", login_server.uri());
    cmd.env("HOME_TUYA_ENDPOINT", first.uri());
    cmd.args(["tuya", "devices"]);
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 7, "{json}");
    assert_eq!(json["error"]["code"], "upstream");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("another host"),
        "the refusal must say why: {message}"
    );
    assert!(
        !message.contains("/v1.0/m/token/"),
        "the token path leaked: {message}"
    );
    assert!(!stdout.contains("dummy-refresh-old"), "{stdout}");
    assert!(
        elsewhere.received_requests().await.unwrap().is_empty(),
        "the second host must never be contacted"
    );
    assert_eq!(
        first.received_requests().await.unwrap().len(),
        1,
        "the first host was hit once"
    );
    // Nothing rotated: the session still has its old tokens.
    assert_eq!(stored_refresh_token(&db), "dummy-refresh-old");
}

/// A database with a Tuya session whose access token has already expired, so
/// the next command refreshes instead of logging in.
fn expired_session(db: &Path) {
    let store = home_store::Store::open(db).unwrap();
    let now = home_store::store::now_unix();
    let json = serde_json::json!({
        "access_token": "dummy-access-old",
        "refresh_token": "dummy-refresh-old",
        "endpoint": "https://apigw.tuyaus.com",
        "user_code": "DummyCode1",
        "terminal_id": "dummy-terminal-1",
        "uid": "dummy-uid-1",
        "expire_time": now - 10,
    });
    store.set_credential("tuya", &json.to_string()).unwrap();
}

fn stored_refresh_token(db: &Path) -> String {
    credential(db)["refresh_token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_rotated_refresh_token_is_persisted_before_the_next_request_uses_it() {
    let (_dir, db) = temp_db();
    expired_session(&db);
    assert_eq!(stored_refresh_token(&db), "dummy-refresh-old");

    let login_server = MockServer::start().await;
    let api = MockServer::start().await;
    let homes: serde_json::Value = serde_json::from_str(HOMES).unwrap();
    let devices: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();

    // The refresh call answers with a new pair. It is the only request whose
    // path carries the old refresh token.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Mock::given(method("GET"))
        .and(path("/v1.0/m/token/dummy-refresh-old"))
        .respond_with(envelope(serde_json::json!({
            "t": now_ms,
            "success": true,
            "result": {
                "accessToken": "dummy-access-new",
                "refreshToken": "dummy-refresh-new",
                "expireTime": 7200,
                "uid": "dummy-uid-1",
            },
        })))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/users/homes"))
        .respond_with(envelope(homes))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/home/devices"))
        .respond_with(envelope(devices))
        .mount(&api)
        .await;

    let mut cmd = home(&db);
    point_at_tuya(&mut cmd, &login_server, &api);
    cmd.args(["tuya", "devices"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");

    // After the command both tokens in SQLite are the new ones.
    let cred = credential(&db);
    assert_eq!(cred["access_token"], "dummy-access-new");
    assert_eq!(cred["refresh_token"], "dummy-refresh-new");

    let requests = api.received_requests().await.unwrap();
    let refresh_calls: Vec<&Request> = requests
        .iter()
        .filter(|r| r.url.path().starts_with("/v1.0/m/token/"))
        .collect();
    assert_eq!(refresh_calls.len(), 1);
    assert_eq!(
        refresh_calls[0].url.path(),
        "/v1.0/m/token/dummy-refresh-old"
    );
    let data_calls: Vec<&Request> = requests
        .iter()
        .filter(|r| !r.url.path().starts_with("/v1.0/m/token/"))
        .collect();
    assert!(!data_calls.is_empty());
    for req in data_calls {
        assert_eq!(
            req.headers["x-token"],
            "dummy-access-new",
            "{} signed with the old access token",
            req.url.path()
        );
    }
}
