//! `home` — one binary for both Backends. stdout is always exactly one JSON
//! document and the exit code follows the contract in `home_store::contract`.

use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Read, Write};
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
    /// Store a Google Home master token or oauth_token: `--token-file`,
    /// `HOME_GH_TOKEN`, or one line from stdin.
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
    /// One-time override for a protected Device; does not remove protection.
    #[arg(long)]
    force: bool,
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
        /// One-time override for a protected Device; does not remove protection.
        #[arg(long)]
        force: bool,
    },
    /// Turn a Tuya Device off.
    Off {
        /// Device ID or name (case-insensitive).
        device: String,
        /// One-time override for a protected Device; does not remove protection.
        #[arg(long)]
        force: bool,
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
        /// One-time override for a protected Device; does not remove protection.
        #[arg(long)]
        force: bool,
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
    /// Read the token from this file instead of stdin (`-` means stdin).
    #[arg(long, value_name = "PATH")]
    token_file: Option<String>,
}

/// Printed when stdin is a terminal and no fallback is set. The URL and the
/// cookie name match the reference guide in `tmp/ghome-src`.
const LOGIN_HELP: &str = "\
How to get a Google Home token:
  Option 1, oauth_token:
    1. Open https://accounts.google.com/EmbeddedSetup in a desktop browser and
       sign in with the Google account that owns your home.
    2. After the \"I agree\" page, open DevTools > Application/Storage > Cookies
       for accounts.google.com and copy the value of the `oauth_token` cookie.
       It starts with oauth2_4/ and is single-use, so paste it right away.
  Option 2, master token: paste an existing token that starts with aas_et/.
Paste the token and press Enter (Ctrl-C to cancel):
Without a terminal, set HOME_GH_TOKEN or pass --token-file <PATH> (--token-file - reads stdin).
";

/// `--token-file`, then `HOME_GH_TOKEN`, then stdin. Active stdin is read one
/// line at a time, so an open terminal or `docker exec -i` does not hang it.
fn login_token(token_file: Option<&str>) -> Result<String, (ErrorBody, Exit)> {
    let raw = match token_file {
        Some("-") => read_stdin_token()?,
        Some(path) => std::fs::read_to_string(path).map_err(|e| {
            (
                ErrorBody::usage(format!("reading token file {path}: {e}")),
                Exit::Usage,
            )
        })?,
        None => match std::env::var("HOME_GH_TOKEN") {
            Ok(token) if !token.trim().is_empty() => token,
            _ => read_stdin_token()?,
        },
    };
    let token = raw.trim().to_string();
    if token.is_empty() {
        return Err((
            ErrorBody::usage("expected a master token (aas_et/…) or an oauth_token (oauth2_4/…)"),
            Exit::Usage,
        ));
    }
    Ok(token)
}

fn read_stdin_token() -> Result<String, (ErrorBody, Exit)> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprint!("{LOGIN_HELP}");
    }
    let mut token = String::new();
    stdin
        .lock()
        .read_line(&mut token)
        .map_err(|e| (ErrorBody::usage(format!("reading stdin: {e}")), Exit::Usage))?;
    Ok(token)
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
                TuyaCmd::On { device, force } => {
                    tuya_backend::on_off_command(&cfg, &store, device, true, *force)
                }
                TuyaCmd::Off { device, force } => {
                    tuya_backend::on_off_command(&cfg, &store, device, false, *force)
                }
                TuyaCmd::Get { device } => tuya_backend::get_command(&cfg, &store, device),
                TuyaCmd::Set {
                    device,
                    code,
                    value,
                    force,
                } => tuya_backend::set_command(&cfg, &store, device, code, value, *force),
            }
        }
        Command::Protect(args) => protect(&store, args),
        Command::Serve(args) => {
            let password = match &args.password_file {
                Some(path) => std::fs::read_to_string(path)
                    .map(|s| s.trim_end_matches(['\r', '\n']).to_string())
                    .map_err(|e| {
                        (
                            ErrorBody::usage(format!("reading password file {path}: {e}")),
                            Exit::Usage,
                        )
                    })?,
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
    let token = login_token(args.token_file.as_deref())?;
    // The Android id is bound to the master token and every Bearer minted
    // from it, so a second login keeps the stored one; only an explicit
    // `--android-id` replaces it. The token is not verified here: a bad one
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
    ghome_backend::set_trait(
        &mut session,
        store,
        &args.device,
        change,
        args.confirm,
        args.force,
    )
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
/// needs it, `SameSite=Strict` because nothing here is ever reached from
/// another site, and a fixed age because Tuya's own access token lives about
/// two hours.
const SESSION_COOKIE: &str = "home_session";
const COOKIE_MAX_AGE: u64 = 8 * 3600;

/// Every response, page or JSON: no framing, no sniffing, no cache, no
/// referrer. The panel is a single-user control surface, so nothing it
/// returns belongs in a proxy or inside another page.
const SECURITY_HEADERS: &[(&str, &str)] = &[
    ("X-Content-Type-Options", "nosniff"),
    ("X-Frame-Options", "DENY"),
    (
        "Content-Security-Policy",
        "frame-ancestors 'none'; base-uri 'none'",
    ),
    ("Referrer-Policy", "no-referrer"),
    ("Cache-Control", "no-store"),
];

/// Failed-password throttle shared by `/login` and the protected-override
/// password check. `Instant` gives the sub-second precision the backoff needs.
struct AuthThrottle {
    failures: u32,
    blocked_until: Option<std::time::Instant>,
}

impl AuthThrottle {
    fn new() -> Self {
        Self {
            failures: 0,
            blocked_until: None,
        }
    }

    /// Check the backoff FIRST, so a right password during it neither
    /// succeeds nor says anything. `Ok(true)` is a match (counter reset),
    /// `Ok(false)` a mismatch, `Err(secs)` the remaining wait.
    fn verify(&mut self, given: &[u8], password: &[u8]) -> Result<bool, u64> {
        let now = std::time::Instant::now();
        if let Some(until) = self.blocked_until {
            if now < until {
                let wait = until - now;
                return Err(wait.as_secs() + u64::from(wait.subsec_nanos() > 0));
            }
        }
        if constant_time_eq(given, password) {
            self.failures = 0;
            self.blocked_until = None;
            return Ok(true);
        }
        self.failures += 1;
        if self.failures >= 3 {
            let shift = (self.failures - 3).min(5);
            self.blocked_until = Some(now + Duration::from_secs(2 << shift));
        }
        Ok(false)
    }
}

/// Everything a handler needs, shared by every connection thread.
struct ServerState {
    store: Arc<Mutex<Store>>,
    password: Vec<u8>,
    /// One counter for every password check on this panel.
    auth_throttle: Mutex<AuthThrottle>,
    /// Session id -> expiry (Unix seconds). The browser's cookie carries the
    /// same age; keeping it here too means a captured id dies on schedule
    /// instead of living until the process restarts.
    sessions: Mutex<HashMap<String, u64>>,
    cookie_secure: bool,
    tuya_reauth: Mutex<Option<String>>,
    /// QR login in flight: `token -> user_code`, so a poll can fall back on
    /// the code the start used without the form repeating it. Bounded in the
    /// start handler so stale tokens cannot grow without limit.
    pending_tuya_qr: Mutex<HashMap<String, String>>,
}

/// One password check against the panel password, through the shared throttle.
fn verify_password(state: &ServerState, given: &[u8]) -> Result<bool, u64> {
    match state.auth_throttle.lock() {
        Ok(mut throttle) => throttle.verify(given, &state.password),
        // A poisoned lock is a programming error, not a password answer.
        Err(_) => Err(1),
    }
}

/// The `force` flag for one API write, from the optional POST-form
/// `password`. Absent means no override (`Some(false)`). A mismatch writes
/// 403 `protected` with zero upstream calls, a blocked throttle writes 429,
/// and both return `None`. A right password applies to this call only and is
/// never stored as a token.
fn override_force(
    state: &ServerState,
    fields: &HashMap<String, String>,
    stream: &mut TcpStream,
) -> Option<bool> {
    let Some(password) = fields.get("password") else {
        return Some(false);
    };
    match verify_password(state, password.as_bytes()) {
        Ok(true) => Some(true),
        Ok(false) => {
            let body = json_err(ErrorBody::new(
                "protected",
                "a protected Device needs its override password to be written",
            ));
            http_reply(stream, 403, "Forbidden", &body, &[]);
            None
        }
        Err(wait) => {
            let body = json_err(ErrorBody::new(
                "rate_limited",
                "too many failed password attempts; try again later",
            ));
            let retry = wait.to_string();
            http_reply(
                stream,
                429,
                "Too Many Requests",
                &body,
                &[("Retry-After", &retry)],
            );
            None
        }
    }
}

fn new_session_id() -> String {
    // 32 bytes from the OS CSPRNG, hex: unguessable even if the panel is
    // reached from off the LAN. getrandom is already a dependency and works
    // on Windows, where `/dev/urandom` does not exist.
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        // No OS randomness: refuse to mint a guessable session rather than
        // fall back on the clock.
        eprintln!("home serve: no OS randomness for a session id");
        return String::new();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Compare past the shorter input and fold the length in, so the timing
    // does not say how long the password is.
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
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
    for (k, v) in SECURITY_HEADERS {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
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

fn exit_http(exit: Exit) -> (u16, &'static str) {
    match exit {
        Exit::Ok => (200, "OK"),
        Exit::Usage => (400, "Bad Request"),
        Exit::Ambiguous => (409, "Conflict"),
        Exit::Offline => (503, "Service Unavailable"),
        Exit::ReauthRequired => (424, "Failed Dependency"),
        Exit::Refused => (403, "Forbidden"),
        Exit::Upstream => (502, "Bad Gateway"),
    }
}

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
    let Some(id) = session_from_cookie(headers) else {
        return false;
    };
    let now = home_store::store::now_unix() as u64;
    let Ok(mut sessions) = state.sessions.lock() else {
        return false;
    };
    match sessions.get(&id) {
        Some(expires_at) if *expires_at > now => true,
        Some(_) => {
            sessions.remove(&id);
            false
        }
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
        store: Arc::new(Mutex::new(store)),
        password: password.as_bytes().to_vec(),
        auth_throttle: Mutex::new(AuthThrottle::new()),
        sessions: Mutex::new(HashMap::new()),
        cookie_secure,
        tuya_reauth: Mutex::new(None),
        pending_tuya_qr: Mutex::new(HashMap::new()),
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
    // `GET /` and `GET /settings` are the same exception but only when Accept
    // asks for HTML; every other request without a session is 401 JSON below.
    if !is_authenticated(state, headers) {
        let html_root = method == "GET"
            && matches!(path, "/" | "/settings")
            && headers
                .get("accept")
                .map(|a| a.contains("text/html"))
                .unwrap_or(false);
        if !matches!(
            (method.as_str(), path),
            ("GET", "/login") | ("POST", "/login")
        ) && !html_root
        {
            let body = json_err(ErrorBody::new("unauthorized", "password required"));
            return http_reply(stream, 401, "Unauthorized", &body, &[]);
        }
    }
    match (method.as_str(), path) {
        ("GET", "/login") => {
            let page = login_page();
            return http_reply(stream, 200, "OK", &page, &[]);
        }
        ("POST", "/login") => {
            let fields = form_fields(body);
            let given = fields.get("password").map(String::as_str).unwrap_or("");
            match verify_password(state, given.as_bytes()) {
                Err(wait) => {
                    let page = login_page_with_error(
                        "rate_limited",
                        "Too many failed attempts. Wait before trying again.",
                    );
                    return http_reply(
                        stream,
                        429,
                        "Too Many Requests",
                        &page,
                        &[("Retry-After", &wait.to_string())],
                    );
                }
                Ok(false) => {
                    let page = login_page_with_error("wrong_password", "Wrong password");
                    return http_reply(stream, 401, "Unauthorized", &page, &[]);
                }
                Ok(true) => {}
            }
            let id = new_session_id();
            if id.is_empty() {
                let body = json_err(ErrorBody::new("storage", "no session id available"));
                return http_reply(stream, 500, "Internal Server Error", &body, &[]);
            }
            let expires_at = home_store::store::now_unix() as u64 + COOKIE_MAX_AGE;
            let _ = state
                .sessions
                .lock()
                .map(|mut s| s.insert(id.clone(), expires_at));
            let page = index_page();
            let secure_attr = if state.cookie_secure { "Secure; " } else { "" };
            return http_reply(
                stream,
                200,
                "OK",
                &page,
                &[( "Set-Cookie",
                    &format!("{SESSION_COOKIE}={id}; HttpOnly; SameSite=Strict; {secure_attr}Path=/; Max-Age={COOKIE_MAX_AGE}"),
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
                        "{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; {secure_attr}Path=/; Max-Age=0"
                    ),
                )],
            );
        }
        ("GET", "/") | ("GET", "/settings") => {
            if !is_authenticated(state, headers) {
                let page = login_page();
                return http_reply(stream, 200, "OK", &page, &[]);
            }
            let page = index_page();
            return http_reply(stream, 200, "OK", &page, &[]);
        }
        _ => {}
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
            // Re-auth state the panel shows: a Tuya refresh failure surfaces
            // here as tuya.status=reauth_required; the panel banners on it.
            // `tuya_user_code_configured` only says a code is available (env or
            // the stored session); it never carries the code itself.
            let reauth = state.tuya_reauth.lock().ok().and_then(|m| m.clone());
            let configured = tuya_user_code_configured(state);
            let body = match reauth {
                None => json_ok(serde_json::json!({
                    "status": "ok",
                    "tuya": {"status": "ok"},
                    "tuya_user_code_configured": configured,
                })),
                Some(msg) => json_ok(serde_json::json!({
                    "status": "degraded",
                    "tuya": {"status": "reauth_required", "message": msg},
                    "tuya_user_code_configured": configured,
                })),
            };
            http_reply(stream, 200, "OK", &body, &[]);
        }
        ("GET", "/api/devices") => match api_devices(state) {
            Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
            Err((b, exit)) => {
                let (status, reason) = exit_http(exit);
                http_reply(stream, status, reason, &json_err(b), &[])
            }
        },
        ("GET", "/api/gh/get") => {
            let fields = form_fields(query);
            let device = fields.get("device").map(String::as_str).unwrap_or("");
            match api_gh_get(state, device) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, exit)) => {
                    let (status, reason) = exit_http(exit);
                    http_reply(stream, status, reason, &json_err(b), &[])
                }
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
            let Some(force) = override_force(state, &fields, stream) else {
                return;
            };
            let change = ghome_backend::TraitChange::parse(trait_name, value, mode, setpoint);
            match change.and_then(|change| api_gh_set(state, device, change, confirm, force)) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, exit)) => {
                    let (status, reason) = exit_http(exit);
                    http_reply(stream, status, reason, &json_err(b), &[])
                }
            }
        }
        ("GET", "/api/tuya/get") => {
            let fields = form_fields(query);
            let device = fields.get("device").map(String::as_str).unwrap_or("");
            match api_tuya_get(state, device) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, exit)) => {
                    let (status, reason) = exit_http(exit);
                    http_reply(stream, status, reason, &json_err(b), &[])
                }
            }
        }
        ("POST", "/api/tuya/on") | ("POST", "/api/tuya/off") => {
            let fields = form_fields(body);
            let device = fields.get("device").map(String::as_str).unwrap_or("");
            let on = path.ends_with("/on");
            let Some(force) = override_force(state, &fields, stream) else {
                return;
            };
            match api_tuya_on_off(state, device, on, force) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, exit)) => {
                    let (status, reason) = exit_http(exit);
                    http_reply(stream, status, reason, &json_err(b), &[])
                }
            }
        }
        ("POST", "/api/tuya/login/start") => {
            // Reached only past `is_authenticated` above, so unauthenticated
            // logins are already 401 JSON before this arm runs.
            let fields = form_fields(body);
            let form_code = fields.get("user_code").map(String::as_str).unwrap_or("");
            match api_tuya_login_start(state, form_code) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, exit)) => {
                    let (status, reason) = exit_http(exit);
                    http_reply(stream, status, reason, &json_err(b), &[])
                }
            }
        }
        ("POST", "/api/tuya/login/poll") => {
            let fields = form_fields(body);
            let token = fields.get("token").map(String::as_str).unwrap_or("");
            let form_code = fields.get("user_code").map(String::as_str).unwrap_or("");
            match api_tuya_login_poll(state, token, form_code) {
                Ok(v) => http_reply(stream, 200, "OK", &json_ok(v), &[]),
                Err((b, exit)) => {
                    let (status, reason) = exit_http(exit);
                    http_reply(stream, status, reason, &json_err(b), &[])
                }
            }
        }
        _ => {
            let body = json_err(ErrorBody::usage(format!("unknown path {path}")));
            http_reply(stream, 404, "Not Found", &body, &[]);
        }
    }
}

/// Whether a Tuya user code is available at all: the env override, or the
/// stored session's own code. Never a boolean about access — only whether a
/// QR login can start without the user typing the code.
fn tuya_user_code_configured(state: &ServerState) -> bool {
    if tuya_backend::Config::from_env()
        .user_code
        .map(|c| !c.trim().is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    let Ok(store) = state.store.lock() else {
        return false;
    };
    tuya_backend::Session::load(&store)
        .ok()
        .flatten()
        .map(|s| !s.user_code.trim().is_empty())
        .unwrap_or(false)
}

/// `POST /api/tuya/login/start`: ask Tuya for a QR code and answer with the
/// token plus its SVG, stashing `token -> user_code` for the polls. The code
/// falls back form → env → stored session; with none it is a usage error
/// pointing at the Smart Life app setting.
fn api_tuya_login_start(
    state: &ServerState,
    form_code: &str,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let cfg = tuya_backend::Config::from_env();
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let form = form_code.trim().to_string();
    let env_code = cfg.user_code.clone().unwrap_or_default();
    let stored_code = tuya_backend::Session::load(&store)
        .ok()
        .flatten()
        .map(|s| s.user_code)
        .unwrap_or_default();
    let user_code = if !form.is_empty() {
        form
    } else if !env_code.trim().is_empty() {
        env_code
    } else if !stored_code.trim().is_empty() {
        stored_code
    } else {
        return Err((
            ErrorBody::usage(
                "a user code is required; find it in the Smart Life app under \
                 Me → Settings → Account and Security → User Code",
            ),
            Exit::Usage,
        ));
    };
    let pending = tuya_backend::start_login(&cfg, &user_code).map_err(|f| (f.body, f.exit))?;
    let svg = tuya_backend::qr_svg(&pending.qr_text).map_err(|f| (f.body, f.exit))?;
    let normalized = tuya_backend::login::normalize_code(&user_code);
    if let Ok(mut map) = state.pending_tuya_qr.lock() {
        // QR tokens are short-lived; beyond 32 pending ones the old ones are
        // stale rounds no one will poll, so they go first.
        if map.len() >= 32 {
            map.clear();
        }
        map.insert(pending.token.clone(), normalized);
    }
    Ok(serde_json::json!({"token": pending.token, "qr_svg": svg}))
}

/// `POST /api/tuya/login/poll`: one check whether the QR was scanned.
/// `Ok(None)` means still waiting; `Ok(Some(session))` is persisted, the
/// reauth flag cleared and the Local Keys backed up best-effort, exactly as
/// `tuya_backend::login_command` does after a CLI login.
fn api_tuya_login_poll(
    state: &ServerState,
    token: &str,
    form_code: &str,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err((ErrorBody::usage("token is required"), Exit::Usage));
    }
    let cfg = tuya_backend::Config::from_env();
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let form = form_code.trim().to_string();
    let pending_code = state
        .pending_tuya_qr
        .lock()
        .ok()
        .and_then(|m| m.get(&token).cloned())
        .unwrap_or_default();
    let env_code = cfg.user_code.clone().unwrap_or_default();
    let stored_code = tuya_backend::Session::load(&store)
        .ok()
        .flatten()
        .map(|s| s.user_code)
        .unwrap_or_default();
    let user_code = if !form.is_empty() {
        form
    } else if !pending_code.is_empty() {
        pending_code
    } else if !env_code.trim().is_empty() {
        env_code
    } else {
        stored_code
    };
    if user_code.trim().is_empty() {
        return Err((
            ErrorBody::usage(
                "a user code is required; find it in the Smart Life app under \
                 Me → Settings → Account and Security → User Code",
            ),
            Exit::Usage,
        ));
    }
    match tuya_backend::poll_login(&cfg, &token, &user_code) {
        Ok(None) => Ok(serde_json::json!({"status": "pending"})),
        Ok(Some(session)) => {
            session.save(&store).map_err(storage)?;
            if let Ok(mut map) = state.pending_tuya_qr.lock() {
                map.remove(&token);
            }
            if let Ok(mut m) = state.tuya_reauth.lock() {
                *m = None;
            }
            // Same best-effort backup as the CLI login: a failure here is
            // reported on stderr and never undoes a completed sign-in.
            if let Err(failure) =
                tuya_backend::TuyaClient::with_session(session.clone(), &cfg, &store)
                    .and_then(|client| tuya_backend::devices::list_devices(&client))
            {
                eprintln!(
                    "signed in, but fetching Local Keys failed: {}",
                    failure.body.message
                );
            }
            Ok(serde_json::json!({
                "status": "authenticated",
                "backend": tuya_backend::BACKEND,
                "uid": session.uid,
                "endpoint": session.endpoint,
                "expires_at": session.expire_time,
            }))
        }
        Err(f) => Err((f.body, f.exit)),
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
                if let Ok(mut m) = state.tuya_reauth.lock() {
                    *m = None;
                }
            }
            Err((b, exit)) => {
                if exit == Exit::ReauthRequired {
                    if let Ok(mut m) = state.tuya_reauth.lock() {
                        *m = Some(b.message.clone());
                    }
                }
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
    change: ghome_backend::TraitChange,
    confirm: bool,
    force: bool,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let cfg = config::Config::from_env();
    let mut session = Session::open(&store, cfg)?;
    ghome_backend::set_trait(&mut session, &store, device, change, confirm, force)
}

fn api_tuya_get(state: &ServerState, device: &str) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let cfg = tuya_backend::Config::from_env();
    let result = tuya_backend::get_command(&cfg, &store, device);
    sync_tuya_reauth(state, &result);
    result
}

fn api_tuya_on_off(
    state: &ServerState,
    device: &str,
    on: bool,
    force: bool,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let store = state.store.lock().map_err(|_| {
        (
            ErrorBody::new("storage", "store lock poisoned"),
            Exit::Upstream,
        )
    })?;
    let cfg = tuya_backend::Config::from_env();
    let result = tuya_backend::on_off_command(&cfg, &store, device, on, force);
    sync_tuya_reauth(state, &result);
    result
}

fn sync_tuya_reauth(state: &ServerState, result: &Result<serde_json::Value, (ErrorBody, Exit)>) {
    match result {
        Ok(_) => {
            if let Ok(mut m) = state.tuya_reauth.lock() {
                *m = None;
            }
        }
        Err((body, Exit::ReauthRequired)) => {
            if let Ok(mut m) = state.tuya_reauth.lock() {
                *m = Some(body.message.clone());
            }
        }
        _ => {}
    }
}

/// The background refresh loop. Uses the panel's own `Store` behind the same
/// serial lock every handler uses, so a refresh can never sign with a token
/// another request just rotated away, and a rotated token is persisted before
/// anyone else uses it. `HOME_TUYA_REFRESH_POLL_MS` (default 10000, min 20)
/// sets the tick; `Ok(None)` (no Tuya session yet) just waits for an outside
/// login instead of marking the backend reauth.
fn background_refresh(state: Arc<ServerState>) -> Result<(), ()> {
    let poll_ms = std::env::var("HOME_TUYA_REFRESH_POLL_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(10_000)
        .max(20);
    eprintln!("home serve: Tuya background refresh running ({poll_ms} ms ticker)");
    loop {
        std::thread::sleep(Duration::from_millis(poll_ms));
        let store = match state.store.lock() {
            Ok(guard) => guard,
            Err(e) => {
                eprintln!("home serve: background refresh lock poisoned: {e}");
                continue;
            }
        };
        let loaded = tuya_backend::Session::load(&store);
        let session = match loaded {
            Ok(None) => continue,
            Ok(Some(s)) => s,
            Err(e) => {
                eprintln!("home serve: background refresh could not load session: {e}");
                if let Ok(mut m) = state.tuya_reauth.lock() {
                    *m = Some(e.to_string());
                }
                continue;
            }
        };
        let cfg = tuya_backend::Config::from_env();
        let client = match tuya_backend::TuyaClient::with_session(session, &cfg, &store) {
            Ok(c) => c,
            Err(f) => {
                eprintln!(
                    "home serve: background refresh open failed: {}",
                    f.body.message
                );
                if let Ok(mut m) = state.tuya_reauth.lock() {
                    *m = Some(f.body.message.clone());
                }
                continue;
            }
        };
        match client.refresh_if_needed() {
            Ok(true) => {
                if let Ok(mut m) = state.tuya_reauth.lock() {
                    *m = None;
                }
                eprintln!(
                    "home serve: Tuya token refreshed, next expiry in {} s",
                    client.secs_until_expiry()
                );
            }
            Ok(false) => {
                if let Ok(mut m) = state.tuya_reauth.lock() {
                    *m = None;
                }
            }
            Err(f) => {
                eprintln!(
                    "home serve: Tuya background refresh failed [{}]: {}",
                    f.body.code, f.body.message
                );
                if let Ok(mut m) = state.tuya_reauth.lock() {
                    *m = Some(f.body.message.clone());
                }
            }
        }
    }
}

fn login_page() -> String {
    login_page_with_error("", "")
}

/// `code` is the stable, localizable token the page carries in `data-code`;
/// `error` is the English fallback text shown when no translation applies.
fn login_page_with_error(code: &str, error: &str) -> String {
    if error.is_empty() {
        LOGIN_HTML.to_string()
    } else {
        let escaped = error
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let code = code
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        LOGIN_HTML.replacen(
            "<div id=\"server-err\"></div>",
            &format!(
                "<div id=\"server-err\"><p role=\"alert\" data-code=\"{code}\">{escaped}</p></div>"
            ),
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
    fn the_login_guidance_names_the_url_the_cookie_and_both_prefixes() {
        assert!(LOGIN_HELP.contains("https://accounts.google.com/EmbeddedSetup"));
        assert!(LOGIN_HELP.contains("oauth_token"));
        assert!(LOGIN_HELP.contains("oauth2_4/"));
        assert!(LOGIN_HELP.contains("aas_et/"));
        assert!(LOGIN_HELP.contains("HOME_GH_TOKEN"));
        assert!(LOGIN_HELP.contains("--token-file"));
        assert!(LOGIN_HELP.contains("Paste the token and press Enter"));
    }

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
    fn the_throttle_blocks_before_comparing_and_grows_the_wait() {
        let mut t = AuthThrottle::new();
        for _ in 0..2 {
            assert_eq!(t.verify(b"wrong", b"pw"), Ok(false));
        }
        // Third failure blocks for 2 s; a right password is refused meanwhile.
        assert_eq!(t.verify(b"wrong", b"pw"), Ok(false));
        assert!(t.verify(b"pw", b"pw").unwrap_err() >= 1);
        assert!(t.verify(b"pw", b"pw").is_err(), "still blocked");
        t.blocked_until = None;
        // Fifth failure: 2 << 2 = 8 s.
        t.failures = 4;
        assert_eq!(t.verify(b"wrong", b"pw"), Ok(false));
        let wait = t.verify(b"pw", b"pw").unwrap_err();
        assert!((7..=8).contains(&wait), "wait was {wait}");
        // A match after the backoff resets the counter.
        t.blocked_until = None;
        assert_eq!(t.verify(b"pw", b"pw"), Ok(true));
        assert_eq!(t.failures, 0);
        assert!(t.blocked_until.is_none());
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

    #[test]
    fn a_session_cookie_stops_working_when_its_age_runs_out() {
        let store = Store::open_in_memory().unwrap();
        let now = home_store::store::now_unix() as u64;
        let state = ServerState {
            store: Arc::new(Mutex::new(store)),
            password: b"pw".to_vec(),
            auth_throttle: Mutex::new(AuthThrottle::new()),
            sessions: Mutex::new(HashMap::from([
                ("live".to_string(), now + 60),
                ("stale".to_string(), now - 1),
            ])),
            cookie_secure: false,
            tuya_reauth: Mutex::new(None),
            pending_tuya_qr: Mutex::new(HashMap::new()),
        };
        let cookie = |v: &str| HashMap::from([("cookie".to_string(), format!("home_session={v}"))]);
        assert!(is_authenticated(&state, &cookie("live")));
        assert!(!is_authenticated(&state, &cookie("stale")));
        assert!(!is_authenticated(&state, &cookie("never-issued")));
        // The expired id is dropped, so it cannot come back.
        assert!(!state.sessions.lock().unwrap().contains_key("stale"));
    }

    #[test]
    fn a_new_session_id_is_unguessable_and_never_empty() {
        let a = new_session_id();
        assert_eq!(a.len(), 64, "{a}");
        assert_ne!(a, new_session_id());
    }

    #[test]
    fn a_poll_without_a_token_is_a_usage_error() {
        let store = Store::open_in_memory().unwrap();
        let state = ServerState {
            store: Arc::new(Mutex::new(store)),
            password: b"".to_vec(),
            auth_throttle: Mutex::new(AuthThrottle::new()),
            sessions: Mutex::new(HashMap::new()),
            cookie_secure: false,
            tuya_reauth: Mutex::new(None),
            pending_tuya_qr: Mutex::new(HashMap::new()),
        };
        let err = api_tuya_login_poll(&state, "", "").unwrap_err();
        assert_eq!(err.1.code(), 2);
    }
}
