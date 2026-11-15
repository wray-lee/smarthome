//! The Google Home Backend: sign in once through a browser-issued
//! `oauth_token`, keep the master token in SQLite, and speak the foyer JSON
//! gateway (positional arrays over gRPC-web) to list Devices.
//!
//! Forked from `piekstra/google-home-cli` v0.10.0 (MIT, see LICENSE-ghome).
//! The auth chain, the codec and the bearer-mint-once rule are upstream's;
//! OS-keychain storage is replaced by [`home_store::Store`].

pub mod config;
pub mod foyer;
pub mod gpsoauth;
pub mod homegraph;

use home_store::{ErrorBody, Exit, Store};
use reqwest::blocking::Client;

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
}

/// Fetch every Device of every home. A 401/403 mints one new Bearer and
/// retries once; a second refusal is `reauth_required`.
pub fn list_devices(
    session: &mut Session,
    store: &Store,
) -> Result<Vec<Device>, (ErrorBody, Exit)> {
    let raw = foyer::rpc(
        session,
        store,
        foyer::GET_HOME_GRAPH,
        &serde_json::Value::Array(vec![]),
    )?;
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
        })
        .collect())
}
