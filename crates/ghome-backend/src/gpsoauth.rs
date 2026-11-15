//! The Android account-token protocol (`android.clients.google.com/auth`).
//! Two form POSTs answered with `Key=Value` lines: the browser's one-time
//! `oauth_token` becomes a long-lived master token, and the master token
//! becomes a short-lived Bearer. `android_id` must match across both.

use std::collections::BTreeMap;

use home_store::{ErrorBody, Exit};
use reqwest::blocking::Client;

const PLAY_SERVICES_VERSION: &str = "240913000";
/// GMS core signing cert, used for the ac2dm master-token exchange.
const GMS_CLIENT_SIG: &str = "38918a453d07199354f8b19af05ec6562ced5788";
/// The Google Home Android app: bearers minted for it are what foyer accepts.
pub const HOME_APP: &str = "com.google.android.apps.chromecast.app";
pub const HOME_APP_SIG: &str = "24bb24c05e47e0aefa68a58a766179d9b613a600";
pub const HOMEGRAPH_SCOPE: &str = "oauth2:https://www.googleapis.com/auth/homegraph";

pub const OAUTH_TOKEN_PREFIX: &str = "oauth2_4/";
pub const MASTER_TOKEN_PREFIX: &str = "aas_et/";

pub struct AuthToken {
    pub auth: String,
    pub expiry: Option<u64>,
}

fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

pub fn exchange_form(oauth_token: &str, android_id: &str, email: &str) -> Vec<(String, String)> {
    kv(&[
        ("accountType", "HOSTED_OR_GOOGLE"),
        ("Email", email),
        ("has_permission", "1"),
        ("add_account", "1"),
        ("ACCESS_TOKEN", "1"),
        ("Token", oauth_token),
        ("service", "ac2dm"),
        ("source", "android"),
        ("androidId", android_id),
        ("device_country", "us"),
        ("operatorCountry", "us"),
        ("lang", "en"),
        ("sdk_version", "17"),
        ("google_play_services_version", PLAY_SERVICES_VERSION),
        ("client_sig", GMS_CLIENT_SIG),
        ("callerSig", GMS_CLIENT_SIG),
        ("droidguard_results", "dummy123"),
    ])
}

pub fn auth_token_form(
    master_token: &str,
    android_id: &str,
    service: &str,
    email: &str,
) -> Vec<(String, String)> {
    kv(&[
        ("accountType", "HOSTED_OR_GOOGLE"),
        ("Email", email),
        ("has_permission", "1"),
        ("EncryptedPasswd", master_token),
        ("service", service),
        ("source", "android"),
        ("androidId", android_id),
        ("app", HOME_APP),
        ("client_sig", HOME_APP_SIG),
        ("device_country", "us"),
        ("operatorCountry", "us"),
        ("lang", "en"),
        ("sdk_version", "17"),
        ("google_play_services_version", PLAY_SERVICES_VERSION),
    ])
}

/// Parse `Key=Value` lines; values may themselves contain `=`.
pub fn parse_response(body: &str) -> BTreeMap<String, String> {
    body.lines()
        .filter_map(|line| {
            let (k, v) = line.trim().split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// A credential's own failure (`BadAuthentication` and friends) is exit 5;
/// anything else is Google's fault (exit 7).
fn failure(what: &str, fields: &BTreeMap<String, String>) -> (ErrorBody, Exit) {
    let error = fields.get("Error").map(String::as_str).unwrap_or("unknown");
    let detail = fields
        .get("ErrorDetail")
        .map(|d| format!(" ({d})"))
        .unwrap_or_default();
    let msg = format!("{what} failed: {error}{detail}");
    match error {
        "BadAuthentication"
        | "NeedsBrowser"
        | "MissingDroidguard"
        | "DeviceManagementRequiredOrSyncDisabled" => crate::reauth(msg),
        _ => (ErrorBody::new("upstream", msg), Exit::Upstream),
    }
}

fn post(
    client: &Client,
    url: &str,
    form: &[(String, String)],
) -> Result<BTreeMap<String, String>, (ErrorBody, Exit)> {
    let resp = client
        .post(url)
        .header(reqwest::header::USER_AGENT, "GoogleAuth/1.4")
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .form(form)
        .send()
        .map_err(|e| {
            (
                ErrorBody::new(
                    "upstream",
                    format!("android auth endpoint: {}", crate::foyer::describe(e)),
                ),
                Exit::Upstream,
            )
        })?;
    // A rejected token answers 403 but still carries the `Error=` line.
    let body = resp.text().map_err(|e| {
        (
            ErrorBody::new(
                "upstream",
                format!(
                    "reading the android auth response: {}",
                    crate::foyer::describe(e)
                ),
            ),
            Exit::Upstream,
        )
    })?;
    Ok(parse_response(&body))
}

/// `oauth_token` → master token.
pub fn exchange_oauth_token(
    client: &Client,
    url: &str,
    oauth_token: &str,
    android_id: &str,
    email: &str,
) -> Result<String, (ErrorBody, Exit)> {
    let fields = post(client, url, &exchange_form(oauth_token, android_id, email))?;
    match fields.get("Token") {
        Some(t) if t.starts_with(MASTER_TOKEN_PREFIX) => Ok(t.clone()),
        Some(_) => Err((
            ErrorBody::new(
                "upstream",
                "oauth_token exchange returned a token of an unexpected shape",
            ),
            Exit::Upstream,
        )),
        None => Err(failure("oauth_token exchange", &fields)),
    }
}

/// Master token → Bearer for `service` (an `oauth2:<scope>` string).
pub fn get_auth_token(
    client: &Client,
    url: &str,
    master_token: &str,
    android_id: &str,
    service: &str,
    email: &str,
) -> Result<AuthToken, (ErrorBody, Exit)> {
    let fields = post(
        client,
        url,
        &auth_token_form(master_token, android_id, service, email),
    )?;
    match fields.get("Auth") {
        Some(auth) => Ok(AuthToken {
            auth: auth.clone(),
            expiry: fields.get("Expiry").and_then(|e| e.parse().ok()),
        }),
        None => Err(failure("bearer mint", &fields)),
    }
}

/// A fresh 16-hex-digit Android id. Google only requires that the same value
/// be used for both exchanges, so it needs no crypto RNG.
pub fn new_android_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.write_u32(std::process::id());
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_value_lines_keep_equals_in_values() {
        let body =
            "Token=aas_et/abc==\nEmail=user@example.com\n\nAuth=ya29.x=y\nExpiry=1700000000\n";
        let m = parse_response(body);
        assert_eq!(m["Token"], "aas_et/abc==");
        assert_eq!(m["Auth"], "ya29.x=y");
        assert_eq!(m["Expiry"], "1700000000");
        assert_eq!(m.len(), 4);
    }

    #[test]
    fn the_forms_carry_the_expected_fields() {
        let f = exchange_form("oauth2_4/tok", "0123456789abcdef", "owner@example.com");
        let get = |k: &str| f.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str());
        assert_eq!(get("service"), Some("ac2dm"));
        assert_eq!(get("Token"), Some("oauth2_4/tok"));
        assert_eq!(get("client_sig"), Some(GMS_CLIENT_SIG));
        assert_eq!(get("androidId"), Some("0123456789abcdef"));

        let f = auth_token_form(
            "aas_et/m",
            "0123456789abcdef",
            HOMEGRAPH_SCOPE,
            "owner@example.com",
        );
        let get = |k: &str| f.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str());
        assert_eq!(get("EncryptedPasswd"), Some("aas_et/m"));
        assert_eq!(get("app"), Some(HOME_APP));
        assert!(get("Token").is_none());
    }

    #[test]
    fn bad_authentication_is_reauth_and_the_rest_is_upstream() {
        let mut m = BTreeMap::new();
        m.insert("Error".to_string(), "BadAuthentication".to_string());
        assert_eq!(failure("x", &m).1.code(), 5);
        m.insert("Error".to_string(), "ServiceUnavailable".to_string());
        assert_eq!(failure("x", &m).1.code(), 7);
    }

    #[test]
    fn android_id_is_sixteen_hex_digits() {
        let id = new_android_id();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// The form holds a master or oauth token, and a 307 would re-send it.
    /// The shared client refuses the hop, and the other host stays untouched.
    #[test]
    fn a_token_form_refused_a_redirect_says_why_and_goes_no_further() {
        use crate::foyer::tests::{ok_body, redirect_to, stub};
        use std::sync::atomic::Ordering;

        let elsewhere = stub(ok_body(""));
        let first = stub(redirect_to(&format!("{}/elsewhere", elsewhere.url)));
        let client = crate::foyer::client();
        let form = [("Token".to_string(), "dummy".to_string())];
        let err = post(&client, &format!("{}/auth", first.url), &form).unwrap_err();
        assert_eq!(err.1.code(), 7);
        assert!(err.0.message.contains("another host"), "{}", err.0.message);
        assert_eq!(
            elsewhere.hits.load(Ordering::SeqCst),
            0,
            "the second host must never be contacted"
        );
        let first_requests = first.requests.lock().unwrap();
        assert_eq!(first_requests.len(), 1);
        assert!(
            String::from_utf8_lossy(&first_requests[0]).contains("Token=dummy"),
            "the form reached only the original host"
        );
        assert!(
            !String::from_utf8_lossy(&first_requests[0])
                .to_ascii_lowercase()
                .contains("x-goog-api-key"),
            "a form request carries no API key"
        );
    }
}
