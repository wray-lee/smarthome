//! The Tuya Backend: QR login, Cloud Control and Telemetry.
//!
//! Ported from Alegzandr/lueur `crates/lueur-shell/src/tuya_cloud.rs`,
//! commit be7ba9b14be8fc1031ee962c9069c744e88e8252, MIT;
//! Copyright (c) 2026 Lueur contributors. The full MIT text is in
//! `LICENSE-tuya` in this crate; the README's Tuya section documents the
//! commands.
//!
//! What the Backend does in this ticket:
//!
//! - [`login::login`]: the Smart Life QR sign-in, no developer platform, with the QR
//!   rendered as terminal text.
//! - [`devices::list_devices`]: every Device of every home, with each Local
//!   Key saved.
//! - [`devices::local_keys`]: every saved Local Key as JSON, for the Vendor
//!   Exit backup.
//! - [`sign`]: the request signing and AES-GCM sealing, byte-for-byte the
//!   official Python SDK's, checked against its fixed vectors.
//!
//! The QR start and poll use [`session::http_client`]; every call after the
//! sign-in — the first device list, which saves the Local Keys, and every
//! later command — goes through the one [`session::TuyaClient`], so requests
//! are serialised and a rotated refresh token is persisted before it is used.
//! This crate depends on `home-store` and nothing else in the workspace; it
//! never depends on the Google Home Backend.
//!
//! Env overrides (tests and, for the endpoint only, a user whose regional
//! answer is wrong):
//!
//! - `HOME_TUYA_LOGIN_URL`: the QR login base, default
//!   `https://apigw.iotbing.com`.
//! - `HOME_TUYA_ENDPOINT`: the regional API base, otherwise the one the
//!   login stored. A `http://127.0.0.1:<port>` value is accepted here only.
//! - `HOME_TUYA_USER_CODE`: the default user code for `login`.
//! - `HOME_TUYA_POLL_MS`, `HOME_TUYA_POLL_ATTEMPTS`: QR poll interval and
//!   number of polls, default 2000 ms and 90 (about three minutes).

#![forbid(unsafe_code)]

pub mod config;
pub mod control;
pub mod devices;
pub mod login;
pub mod session;
pub mod sign;

pub use config::Config;
pub use devices::{Device, KeyChange, Listing};
pub use login::{login, poll_login, qr_payload, qr_svg, qr_terminal, start_login, Pending};
pub use session::{Failure, Session, TuyaClient, BACKEND};

use home_store::{ErrorBody, Exit, Store};

/// `home tuya login`: run the QR sign-in, save every Local Key, and report
/// metadata only.
///
/// `user_code` falls back to `HOME_TUYA_USER_CODE`; with neither, it is a
/// usage error. `announce` receives the pending sign-in once, so the caller
/// writes the QR to stderr while stdout stays a single JSON document. After
/// the session is saved, one device list is fetched so the keys are stored
/// without a separate `home tuya devices`; that fetch failing is reported on
/// stderr and never fails the sign-in. The returned value carries no token:
/// `{"backend":"tuya","endpoint":…, "uid":…, "expires_at":…}`.
pub fn login_command(
    cfg: &Config,
    store: &Store,
    user_code: Option<&str>,
    announce: &mut dyn FnMut(&Pending),
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let code = user_code
        .map(str::to_string)
        .or_else(|| cfg.user_code.clone())
        .ok_or_else(|| {
            (
                ErrorBody::new(
                    "usage",
                    "pass --user-code, or set HOME_TUYA_USER_CODE; find the code in the \
                     Smart Life app under Me → Settings → Account and Security → User Code",
                ),
                Exit::Usage,
            )
        })?;
    let session = login(cfg, store, &code, announce).map_err(into_pair)?;
    // The session is already saved; fetching the Local Keys now means a
    // sign-in alone backs them up. A failure here must not undo the sign-in,
    // and its message is the URL-free one every Tuya failure carries.
    if let Err(failure) = TuyaClient::with_session(session.clone(), cfg, store)
        .and_then(|client| devices::list_devices(&client))
    {
        eprintln!(
            "signed in, but fetching Local Keys failed: {}",
            failure.body.message
        );
    }
    Ok(serde_json::json!({
        "backend": BACKEND,
        "endpoint": session.endpoint,
        "uid": session.uid,
        "expires_at": session.expire_time,
    }))
}

/// The pair the CLI prints, from a [`Failure`].
fn into_pair(f: Failure) -> (ErrorBody, Exit) {
    (f.body, f.exit)
}

/// `home tuya devices`: the Device metadata, as one JSON array.
pub fn devices_command(
    cfg: &Config,
    store: &Store,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    let client = TuyaClient::open(store, cfg).map_err(into_pair)?;
    let listing = devices::list_devices(&client).map_err(into_pair)?;
    // The cache row is the same list, for later commands that do not need
    // the cloud; a failed cache write is not worth failing the command over.
    let json = listing.devices_json().to_string();
    let _ = store.cache_devices(BACKEND, &json);
    Ok(listing.devices_json())
}

/// `home tuya keys`: every saved Local Key as one JSON array.
pub fn keys_command(_cfg: &Config, store: &Store) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    // No network call: keys come straight from SQLite.
    devices::local_keys(store).map_err(into_pair)
}

/// `home tuya on|off <device>`: the switch command.
pub fn on_off_command(
    cfg: &Config,
    store: &Store,
    device: &str,
    on: bool,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    control::on_off_command(cfg, store, device, on)
}

/// `home tuya get <device>`: data points, Telemetry and calibration.
pub fn get_command(
    cfg: &Config,
    store: &Store,
    device: &str,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    control::get_command(cfg, store, device)
}

/// `home tuya set <device> <code> <value>`: one raw data point.
pub fn set_command(
    cfg: &Config,
    store: &Store,
    device: &str,
    code: &str,
    value: &str,
) -> Result<serde_json::Value, (ErrorBody, Exit)> {
    control::set_command(cfg, store, device, code, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_name_is_tuya() {
        assert_eq!(BACKEND, "tuya");
    }
}
