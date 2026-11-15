//! Black-box tests: run the `home` binary and assert its stdout JSON and
//! exit code. The Google Home upstreams are faked with wiremock and every
//! test gets its own temporary SQLite file.

use std::path::{Path, PathBuf};
use std::process::Command;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const GRAPH: &str = include_str!("fixtures/GetHomeGraph.json");

fn home(db: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_home"));
    cmd.arg("--db").arg(db);
    cmd.env_remove("HOME_GH_EMAIL");
    cmd
}

fn run(cmd: &mut Command) -> (i32, serde_json::Value) {
    let out = cmd.output().expect("running the home binary");
    let stdout = String::from_utf8(out.stdout).expect("stdout is UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {stdout:?}"));
    (out.status.code().expect("an exit code"), value)
}

fn temp_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("home.sqlite");
    (dir, db)
}

/// A fake foyer answering `GetHomeGraph` with the ticket-01 fixture, and a
/// fake account-token endpoint minting a Bearer.
async fn fakes() -> (MockServer, MockServer) {
    let foyer = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/$rpc/google.internal.home.foyer.v1.StructuresService/GetHomeGraph",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(GRAPH),
        )
        .mount(&foyer)
        .await;
    let auth = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth"))
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
    cmd.env("HOME_GPSOAUTH_URL", format!("{}/auth", auth.uri()));
}

fn login(db: &Path, foyer: &MockServer, auth: &MockServer) {
    let mut cmd = home(db);
    point_at(&mut cmd, foyer, auth);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    cmd.stdin(std::process::Stdio::piped());
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

#[tokio::test]
async fn the_device_list_replays_the_foyer_fixture() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    login(&db, &foyer, &auth);

    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "devices"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    let devices = json["data"].as_array().expect("a device array");
    assert_eq!(devices.len(), 2);
    let home_plug = &devices[0];
    assert_eq!(home_plug["id"], "00000000-0000-4000-8000-00000000h0me");
    assert_eq!(home_plug["name"], "Home");
    assert_eq!(home_plug["room"], "主卧");
    assert_eq!(home_plug["kind"], "action.devices.types.OUTLET");
    assert_eq!(home_plug["traits"][0], "action.devices.traits.OnOff");
    assert_eq!(home_plug["backend"], "google_home");
    let second = &devices[1];
    assert_eq!(second["id"], "00000000-0000-4000-8000-0000000001ab");
    assert_eq!(second["room"], "实验室");
    assert_eq!(second["backend"], "google_home");
    assert!(!second["name"].as_str().unwrap().is_empty());

    // The request carried a Bearer, and no API key wherever it went.
    for server in [&foyer, &auth] {
        for req in server.received_requests().await.unwrap() {
            assert!(
                !req.headers.contains_key("x-goog-api-key"),
                "an API key went to {}",
                req.url
            );
        }
    }
    let foyer_requests = foyer.received_requests().await.unwrap();
    assert_eq!(foyer_requests.len(), 1);
    assert_eq!(
        foyer_requests[0].headers["authorization"],
        "Bearer dummy-bearer"
    );
    assert_eq!(
        foyer_requests[0].headers["content-type"],
        "application/json+protobuf"
    );
    assert_eq!(foyer_requests[0].body, b"[]");
}

#[tokio::test]
async fn the_device_list_is_cached_in_the_database_with_mode_600() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    login(&db, &foyer, &auth);
    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "devices"]);
    assert_eq!(run(&mut cmd).0, 0);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&db).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the database must be owner-only");
    }
    let conn = rusqlite::Connection::open(&db).unwrap();
    let cached: String = conn
        .query_row(
            "SELECT json FROM device_cache WHERE backend = 'google_home'",
            [],
            |r| r.get(0),
        )
        .expect("the device list is cached");
    let cached: serde_json::Value = serde_json::from_str(&cached).unwrap();
    assert_eq!(cached.as_array().unwrap().len(), 2);
    let cred: String = conn
        .query_row(
            "SELECT json FROM credentials WHERE backend = 'google_home'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let cred: serde_json::Value = serde_json::from_str(&cred).unwrap();
    assert_eq!(cred["master"], "aas_et/dummy-master");
    assert_eq!(cred["android_id"].as_str().unwrap().len(), 16);
}

#[tokio::test]
async fn a_usage_error_is_exit_2_with_one_json_document() {
    let (_dir, db) = temp_db();
    let mut cmd = home(&db);
    cmd.args(["gh", "login"]);
    cmd.stdin(std::process::Stdio::null());
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "usage");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--email"));
}

#[tokio::test]
async fn an_unknown_subcommand_is_exit_2_with_json() {
    let (_dir, db) = temp_db();
    let mut cmd = home(&db);
    cmd.args(["nonsense"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
}

#[tokio::test]
async fn devices_without_a_credential_is_reauth_exit_5() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "devices"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 5, "{json}");
    assert_eq!(json["error"]["code"], "reauth_required");
    assert_eq!(json["error"]["backend"], "google_home");
    assert!(foyer.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_bad_login_token_is_exit_2_and_stores_nothing() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not-a-token\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {:?}; stderr: {:?}; status: {:?}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
            out.status
        )
    });
    assert_eq!(out.status.code(), Some(2), "{json}");
    assert_eq!(json["error"]["code"], "usage");
    assert!(auth.received_requests().await.unwrap().is_empty());
    assert!(!db.exists() || credential_rows(&db) == 0);
}

/// Row count in `credentials`, or 0 when the table does not exist yet.
fn credential_rows(db: &Path) -> i64 {
    match rusqlite::Connection::open(db) {
        Ok(conn) => conn
            .query_row("SELECT count(*) FROM credentials", [], |r| r.get(0))
            .unwrap_or(0),
        Err(_) => 0,
    }
}

#[tokio::test]
async fn protect_list_and_remove_round_trip() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["protect", "gh:dev-1"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["backend"], "google_home");
    assert_eq!(json["data"]["device_id"], "dev-1");

    let mut cmd = home(&db);
    cmd.args(["protect", "tuya:dev-2"]);
    assert_eq!(run(&mut cmd).0, 0);

    let mut cmd = home(&db);
    cmd.args(["protect", "--list"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    let rows = json["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["backend"], "google_home");
    assert_eq!(rows[1]["backend"], "tuya");

    let mut cmd = home(&db);
    cmd.args(["protect", "--remove", "gh:dev-1"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["data"]["removed"], true);

    let mut cmd = home(&db);
    cmd.args(["protect", "--list"]);
    let (_, json) = run(&mut cmd);
    let rows = json["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["device_id"], "dev-2");
}

#[tokio::test]
async fn protecting_a_device_without_a_backend_is_exit_2() {
    let (_dir, db) = temp_db();
    let mut cmd = home(&db);
    cmd.args(["protect", "dev-1"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
}
