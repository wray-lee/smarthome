//! The stored session and the one serialised client every Tuya call goes
//! through.
//!
//! Ported from Alegzandr/lueur `crates/lueur-shell/src/tuya_cloud.rs`,
//! commit be7ba9b14be8fc1031ee962c9069c744e88e8252, MIT; Copyright (c) 2026
//! Lueur contributors. See `LICENSE-tuya` in this crate.
//!
//! Tuya's sharing API signs each request with the refresh token, so two
//! concurrent requests can sign with a token the other has just rotated
//! away. The fix is the one the SDK's own issue tracker recommends: one
//! session behind a lock, every call in turn, and a rotated refresh token
//! written to SQLite before the next request uses it.

use std::sync::Mutex;

use home_store::{ErrorBody, Exit, Store};
use reqwest::redirect;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use crate::sign;

/// The Backend name every table and JSON document uses.
pub const BACKEND: &str = "tuya";

/// How many times a transient `1108` answer is retried (twice in total).
pub const MAX_TRANSIENT_RETRIES: u32 = 2;

/// What `home tuya login` stores, under the `tuya` credential row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// `https://apigw.<region>` as the login returned it, validated.
    pub endpoint: String,
    pub user_code: String,
    #[serde(default)]
    pub terminal_id: String,
    #[serde(default)]
    pub uid: String,
    /// Wall-clock expiry in Unix seconds.
    #[serde(default)]
    pub expire_time: i64,
}

impl Session {
    pub fn load(store: &Store) -> Result<Option<Self>, home_store::StoreError> {
        match store.credential(BACKEND)? {
            Some(json) => Ok(serde_json::from_str(&json).ok()),
            None => Ok(None),
        }
    }

    /// Write the session, refresh token included, to the one SQLite file.
    pub fn save(&self, store: &Store) -> Result<(), home_store::StoreError> {
        let json = serde_json::to_string(self).unwrap_or_default();
        store.set_credential(BACKEND, &json)
    }
}

/// The stored session, or the exit-5 error that tells the user to sign in.
pub fn require_session(store: &Store) -> Result<Session, Failure> {
    match Session::load(store) {
        Ok(Some(session)) => Ok(session),
        Ok(None) => Err(Failure::reauth(
            "no Tuya session stored; run `home tuya login` and scan the QR code",
        )),
        Err(e) => Err(Failure::reauth(format!(
            "reading the stored Tuya session failed: {e}"
        ))),
    }
}

/// One failure, with the JSON body and exit code the CLI prints.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub body: ErrorBody,
    pub exit: Exit,
    /// The upstream code, when the failure came from an envelope.
    pub code: Option<String>,
}

impl Failure {
    pub fn reauth(message: impl Into<String>) -> Self {
        Failure {
            body: ErrorBody::new("reauth_required", message).backend(BACKEND),
            exit: Exit::ReauthRequired,
            code: None,
        }
    }

    pub fn upstream(message: impl Into<String>) -> Self {
        Failure {
            body: ErrorBody::new("upstream", message).backend(BACKEND),
            exit: Exit::Upstream,
            code: None,
        }
    }

    pub fn storage(e: home_store::StoreError) -> Self {
        Failure {
            body: ErrorBody::new("storage", e.to_string()).backend(BACKEND),
            exit: Exit::Upstream,
            code: None,
        }
    }

    /// `1108` is the SDK's transient "uri path invalid"; retry it.
    pub fn is_transient(&self) -> bool {
        self.code.as_deref() == Some("1108")
    }
}

impl From<Failure> for (ErrorBody, Exit) {
    fn from(f: Failure) -> Self {
        (f.body, f.exit)
    }
}

/// The one error every Tuya path uses for `sign invalid` / `1010`.
pub fn reauth(message: impl Into<String>) -> (ErrorBody, Exit) {
    Failure::reauth(message).into()
}

/// An upstream refusal: exit 7.
pub fn upstream(message: impl Into<String>) -> (ErrorBody, Exit) {
    Failure::upstream(message).into()
}

/// A storage failure: exit 7, like every other Backend's.
pub fn storage(e: home_store::StoreError) -> (ErrorBody, Exit) {
    Failure::storage(e).into()
}

/// A reqwest error as text a user may see.
///
/// Two things are needed beyond `Display`: the URL is stripped (reqwest
/// appends the full URL to `send` errors, and the refresh path puts the
/// refresh token in the URL's path), and the causes below the top-level
/// error are appended, because `Display` alone would drop the connect reason
/// and the redirect policy's own message.
pub(crate) fn describe_error(e: reqwest::Error) -> String {
    use std::error::Error as _;
    let causes: Vec<String> = {
        let mut causes = Vec::new();
        let mut source = e.source();
        while let Some(cause) = source {
            causes.push(cause.to_string());
            source = cause.source();
        }
        causes
    };
    let mut text = e.without_url().to_string();
    for cause in causes {
        text.push_str(": ");
        text.push_str(&cause);
    }
    text
}

/// One serialised Tuya client.
///
/// The `Mutex` is the whole point: login polling, refresh, homes and devices
/// all go through it, so a request can never sign with a token another
/// request has already rotated.
pub struct TuyaClient<'a> {
    inner: Mutex<Session>,
    http: reqwest::blocking::Client,
    endpoint: String,
    store: &'a Store,
}

impl<'a> TuyaClient<'a> {
    /// Open the client for a stored session, checking the endpoint first.
    pub fn open(store: &'a Store, cfg: &Config) -> Result<Self, Failure> {
        let session = require_session(store)?;
        Self::with_session(session, cfg, store)
    }

    /// Open the client for a session already in hand (right after login),
    /// checking the endpoint the same way.
    pub fn with_session(session: Session, cfg: &Config, store: &'a Store) -> Result<Self, Failure> {
        let endpoint = cfg
            .endpoint_for(&session.endpoint)
            .map_err(|e| Failure::upstream(e.to_string()))?;
        Ok(TuyaClient {
            inner: Mutex::new(session),
            http: http_client(&endpoint),
            endpoint,
            store,
        })
    }

    /// The endpoint this client talks to, for error messages and tests.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The store this client persists tokens to.
    pub fn store(&self) -> &Store {
        self.store
    }

    /// A copy of the current session (tokens included; never printed).
    pub fn session(&self) -> Session {
        self.lock().clone()
    }

    /// Whether the access token is within a minute of expiry, which is the
    /// SDK's own threshold.
    pub fn needs_refresh(&self) -> bool {
        let session = self.lock();
        session.expire_time > 0 && now_unix() + 60 >= session.expire_time
    }

    /// The stored access-token expiry, in Unix seconds.
    pub fn expires_at(&self) -> i64 {
        self.lock().expire_time
    }

    /// Seconds from now until the access token expires (may be negative).
    pub fn secs_until_expiry(&self) -> i64 {
        let expire = self.expires_at();
        if expire <= 0 {
            return 0;
        }
        expire - now_unix()
    }

    /// The active refresh: rotate the tokens now, whether or not a `call`
    /// is in flight. This is the entry point the background task uses; it
    /// is the same serialised, persist-first [`TuyaClient::refresh`] that
    /// [`TuyaClient::call`] uses, separated out so a caller can refresh
    /// without making any other request.
    ///
    /// Returns `true` when a refresh happened, `false` when the stored
    /// token was still fresh.
    pub fn refresh_if_needed(&self) -> Result<bool, Failure> {
        if !self.needs_refresh() {
            return Ok(false);
        }
        self.refresh()?;
        Ok(true)
    }

    /// One signed request, then the unsealed `result`.
    ///
    /// If the access token is about to expire the session is refreshed first;
    /// the refreshed refresh token is written to SQLite before it is used.
    /// An `1108` answer is retried up to [`MAX_TRANSIENT_RETRIES`] times.
    pub fn call(
        &self,
        method: Method,
        path: &str,
        query: Option<&Value>,
        body: Option<&Value>,
    ) -> Result<Value, Failure> {
        if self.needs_refresh() {
            self.refresh()?;
        }
        let session = self.lock();
        let mut attempt = 0;
        loop {
            match self.request(&session, method, path, query, body) {
                Err(err) if err.is_transient() && attempt < MAX_TRANSIENT_RETRIES => {
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Session> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn request(
        &self,
        session: &Session,
        method: Method,
        path: &str,
        query: Option<&Value>,
        body: Option<&Value>,
    ) -> Result<Value, Failure> {
        Ok(self.request_timed(session, method, path, query, body)?.1)
    }

    /// [`TuyaClient::request`], plus the envelope's own `t` (milliseconds),
    /// which the token refresh needs: `expireTime` is a lifetime counted from
    /// that instant, not from the local clock.
    fn request_timed(
        &self,
        session: &Session,
        method: Method,
        path: &str,
        query: Option<&Value>,
        body: Option<&Value>,
    ) -> Result<(Option<i64>, Value), Failure> {
        let rid = uuid::Uuid::new_v4().to_string();
        let hash_key = sign::hash_key(&rid, &session.refresh_token);
        let secret = sign::aes_secret(&rid, &hash_key);
        let nonce = sign::random_nonce().map_err(|e| Failure::upstream(e.to_string()))?;

        let query_enc = match query.filter(|q| !is_empty_object(q)) {
            Some(q) => sign::seal(&sign::compact_json(q), &secret, &nonce)
                .map_err(|e| Failure::upstream(e.to_string()))?,
            None => String::new(),
        };
        let body_enc = match body.filter(|b| !is_empty_object(b)) {
            Some(b) => sign::seal(&sign::compact_json(b), &secret, &nonce)
                .map_err(|e| Failure::upstream(e.to_string()))?,
            None => String::new(),
        };

        let time = now_millis().to_string();
        let headers = [
            ("X-appKey", crate::config::CLIENT_ID),
            ("X-requestId", rid.as_str()),
            ("X-sid", ""),
            ("X-time", time.as_str()),
            ("X-token", session.access_token.as_str()),
        ];
        let signature = sign::sign(&hash_key, &headers, &query_enc, &body_enc);

        let url = format!("{}{path}", self.endpoint);
        let mut req = match method {
            Method::Get => self.http.get(&url),
            Method::Post => self.http.post(&url),
        };
        for (k, v) in headers {
            req = req.header(k, v);
        }
        req = req.header("X-sign", signature);
        if !query_enc.is_empty() {
            req = req.query(&[("encdata", query_enc.as_str())]);
        }
        if !body_enc.is_empty() {
            req = req.json(&serde_json::json!({ "encdata": body_enc }));
        }

        let resp = req
            .send()
            .map_err(|e| Failure::upstream(describe_error(e)))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Failure::upstream(format!("Tuya answered HTTP {status}")));
        }
        let envelope: Value = resp
            .json()
            .map_err(|e| Failure::upstream(describe_error(e)))?;
        let envelope = classify(envelope)?;
        let answered_at = envelope.get("t").and_then(Value::as_i64);
        let result = match envelope.get("result") {
            Some(Value::String(sealed)) => {
                let plain =
                    sign::open(sealed, &secret).map_err(|e| Failure::upstream(e.to_string()))?;
                serde_json::from_str(&plain).unwrap_or(Value::String(plain))
            }
            Some(other) => other.clone(),
            None => Value::Null,
        };
        Ok((answered_at, result))
    }

    /// Rotate the session's tokens from `GET /v1.0/m/token/{refresh_token}`.
    ///
    /// The new refresh token replaces the old one and is written to SQLite
    /// **before** it is used for any further request: the store row is
    /// written first, then the in-memory session is replaced under the same
    /// lock, so no later request can sign with a token that is not yet
    /// persisted.
    pub fn refresh(&self) -> Result<Session, Failure> {
        let mut session = self.lock();
        let path = format!("/v1.0/m/token/{}", session.refresh_token);
        let (answered_at, value) = self.request_timed(&session, Method::Get, &path, None, None)?;
        let text = |k: &str| {
            value
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let new_refresh = text("refreshToken");
        if new_refresh.is_empty() {
            return Err(Failure::upstream(
                "the token refresh returned no refresh token",
            ));
        }
        let mut updated = session.clone();
        updated.refresh_token = new_refresh;
        let access = text("accessToken");
        if !access.is_empty() {
            updated.access_token = access;
        }
        if !text("uid").is_empty() {
            updated.uid = text("uid");
        }
        let lifetime_s = value.get("expireTime").and_then(Value::as_i64).unwrap_or(0);
        if lifetime_s > 0 {
            // The SDK reads `expireTime` as a lifetime in seconds counted
            // from the envelope's own `t` (milliseconds), so a clock skew
            // between here and Tuya cannot expire the session early.
            let base_ms = answered_at.unwrap_or_else(now_millis_i64);
            updated.expire_time = base_ms / 1000 + lifetime_s;
        }
        updated.save(self.store).map_err(Failure::storage)?;
        *session = updated.clone();
        Ok(updated)
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether the answer means "sign in again" rather than "something else went
/// wrong". The SDK's `sign invalid` and code `1010` are the two signals.
pub fn is_reauth(code: &Value, msg: &str) -> bool {
    let code_matches = match code {
        Value::Number(n) => n.as_i64() == Some(1010),
        Value::String(s) => s == "1010",
        _ => false,
    };
    code_matches || msg.to_ascii_lowercase().contains("sign invalid")
}

/// Classify an envelope: `success: true` passes through; `1010` and
/// `sign invalid` become exit 5; everything else is exit 7. The code is kept
/// so [`Failure::is_transient`] can spot `1108`.
fn classify(envelope: Value) -> Result<Value, Failure> {
    if envelope.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(envelope);
    }
    let code = envelope.get("code").cloned().unwrap_or(Value::Null);
    let msg = envelope
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let code_text = match &code {
        Value::Null => String::new(),
        other => other.to_string().trim_matches('"').to_string(),
    };
    if is_reauth(&code, &msg) {
        return Err(Failure {
            body: ErrorBody::new(
                "reauth_required",
                format!("Tuya refused the session ({code} {msg}); run `home tuya login` again"),
            )
            .backend(BACKEND),
            exit: Exit::ReauthRequired,
            code: Some(code_text),
        });
    }
    Err(Failure {
        body: ErrorBody::new(
            "upstream",
            format!("Tuya refused the request ({code} {msg})"),
        )
        .backend(BACKEND),
        exit: Exit::Upstream,
        code: Some(code_text),
    })
}

fn is_empty_object(value: &Value) -> bool {
    value.as_object().is_some_and(|o| o.is_empty())
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn now_millis_i64() -> i64 {
    i64::try_from(now_millis()).unwrap_or(0)
}

/// The two verbs the sharing API uses for the calls this ticket makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// The blocking HTTP client, with the one timeout the SDK uses.
///
/// HTTPS is required unless `base` is a loopback HTTP fake (the test
/// upstream, which `config::is_local_http` gates exactly like the endpoint
/// check does), so a real endpoint can never be reached over cleartext. A
/// redirect is followed only within the origin of the first request: a
/// 30x to another host would re-send the `X-token` header, and on the refresh
/// path the URL itself carries the refresh token.
pub fn http_client(base: &str) -> reqwest::blocking::Client {
    client_builder(base)
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// The builder behind [`http_client`], separate so a test can inspect the
/// policy without a live request.
fn client_builder(base: &str) -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .timeout(crate::config::TIMEOUT)
        .https_only(!crate::config::is_local_http(base))
        .redirect(same_host_redirects())
}

/// The same ceiling reqwest's default policy uses.
const MAX_REDIRECTS: usize = 10;

/// Redirects stay on the origin of the first request. A redirect anywhere
/// else is an error, and the client sends nothing to the other host.
fn same_host_redirects() -> redirect::Policy {
    redirect::Policy::custom(|attempt| {
        let next = attempt.url().clone();
        let initial = attempt.previous().first().cloned();
        let hops = attempt.previous().len();
        if hops > MAX_REDIRECTS {
            return attempt.error("refusing more than 10 redirects");
        }
        match initial {
            Some(initial) if same_origin(&initial, &next) => attempt.follow(),
            Some(_) => attempt.error("refusing to follow a redirect to another host"),
            None => attempt.error("refusing a redirect without a previous URL"),
        }
    })
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reauth_codes_are_recognized() {
        assert!(is_reauth(&Value::from(1010), ""));
        assert!(is_reauth(&Value::from("1010"), ""));
        assert!(is_reauth(
            &Value::Null,
            "network error:(-9999999) sign invalid"
        ));
        assert!(!is_reauth(&Value::from(1108), "uri path invalid"));
        assert!(!is_reauth(&Value::Null, ""));
    }

    #[test]
    fn only_1108_is_transient() {
        let value = serde_json::json!({"success": false, "code": 1108, "msg": "uri path invalid"});
        let err = classify(value).unwrap_err();
        assert!(err.is_transient());
        assert_eq!(err.exit, Exit::Upstream);
        let other =
            classify(serde_json::json!({"success": false, "code": 999, "msg": "no"})).unwrap_err();
        assert!(!other.is_transient());
    }

    #[test]
    fn a_session_round_trips_through_the_credential_row() {
        let store = Store::open_in_memory().unwrap();
        assert!(Session::load(&store).unwrap().is_none());
        let session = Session {
            access_token: "dummy-access".into(),
            refresh_token: "dummy-refresh".into(),
            endpoint: "https://apigw.tuyaus.com".into(),
            user_code: "DummyCode1".into(),
            terminal_id: "dummy-terminal".into(),
            uid: "dummy-uid".into(),
            expire_time: 1_700_000_000,
        };
        session.save(&store).unwrap();
        assert_eq!(Session::load(&store).unwrap(), Some(session));
        let raw = store.credential(BACKEND).unwrap().unwrap();
        assert!(raw.contains("dummy-refresh"));
    }

    #[test]
    fn a_missing_session_is_reauth_required() {
        let store = Store::open_in_memory().unwrap();
        let err = require_session(&store).unwrap_err();
        assert_eq!(err.exit, Exit::ReauthRequired);
        assert_eq!(err.body.backend, Some(BACKEND));
        assert_eq!(err.body.code, "reauth_required");
    }

    /// A URL with something secret in its path, sent to a port nothing
    /// listens on: the error `send` returns must not carry the URL.
    #[test]
    fn a_send_error_is_described_without_its_url() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let base = format!("http://127.0.0.1:{port}");
        let err = http_client(&base)
            .get(format!("{base}/v1.0/m/token/canary-refresh-token"))
            .send()
            .unwrap_err();
        let text = describe_error(err);
        assert!(
            !text.contains("canary-refresh-token"),
            "the URL's path leaked: {text}"
        );
        assert!(!text.contains(&base), "the URL's host leaked: {text}");
        assert!(
            text.contains("error sending request"),
            "the cause must still be described: {text}"
        );
    }

    #[test]
    fn a_decode_error_is_described_without_its_url() {
        // A server that answers with something that is not JSON.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let _ = std::io::Write::write_all(
                    &mut stream,
                    b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 4\r\n\r\nnope",
                );
            }
        });
        let base = format!("http://127.0.0.1:{port}");
        let err = http_client(&base)
            .get(format!("{base}/v1.0/m/token/canary-refresh-token"))
            .send()
            .unwrap()
            .json::<Value>()
            .unwrap_err();
        let text = describe_error(err);
        assert!(!text.contains("canary-refresh-token"), "{text}");
        assert!(!text.contains(&base), "{text}");
        assert!(
            text.contains("error decoding response body"),
            "the kind must still be described: {text}"
        );
    }

    /// `https_only` applies to a real endpoint and is dropped only for the
    /// loopback HTTP fake the tests use; the check is the same one the
    /// endpoint gate applies.
    #[test]
    fn https_is_required_except_for_the_loopback_fake() {
        assert!(
            crate::config::is_local_http("http://127.0.0.1:9"),
            "a loopback fake is the only cleartext base"
        );
        assert!(!crate::config::is_local_http("https://apigw.tuyaus.com"));
        // A real endpoint: an http URL is refused before any connection.
        let err = http_client("https://apigw.tuyaus.com")
            .get("http://apigw.tuyaus.com/v1.0/m/token/canary-refresh-token")
            .send()
            .unwrap_err();
        let text = describe_error(err);
        assert!(
            text.contains("URL scheme is not allowed"),
            "cleartext to a real endpoint must be refused: {text}"
        );
        assert!(!text.contains("canary-refresh-token"), "{text}");

        // The loopback fake is allowed over http.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let _ = std::io::Write::write_all(
                    &mut stream,
                    b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 10\r\n\r\n{\"a\": \"b\"}",
                );
            }
        });
        let base = format!("http://127.0.0.1:{port}");
        let value: Value = http_client(&base)
            .get(&base)
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(value, serde_json::json!({"a": "b"}));
    }

    #[test]
    fn same_origin_compares_scheme_host_and_port() {
        let parse = |s: &str| Url::parse(s).unwrap();
        assert!(same_origin(
            &parse("https://apigw.tuyaus.com/a"),
            &parse("https://apigw.tuyaus.com/b/c")
        ));
        assert!(!same_origin(
            &parse("https://apigw.tuyaus.com/a"),
            &parse("http://apigw.tuyaus.com/a")
        ));
        assert!(!same_origin(
            &parse("https://apigw.tuyaus.com/a"),
            &parse("https://evil.example/a")
        ));
        assert!(!same_origin(
            &parse("http://127.0.0.1:1/a"),
            &parse("http://127.0.0.1:2/a")
        ));
    }
}
