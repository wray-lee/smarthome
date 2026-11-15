//! Black-box tests for ticket 03: `home gh get` and `home gh set` against a
//! fake foyer.
//!
//! Every test runs the real `home` binary, points both upstreams at wiremock
//! servers, and gives the process its own temporary SQLite file — the same
//! harness as `tests/cli.rs`. Device IDs are the fake ones from
//! `.scratch/smarthome-v1/fixtures/foyer/` or dummies built in that style;
//! no real account or Device appears here.
//!
//! The `gh get` / `gh set` subcommands are wired by the integration ticket;
//! until then these tests fail at argument parsing, by design.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The two fake IDs the ticket-01 capture uses.
const LAMP_A: &str = "00000000-0000-4000-8000-00000000h0me";
const LAMP_B: &str = "00000000-0000-4000-8000-0000000001ab";
/// Dummies in the same style for the traits the plug capture lacks.
const LOCK: &str = "00000000-0000-4000-8000-00000000l0ck";
const ALARM: &str = "00000000-0000-4000-8000-00000000a1ar";
const THERMOSTAT: &str = "00000000-0000-4000-8000-00000000h3at";
const SPEAKER: &str = "00000000-0000-4000-8000-00000000v0l0";

const GRAPH_PATH: &str = "/$rpc/google.internal.home.foyer.v1.StructuresService/GetHomeGraph";
const GET_PATH: &str = "/$rpc/google.internal.home.foyer.v1.HomeControlService/GetTraits";
const UPDATE_PATH: &str = "/$rpc/google.internal.home.foyer.v1.HomeControlService/UpdateTraits";
const AUTH_PATH: &str = "/auth";

fn home(db: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    cmd.arg("--db").arg(db);
    cmd.env_remove("HOME_GH_EMAIL");
    cmd
}

/// Run the binary and demand exactly one JSON document on stdout.
fn run_full(cmd: &mut Command) -> (i32, Value, String, String) {
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

fn run(cmd: &mut Command) -> (i32, Value) {
    let (code, value, _, _) = run_full(cmd);
    (code, value)
}

fn temp_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("home.sqlite");
    (dir, db)
}

/// One Device entry of a `GetHomeGraph` answer.
fn device(id: &str, partner: &str, name: &str, kind: &str, traits: &[&str]) -> Value {
    json!([
        [id, ["tuya-smart-cee31", partner]],
        null,
        null,
        name,
        null,
        kind,
        traits,
    ])
}

/// One room entry, with the device ids it holds.
fn room(name: &str, ids: &[&str]) -> Value {
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

/// Two rooms, two Lamps named alike, plus a lock, an alarm, a thermostat and
/// a speaker that each advertise one more trait.
fn graph() -> Value {
    json!([
        "<num>",
        [[
            "home-1",
            "home",
            [],
            [],
            null,
            [
                room("客厅", &[LAMP_A, THERMOSTAT, SPEAKER]),
                room("书房", &[LAMP_B]),
                room("玄关", &[LOCK, ALARM]),
            ],
            [
                device(
                    LAMP_A,
                    "tuyaFakeHome000000001_all",
                    "Lamp",
                    "action.devices.types.LIGHT",
                    &[
                        "action.devices.traits.OnOff",
                        "action.devices.traits.Brightness",
                        "action.devices.traits.ColorSetting",
                    ],
                ),
                device(
                    LAMP_B,
                    "tuyaFakeLab0000000002_all",
                    "Lamp",
                    "action.devices.types.LIGHT",
                    &[
                        "action.devices.traits.OnOff",
                        "action.devices.traits.Brightness",
                        "action.devices.traits.ColorSetting",
                    ],
                ),
                device(
                    LOCK,
                    "lockPartnerId_all",
                    "Front Door",
                    "action.devices.types.LOCK",
                    &[
                        "action.devices.traits.OnOff",
                        "action.devices.traits.LockUnlock",
                    ],
                ),
                device(
                    ALARM,
                    "alarmPartnerId_all",
                    "Alarm",
                    "action.devices.types.SECURITYSYSTEM",
                    &[
                        "action.devices.traits.OnOff",
                        "action.devices.traits.ArmDisarm",
                    ],
                ),
                device(
                    THERMOSTAT,
                    "nestPartnerId_all",
                    "Nest",
                    "action.devices.types.THERMOSTAT",
                    &["action.devices.traits.TemperatureSetting"],
                ),
                device(
                    SPEAKER,
                    "speakerPartnerId_all",
                    "Speaker",
                    "action.devices.types.SPEAKER",
                    &[
                        "action.devices.traits.OnOff",
                        "action.devices.traits.Volume",
                    ],
                ),
            ]
        ]]
    ])
}

fn bool_w(v: bool) -> Value {
    json!([null, null, null, if v { 1 } else { 0 }])
}

fn st(v: &str) -> Value {
    json!([null, null, v])
}

/// The traits one Device reports, with `online` as 1 or 0.
fn device_state(id: &str, online: i64) -> Value {
    let mut traits = vec![json!([
        "deviceStatus",
        [["online", [null, null, null, online]]]
    ])];
    match id {
        LAMP_A | LAMP_B | SPEAKER => {
            traits.push(json!(["onOff", [["onOff", bool_w(true)]]]));
            if id != SPEAKER {
                traits.push(json!(["brightness", [["brightness", [null, 40]]]]));
                traits.push(json!(["color", [["colorTemperature", [null, 2700]]]]));
            } else {
                traits.push(json!(["volume", [["currentVolume", [null, 30]]]]));
            }
        }
        LOCK => traits.push(json!(["lockUnlock", [["isLocked", bool_w(true)]]])),
        ALARM => traits.push(json!(["armDisarm", [["isArmed", bool_w(true)]]])),
        THERMOSTAT => traits.push(json!([
            "temperatureSetting",
            [
                ["mode", st("heat")],
                ["thermostatTemperatureSetpoint", [null, 21]]
            ]
        ])),
        _ => {}
    }
    json!([[id], traits])
}

/// A `GetTraits`/`UpdateTraits` answer mentioning every Device.
fn states() -> Value {
    json!([[
        device_state(LAMP_A, 1),
        device_state(LAMP_B, 1),
        device_state(LOCK, 1),
        device_state(ALARM, 1),
        device_state(THERMOSTAT, 1),
        device_state(SPEAKER, 1),
    ]])
}

/// The same answer with LAMP_A reported offline.
fn states_with_lamp_a_offline() -> Value {
    json!([[
        device_state(LAMP_A, 0),
        device_state(LAMP_B, 1),
        device_state(LOCK, 1),
        device_state(ALARM, 1),
        device_state(THERMOSTAT, 1),
        device_state(SPEAKER, 1),
    ]])
}

async fn mock_get_traits(foyer: &MockServer, answer: &Value) {
    Mock::given(method("POST"))
        .and(path(GET_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(answer.to_string()),
        )
        .mount(foyer)
        .await;
}

async fn mock_update_traits(foyer: &MockServer) {
    Mock::given(method("POST"))
        .and(path(UPDATE_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(states().to_string()),
        )
        .mount(foyer)
        .await;
}

/// A fake foyer replaying the graph and the trait answers, plus a fake
/// account-token endpoint minting a Bearer.
async fn fakes(get_answer: &Value) -> (MockServer, MockServer) {
    let foyer = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(GRAPH_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(graph().to_string()),
        )
        .mount(&foyer)
        .await;
    mock_get_traits(&foyer, get_answer).await;
    mock_update_traits(&foyer).await;
    let auth = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(AUTH_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string("Auth=dummy-bearer\n"))
        .mount(&auth)
        .await;
    (foyer, auth)
}

fn point_at(cmd: &mut Command, foyer: &MockServer, auth: &MockServer) {
    cmd.env(
        "HOME_FOYER_URL",
        format!("{}/$rpc/google.internal.home.foyer.v1.", foyer.uri()),
    );
    cmd.env("HOME_GPSOAUTH_URL", format!("{}{AUTH_PATH}", auth.uri()));
}

fn login(db: &Path, foyer: &MockServer, auth: &MockServer) {
    let mut cmd = home(db);
    point_at(&mut cmd, foyer, auth);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    cmd.stdin(Stdio::piped());
    let mut child = cmd.spawn().expect("spawning login");
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"aas_et/dummy-master\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Every request the fake foyer saw on `path`, decoded when it is JSON.
async fn requests_to(foyer: &MockServer, path: &str) -> Vec<Value> {
    foyer
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == path)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

fn gh_get(db: &Path, foyer: &MockServer, auth: &MockServer, device: &str) -> Command {
    let mut cmd = home(db);
    point_at(&mut cmd, foyer, auth);
    cmd.args(["gh", "get", device]);
    cmd
}

fn gh_set(
    db: &Path,
    foyer: &MockServer,
    auth: &MockServer,
    device: &str,
    trait_name: &str,
    value: &str,
) -> Command {
    let mut cmd = home(db);
    point_at(&mut cmd, foyer, auth);
    cmd.args(["gh", "set", device, trait_name, value]);
    cmd
}

#[tokio::test]
async fn get_prints_the_current_traits_with_the_kind_field() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let mut cmd = gh_get(&db, &foyer, &auth, LAMP_A);
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["id"], LAMP_A);
    assert_eq!(data["name"], "Lamp");
    assert_eq!(data["room"], "客厅");
    assert_eq!(data["kind"], "action.devices.types.LIGHT");
    assert_eq!(data["backend"], "google_home");
    assert_eq!(data["online"], true);
    assert_eq!(data["traits"]["onOff"]["onOff"], true);
    assert_eq!(data["traits"]["brightness"]["brightness"], 40);
    assert_eq!(data["traits"]["color"]["colorTemperature"], 2700);
    assert!(
        data.get("type").is_none(),
        "device metadata uses kind, never type: {data}"
    );
    // The GetTraits body is `[[["id"]]]`.
    assert_eq!(
        requests_to(&foyer, GET_PATH).await,
        vec![json!([[[LAMP_A]]])]
    );
}

#[tokio::test]
async fn get_reads_volume_lock_and_thermostat_states() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let (code, json) = run(&mut gh_get(&db, &foyer, &auth, SPEAKER));
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["traits"]["volume"]["currentVolume"], 30);

    let (code, json) = run(&mut gh_get(&db, &foyer, &auth, LOCK));
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["traits"]["lockUnlock"]["isLocked"], true);

    let (code, json) = run(&mut gh_get(&db, &foyer, &auth, THERMOSTAT));
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["traits"]["temperatureSetting"]["mode"], "heat");
    assert_eq!(
        json["data"]["traits"]["temperatureSetting"]["thermostatTemperatureSetpoint"],
        21
    );
}

#[tokio::test]
async fn set_turns_the_lamp_on_and_off() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let mut cmd = gh_set(&db, &foyer, &auth, LAMP_A, "onOff", "false");
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["id"], LAMP_A);
    assert_eq!(json["data"]["kind"], "action.devices.types.LIGHT");
    assert_eq!(json["data"]["changed"]["trait"], "onOff");
    assert_eq!(json["data"]["changed"]["value"], false);

    let sent = requests_to(&foyer, UPDATE_PATH).await;
    assert_eq!(sent.len(), 1, "one UpdateTraits request");
    assert_eq!(
        sent[0],
        json!([[[
            [LAMP_A, ["tuya-smart-cee31", "tuyaFakeHome000000001_all"]],
            [["onOff", [["onOff", [null, null, null, 0]]]]]
        ]]])
    );
}

#[tokio::test]
async fn set_brightness_and_colour_temperature() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    assert_eq!(
        run(&mut gh_set(&db, &foyer, &auth, LAMP_A, "brightness", "75")).0,
        0
    );
    assert_eq!(
        run(&mut gh_set(
            &db,
            &foyer,
            &auth,
            LAMP_A,
            "color.colorTemperature",
            "3000"
        ))
        .0,
        0
    );
    let sent = requests_to(&foyer, UPDATE_PATH).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[0][0][0][1],
        json!([["brightness", [["brightness", [null, 75]]]]])
    );
    assert_eq!(
        sent[1][0][0][1],
        json!([["color", [["colorTemperature", [null, 3000]]]]])
    );
}

#[tokio::test]
async fn set_volume_uses_current_volume() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let (code, json) = run(&mut gh_set(
        &db,
        &foyer,
        &auth,
        SPEAKER,
        "volume.currentVolume",
        "30",
    ));
    assert_eq!(code, 0, "{json}");
    let sent = requests_to(&foyer, UPDATE_PATH).await;
    assert_eq!(
        sent[0][0][0][1],
        json!([["volume", [["currentVolume", [null, 30]]]]])
    );
}

#[tokio::test]
async fn set_thermostat_sends_mode_and_setpoint_in_one_request() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let mut cmd = gh_set(&db, &foyer, &auth, THERMOSTAT, "thermostat", "21");
    cmd.args(["--mode", "heat"]);
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(json["data"]["changed"]["mode"], "heat");
    assert_eq!(json["data"]["changed"]["setpoint"], 21.0);

    let sent = requests_to(&foyer, UPDATE_PATH).await;
    assert_eq!(sent.len(), 1);
    // Both fields sit in one temperatureSetting entry.
    assert_eq!(
        sent[0][0][0][1],
        json!([[
            "temperatureSetting",
            [
                ["mode", [null, null, "heat"]],
                ["thermostatTemperatureSetpoint", [null, 21]]
            ]
        ]])
    );
}

#[tokio::test]
async fn a_thermostat_change_missing_a_field_is_usage() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    // No --setpoint, and the value is not a number either.
    let (code, json) = run(&mut gh_set(
        &db,
        &foyer,
        &auth,
        THERMOSTAT,
        "thermostat",
        "heat",
    ));
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
    assert!(requests_to(&foyer, UPDATE_PATH).await.is_empty());
}

#[tokio::test]
async fn resolution_is_exact_id_then_room_name_then_bare_name() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    // Exact ID.
    let (code, json) = run(&mut gh_get(&db, &foyer, &auth, LAMP_B));
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["id"], LAMP_B);

    // room/name, case-insensitive, UTF-8 Chinese room intact.
    let (code, json) = run(&mut gh_get(&db, &foyer, &auth, "客厅/lamp"));
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["id"], LAMP_A);

    // A bare name that is unique.
    let (code, json) = run(&mut gh_get(&db, &foyer, &auth, "Front Door"));
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["id"], LOCK);
}

#[tokio::test]
async fn a_duplicated_name_gives_exit_3_with_the_candidates() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let (code, json, stdout, _) = run_full(&mut gh_get(&db, &foyer, &auth, "Lamp"));
    assert_eq!(code, 3, "{stdout}");
    assert_eq!(json["error"]["code"], "ambiguous");
    assert_eq!(json["error"]["backend"], "google_home");
    // Each candidate is one JSON document encoded as a string, so the error
    // document keeps its fixed shape.
    let candidates = json["error"]["candidates"]
        .as_array()
        .expect("a candidate array");
    assert_eq!(candidates.len(), 2);
    let candidates: Vec<Value> = candidates
        .iter()
        .map(|c| serde_json::from_str(c.as_str().expect("a JSON string")).unwrap())
        .collect();
    let ids: Vec<&str> = candidates
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&LAMP_A) && ids.contains(&LAMP_B),
        "{candidates:?}"
    );
    assert_eq!(candidates[0]["kind"], "action.devices.types.LIGHT");
    assert!(candidates[0]["room"].is_string());
    // No state was read or written for an ambiguous name.
    assert!(requests_to(&foyer, GET_PATH).await.is_empty());
    assert!(requests_to(&foyer, UPDATE_PATH).await.is_empty());
}

#[tokio::test]
async fn unlock_and_disarm_without_confirm_are_exit_6_and_never_sent() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    for (device, trait_name) in [(LOCK, "lockUnlock.isLocked"), (ALARM, "armDisarm.isArmed")] {
        let (code, json, stdout, _) =
            run_full(&mut gh_set(&db, &foyer, &auth, device, trait_name, "false"));
        assert_eq!(code, 6, "{device}: {stdout}");
        assert_eq!(json["error"]["code"], "sensitive_action");
        assert_eq!(json["error"]["backend"], "google_home");
    }
    assert!(
        requests_to(&foyer, UPDATE_PATH).await.is_empty(),
        "a Sensitive Action without --confirm must send nothing"
    );
}

#[tokio::test]
async fn a_confirmed_unlock_is_sent() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let mut cmd = gh_set(&db, &foyer, &auth, LOCK, "lockUnlock.isLocked", "false");
    cmd.arg("--confirm");
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(
        requests_to(&foyer, UPDATE_PATH).await[0][0][0][1],
        json!([["lockUnlock", [["isLocked", [null, null, null, 0]]]]])
    );
}

#[tokio::test]
async fn an_offline_device_is_exit_4_on_get_and_on_set() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states_with_lamp_a_offline()).await;
    login(&db, &foyer, &auth);

    let (code, json) = run(&mut gh_get(&db, &foyer, &auth, LAMP_A));
    assert_eq!(code, 4, "{json}");
    assert_eq!(json["error"]["code"], "device_offline");

    let (code, json) = run(&mut gh_set(&db, &foyer, &auth, LAMP_A, "onOff", "true"));
    assert_eq!(code, 4, "{json}");
    assert_eq!(json["error"]["code"], "device_offline");
    assert!(requests_to(&foyer, UPDATE_PATH).await.is_empty());
}

#[tokio::test]
async fn a_protected_device_is_refused_even_with_confirm_but_still_readable() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let mut cmd = home(&db);
    cmd.args(["protect", &format!("gh:{LAMP_A}")]);
    assert_eq!(run(&mut cmd).0, 0);

    let mut cmd = gh_set(&db, &foyer, &auth, LAMP_A, "onOff", "false");
    cmd.arg("--confirm");
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 6, "{stdout}");
    assert_eq!(json["error"]["code"], "protected");
    assert_eq!(json["error"]["backend"], "google_home");
    assert!(
        requests_to(&foyer, UPDATE_PATH).await.is_empty(),
        "a protected Device must never receive UpdateTraits"
    );

    // Reading the protected Device still works.
    let (code, json) = run(&mut gh_get(&db, &foyer, &auth, LAMP_A));
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["traits"]["onOff"]["onOff"], true);
}

#[tokio::test]
async fn a_rejected_bearer_is_minted_again_once_and_the_call_retried() {
    let (_dir, db) = temp_db();
    let foyer = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(GRAPH_PATH))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&foyer)
        .await;
    Mock::given(method("POST"))
        .and(path(GRAPH_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(graph().to_string()),
        )
        .mount(&foyer)
        .await;
    mock_get_traits(&foyer, &states()).await;
    let auth = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(AUTH_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string("Auth=dummy-bearer\n"))
        .mount(&auth)
        .await;
    login(&db, &foyer, &auth);

    let mut cmd = gh_get(&db, &foyer, &auth, LAMP_A);
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(json["data"]["online"], true);

    let graph_hits = foyer
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == GRAPH_PATH)
        .count();
    assert_eq!(graph_hits, 2, "one retry after the 401");
    // One mint for login, one for the refresh.
    let auth_hits = auth.received_requests().await.unwrap().len();
    assert_eq!(auth_hits, 2, "the Bearer is minted once more, not twice");
}

#[tokio::test]
async fn a_second_rejection_is_exit_5_reauth_naming_the_backend() {
    let (_dir, db) = temp_db();
    let foyer = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(GRAPH_PATH))
        .respond_with(ResponseTemplate::new(403))
        .mount(&foyer)
        .await;
    let auth = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(AUTH_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string("Auth=dummy-bearer\n"))
        .mount(&auth)
        .await;
    login(&db, &foyer, &auth);

    let mut cmd = gh_get(&db, &foyer, &auth, LAMP_A);
    let (code, json, stdout, _) = run_full(&mut cmd);
    assert_eq!(code, 5, "{stdout}");
    assert_eq!(json["error"]["code"], "reauth_required");
    assert_eq!(json["error"]["backend"], "google_home");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Google Home Backend"),
        "the message must name the Backend: {json}"
    );
    let graph_hits = foyer
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == GRAPH_PATH)
        .count();
    assert_eq!(graph_hits, 2, "exactly one retry");
    assert_eq!(auth.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_trait_the_device_lacks_is_usage_and_never_sent() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    // The thermostat advertises TemperatureSetting only.
    let (code, json, stdout, _) =
        run_full(&mut gh_set(&db, &foyer, &auth, THERMOSTAT, "onOff", "true"));
    assert_eq!(code, 2, "{stdout}");
    assert_eq!(json["error"]["code"], "unsupported_trait");
    assert!(requests_to(&foyer, UPDATE_PATH).await.is_empty());
}

#[tokio::test]
async fn a_device_list_still_hides_the_partner_ids() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes(&states()).await;
    login(&db, &foyer, &auth);

    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "devices"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    let devices = json["data"].as_array().unwrap();
    assert_eq!(devices.len(), 6);
    for d in devices {
        assert!(d.get("agent_id").is_none(), "{d}");
        assert!(d.get("partner_device_id").is_none(), "{d}");
        assert!(d.get("kind").is_some(), "{d}");
        assert!(d.get("type").is_none(), "{d}");
    }
}
