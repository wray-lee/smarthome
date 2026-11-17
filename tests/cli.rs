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
    cmd.env_remove("HOME_GH_TOKEN");
    cmd
}

/// Run the binary and demand exactly one JSON document on stdout. Trailing
/// whitespace is allowed; a second value or any prose is not.
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

/// Spawn with stdin piped, optionally write `input`, and keep stdin open
/// (docker exec -i style) until the process exits. Waits at most 5 seconds.
fn run_with_open_stdin(mut cmd: Command, input: Option<&[u8]>) -> (i32, String, String) {
    use std::io::Write;
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().expect("spawning the home binary");
    let mut stdin = child.stdin.take().unwrap();
    if let Some(bytes) = input {
        stdin.write_all(bytes).expect("writing stdin");
        stdin.flush().expect("flushing stdin");
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("waiting for the binary") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the binary still waited for EOF on stdin after 5 s"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    let mut stdout = Vec::new();
    std::io::Read::read_to_end(&mut child.stdout.take().unwrap(), &mut stdout).unwrap();
    let mut stderr = Vec::new();
    std::io::Read::read_to_end(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    drop(stdin);
    (
        status.code().expect("an exit code"),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

/// The stored master token, or None when nothing was saved.
fn stored_master(db: &Path) -> Option<String> {
    let conn = rusqlite::Connection::open(db).ok()?;
    let cred: String = conn
        .query_row(
            "SELECT json FROM credentials WHERE backend = 'google_home'",
            [],
            |r| r.get(0),
        )
        .ok()?;
    let cred: serde_json::Value = serde_json::from_str(&cred).ok()?;
    cred["master"].as_str().map(str::to_string)
}

/// Pasting one line and pressing Enter must not wait for EOF: a docker exec
/// without -i, or a terminal left open, never sends one.
#[tokio::test]
async fn interactive_paste_exits_without_eof() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    let (code, stdout, stderr) = run_with_open_stdin(cmd, Some(b"aas_et/pasted-master\n"));
    assert_eq!(code, 0, "{stdout} {stderr}");
    assert_eq!(stored_master(&db).as_deref(), Some("aas_et/pasted-master"));
    assert!(
        !stderr.contains("EmbeddedSetup"),
        "piped stdin is not a terminal, so no guidance: {stderr}"
    );
}

/// Under a real pty the guidance goes to stderr and stdout stays one JSON
/// document. `script` allocates the pty; the child's stdout goes to a file so
/// the two streams can be told apart.
#[tokio::test]
async fn a_terminal_stdin_gets_step_by_step_guidance() {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("skipping: no script(1) to allocate a pty");
        return;
    }
    let (dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    let bin = env!("CARGO_BIN_EXE_home");
    let token_file = dir.path().join("pasted");
    let out_file = dir.path().join("login.json");
    std::fs::write(&token_file, "aas_et/pty-master\n").unwrap();
    let command_line = format!(
        "{} --db {} gh login --email user@example.com > {}",
        shell_quote(bin),
        shell_quote(db.to_str().unwrap()),
        shell_quote(out_file.to_str().unwrap()),
    );
    let mut cmd = Command::new("script");
    cmd.args(["-qec", &command_line, "/dev/null"]);
    cmd.env_remove("HOME_GH_EMAIL");
    cmd.env_remove("HOME_GH_TOKEN");
    cmd.env("HOME_GPSOAUTH_URL", format!("{}/auth", auth.uri()));
    cmd.env(
        "HOME_FOYER_URL",
        format!("{}/$rpc/google.internal.home.foyer.v1.", foyer.uri()),
    );
    cmd.stdin(std::fs::File::open(&token_file).unwrap())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let out = cmd.output().expect("spawning script");
    let pty = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{pty} {stderr}");
    for marker in [
        "https://accounts.google.com/EmbeddedSetup",
        "oauth_token",
        "oauth2_4/",
        "aas_et/",
        "Paste the token and press Enter",
        "HOME_GH_TOKEN",
        "--token-file",
    ] {
        assert!(pty.contains(marker), "missing {marker:?} in {pty:?}");
    }
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&out_file).unwrap()).expect("one JSON document");
    assert_eq!(json["ok"], true, "{json}");
    assert_eq!(stored_master(&db).as_deref(), Some("aas_et/pty-master"));
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// `printf 'aas_et/...' | home gh login` sends no trailing newline.
#[tokio::test]
async fn a_piped_token_without_a_newline_is_stored() {
    use std::io::Write;
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    cmd.stdin(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"aas_et/piped-master")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(stored_master(&db).as_deref(), Some("aas_et/piped-master"));
}

/// `HOME_GH_TOKEN` wins over stdin and never touches it, so a docker exec
/// without -i exits instead of hanging.
#[tokio::test]
async fn home_gh_token_needs_no_stdin() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    cmd.env("HOME_GH_TOKEN", "aas_et/env-master");
    let (code, stdout, stderr) = run_with_open_stdin(cmd, Some(b"aas_et/stdin-master\n"));
    assert_eq!(code, 0, "{stdout} {stderr}");
    assert_eq!(stored_master(&db).as_deref(), Some("aas_et/env-master"));
}

/// Both fallbacks leave stdin alone; the file beats the env, and `-` reads
/// stdin like a pipe.
#[tokio::test]
async fn a_token_file_needs_no_stdin_and_beats_the_env() {
    let (dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    let file = dir.path().join("token");
    std::fs::write(&file, "aas_et/file-master\n").unwrap();

    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    cmd.arg("--token-file").arg(&file);
    cmd.env("HOME_GH_TOKEN", "aas_et/env-master");
    let (code, stdout, stderr) = run_with_open_stdin(cmd, Some(b"aas_et/stdin-master\n"));
    assert_eq!(code, 0, "{stdout} {stderr}");
    assert_eq!(stored_master(&db).as_deref(), Some("aas_et/file-master"));

    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    cmd.arg("--token-file").arg("-");
    cmd.env("HOME_GH_TOKEN", "aas_et/env-master");
    let (code, stdout, stderr) = run_with_open_stdin(cmd, Some(b"aas_et/dash-master\n"));
    assert_eq!(code, 0, "{stdout} {stderr}");
    assert_eq!(stored_master(&db).as_deref(), Some("aas_et/dash-master"));
}

#[tokio::test]
async fn a_missing_or_unreadable_token_file_is_usage_exit_2() {
    let (dir, db) = temp_db();
    for path in [dir.path().join("missing"), dir.path().to_path_buf()] {
        let mut cmd = home(&db);
        cmd.args(["gh", "login", "--email", "user@example.com"]);
        cmd.arg("--token-file").arg(&path);
        cmd.stdin(std::process::Stdio::null());
        let (code, json) = run(&mut cmd);
        assert_eq!(code, 2, "{json}");
        assert_eq!(json["error"]["code"], "usage");
        assert!(
            json["error"]["message"]
                .as_str()
                .unwrap()
                .contains("token file"),
            "{json}"
        );
    }
    assert_eq!(credential_rows(&db), 0);
}

/// A rejected token must not be echoed back, whatever its source.
#[tokio::test]
async fn an_invalid_token_is_never_echoed() {
    let (_dir, db) = temp_db();
    let mut cmd = home(&db);
    cmd.args(["gh", "login", "--email", "user@example.com"]);
    cmd.env("HOME_GH_TOKEN", "secret-not-a-token-123");
    let (code, stdout, stderr) = run_with_open_stdin(cmd, None);
    assert_eq!(code, 2, "{stdout} {stderr}");
    assert!(!stdout.contains("secret-not-a-token-123"), "{stdout}");
    assert!(!stderr.contains("secret-not-a-token-123"), "{stderr}");
}

/// An expired or reused oauth_token fails as reauth_required and says how to
/// get a fresh one, without echoing the token.
#[tokio::test]
async fn a_rejected_oauth_token_says_how_to_get_a_fresh_one() {
    let (_dir, db) = temp_db();
    let auth = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Error=BadAuthentication\n"))
        .mount(&auth)
        .await;
    let mut cmd = home(&db);
    cmd.args(["gh", "login", "--email", "you@example.com"]);
    cmd.env("HOME_GPSOAUTH_URL", format!("{}/auth", auth.uri()));
    cmd.env("HOME_GH_TOKEN", "oauth2_4/stale-token");
    let (code, stdout, stderr) = run_with_open_stdin(cmd, None);
    assert_eq!(code, 5, "{stdout} {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("one JSON document");
    assert_eq!(json["error"]["code"], "reauth_required", "{json}");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(
            "oauth_token is single-use and expires within minutes: get a fresh one from https://accounts.google.com/EmbeddedSetup and retry"
        ),
        "{message}"
    );
    assert!(!stdout.contains("stale-token"), "{stdout}");
    assert!(!stderr.contains("stale-token"), "{stderr}");
    assert_eq!(credential_rows(&db), 0);
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

/// `--help` and `--version` keep clap's text on stderr; stdout stays one JSON
/// document and the exit code is 0.
#[tokio::test]
async fn help_and_version_put_their_text_on_stderr_and_one_json_on_stdout() {
    let (_dir, db) = temp_db();
    let version = concat!("home ", env!("CARGO_PKG_VERSION"));
    for (args, marker) in [
        (&["--help"][..], "Usage: home"),
        (&["--version"][..], version),
        (&["gh", "--help"][..], "Usage: home gh"),
        (&["protect", "--help"][..], "Usage: home protect"),
    ] {
        let mut cmd = home(&db);
        cmd.args(args);
        let (code, json, stdout, stderr) = run_full(&mut cmd);
        assert_eq!(code, 0, "{args:?}: {stdout} {stderr}");
        assert_eq!(json["ok"], true, "{args:?}: {stdout}");
        assert_eq!(json["data"], serde_json::json!({}), "{args:?}");
        assert!(
            stderr.contains(marker),
            "{args:?}: the help text must reach stderr, got {stderr:?}"
        );
        assert!(
            !stdout.contains("Usage:") && !stdout.contains(version),
            "{args:?}: the help text must not reach stdout: {stdout:?}"
        );
    }
}

/// The guard line some clients6 answers carry must not turn a device list
/// into an upstream error, nor leak into the printed JSON.
#[tokio::test]
async fn a_foyer_answer_with_the_xssi_guard_still_lists_devices() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    login(&db, &foyer, &auth);
    let guarded = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/$rpc/google.internal.home.foyer.v1.StructuresService/GetHomeGraph",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(format!(")]}}'\n{GRAPH}")),
        )
        .mount(&guarded)
        .await;
    let mut cmd = home(&db);
    point_at(&mut cmd, &guarded, &auth);
    cmd.args(["gh", "devices"]);
    let (code, json) = run(&mut cmd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    let devices = json["data"].as_array().expect("a device array");
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0]["name"], "Home");
    assert_eq!(devices[1]["room"], "实验室");
}

/// A repeated login keeps the Android id the master token was bound to.
#[tokio::test]
async fn a_second_login_keeps_the_stored_android_id() {
    let (_dir, db) = temp_db();
    let (foyer, auth) = fakes().await;
    login(&db, &foyer, &auth);
    let first = stored_android_id(&db);
    login(&db, &foyer, &auth);
    let second = stored_android_id(&db);
    assert_eq!(first.len(), 16);
    assert_eq!(
        first, second,
        "a second login must not rotate the Android id"
    );

    let mut cmd = home(&db);
    point_at(&mut cmd, &foyer, &auth);
    cmd.args(["gh", "login", "--email", "user@example.com", "--android-id"]);
    cmd.arg("fedcba9876543210");
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
    assert_eq!(out.status.code(), Some(0), "{:?}", out.stdout);
    assert_eq!(stored_android_id(&db), "fedcba9876543210");
}

fn stored_android_id(db: &Path) -> String {
    let conn = rusqlite::Connection::open(db).unwrap();
    let cred: String = conn
        .query_row(
            "SELECT json FROM credentials WHERE backend = 'google_home'",
            [],
            |r| r.get(0),
        )
        .expect("the credential is stored");
    let cred: serde_json::Value = serde_json::from_str(&cred).unwrap();
    cred["android_id"]
        .as_str()
        .expect("the credential carries an Android id")
        .to_string()
}
