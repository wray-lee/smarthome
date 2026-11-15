//! The Google Home Backend: sign in once through a browser-issued
//! `oauth_token`, keep the master token in SQLite, and speak the foyer JSON
//! gateway (positional arrays over gRPC-web).
//!
//! Forked from `piekstra/google-home-cli` v0.10.0 (MIT, see LICENSE-ghome).
//! The auth chain, the codec and the bearer-mint-once rule are upstream's;
//! OS-keychain storage is replaced by [`home_store::Store`].
//!
//! # The operating seam
//!
//! Everything outside this crate operates Devices through exactly three
//! functions — [`list_devices`], [`get_state`] and [`set_trait`] — plus the
//! types they take ([`Device`], [`TraitChange`]). The foyer details
//! (`homegraph`, `control`, `resolve`) stay inside the crate, so an
//! implementation on top of Google's official Home MCP server can replace
//! this one without touching the CLI or the Web Panel.
//!
//! `config` and `gpsoauth` remain public only because the not-yet-rewired
//! `home gh login` in `src/main.rs` drives them directly; sign-in is not part
//! of the operating seam.

pub mod config;
pub mod foyer;
pub mod gpsoauth;

mod control;
mod homegraph;
mod resolve;

pub use control::TraitChange;

use home_store::{guard, ErrorBody, Exit, Store};
use reqwest::blocking::Client;
use serde_json::Value;

/// The Backend name every table and JSON document uses.
pub const BACKEND: &str = "google_home";

/// What `home gh login` stores: the master token plus the fixed Android id
/// it was bound to. The Android id is not secret but must never change.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Credential {
    pub master: String,
    pub android_id: String,
    pub email: String,
    #[serde(default)]
    pub bearer: Option<String>,
    #[serde(default)]
    pub bearer_expires_at: Option<u64>,
}

impl Credential {
    pub fn load(store: &Store) -> Result<Option<Self>, home_store::StoreError> {
        match store.credential(BACKEND)? {
            Some(json) => Ok(serde_json::from_str(&json).ok()),
            None => Ok(None),
        }
    }

    pub fn save(&self, store: &Store) -> Result<(), home_store::StoreError> {
        let json = serde_json::to_string(self).unwrap_or_default();
        store.set_credential(BACKEND, &json)
    }
}

/// A usable session: the credential plus a Bearer it can renew once.
pub struct Session {
    pub cred: Credential,
    client: Client,
    cfg: config::Config,
}

impl Session {
    /// The stored session, or the exit-5 error that tells the user to sign in.
    pub fn open(store: &Store, cfg: config::Config) -> Result<Self, (ErrorBody, Exit)> {
        let cred = Credential::load(store)
            .map_err(|e| reauth(format!("reading the stored credential failed: {e}")))?
            .ok_or_else(|| {
                reauth("no Google Home credential stored; run `home gh login`".to_string())
            })?;
        let client = foyer::client();
        Ok(Session { cred, client, cfg })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn cfg(&self) -> &config::Config {
        &self.cfg
    }

    /// A Bearer for the Home Graph scope: the cached one while it lasts, else
    /// a freshly minted one written back to the store.
    pub fn bearer(&mut self, store: &Store) -> Result<String, (ErrorBody, Exit)> {
        if let (Some(token), Some(expires)) = (&self.cred.bearer, self.cred.bearer_expires_at) {
            if now_unix() + 60 < expires {
                return Ok(token.clone());
            }
        }
        self.mint(store)
    }

    /// Drop the cached Bearer (after a 401/403) and mint a new one.
    pub fn refresh_bearer(&mut self, store: &Store) -> Result<String, (ErrorBody, Exit)> {
        self.cred.bearer = None;
        self.cred.bearer_expires_at = None;
        self.mint(store)
    }

    fn mint(&mut self, store: &Store) -> Result<String, (ErrorBody, Exit)> {
        let minted = gpsoauth::get_auth_token(
            &self.client,
            &self.cfg.gpsoauth_url,
            &self.cred.master,
            &self.cred.android_id,
            gpsoauth::HOMEGRAPH_SCOPE,
            &self.cred.email,
        )?;
        let now = now_unix();
        let cap = now + 55 * 60;
        self.cred.bearer = Some(minted.auth.clone());
        self.cred.bearer_expires_at = Some(
            minted
                .expiry
                .filter(|e| *e > now)
                .map_or(cap, |e| e.min(cap)),
        );
        // A failed cache write is not worth failing the command over.
        let _ = self.cred.save(store);
        Ok(minted.auth)
    }
}

pub fn reauth(message: String) -> (ErrorBody, Exit) {
    (
        ErrorBody::new("reauth_required", message).backend(BACKEND),
        Exit::ReauthRequired,
    )
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One Device as `home gh devices` reports it.
///
/// `agent_id` and `partner_device_id` identify the Device inside its Linked
/// Service; `UpdateTraits` needs both, but they are internal plumbing and
/// never appear in the JSON (the field order of the printed document is
/// unchanged).
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub traits: Vec<String>,
    pub backend: &'static str,
    #[serde(skip_serializing)]
    pub agent_id: Option<String>,
    #[serde(skip_serializing)]
    pub partner_device_id: Option<String>,
}

/// Fetch every Device of every home. A 401/403 mints one new Bearer and
/// retries once; a second refusal is `reauth_required`.
pub fn list_devices(
    session: &mut Session,
    store: &Store,
) -> Result<Vec<Device>, (ErrorBody, Exit)> {
    let devices = fetch_graph(session, store)?;
    Ok(devices)
}

/// The parsed home graph, as the Backend's Devices.
fn fetch_graph(session: &mut Session, store: &Store) -> Result<Vec<Device>, (ErrorBody, Exit)> {
    let raw = foyer::rpc(
        session,
        store,
        foyer::GET_HOME_GRAPH,
        &serde_json::Value::Array(vec![]),
    )
    .map_err(named_reauth)?;
    Ok(homegraph::parse(&raw)
        .homes
        .into_iter()
        .flat_map(|h| h.devices)
        .map(|d| Device {
            id: d.id,
            name: d.name,
            room: d.room,
            kind: d.kind,
            traits: d.traits,
            backend: BACKEND,
            agent_id: d.agent_id,
            partner_device_id: d.partner_device_id,
        })
        .collect())
}

/// Name a reauth failure after the Backend the caller was using.
fn named_reauth(e: (ErrorBody, Exit)) -> (ErrorBody, Exit) {
    let (body, exit) = e;
    if exit == Exit::ReauthRequired {
        (
            ErrorBody::new(
                "reauth_required",
                format!("Google Home Backend: {}", body.message),
            )
            .backend(BACKEND),
            exit,
        )
    } else {
        (body, exit)
    }
}

/// One `HomeControlService` call, with the Bearer rule and the Backend name.
fn control(
    session: &mut Session,
    store: &Store,
    method: &str,
    body: &Value,
) -> Result<Value, (ErrorBody, Exit)> {
    foyer::rpc_service(session, store, foyer::HOME_CONTROL, method, body).map_err(named_reauth)
}

/// The `device_offline` refusal for get and set.
fn offline(device: &Device) -> (ErrorBody, Exit) {
    (
        ErrorBody::new(
            "device_offline",
            format!(
                "Google Home device {} ({}) is offline; its vendor reports it unreachable",
                device.name, device.id
            ),
        )
        .backend(BACKEND),
        Exit::Offline,
    )
}

/// Read one Device's current traits.
///
/// `device_ref` is an exact Device ID, then `room/name`, then a bare name
/// (case-insensitive for both room and name); an ambiguous reference is
/// exit 3 with the candidates attached. The foyer answer supplies the state;
/// a Device the vendor reports offline is exit 4 (`device_offline`).
pub fn get_state(
    session: &mut Session,
    store: &Store,
    device_ref: &str,
) -> Result<Value, (ErrorBody, Exit)> {
    let devices = fetch_graph(session, store)?;
    let device = resolve::resolve(&devices, device_ref)?;
    let raw = control(
        session,
        store,
        control::GET_TRAITS,
        &control::get_traits_body(std::slice::from_ref(&device.id)),
    )?;
    let traits = control::traits_for(&raw, &device.id);
    let online = control::online(&traits);
    if online.is_none() {
        return Err((
            ErrorBody::new(
                "upstream",
                format!(
                    "foyer returned no state for Google Home device {} ({})",
                    device.name, device.id
                ),
            )
            .backend(BACKEND),
            Exit::Upstream,
        ));
    }
    if online == Some(false) {
        return Err(offline(&device));
    }
    Ok(control::state_document(
        &device.id,
        &device.name,
        device.room.as_deref(),
        device.kind.as_deref(),
        &traits,
        online,
    ))
}

/// Change one trait of one Device.
///
/// - Name resolution and ID precedence are [`get_state`]'s.
/// - A protected Device (ticket 02) is refused with exit 6 and `protected`
///   before anything is sent, and `confirm` does not override that.
/// - A Sensitive Action (`lockUnlock.isLocked` false, `armDisarm.isArmed`
///   false) without `confirm` is exit 6 with `sensitive_action`.
/// - A trait the Device does not advertise is exit 2 with `unsupported_trait`.
/// - An offline Device is exit 4 with `device_offline`.
/// - Only then is `UpdateTraits` sent. Its answer echoes the resulting
///   traits; the returned document is the same shape as [`get_state`]'s plus
///   `changed`.
pub fn set_trait(
    session: &mut Session,
    store: &Store,
    device_ref: &str,
    change: TraitChange,
    confirm: bool,
) -> Result<Value, (ErrorBody, Exit)> {
    let devices = fetch_graph(session, store)?;
    let device = resolve::resolve(&devices, device_ref)?;

    // Protected beats everything, `--confirm` included: the guard is the one
    // write gate, and no flag bypasses it.
    guard::require(store, BACKEND, &device.id)?;

    if change.is_sensitive() && !confirm {
        return Err((
            ErrorBody::new(
                "sensitive_action",
                format!(
                    "{} on Google Home device {} ({}) is a Sensitive Action; \
                     pass --confirm to allow it",
                    change.action_name(),
                    device.name,
                    device.id
                ),
            )
            .backend(BACKEND),
            Exit::Refused,
        ));
    }

    if !control::supports(&device.traits, &change) {
        return Err((
            ErrorBody::new(
                "unsupported_trait",
                format!(
                    "Google Home device {} ({}) does not advertise the trait `{}` \
                     (it has: {})",
                    device.name,
                    device.id,
                    change.required_trait(),
                    if device.traits.is_empty() {
                        "none".to_string()
                    } else {
                        device.traits.join(", ")
                    }
                ),
            )
            .backend(BACKEND),
            Exit::Usage,
        ));
    }

    // A command to a Device the vendor reports unreachable would silently go
    // nowhere; the spec wants exit 4 instead.
    let before = control(
        session,
        store,
        control::GET_TRAITS,
        &control::get_traits_body(std::slice::from_ref(&device.id)),
    )?;
    let before_traits = control::traits_for(&before, &device.id);
    if control::online(&before_traits) == Some(false) {
        return Err(offline(&device));
    }

    let body = control::update_body(
        &device.id,
        device.agent_id.as_deref(),
        device.partner_device_id.as_deref(),
        &change,
    );
    let raw = control(session, store, control::UPDATE_TRAITS, &body)?;

    // The answer echoes the resulting traits. When it does not mention the
    // Device, report the state read just before the write rather than an
    // empty object.
    let echoed = control::traits_for(&raw, &device.id);
    let traits = if echoed.is_empty() {
        before_traits
    } else {
        echoed
    };
    let mut doc = control::state_document(
        &device.id,
        &device.name,
        device.room.as_deref(),
        device.kind.as_deref(),
        &traits,
        control::online(&traits),
    );
    if let Some(map) = doc.as_object_mut() {
        map.insert("changed".into(), change.as_json());
    }
    Ok(doc)
}

#[cfg(test)]
mod tests;
