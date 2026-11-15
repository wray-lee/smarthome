//! `home` — one binary for both Backends. stdout is always exactly one JSON
//! document and the exit code follows the contract in `home_store::contract`.

use std::io::Read;
use std::path::PathBuf;

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
    /// Add, list or remove protected Devices. They are refused by every write.
    Protect(ProtectArgs),
}

#[derive(Subcommand, Debug)]
enum GhCmd {
    /// Store a Google Home master token (or oauth_token) read from stdin.
    Login(LoginArgs),
    /// Print every Google Home Device as JSON.
    Devices,
}

#[derive(Args, Debug)]
struct LoginArgs {
    /// The Google account the token belongs to.
    #[arg(long, env = "HOME_GH_EMAIL", value_name = "EMAIL")]
    email: Option<String>,
    /// Reuse a known Android id instead of generating one.
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
        Err(e) => {
            use clap::error::ErrorKind;
            match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    print!("{e}");
                    std::process::exit(0);
                }
                _ => fail(ErrorBody::usage(e.to_string()), Exit::Usage),
            }
        }
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
        },
        Command::Protect(args) => protect(&store, args),
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
    let android_id = args
        .android_id
        .clone()
        .unwrap_or_else(gpsoauth::new_android_id);
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
    Credential {
        master,
        android_id,
        email,
        bearer: None,
        bearer_expires_at: None,
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
}
