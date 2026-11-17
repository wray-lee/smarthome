//! Black-box tests for ticket 05: Tuya control and Telemetry.
//!
//! The `home` binary runs against a fake Tuya upstream (wiremock) seeded with
//! the ticket-01 redacted fixtures. No test contains a real Device ID, token
//! or Local Key; the Home fake Device is index 1 of the fixtures
//! (`tuyaFakeHome000000001`), and synthetic devices are made up here.
//!

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn home(db: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    cmd.arg("--db").arg(db);
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

fn envelope(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(body.to_string())
}

const HOMES: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/homes.json");
const HOME_DEVICES: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/home-devices.json");
const SPEC_1: &str = include_str!("../.scratch/smarthome-v1/fixtures/tuya/spec-1.json");
const DETAIL_FALSE: &str =
    include_str!("../.scratch/smarthome-v1/fixtures/tuya/detail-after-false.json");

/// The Home fake Device ID, read from the fixture instead of spelled out.
fn home_id() -> String {
    fixture_device_ids()[1].clone()
}

fn fixture_device_ids() -> Vec<String> {
    let list: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
    list["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_string())
        .collect()
}

fn seed_session(db: &Path, expire_offset: i64) {
    let now = home_store::store::now_unix();
    let store = home_store::Store::open(db).unwrap();
    let json = serde_json::json!({
        "access_token": "dummy-access-1",
        "refresh_token": "dummy-refresh-1",
        "endpoint": "https://apigw.tuyaus.com",
        "user_code": "DummyCode1",
        "terminal_id": "dummy-terminal-1",
        "uid": "dummy-uid-1",
        "expire_time": now + expire_offset,
    });
    store.set_credential("tuya", &json.to_string()).unwrap();
}

fn point_at(cmd: &mut Command, api: &MockServer) {
    cmd.env("HOME_TUYA_LOGIN_URL", api.uri());
    cmd.env("HOME_TUYA_ENDPOINT", api.uri());
}

/// Mount homes + device list (PLAIN, unsealed) with the fixture devices.
async fn mount_devices(api: &MockServer, devices: serde_json::Value) {
    let homes: serde_json::Value = serde_json::from_str(HOMES).unwrap();
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/users/homes"))
        .respond_with(envelope(homes))
        .mount(api)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/home/devices"))
        .respond_with(envelope(devices))
        .mount(api)
        .await;
}

/// Mount the plain fixture devices.
async fn mount_fixture_devices(api: &MockServer) {
    let devices: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
    mount_devices(api, devices).await;
}

/// Mount detail + spec for the Home fixture device.
async fn mount_get(api: &MockServer, detail: serde_json::Value, spec: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/devices/detail"))
        .respond_with(envelope(detail))
        .mount(api)
        .await;
    let id = home_id();
    Mock::given(method("GET"))
        .and(path(format!("/v1.1/m/life/{id}/specifications")))
        .respond_with(envelope(spec))
        .mount(api)
        .await;
}

/// Mount the commands endpoint with a fixed envelope.
async fn mount_command_fixed(api: &MockServer, id: &str, envelope_json: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path(format!("/v1.1/m/thing/{id}/commands")))
        .respond_with(envelope(envelope_json))
        .mount(api)
        .await;
}

fn command_ok() -> serde_json::Value {
    serde_json::json!({"t": 1794759267782i64, "success": true, "result": true})
}

fn command_fail(code: i64, msg: &str) -> serde_json::Value {
    serde_json::json!({"success": false, "code": code, "msg": msg})
}

/// Decrypt a sealed command body from a recorded request: derive the secret
/// from its `x-requestid` header and the known refresh token, AES-128-GCM
/// decrypt `body["encdata"]`, and return the parsed commands object.
fn decrypt_commands(req: &Request, refresh_token: &str) -> serde_json::Value {
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
    let body: serde_json::Value = serde_json::from_slice(&req.body).expect("a JSON body");
    let encdata = body["encdata"].as_str().expect("encdata in the body");
    let b64 = base64::engine::general_purpose::STANDARD;
    let raw = b64.decode(encdata).expect("base64 encdata");
    assert!(raw.len() > 12, "sealed body too short");
    let (nonce, sealed) = raw.split_at(12);
    let cipher = aes_gcm::Aes128Gcm::new_from_slice(secret.as_bytes()).unwrap();
    let nonce_arr = <aes_gcm::Nonce<aes_gcm::aead::consts::U12>>::try_from(nonce).unwrap();
    let plain = cipher.decrypt(&nonce_arr, sealed).expect("AES-GCM decrypt");
    serde_json::from_slice(&plain).expect("a JSON plaintext")
}

async fn requests_for(api: &MockServer, substring: &str) -> Vec<Request> {
    api.received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().contains(substring))
        .collect()
}

// ---------------------------------------------------------------------------
// 1. get prints every data point with Telemetry and calibration
#[tokio::test]
async fn get_prints_all_data_points_with_four_telemetry_and_four_calibration() {
    let (_dir, db) = temp_db();
    seed_session(&db, 7200);
    let api = MockServer::start().await;
    mount_fixture_devices(&api).await;
    let detail: serde_json::Value = serde_json::from_str(DETAIL_FALSE).unwrap();
    let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
    mount_get(&api, detail, spec).await;

    let id = home_id();
    let mut cmd = home(&db);
    point_at(&mut cmd, &api);
    cmd.args(["tuya", "get", &id]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["id"], id);
    assert_eq!(json["data"]["name"], "Home");
    assert_eq!(json["data"]["backend"], "tuya");
    assert_eq!(json["data"]["online"], true);

    let detail_points = {
        let d: serde_json::Value = serde_json::from_str(DETAIL_FALSE).unwrap();
        d["result"][0]["status"].as_array().unwrap().clone()
    };
    let points = json["data"]["data_points"].as_array().unwrap();
    assert_eq!(points.len(), detail_points.len());
    let codes: Vec<&str> = points.iter().map(|p| p["code"].as_str().unwrap()).collect();
    for want in ["switch_1", "fault", "relay_status"] {
        assert!(codes.contains(&want), "missing {want}: {codes:?}");
    }

    let telemetry = json["data"]["telemetry"].as_object().unwrap();
    let keys: Vec<&String> = telemetry.keys().collect();
    assert_eq!(keys.len(), 4, "telemetry: {keys:?}");
    for k in ["cur_current", "cur_power", "cur_voltage", "add_ele"] {
        assert!(telemetry.contains_key(k), "missing telemetry {k}: {keys:?}");
    }
    assert_eq!(telemetry["cur_voltage"]["raw"], 1035);
    assert_eq!(telemetry["cur_voltage"]["scale"], 1);
    assert_eq!(telemetry["cur_voltage"]["value"], 103.5);
    assert_eq!(telemetry["cur_voltage"]["unit"], "V");
    assert_eq!(telemetry["add_ele"]["raw"], 2);
    assert_eq!(telemetry["add_ele"]["scale"], 3);
    assert_eq!(telemetry["add_ele"]["value"], 0.002);
    assert_eq!(telemetry["add_ele"]["unit"], "kwh");
    assert_eq!(telemetry["cur_current"]["raw"], 0);
    assert_eq!(telemetry["cur_current"]["scale"], 0);
    assert_eq!(telemetry["cur_current"]["value"], 0);
    assert_eq!(telemetry["cur_current"]["unit"], "mA");

    let calibration = json["data"]["calibration"].as_object().unwrap();
    assert_eq!(calibration.len(), 4);
    for k in [
        "voltage_coe",
        "electric_coe",
        "power_coe",
        "electricity_coe",
    ] {
        assert!(calibration.contains_key(k), "missing calibration {k}");
        assert!(
            !telemetry.contains_key(k),
            "calibration {k} must not be in telemetry"
        );
    }
    assert_eq!(calibration["voltage_coe"], 560);
}

// ---------------------------------------------------------------------------
// 2. A declared scale wins over a hard-coded one
#[tokio::test]
async fn get_uses_the_declared_scale_not_a_hard_coded_one() {
    let (_dir, db) = temp_db();
    seed_session(&db, 7200);
    let api = MockServer::start().await;

    let skew_id = "tuyaFakeSkew000000003";
    let mut devices: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
    let mut skew = devices["result"][1].clone();
    skew["id"] = json!(skew_id);
    skew["name"] = json!("Skew");
    skew["online"] = json!(true);
    devices["result"].as_array_mut().unwrap().push(skew);
    mount_devices(&api, devices).await;

    let mut spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
    for entry in spec["result"]["status"].as_array_mut().unwrap() {
        if entry["code"] == "cur_power" {
            entry["values"] = json!(r#"{"unit":"W","min":0,"max":200000,"scale":2,"step":1}"#);
        }
    }
    Mock::given(method("GET"))
        .and(path(format!("/v1.1/m/life/{skew_id}/specifications")))
        .respond_with(envelope(spec))
        .mount(&api)
        .await;
    let mut detail: serde_json::Value = serde_json::from_str(DETAIL_FALSE).unwrap();
    detail["result"][0]["id"] = json!(skew_id);
    detail["result"][0]["name"] = json!("Skew");
    for point in detail["result"][0]["status"].as_array_mut().unwrap() {
        if point["code"] == "cur_power" {
            point["value"] = json!(2484);
        }
    }
    Mock::given(method("GET"))
        .and(path("/v1.0/m/life/ha/devices/detail"))
        .respond_with(envelope(detail))
        .mount(&api)
        .await;

    let mut cmd = home(&db);
    point_at(&mut cmd, &api);
    cmd.args(["tuya", "get", skew_id]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    let telemetry = &json["data"]["telemetry"];
    assert_eq!(telemetry["cur_power"]["raw"], 2484);
    assert_eq!(telemetry["cur_power"]["scale"], 2);
    assert_eq!(telemetry["cur_power"]["value"], 24.84);
    assert_eq!(telemetry["cur_power"]["unit"], "W");
}

// ---------------------------------------------------------------------------
// 3. Resolution
#[tokio::test]
async fn resolution_exact_id_bare_name_case_and_ambiguous() {
    let (_dir, db) = temp_db();
    seed_session(&db, 7200);

    // exact ID
    {
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let detail: serde_json::Value = serde_json::from_str(DETAIL_FALSE).unwrap();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        mount_get(&api, detail, spec).await;
        let id = home_id();
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "get", &id]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        assert_eq!(json["data"]["id"], id);
    }

    // bare name, case-insensitive: "home" -> Home
    {
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let detail: serde_json::Value = serde_json::from_str(DETAIL_FALSE).unwrap();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        mount_get(&api, detail, spec).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "get", "home"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        assert_eq!(json["data"]["id"], home_id());
    }

    // two devices sharing a name -> exit 3 with two candidates
    {
        let api = MockServer::start().await;
        let mut devices: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
        let mut extra = devices["result"][1].clone();
        extra["id"] = json!("tuyaFakeExtra000000004");
        extra["name"] = json!("Home");
        devices["result"].as_array_mut().unwrap().push(extra);
        mount_devices(&api, devices).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "get", "Home"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 3, "{json}");
        assert_eq!(json["error"]["code"], "ambiguous");
        assert_eq!(json["error"]["candidates"].as_array().unwrap().len(), 2);
    }

    // a third device whose name is exactly the Home id does not hinder the exact id
    {
        let api = MockServer::start().await;
        let mut devices: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
        let hid = home_id();
        let mut imposter = devices["result"][1].clone();
        imposter["id"] = json!("tuyaFakeImposter000000005");
        imposter["name"] = json!(hid);
        devices["result"].as_array_mut().unwrap().push(imposter);
        mount_devices(&api, devices).await;
        let detail: serde_json::Value = serde_json::from_str(DETAIL_FALSE).unwrap();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        mount_get(&api, detail, spec).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "get", &hid]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        assert_eq!(json["data"]["id"], hid);
    }
}

// ---------------------------------------------------------------------------
// 4. on/off
#[tokio::test]
async fn on_and_off_send_switch_1_with_exact_command_bodies() {
    for (verb, want) in [("off", false), ("on", true)] {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let id = home_id();
        mount_command_fixed(&api, &id, command_ok()).await;

        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", verb, &id]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        assert_eq!(json["data"]["changed"]["code"], "switch_1");
        assert_eq!(json["data"]["changed"]["value"], want);
        let posts = requests_for(&api, "/commands").await;
        assert_eq!(posts.len(), 1);
        let commands = decrypt_commands(&posts[0], "dummy-refresh-1");
        assert_eq!(
            commands,
            serde_json::json!({"commands":[{"code":"switch_1","value":want}]})
        );
    }
}

// ---------------------------------------------------------------------------
// 5. set
#[tokio::test]
async fn set_converts_by_the_declared_type_and_rejects_bad_values() {
    // child_lock true -> Boolean true
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let id = home_id();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/v1.1/m/life/{id}/specifications")))
            .respond_with(envelope(spec))
            .mount(&api)
            .await;
        mount_command_fixed(&api, &id, command_ok()).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "set", &id, "child_lock", "true"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        let posts = requests_for(&api, "/commands").await;
        assert_eq!(posts.len(), 1);
        let commands = decrypt_commands(&posts[0], "dummy-refresh-1");
        assert_eq!(
            commands,
            serde_json::json!({"commands":[{"code":"child_lock","value":true}]})
        );
    }

    // countdown_1 30 -> number 30 (raw, no scale reversal)
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let id = home_id();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/v1.1/m/life/{id}/specifications")))
            .respond_with(envelope(spec))
            .mount(&api)
            .await;
        mount_command_fixed(&api, &id, command_ok()).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "set", &id, "countdown_1", "30"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        let posts = requests_for(&api, "/commands").await;
        assert_eq!(posts.len(), 1);
        let commands = decrypt_commands(&posts[0], "dummy-refresh-1");
        assert_eq!(
            commands,
            serde_json::json!({"commands":[{"code":"countdown_1","value":30}]})
        );
    }

    // light_mode pos -> string "pos"
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let id = home_id();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/v1.1/m/life/{id}/specifications")))
            .respond_with(envelope(spec))
            .mount(&api)
            .await;
        mount_command_fixed(&api, &id, command_ok()).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "set", &id, "light_mode", "pos"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        let posts = requests_for(&api, "/commands").await;
        assert_eq!(posts.len(), 1);
        let commands = decrypt_commands(&posts[0], "dummy-refresh-1");
        assert_eq!(
            commands,
            serde_json::json!({"commands":[{"code":"light_mode","value":"pos"}]})
        );
    }

    // unknown code -> exit 2, 0 POST
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let id = home_id();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/v1.1/m/life/{id}/specifications")))
            .respond_with(envelope(spec))
            .mount(&api)
            .await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "set", &id, "no_such_code", "1"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 2, "{json}");
        assert_eq!(json["error"]["code"], "usage");
        assert!(requests_for(&api, "/commands").await.is_empty());
    }

    // Boolean given "yes" -> exit 2, 0 POST
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let id = home_id();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/v1.1/m/life/{id}/specifications")))
            .respond_with(envelope(spec))
            .mount(&api)
            .await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "set", &id, "child_lock", "yes"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 2, "{json}");
        assert!(requests_for(&api, "/commands").await.is_empty());
    }
}

// ---------------------------------------------------------------------------
// 6. 1108 retries
#[tokio::test]
async fn transient_1108_is_retried_twice_then_succeeds() {
    let (_dir, db) = temp_db();
    seed_session(&db, 7200);
    let api = MockServer::start().await;
    mount_fixture_devices(&api).await;
    let id = home_id();
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attempts_in = attempts.clone();
    Mock::given(method("POST"))
        .and(path(format!("/v1.1/m/thing/{id}/commands")))
        .respond_with(move |_req: &Request| {
            let n = attempts_in.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 {
                envelope(command_fail(1108, "uri path invalid"))
            } else {
                envelope(command_ok())
            }
        })
        .mount(&api)
        .await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &api);
    cmd.args(["tuya", "off", &id]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[tokio::test]
async fn transient_1108_three_times_is_exit_7() {
    let (_dir, db) = temp_db();
    seed_session(&db, 7200);
    let api = MockServer::start().await;
    mount_fixture_devices(&api).await;
    let id = home_id();
    Mock::given(method("POST"))
        .and(path(format!("/v1.1/m/thing/{id}/commands")))
        .respond_with(envelope(command_fail(1108, "uri path invalid")))
        .mount(&api)
        .await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &api);
    cmd.args(["tuya", "off", &id]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 7, "{json}");
    assert_eq!(requests_for(&api, "/commands").await.len(), 3);
}

// ---------------------------------------------------------------------------
// 7. reauth on commands
#[tokio::test]
async fn sign_invalid_and_1010_on_commands_are_exit_5() {
    for envelope_json in [
        command_fail(1010, "token invalid"),
        command_fail(-9999999, "sign invalid"),
    ] {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let id = home_id();
        mount_command_fixed(&api, &id, envelope_json).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "off", &id]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 5, "{json}");
        assert_eq!(json["error"]["code"], "reauth_required");
        assert_eq!(json["error"]["backend"], "tuya");
        assert_eq!(requests_for(&api, "/commands").await.len(), 1);
    }
}

// ---------------------------------------------------------------------------
// 8. Repeated commands are not dropped
#[tokio::test]
async fn two_offs_in_a_row_send_two_commands() {
    let (_dir, db) = temp_db();
    seed_session(&db, 7200);
    let api = MockServer::start().await;
    mount_fixture_devices(&api).await;
    let id = home_id();
    mount_command_fixed(&api, &id, command_ok()).await;

    for _ in 0..2 {
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "off", &id]);
        assert_eq!(run(&mut cmd).0, 0);
    }
    let posts = requests_for(&api, "/commands").await;
    assert_eq!(posts.len(), 2);
    for post in &posts {
        let commands = decrypt_commands(post, "dummy-refresh-1");
        assert_eq!(
            commands,
            serde_json::json!({"commands":[{"code":"switch_1","value":false}]})
        );
    }
}

// ---------------------------------------------------------------------------
// 9. offline
#[tokio::test]
async fn offline_device_is_exit_4_without_spec_or_command_calls() {
    for verb in ["get", "off"] {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        let api = MockServer::start().await;
        let mut devices: serde_json::Value = serde_json::from_str(HOME_DEVICES).unwrap();
        for device in devices["result"].as_array_mut().unwrap() {
            if device["id"] == home_id() {
                device["online"] = json!(false);
            }
        }
        mount_devices(&api, devices).await;
        let id = home_id();
        // Any spec/detail/commands request would be answered, but none must arrive.
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/v1.1/m/life/{id}/specifications")))
            .respond_with(envelope(spec))
            .mount(&api)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1.0/m/life/ha/devices/detail"))
            .respond_with(envelope(serde_json::from_str(DETAIL_FALSE).unwrap()))
            .mount(&api)
            .await;

        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", verb, &id]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 4, "{verb}: {json}");
        assert_eq!(json["error"]["code"], "device_offline");
        assert_eq!(json["error"]["backend"], "tuya");
        assert!(requests_for(&api, "/commands").await.is_empty());
        assert!(requests_for(&api, "/specifications").await.is_empty());
        assert!(requests_for(&api, "/detail").await.is_empty());
    }
}

// ---------------------------------------------------------------------------
// 10. protected
#[tokio::test]
async fn a_protected_device_is_refused_before_any_write() {
    let id = home_id();

    // By ID: zero upstream requests at all.
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        home_store::Store::open(&db)
            .unwrap()
            .protect("tuya", &id)
            .unwrap();
        let api = MockServer::start().await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "off", &id]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 6, "{json}");
        assert_eq!(json["error"]["code"], "protected");
        assert_eq!(json["error"]["backend"], "tuya");
        assert!(json["error"]["message"].as_str().unwrap().contains(&id));
        assert!(
            api.received_requests().await.unwrap().is_empty(),
            "no upstream request may be made"
        );
    }

    // By name: only homes/devices GETs, zero POSTs.
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        home_store::Store::open(&db)
            .unwrap()
            .protect("tuya", &id)
            .unwrap();
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "off", "Home"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 6, "{json}");
        assert_eq!(json["error"]["code"], "protected");
        let all = api.received_requests().await.unwrap();
        assert_eq!(all.len(), 2, "only homes and devices calls: {all:?}");
        for req in &all {
            assert_eq!(req.method.as_str(), "GET");
        }
    }

    // set on the protected device: exit 6, zero spec/POST.
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        home_store::Store::open(&db)
            .unwrap()
            .protect("tuya", &id)
            .unwrap();
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "set", "Home", "switch_1", "false"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 6, "{json}");
        assert!(requests_for(&api, "/commands").await.is_empty());
        assert!(requests_for(&api, "/specifications").await.is_empty());
    }

    // get on the protected device still works and Telemetry is normal.
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        home_store::Store::open(&db)
            .unwrap()
            .protect("tuya", &id)
            .unwrap();
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let detail: serde_json::Value = serde_json::from_str(DETAIL_FALSE).unwrap();
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        mount_get(&api, detail, spec).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "get", &id]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        assert_eq!(json["data"]["telemetry"]["cur_voltage"]["value"], 103.5);
    }
}

#[tokio::test]
async fn force_writes_a_protected_device_and_the_protected_list_is_unchanged() {
    let id = home_id();
    for (verb, want) in [("on", true), ("off", false)] {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        home_store::Store::open(&db)
            .unwrap()
            .protect("tuya", &id)
            .unwrap();
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        mount_command_fixed(&api, &id, command_ok()).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", verb, &id, "--force"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{verb}: {json}");
        assert_eq!(json["data"]["changed"]["value"], want);
        assert_eq!(requests_for(&api, "/commands").await.len(), 1);
        // The protection is not consumed: it is still listed.
        let mut list = home(&db);
        list.args(["protect", "--list"]);
        let (_, json) = run(&mut list);
        let rows = json["data"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["device_id"], id);
    }

    // `set --force` writes too, and the following no-force call is still 403.
    {
        let (_dir, db) = temp_db();
        seed_session(&db, 7200);
        home_store::Store::open(&db)
            .unwrap()
            .protect("tuya", &id)
            .unwrap();
        let api = MockServer::start().await;
        mount_fixture_devices(&api).await;
        let spec: serde_json::Value = serde_json::from_str(SPEC_1).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/v1.1/m/life/{id}/specifications")))
            .respond_with(envelope(spec))
            .mount(&api)
            .await;
        mount_command_fixed(&api, &id, command_ok()).await;
        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "set", &id, "child_lock", "true", "--force"]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 0, "{json}");
        assert_eq!(requests_for(&api, "/commands").await.len(), 1);

        let mut cmd = home(&db);
        point_at(&mut cmd, &api);
        cmd.args(["tuya", "off", &id]);
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 6, "{json}");
        assert_eq!(json["error"]["code"], "protected");
        assert_eq!(
            requests_for(&api, "/commands").await.len(),
            1,
            "the no-force call sends nothing"
        );
    }
}

// ---------------------------------------------------------------------------
// 11. Missing parameters
#[tokio::test]
async fn missing_arguments_are_usage() {
    let (_dir, db) = temp_db();
    seed_session(&db, 7200);
    let api = MockServer::start().await;
    let id = home_id();

    let mut cmd = home(&db);
    point_at(&mut cmd, &api);
    cmd.args(["tuya", "set", &id, "child_lock"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 2, "{json}");

    let mut cmd = home(&db);
    point_at(&mut cmd, &api);
    cmd.args(["tuya", "on"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 2, "{json}");
}

// ---------------------------------------------------------------------------
// 12. No session
#[tokio::test]
async fn get_without_a_session_is_reauth_exit_5() {
    let (_dir, db) = temp_db();
    let api = MockServer::start().await;
    let id = home_id();
    let mut cmd = home(&db);
    point_at(&mut cmd, &api);
    cmd.args(["tuya", "get", &id]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 5, "{json}");
    assert_eq!(json["error"]["code"], "reauth_required");
    assert_eq!(json["error"]["backend"], "tuya");
    assert!(api.received_requests().await.unwrap().is_empty());
}
