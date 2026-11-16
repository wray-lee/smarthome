//! The Smart Life QR sign-in.
//!
//! Ported from Alegzandr/lueur `crates/lueur-shell/src/tuya_cloud.rs`,
//! commit be7ba9b14be8fc1031ee962c9069c744e88e8252, MIT; Copyright (c) 2026
//! Lueur contributors. See `LICENSE-tuya` in this crate.
//!
//! `POST /v1.0/m/life/home-assistant/qrcode/tokens` asks for a token, the
//! user scans `tuyaSmart--qrLogin?token=<token>` in the Smart Life app, and
//! `GET` on the same path with the token returns the session once it has been
//! approved. Until then the envelope is `success: false`, which is not an
//! error here: the poll simply goes round again.
//!
//! Nothing in this module prints a token. It returns what the caller needs
//! and the caller decides what to show.

use std::time::Duration;

use home_store::Store;
use serde_json::Value;

use crate::config::{Config, CLIENT_ID, SCHEMA};
use crate::session::{self, Failure, Session};

/// A sign-in waiting for its QR code to be scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The QR token, used for the polls. Not a credential, but not shown
    /// either beyond the QR itself.
    pub token: String,
    /// What the QR code encodes: `tuyaSmart--qrLogin?token=<token>`.
    pub qr_text: String,
    /// The same QR as UTF-8 terminal text, ready to write to stderr.
    pub qr_terminal: String,
}

/// User codes and QR tokens are short alphanumerics; anything else is refused
/// before it reaches a URL.
pub fn valid_code(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A user code as typed or pasted, without the spaces and dashes people put
/// in to read it more easily. Letter case is kept: the codes mix both, and
/// changing it would turn a good code into a wrong one.
pub fn normalize_code(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(c, '-' | '_' | '.' | '\u{2010}'..='\u{2015}' | '\u{2212}')
        })
        .collect()
}

/// The QR code as block characters, two module rows per terminal line.
///
/// `Dense1x2` needs no image crate (the `qrcode` dependency is built with no
/// default features), and every byte is UTF-8 text, so stderr is safe.
pub fn qr_terminal(text: &str) -> Result<String, Failure> {
    let code = qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| Failure::upstream(format!("could not build the QR code: {e}")))?;
    use qrcode::render::unicode::Dense1x2;
    let image = code
        .render::<Dense1x2>()
        .quiet_zone(true)
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .build();
    Ok(image)
}

/// The QR code as a compact inline SVG string for the panel.
///
/// `qrcode`'s render module is available without default features, so the
/// modules are stitched by hand: a white background rect for the quiet zone
/// and one black path of unit rectangles. `viewBox` is in module units (with
/// a quiet zone of 4 modules per side), `shape-rendering="crispEdges"` keeps
/// the modules sharp, and the image is labelled for assistive technology.
pub fn qr_svg(text: &str) -> Result<String, Failure> {
    let code = qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| Failure::upstream(format!("could not build the QR code: {e}")))?;
    const QUIET: u16 = 4;
    let modules = u16::try_from(code.width()).unwrap_or(0);
    let side = modules.saturating_add(QUIET * 2);
    // Empty text yields no modules; refuse rather than emit a blank image.
    if modules == 0 {
        return Err(Failure::upstream("could not build the QR code: empty content"));
    }
    let mut d = String::new();
    let colors = code.to_colors();
    for (i, color) in colors.iter().enumerate() {
        if *color == qrcode::Color::Dark {
            let x = (i % modules as usize) as u16;
            let y = (i / modules as usize) as u16;
            let dx = x + QUIET;
            let dy = y + QUIET;
            d.push_str(&format!("M{dx} {dy}h1v1h-1Z"));
        }
    }
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} {side}" shape-rendering="crispEdges" role="img" aria-label="Tuya sign-in QR code"><rect fill="#ffffff" width="{side}" height="{side}"/><path fill="#000000" d="{d}"/></svg>"##
    ))
}

/// Ask for a sign-in QR code for the account with `user_code` (shown in the
/// Smart Life app under Me → Settings → Account and Security → User Code).
pub fn start_login(cfg: &Config, user_code: &str) -> Result<Pending, Failure> {
    let user_code = normalize_code(user_code);
    if !valid_code(&user_code) {
        return Err(Failure {
            body: home_store::ErrorBody::new(
                "usage",
                "a user code is 4 to 64 letters and digits; find it in the Smart Life app \
                 under Me → Settings → Account and Security → User Code",
            ),
            exit: home_store::Exit::Usage,
            code: None,
        });
    }
    let url = format!("{}/v1.0/m/life/home-assistant/qrcode/tokens", cfg.login_url);
    let response = session::http_client(&cfg.login_url)
        .post(&url)
        .query(&[
            ("clientid", CLIENT_ID),
            ("usercode", user_code.as_str()),
            ("schema", SCHEMA),
        ])
        .send()
        .map_err(|e| Failure::upstream(session::describe_error(e)))?;
    let envelope = read_envelope(response)?;
    let token = envelope
        .pointer("/result/qrcode")
        .and_then(Value::as_str)
        .filter(|t| valid_code(t))
        .map(str::to_string)
        .ok_or_else(|| Failure::upstream("the login answer carried no QR token"))?;
    let qr_text = qr_payload(&token);
    let qr_terminal = qr_terminal(&qr_text)?;
    Ok(Pending {
        token,
        qr_text,
        qr_terminal,
    })
}

/// The literal string the Smart Life app expects in the QR code.
pub fn qr_payload(token: &str) -> String {
    format!("tuyaSmart--qrLogin?token={token}")
}

/// One poll: `Some` once the code has been approved, `None` while it has not.
pub fn poll_login(cfg: &Config, token: &str, user_code: &str) -> Result<Option<Session>, Failure> {
    if !valid_code(token) {
        return Err(Failure::upstream("unusable QR token"));
    }
    let user_code = normalize_code(user_code);
    if !valid_code(&user_code) {
        return Err(Failure::upstream("unusable user code"));
    }
    let url = format!(
        "{}/v1.0/m/life/home-assistant/qrcode/tokens/{token}",
        cfg.login_url
    );
    let response = session::http_client(&cfg.login_url)
        .get(&url)
        .query(&[("clientid", CLIENT_ID), ("usercode", user_code.as_str())])
        .send()
        .map_err(|e| Failure::upstream(session::describe_error(e)))?;
    let envelope = read_envelope(response)?;
    if envelope.get("success").and_then(Value::as_bool) != Some(true) {
        // Not scanned yet (or the code expired); the caller decides whether
        // to keep waiting.
        return Ok(None);
    }
    let result = envelope.get("result").cloned().unwrap_or(Value::Null);
    let text = |key: &str| {
        result
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let endpoint = text("endpoint").trim_end_matches('/').to_string();
    // The login host itself is fixed; only a fake upstream (an explicit
    // `HOME_TUYA_LOGIN_URL` on loopback) may hand back a loopback endpoint.
    // Otherwise it must be a Tuya gateway, so a tampered answer cannot point
    // the client elsewhere.
    if !crate::config::valid_endpoint(&endpoint)
        && !(cfg.login_is_local() && crate::config::is_local_http(&endpoint))
    {
        return Err(Failure::upstream(format!(
            "unexpected Tuya API host `{endpoint}`"
        )));
    }
    let access_token = text("access_token");
    let refresh_token = text("refresh_token");
    if access_token.is_empty() || refresh_token.is_empty() {
        return Err(Failure::upstream(
            "the login answer carried no access or refresh token",
        ));
    }
    // `expire_time` is a lifetime in seconds counted from the envelope's `t`,
    // not from the local clock (the SDK does the same).
    let t = envelope.get("t").and_then(Value::as_i64).unwrap_or(0);
    let lifetime = result
        .get("expire_time")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Ok(Some(Session {
        access_token,
        refresh_token,
        endpoint,
        user_code: user_code.clone(),
        terminal_id: text("terminal_id"),
        uid: text("uid"),
        expire_time: if lifetime > 0 { t / 1000 + lifetime } else { 0 },
    }))
}

/// The whole QR sign-in: ask, poll every `cfg.poll_ms` up to
/// `cfg.poll_attempts` times, and persist the session on success.
///
/// `announce` is called once with the rendered QR so the caller can put it
/// on stderr (stdout stays reserved for the one JSON document).
pub fn login(
    cfg: &Config,
    store: &Store,
    user_code: &str,
    announce: &mut dyn FnMut(&Pending),
) -> Result<Session, Failure> {
    let pending = start_login(cfg, user_code)?;
    announce(&pending);
    for attempt in 0..cfg.poll_attempts {
        if attempt > 0 && cfg.poll_ms > 0 {
            std::thread::sleep(Duration::from_millis(cfg.poll_ms));
        }
        if let Some(session) = poll_login(cfg, &pending.token, user_code)? {
            session.save(store).map_err(Failure::storage)?;
            return Ok(session);
        }
    }
    Err(Failure::upstream(format!(
        "the QR code was not scanned in time ({} polls of {} ms); run `home tuya login` again",
        cfg.poll_attempts, cfg.poll_ms
    )))
}

fn read_envelope(response: reqwest::blocking::Response) -> Result<Value, Failure> {
    let status = response.status();
    if !status.is_success() {
        return Err(Failure::upstream(format!("Tuya answered HTTP {status}")));
    }
    response
        .json::<Value>()
        .map_err(|e| Failure::upstream(session::describe_error(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_checked() {
        assert!(valid_code("CavB15p"));
        assert!(!valid_code("ab"));
        assert!(!valid_code("abc/../def"));
        assert!(!valid_code("abc&x=1"));
    }

    #[test]
    fn user_codes_lose_separators_but_keep_case() {
        assert_eq!(normalize_code("CavB15p"), "CavB15p");
        assert_eq!(normalize_code("  CavB 15p \n"), "CavB15p");
        assert_eq!(normalize_code("Cav-B15-p"), "CavB15p");
        assert_eq!(normalize_code("Cav_B.15p"), "CavB15p");
        assert_eq!(normalize_code("Cav\u{00A0}B\u{202F}15\u{2013}p"), "CavB15p");
        assert_eq!(normalize_code("cAVb15P"), "cAVb15P");
        // Separators only: anything that could reach a URL still fails.
        assert!(!valid_code(&normalize_code("abc/../def")));
        assert!(!valid_code(&normalize_code("abc&x=1")));
        assert!(!valid_code(&normalize_code("Cavé15p")));
        assert!(!valid_code(&normalize_code(" - - ")));
    }

    #[test]
    fn the_qr_text_is_the_sdk_payload() {
        assert_eq!(qr_payload("abc123"), "tuyaSmart--qrLogin?token=abc123");
    }

    #[test]
    fn the_qr_renders_as_utf8_block_characters() {
        let image = qr_terminal(&qr_payload("abc123")).unwrap();
        assert!(image.lines().count() > 8, "{image}");
        assert!(
            image.contains('\u{2588}') || image.contains('\u{2580}') || image.contains('\u{2584}')
        );
        // Two module rows per line, so half-height blocks are in use.
        assert!(image.contains('\u{2580}') || image.contains('\u{2584}'));
        // No escape sequences: this goes to stderr as plain text.
        assert!(!image.contains('\u{1b}'));
    }

    #[test]
    fn the_qr_renders_as_inline_svg_without_scripts() {
        let svg = qr_svg(&qr_payload("abc123")).unwrap();
        assert!(svg.starts_with("<svg"), "{svg}");
        assert!(svg.contains("xmlns=\"http://www.w3.org/2000/svg\""), "{svg}");
        assert!(svg.contains("viewBox="), "{svg}");
        assert!(svg.contains("shape-rendering=\"crispEdges\""), "{svg}");
        assert!(svg.contains("role=\"img\""), "{svg}");
        assert!(svg.contains("aria-label=\"Tuya sign-in QR code\""), "{svg}");
        assert!(svg.contains("<rect fill=\"#ffffff\""), "{svg}");
        assert!(svg.contains("<path fill=\"#000000\" d=\""), "{svg}");
        assert!(!svg.contains("<script"), "{svg}");
        assert!(!svg.contains('\u{1b}'), "{svg}");
        assert!(svg.ends_with("</svg>"), "{svg}");
    }
}
