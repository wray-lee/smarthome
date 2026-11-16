//! `home` — one binary for both Backends. stdout is always exactly one JSON
//! document and the exit code follows the contract in `home_store::contract`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};
use ghome_backend::{config, gpsoauth, Credential, Session, BACKEND as GH};
use home_store::contract::{ok, ErrorBody, Exit};
use home_store::{guard, Store};

#[derive(Parser, Debug)]
#[command(name = "home", version, about = "Smart home devices from the terminal")]
struct Cli {
    /// SQLite database path (default: $XDG_DATA_HOME/home/home.sqlite).
    #[arg(long, global = true, env = "HOME_DB", value_name = "PATH")]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Google Home backend.
    #[command(subcommand)]
    Gh(GhCmd),
    /// Tuya backend.
    #[command(subcommand)]
    Tuya(TuyaCmd),
    /// Add, list or remove protected Devices. They are refused by every write.
    Protect(ProtectArgs),
    /// Serve the Web Panel and its JSON API, with a background Tuya refresh.
    Serve(ServeArgs),
}

#[derive(Args, Debug)]
struct ServeArgs {
    /// Bind address (`HOME_HOST`), then port from the env, or both flags.
    #[arg(long, value_name = "ADDR")]
    host: Option<String>,
    /// Port to listen on.
    #[arg(long, value_name = "PORT")]
    port: Option<u16>,
    /// Panel password. When absent it comes from `HOME_PASSWORD`, and
    /// `home serve` refuses to start without one.
    #[arg(long, env = "HOME_PASSWORD", value_name = "PASSWORD")]
    password: Option<String>,
    /// Read the panel password from this file instead of argv or the env.
    #[arg(long, env = "HOME_PASSWORD_FILE", value_name = "PATH")]
    password_file: Option<String>,
    /// Add the `Secure` attribute to the session cookie. Set this when the
    /// panel is reached over HTTPS through a reverse proxy.
    #[arg(long, env = "HOME_COOKIE_SECURE")]
    secure_cookie: bool,
}

#[derive(Subcommand, Debug)]
enum GhCmd {
    /// Store a Google Home master token (or oauth_token) read from stdin.
    Login(LoginArgs),
    /// Print every Google Home Device as JSON.
    Devices,
    /// Print one Device's current trait state.
    Get {
        /// Device ID, room/name, or bare name (case-insensitive).
        device: String,
    },
    /// Change one trait of one Device.
    Set(SetArgs),
}

#[derive(Args, Debug)]
struct SetArgs {
    /// Device ID, room/name, or bare name (case-insensitive).
    device: String,
    /// onOff | brightness | color.colorTemperature | volume.currentVolume
    /// | lockUnlock.isLocked | armDisarm.isArmed | thermostat
    #[arg(name = "trait")]
    trait_name: String,
    /// The new value; for thermostat either the setpoint or the mode.
    value: String,
    /// Thermostat mode, e.g. heat.
    #[arg(long, value_name = "MODE")]
    mode: Option<String>,
    /// Thermostat setpoint in degrees.
    #[arg(long, value_name = "N")]
    setpoint: Option<f64>,
    /// Allow a Sensitive Action (unlock, disarm).
    #[arg(long)]
    confirm: bool,
}

#[derive(Subcommand, Debug)]
enum TuyaCmd {
    /// Sign in by scanning a QR code with the Smart Life app.
    Login(TuyaLoginArgs),
    /// Print every Tuya Device as JSON.
    Devices,
    /// Print every saved Local Key as JSON, for the Vendor Exit backup.
    Keys,
    /// Turn a Tuya Device on.
    On {
        /// Device ID or name (case-insensitive).
        device: String,
    },
    /// Turn a Tuya Device off.
    Off {
        /// Device ID or name (case-insensitive).
        device: String,
    },
    /// Print data points, Telemetry and calibration for one Tuya Device.
    Get {
        /// Device ID or name (case-insensitive).
        device: String,
    },
    /// Write one raw data point of a Tuya Device.
    Set {
        /// Device ID or name (case-insensitive).
        device: String,
        /// The data point code, e.g. switch_1.
        code: String,
        /// The raw value to write.
        value: String,
    },
}

#[derive(Args, Debug)]
struct TuyaLoginArgs {
    /// Smart Life user code (Me → Settings → Account and Security); the code
    /// itself is not secret, but it is read from HOME_TUYA_USER_CODE too.
    #[arg(long, value_name = "CODE")]
    user_code: Option<String>,
}

#[derive(Args, Debug)]
struct LoginArgs {
    /// The Google account the token belongs to.
    #[arg(long, env = "HOME_GH_EMAIL", value_name = "EMAIL")]
    email: Option<String>,
    /// Replace the stored Android id instead of reusing it.
    #[arg(long, value_name = "ID")]
    android_id: Option<String>,
}

#[derive(Args, Debug)]
struct ProtectArgs {
    /// BACKEND:DEVICE_ID, e.g. google_home:abc or tuya:xyz (gh: is an alias).
    device: Option<String>,
    /// Show the protected list.
    #[arg(long)]
    list: bool,
    /// Remove the Device from the protected list.
    #[arg(long)]
    remove: bool,
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => match e.kind() {
            // `--help` and `--version` are informational, so their text goes
            // to stderr; stdout still carries exactly one JSON document.
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                eprint!("{e}");
                emit(ok(serde_json::json!({})), Exit::Ok);
            }
            _ => fail(ErrorBody::usage(e.to_string()), Exit::Usage),
        },
    };
    match run(&cli) {
        Ok(value) => emit(ok(value), Exit::Ok),
        Err((body, exit)) => fail(body, exit),
    }
}

fn emit(value: serde_json::Value, exit: Exit) -> ! {
    println!("{value}");
    std::process::exit(exit.code());
}

fn fail(body: ErrorBody, exit: Exit) -> ! {
    let (value, code) = body.exit_with(exit);
    println!("{value}");
    std::process::exit(code);
}

fn default_db() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("home").join("home.sqlite")
}

fn storage(e: home_store::StoreError) -> (ErrorBody, Exit) {
    (ErrorBody::new("storage", e.to_string()), Exit::Upstream)
}

fn run(cli: &Cli) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let path = cli.db.clone().unwrap_or_else(default_db);
    let store = Store::open(&path).map_err(storage)?;
    match &cli.command {
        Command::Gh(cmd) => match cmd {
            GhCmd::Login(args) => login(&store, args),
            GhCmd::Devices => devices(store),
            GhCmd::Get { device } => get(&store, device),
            GhCmd::Set(args) => set(&store, args),
        },
        Command::Tuya(cmd) => {
            let cfg = tuya_backend::Config::from_env();
            match cmd {
                TuyaCmd::Login(args) => tuya_login(&cfg, &store, args),
                TuyaCmd::Devices => tuya_backend::devices_command(&cfg, &store),
                TuyaCmd::Keys => tuya_backend::keys_command(&cfg, &store),
                TuyaCmd::On { device } => tuya_backend::on_off_command(&cfg, &store, device, true),
                TuyaCmd::Off { device } => {
                    tuya_backend::on_off_command(&cfg, &store, device, false)
                }
                TuyaCmd::Get { device } => tuya_backend::get_command(&cfg, &store, device),
                TuyaCmd::Set {
                    device,
                    code,
                    value,
                } => tuya_backend::set_command(&cfg, &store, device, code, value),
            }
        }
        Command::Protect(args) => protect(&store, args),
        Command::Serve(args) => {
            let password = match &args.password_file {
                Some(path) => std::fs::read_to_string(path)
                    .map(|s| s.trim_end_matches(['\r', '\n']).to_string())
                    .unwrap_or_default(),
                None => args.password.clone().unwrap_or_default(),
            };
            if password.trim().is_empty() {
                let body = ErrorBody::usage(
                    "set HOME_PASSWORD, HOME_PASSWORD_FILE or pass --password; the panel refuses to start unguarded",
                );
                return Err((body, Exit::Usage));
            }
            let host = args
                .host
                .clone()
                .or_else(|| std::env::var("HOME_HOST").ok())
                .unwrap_or_else(|| "127.0.0.1".into());
            let port = match args.port {
                Some(p) => p,
                None => std::env::var("HOME_PORT")
                    .ok()
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(8080),
            };
            let db = cli.db.clone().unwrap_or_else(default_db);
            serve(&db, &host, port, &password, args.secure_cookie)
        }
    }
}

fn login(store: &Store, args: &LoginArgs) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let email = args.email.clone().ok_or_else(|| {
        (
            ErrorBody::usage("pass --email or set HOME_GH_EMAIL"),
            Exit::Usage,
        )
    })?;
    let mut token = String::new();
    std::io::stdin()
        .read_to_string(&mut token)
        .map_err(|e| (ErrorBody::usage(format!("reading stdin: {e}")), Exit::Usage))?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err((
            ErrorBody::usage(
                "expected a master token (aas_et/…) or an oauth_token (oauth2_4/…) on stdin",
            ),
            Exit::Usage,
        ));
    }
    // The Android id is bound to the master token and every Bearer minted
    // from it, so a second login keeps the stored one; only an explicit
    // `--android_id` replaces it. The token is not verified here: a bad one
    // shows up at the first `home gh devices` (see README).
    let stored = Credential::load(store).map_err(storage)?;
    let android_id = match &args.android_id {
        Some(id) => id.clone(),
        None => match &stored {
            Some(stored) => stored.android_id.clone(),
            None => gpsoauth::new_android_id(),
        },
    };
    let master = if token.starts_with(gpsoauth::MASTER_TOKEN_PREFIX) {
        token
    } else if token.starts_with(gpsoauth::OAUTH_TOKEN_PREFIX) {
        let cfg = config::Config::from_env();
        gpsoauth::exchange_oauth_token(
            &ghome_backend::foyer::client(),
            &cfg.gpsoauth_url,
            &token,
            &android_id,
            &email,
        )?
    } else {
        return Err((
            ErrorBody::usage(format!(
                "expected a token starting with {} or {}",
                gpsoauth::MASTER_TOKEN_PREFIX,
                gpsoauth::OAUTH_TOKEN_PREFIX
            )),
            Exit::Usage,
        ));
    };
    // A cached Bearer outlives a re-login only while the identity behind it
    // stays the same: the Android id and the minting account.
    let keeps_bearer = stored
        .filter(|old| old.android_id == android_id && old.email == email)
        .and_then(|old| old.bearer.zip(old.bearer_expires_at));
    let (bearer, bearer_expires_at) = keeps_bearer.unzip();
    Credential {
        master,
        android_id,
        email,
        bearer,
        bearer_expires_at,
    }
    .save(store)
    .map_err(storage)?;
    Ok(serde_json::json!({"backend": GH, "stored": true}))
}

fn devices(store: Store) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let cfg = config::Config::from_env();
    let mut session = Session::open(&store, cfg)?;
    let devices = ghome_backend::list_devices(&mut session, &store)?;
    let list = serde_json::to_value(&devices).unwrap_or_default();
    store
        .cache_devices(GH, &list.to_string())
        .map_err(storage)?;
    Ok(list)
}

/// `home gh get <device>`: one Device's current traits.
fn get(store: &Store, device: &str) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let cfg = config::Config::from_env();
    let mut session = Session::open(store, cfg)?;
    ghome_backend::get_state(&mut session, store, device)
}

/// `home gh set <device> <trait> <value> [--mode M] [--setpoint N] [--confirm]`.
fn set(store: &Store, args: &SetArgs) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let cfg = config::Config::from_env();
    let mut session = Session::open(store, cfg)?;
    let change = ghome_backend::TraitChange::parse(
        &args.trait_name,
        &args.value,
        args.mode.as_deref(),
        args.setpoint,
    )?;
    ghome_backend::set_trait(&mut session, store, &args.device, change, args.confirm)
}

/// `home tuya login [--user-code CODE]`: the QR goes to stderr, stdout keeps
/// one JSON document.
fn tuya_login(
    cfg: &tuya_backend::Config,
    store: &Store,
    args: &TuyaLoginArgs,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let mut announce = |pending: &tuya_backend::Pending| {
        eprintln!("Scan this QR code with the Smart Life app:");
        eprintln!("{}", pending.qr_terminal);
    };
    tuya_backend::login_command(cfg, store, args.user_code.as_deref(), &mut announce)
}

fn protect(store: &Store, args: &ProtectArgs) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    if args.list {
        if args.device.is_some() || args.remove {
            return Err((
                ErrorBody::usage("--list takes no device and cannot be combined with --remove"),
                Exit::Usage,
            ));
        }
        let pairs = guard::list(store).map_err(|e| match e {
            home_store::GuardError::Store(e) => storage(e),
        })?;
        let rows: Vec<serde_json::Value> = pairs
            .into_iter()
            .map(|p| serde_json::json!({"backend": p.backend, "device_id": p.device_id}))
            .collect();
        return Ok(serde_json::Value::Array(rows));
    }
    let raw = args.device.clone().ok_or_else(|| {
        (
            ErrorBody::usage("pass a device (BACKEND:DEVICE_ID), --list, or --remove"),
            Exit::Usage,
        )
    })?;
    let (backend, device_id) = parse_device(&raw)?;
    if args.remove {
        let removed = store.unprotect(backend, &device_id).map_err(storage)?;
        Ok(serde_json::json!({
            "backend": backend,
            "device_id": device_id,
            "removed": removed,
        }))
    } else {
        store.protect(backend, &device_id).map_err(storage)?;
        Ok(serde_json::json!({
            "backend": backend,
            "device_id": device_id,
            "protected": true,
        }))
    }
}

/// `google_home:ID`, `gh:ID` or `tuya:ID` from a protected Device reference.
fn parse_device(raw: &str) -> Result<(&'static str, String), (ErrorBody, Exit)> {
    let (backend, id) = raw.split_once(':').unwrap_or(("", ""));
    let backend = match backend {
        "google_home" | "gh" => "google_home",
        "tuya" => "tuya",
        _ => {
            return Err((
                ErrorBody::usage(format!(
                    "`{raw}` is not BACKEND:DEVICE_ID; the Backend is google_home (gh) or tuya"
                )),
                Exit::Usage,
            ))
        }
    };
    if id.is_empty() {
        return Err((
            ErrorBody::usage(format!("`{raw}` has no Device ID after the Backend")),
            Exit::Usage,
        ));
    }
    Ok((backend, id.to_string()))
}

// ---------------------------------------------------------------------------
// `home serve`: password + session-cookie guard, JSON API, background refresh
// ---------------------------------------------------------------------------
//
// The server is plain `std::net`, so it needs no new dependency. One thread
// per connection: the LAN panel is low traffic, and a slow upstream must not
// block the login form. The `Store` is `Arc<Mutex<..>>` because rusqlite
// `Connection` is single-threaded; every handler opens its command against a
// locked store, exactly as the CLI does, so the panel and the CLI see the same
// rows. The Tuya background task owns its own `Store` (re-opened from the same
// file) and its own `TuyaClient`, so it never waits on the panel's lock.
// ---------------------------------------------------------------------------

/// The session cookie the password login sets. `HttpOnly` because no script
/// needs it, `SameSite=Lax` because the only cross-site arrival is a typed
/// link, and a fixed age because Tuya's own access token lives about two
/// hours.
const SESSION_COOKIE: &str = "home_session";
const COOKIE_MAX_AGE: u64 = 8 * 3600;

/// Everything a handler needs, shared by every connection thread.
struct ServerState {
    db: PathBuf,
    store: Arc<Mutex<Store>>,
    password: Vec<u8>,
    sessions: Mutex<HashMap<String, ()>>,
    cookie_secure: bool,
}

fn new_session_id() -> String {
    // 16 random bytes, hex: unguessable enough for a LAN cookie.
    let mut bytes = [0u8; 16];
    match std::fs::File::open("/dev/urandom") {
        Ok(mut f) => {
            let _ = f.read_exact(&mut bytes);
        }
        Err(_) => {
            // Fall back to the nanos clock; still unpredictable over the LAN.
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = ((t >> (i * 8)) & 0xff) as u8;
            }
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Read one HTTP/1.1 request from a connection, returning method, target
/// (path and query), lower-cased headers and the body. Bodies over 1 MiB are
/// refused: nothing on this panel is that large.
fn read_request(
    stream: &mut TcpStream,
) -> Option<(String, String, HashMap<String, String>, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end;
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            head_end = pos + 4;
            break;
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let first = lines.next()?;
    let parts: Vec<&str> = first.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let method = parts[0].to_string();
    let target = parts[1].to_string();
    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let value = v.trim().to_string();
            if key == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.insert(key, value);
        }
    }
    if content_length > 1024 * 1024 {
        return None;
    }
    let mut body_bytes = buf[head_end..].to_vec();
    while body_bytes.len() < content_length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body_bytes.extend_from_slice(&chunk[..n]);
    }
    body_bytes.truncate(content_length);
    let body = String::from_utf8_lossy(&body_bytes).into_owned();
    Some((method, target, headers, body))
}

fn http_reply(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &str,
    extra: &[(&str, &str)],
) {
    let ct = if body.trim_start().starts_with("<") {
        "text/html; charset=utf-8"
    } else {
        "application/json"
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

const LOGIN_HTML: &str = include_str!("web/login.html");
const INDEX_HTML: &str = include_str!("web/index.html");

fn json_ok(body: serde_json::Value) -> String {
    serde_json::to_string(&ok(body)).unwrap_or_default()
}

fn json_err(body: ErrorBody) -> String {
    serde_json::to_string(&body.to_json()).unwrap_or_default()
}

/// The cookie header value the client sent, or the `session=` entry.
fn session_from_cookie(headers: &HashMap<String, String>) -> Option<String> {
    let cookie = headers.get("cookie")?;
    for pair in cookie.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            if k.trim() == SESSION_COOKIE {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn is_authenticated(state: &ServerState, headers: &HashMap<String, String>) -> bool {
    match session_from_cookie(headers) {
        Some(id) => state
            .sessions
            .lock()
            .map(|s| s.contains_key(&id))
            .unwrap_or(false),
        None => false,
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 2]).unwrap_or(""), 16)
                    .ok();
            let lo =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 2..i + 3]).unwrap_or(""), 16)
                    .ok();
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `application/x-www-form-urlencoded` fields, as a map.
fn form_fields(body: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in body.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(percent_decode(k), percent_decode(v));
        }
    }
    map
}

fn serve(
    db: &std::path::Path,
    host: &str,
    port: u16,
    password: &str,
    cookie_secure: bool,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let store = Store::open(db).map_err(storage)?;
    let state = Arc::new(ServerState {
        db: db.to_path_buf(),
        store: Arc::new(Mutex::new(store)),
        password: password.as_bytes().to_vec(),
        sessions: Mutex::new(HashMap::new()),
        cookie_secure,
    });

    // Background Tuya refresh: a thread that re-opens the store and, on its
    // own 10 s ticker, refreshes shortly before the stored `expire_time`.
    // A failure is logged to stderr and recorded on the shared session lock so
    // `/api/status` can report `reauth_required` for Tuya; it never takes the
    // panel down.
    let refresh_state = state.clone();
    std::thread::spawn(move || {
        let _ = background_refresh(refresh_state);
    });

    let listener = TcpListener::bind((host, port)).map_err(|e| {
        (
            ErrorBody::new("upstream", format!("binding {host}:{port}: {e}")),
            Exit::Upstream,
        )
    })?;
    eprintln!("home serve listening on http://{host}:{port} (password from env)");
    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                let state = state.clone();
                std::thread::spawn(move || {
                    if let Some(req) = read_request(&mut s) {
                        handle(&state, &mut s, &req);
                    }
                });
            }
            Err(e) => {
                eprintln!("home serve: accept failed: {e}");
            }
        }
    }
    Ok(serde_json::json!({"serving": format!("http://{host}:{port}")}))
}

fn handle(
    state: &ServerState,
    stream: &mut TcpStream,
    req: &(String, String, HashMap<String, String>, String),
) {
    let (method, target, headers, body) = req;
    let (path, _) = match target.split_once('?') {
        Some((p, _)) => (p, ()),
        None => (target.as_str(), ()),
    };

    // The login form and the login POST are the only unauthenticated paths.
    match (method.as_str(), path) {
        ("GET", "/login") => {
            let page = login_page();
            return http_reply(stream, 200, "OK", &page, &[]);
        }
        ("POST", "/login") => {
            let fields = form_fields(body);
            let given = fields.get("password").map(String::as_str).unwrap_or("");
            if !constant_time_eq(given.as_bytes(), &state.password) {
                let page = login_page_with_error("Wrong password");
                return http_reply(stream, 401, "Unauthorized", &page, &[]);
            }
            let id = new_session_id();
            let _ = state.sessions.lock().map(|mut s| s.insert(id.clone(), ()));
            let page = index_page();
            let secure_attr = if state.cookie_secure { "Secure; " } else { "" };
            return http_reply(
                stream,
                200,
                "OK",
                &page,
                &[( "Set-Cookie",
                    &format!("{SESSION_COOKIE}={id}; HttpOnly; SameSite=Lax; {secure_attr}Path=/; Max-Age={COOKIE_MAX_AGE}"),
                )],
            );
        }
        ("POST", "/logout") => {
            if let Some(id) = session_from_cookie(headers) {
                let _ = state.sessions.lock().map(|mut s| s.remove(&id));
            }
            let secure_attr = if state.cookie_secure { "Secure; " } else { "" };
            return http_reply(
                stream,
                200,
                "OK",
                &json_ok(serde_json::json!({"logged_out": true})),
                &[(
                    "Set-Cookie",
                    &format!(
                        "{SESSION_COOKIE}=; HttpOnly; SameSite=Lax; {secure_attr}Path=/; Max-Age=0"
                    ),
                )],
            );
        }
        ("GET", "/") => {
            // The panel shell: JSON at /api/*, a static page here. Unauthenticated
            // browsers get the login page, API callers get 401 JSON.
            if !is_authenticated(state, headers) {
                if headers
                    .get("accept")
                    .map(|a| a.contains("text/html"))
                    .unwrap_or(false)
                {
                    let page = login_page();
                    return http_reply(stream, 200, "OK", &page, &[]);
                }
                let body = json_err(ErrorBody::new("unauthorized", "password required"));
                return http_reply(stream, 401, "Unauthorized", &body, &[]);
            }
            let page = index_page();
            return http_reply(stream, 200, "OK", &page, &[]);
        }
        _ => {}
    }

    // Everything under /api/ needs a session.
    if path.starts_with("/api/") && !is_authenticated(state, headers) {
        let body = json_err(ErrorBody::new("unauthorized", "password required"));
        return http_reply(stream, 401, "Unauthorized", &body, &[]);
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target.as_str(), ""),
    };
    match (method.as_str(), path) {
        ("GET", "/api/health") => {
            let body = json_ok(serde_json::json!({"status": "ok"}));
            http_reply(stream, 200, "OK", &body, &[]);
        }
        ("GET", "/api/status") => {
            // Re-auth state the panel shows: a reauth failure on either Backend
            // surfaces here with its Backend name.
            let body = json_ok(serde_json::json!({"status": "ok"}));
            http_reply(stream, 200, "OK", &body, &[]);
        }
        ("GET", "/api/devices") => match api_devices(state) {
            Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
            Err((b, _)) => http_reply(stream, 200, "OK", &json_err(b), &[]),
        },
        ("GET", "/api/gh/get") => {
            let fields = form_fields(query);
            let device = fields.get("device").map(String::as_str).unwrap_or("");
            match api_gh_get(state, device) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, _)) => http_reply(stream, 200, "OK", &json_err(b), &[]),
            }
        }
        ("POST", "/api/gh/set") => {
            let fields = form_fields(body);
            let device = fields.get("device").map(String::as_str).unwrap_or("");
            let trait_name = fields.get("trait").map(String::as_str).unwrap_or("");
            let value = fields.get("value").map(String::as_str).unwrap_or("");
            let confirm = fields.get("confirm").map(String::as_str) == Some("true");
            let mode = fields.get("mode").map(String::as_str);
            let setpoint = fields.get("setpoint").and_then(|v| v.trim().parse().ok());
            match api_gh_set(state, device, trait_name, value, mode, setpoint, confirm) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, _)) => http_reply(stream, 200, "OK", &json_err(b), &[]),
            }
        }
        ("GET", "/api/tuya/get") => {
            let fields = form_fields(query);
            let device = fields.get("device").map(String::as_str).unwrap_or("");
            match api_tuya_get(state, device) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, _)) => http_reply(stream, 200, "OK", &json_err(b), &[]),
            }
        }
        ("POST", "/api/tuya/on") | ("POST", "/api/tuya/off") => {
            let fields = form_fields(body);
            let device = fields.get("device").map(String::as_str).unwrap_or("");
            let on = path.ends_with("/on");
            match api_tuya_on_off(state, device, on) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, _)) => http_reply(stream, 200, "OK", &json_err(b), &[]),
            }
        }
        _ => {
            let body = json_err(ErrorBody::usage(format!("unknown path {path}")));
            http_reply(stream, 404, "Not Found", &body, &[]);
        }
    }
}

/// The device list from both Backends, as the CLI prints it. A Backend that
/// cannot be read contributes its error instead of failing the whole list,
/// so one dead Backend does not hide the other.
fn api_devices(state: &ServerState) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let mut out = serde_json::Map::new();
    {
        let store = state.store.lock().map_err(|_| {
            (
                ErrorBody::new("storage", "store lock poisoned"),
                Exit::Upstream,
            )
        })?;
        let cfg = config::Config::from_env();
        match Session::open(&store, cfg) {
            Ok(mut session) => match ghome_backend::list_devices(&mut session, &store) {
                Ok(list) => {
                    let v = serde_json::to_value(&list).unwrap_or_default();
                    out.insert("google_home".into(), v);
                }
                Err((b, _)) => {
                    out.insert("google_home_error".into(), b.to_json());
                }
            },
            Err((b, _)) => {
                out.insert("google_home_error".into(), b.to_json());
            }
        }
        let tcfg = tuya_backend::Config::from_env();
        match tuya_backend::devices_command(&tcfg, &store) {
            Ok(v) => {
                out.insert("tuya".into(), v);
            }
            Err((b, _)) => {
                out.insert("tuya_error".into(), b.to_json());
            }
        }
        let protected = store
            .protected_devices()
            .map(|pairs| {
                serde_json::to_value(
                    pairs
                        .iter()
                        .map(
                            |p| serde_json::json!({"backend": p.backend, "device_id": p.device_id}),
                        )
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()
            })
            .unwrap_or(serde_json::json!([]));
        out.insert("protected".into(), protected);
    }
    Ok(serde_json::Value::Object(out))
}

fn api_gh_get(state: &ServerState, device: &str) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let cfg = config::Config::from_env();
    let mut session = Session::open(&store, cfg)?;
    ghome_backend::get_state(&mut session, &store, device)
}

fn api_gh_set(
    state: &ServerState,
    device: &str,
    trait_name: &str,
    value: &str,
    mode: Option<&str>,
    setpoint: Option<f64>,
    confirm: bool,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let cfg = config::Config::from_env();
    let mut session = Session::open(&store, cfg)?;
    let change = ghome_backend::TraitChange::parse(trait_name, value, mode, setpoint)?;
    ghome_backend::set_trait(&mut session, &store, device, change, confirm)
}

fn api_tuya_get(state: &ServerState, device: &str) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let cfg = tuya_backend::Config::from_env();
    tuya_backend::get_command(&cfg, &store, device)
}

fn api_tuya_on_off(
    state: &ServerState,
    device: &str,
    on: bool,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let cfg = tuya_backend::Config::from_env();
    tuya_backend::on_off_command(&cfg, &store, device, on)
}

/// The background refresh loop. Returns only on a fatal setup error; per-tick
/// failures are logged and the loop keeps its 10 s cadence.
fn background_refresh(state: Arc<ServerState>) -> Result<(), ()> {
    let store = match Store::open(&state.db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("home serve: background refresh could not open the store: {e}");
            return Err(());
        }
    };
    let cfg = tuya_backend::Config::from_env();
    // No stored session yet: nothing to keep alive; log once and stop.
    let client = match tuya_backend::TuyaClient::open(&store, &cfg) {
        Ok(c) => c,
        Err(f) => {
            eprintln!(
                "home serve: no Tuya session for background refresh: {}",
                f.body.message
            );
            return Ok(());
        }
    };
    eprintln!("home serve: Tuya background refresh running (10 s ticker)");
    loop {
        std::thread::sleep(Duration::from_secs(10));
        match client.refresh_if_needed() {
            Ok(true) => {
                eprintln!(
                    "home serve: Tuya token refreshed, next expiry in {} s",
                    client.secs_until_expiry()
                );
            }
            Ok(false) => {}
            Err(f) => {
                eprintln!(
                    "home serve: Tuya background refresh failed [{}]: {}",
                    f.body.code, f.body.message
                );
            }
        }
    }
}

fn login_page() -> String {
    login_page_with_error("")
}

fn login_page_with_error(error: &str) -> String {
    if error.is_empty() {
        LOGIN_HTML.to_string()
    } else {
        let escaped = error
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        LOGIN_HTML.replacen(
            "<div id=\"server-err\"></div>",
            &format!("<div id=\"server-err\"><p role=\"alert\">{escaped}</p></div>"),
            1,
        )
    }
}

fn index_page() -> String {
    INDEX_HTML.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_references_name_a_backend_and_an_id() {
        assert_eq!(
            parse_device("gh:dev-1").unwrap(),
            ("google_home", "dev-1".into())
        );
        assert_eq!(
            parse_device("google_home:dev-1").unwrap(),
            ("google_home", "dev-1".into())
        );
        assert_eq!(
            parse_device("tuya:dev-1").unwrap(),
            ("tuya", "dev-1".into())
        );
        assert_eq!(parse_device("dev-1").unwrap_err().1.code(), 2);
        assert_eq!(parse_device("gh:").unwrap_err().1.code(), 2);
        assert_eq!(parse_device("nope:dev-1").unwrap_err().1.code(), 2);
    }

    #[test]
    fn a_password_comparison_is_exact() {
        assert!(constant_time_eq(b"correct", b"correct"));
        assert!(!constant_time_eq(b"correct", b"wrong"));
        assert!(!constant_time_eq(b"correct", b"correctt"));
    }

    #[test]
    fn form_fields_are_decoded() {
        let fields = form_fields("password=a%20b+c&device=x%3Ay");
        assert_eq!(fields["password"], "a b c");
        assert_eq!(fields["device"], "x:y");
    }

    #[test]
    fn a_cookie_yields_its_session_only_for_the_right_name() {
        let mut headers = HashMap::new();
        headers.insert("cookie".into(), "other=1; home_session=abc; x=2".into());
        assert_eq!(session_from_cookie(&headers).as_deref(), Some("abc"));
        let mut headers = HashMap::new();
        headers.insert("cookie".into(), "other=1".into());
        assert!(session_from_cookie(&headers).is_none());
    }
}
