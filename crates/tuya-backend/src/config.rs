//! Where the Backend talks to, and how long it waits.
//!
//! Ported from Alegzandr/lueur `crates/lueur-shell/src/tuya_cloud.rs`,
//! commit be7ba9b14be8fc1031ee962c9069c744e88e8252, MIT; Copyright (c) 2026
//! Lueur contributors. See `LICENSE-tuya` in this crate.
//!
//! The login host is fixed: the QR flow lives there whatever the account's
//! region. Every later call goes to the regional endpoint the login returns,
//! and that endpoint must name a Tuya gateway. Tests point both at a fake
//! upstream with `HOME_TUYA_LOGIN_URL` / `HOME_TUYA_ENDPOINT`; an endpoint
//! override is the only way a `http://127.0.0.1:<port>` address is accepted.

use std::time::Duration;

/// The partner identity the QR sign-in is issued under: Home Assistant's
/// public app registration, the same one the official Python SDK uses.
pub const CLIENT_ID: &str = "HA_3y9q4ak7g4ephrvke";
pub const SCHEMA: &str = "haauthorize";
pub const LOGIN_HOST: &str = "https://apigw.iotbing.com";

/// Domains the regional API host the sign-in hands back may belong to.
const API_DOMAINS: &[&str] = &[
    "tuyaeu.com",
    "tuyaus.com",
    "tuyacn.com",
    "tuyain.com",
    "iotbing.com",
];

/// `https://<host>` under one of the domains above, and nothing else: no port,
/// path, credentials or query.
pub fn valid_endpoint(url: &str) -> bool {
    let Some(host) = url.strip_prefix("https://") else {
        return false;
    };
    let clean = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    clean
        && API_DOMAINS
            .iter()
            .any(|d| host.strip_suffix(d).is_some_and(|rest| rest.ends_with('.')))
}

/// The default wait between QR code polls and the default number of polls:
/// about two seconds for three minutes, as the SDK does.
pub const DEFAULT_POLL_MS: u64 = 2_000;
pub const DEFAULT_POLL_ATTEMPTS: u32 = 90;

/// The request timeout for one HTTP call.
pub const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the QR login calls, no trailing slash.
    pub login_url: String,
    /// Set only when the test (or the user) overrides the regional endpoint;
    /// otherwise the endpoint stored by the login is used.
    pub endpoint_override: Option<String>,
    /// User code to fall back on when the CLI does not pass one.
    pub user_code: Option<String>,
    pub poll_ms: u64,
    pub poll_attempts: u32,
}

impl Config {
    pub fn from_env() -> Self {
        let get = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
        let parse = |key: &str, default: u64| {
            get(key)
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(default)
        };
        Config {
            login_url: get("HOME_TUYA_LOGIN_URL")
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_else(|| LOGIN_HOST.to_string()),
            endpoint_override: get("HOME_TUYA_ENDPOINT")
                .map(|v| v.trim_end_matches('/').to_string()),
            user_code: get("HOME_TUYA_USER_CODE"),
            poll_ms: parse("HOME_TUYA_POLL_MS", DEFAULT_POLL_MS),
            poll_attempts: u32::try_from(parse(
                "HOME_TUYA_POLL_ATTEMPTS",
                u64::from(DEFAULT_POLL_ATTEMPTS),
            ))
            .unwrap_or(DEFAULT_POLL_ATTEMPTS),
        }
    }

    /// The regional endpoint requests actually use: the override when one is
    /// set, else what the login stored. Either must be a Tuya gateway, or a
    /// loopback fake when the login host is itself loopback.
    pub fn endpoint_for(&self, stored: &str) -> Result<String, EndpointError> {
        match &self.endpoint_override {
            // An explicit override may be a Tuya gateway or a loopback fake;
            // both are accepted here and nowhere else.
            Some(url) if is_local_http(url) || valid_endpoint(url) => Ok(url.clone()),
            Some(url) => Err(EndpointError(url.clone())),
            // A stored endpoint is normally a Tuya gateway. The one exception
            // is a run against a fake upstream: the login host itself was
            // loopback, so the answer it handed back is allowed to be.
            None if valid_endpoint(stored) || (self.login_is_local() && is_local_http(stored)) => {
                Ok(stored.to_string())
            }
            None => Err(EndpointError(stored.to_string())),
        }
    }

    /// Whether the login URL is a loopback HTTP fake, which is the only case
    /// where a non-Tuya host is accepted at all.
    pub fn login_is_local(&self) -> bool {
        is_local_http(&self.login_url)
    }
}

/// `http://127.0.0.1:<port>` or `http://localhost:<port>`, for the fake
/// upstream only. Never accepted from the login answer.
pub fn is_local_http(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| url.strip_prefix("http://localhost:"))
    else {
        return false;
    };
    !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
        && rest.parse::<u16>().is_ok_and(|p| p != 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointError(pub String);

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unexpected Tuya API host `{}`", self.0)
    }
}

impl std::error::Error for EndpointError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tuya_hosts_are_trusted() {
        assert!(valid_endpoint("https://apigw.tuyaeu.com"));
        assert!(valid_endpoint("https://apigw-weaz.tuyaeu.com"));
        assert!(valid_endpoint("https://apigw.tuyaus.com"));
        assert!(valid_endpoint("https://apigw.iotbing.com"));
        assert!(!valid_endpoint("http://apigw.tuyaeu.com"));
        assert!(!valid_endpoint("https://tuyaeu.com.evil.example"));
        assert!(!valid_endpoint("https://eviltuyaeu.com"));
        assert!(!valid_endpoint("https://apigw.tuyaeu.com/x"));
        assert!(!valid_endpoint("https://user@apigw.tuyaeu.com"));
        assert!(!valid_endpoint("https://apigw.tuyaeu.com:444"));
        assert!(!valid_endpoint(""));
    }

    #[test]
    fn loopback_http_is_recognized_only_with_a_port() {
        assert!(is_local_http("http://127.0.0.1:8080"));
        assert!(is_local_http("http://localhost:1"));
        assert!(!is_local_http("http://127.0.0.1"));
        assert!(!is_local_http("http://127.0.0.1:"));
        assert!(!is_local_http("http://127.0.0.1:0"));
        assert!(!is_local_http("https://127.0.0.1:8080"));
        assert!(!is_local_http("http://10.0.0.1:8080"));
        assert!(!is_local_http("http://127.0.0.1:8080/x"));
    }

    #[test]
    fn a_stored_endpoint_is_checked_but_a_local_override_is_allowed() {
        let cfg = Config {
            login_url: LOGIN_HOST.to_string(),
            endpoint_override: None,
            user_code: None,
            poll_ms: DEFAULT_POLL_MS,
            poll_attempts: DEFAULT_POLL_ATTEMPTS,
        };
        assert_eq!(
            cfg.endpoint_for("https://apigw.tuyaus.com").unwrap(),
            "https://apigw.tuyaus.com"
        );
        assert!(cfg.endpoint_for("http://127.0.0.1:9").is_err());

        let with_override = |url: &str| Config {
            endpoint_override: Some(url.to_string()),
            ..cfg.clone()
        };
        assert_eq!(
            with_override("http://127.0.0.1:3456")
                .endpoint_for("https://ignored.example")
                .unwrap(),
            "http://127.0.0.1:3456"
        );
        assert!(with_override("https://evil.example")
            .endpoint_for("https://apigw.tuyaus.com")
            .is_err());
    }
}
