//! The foyer RPC client: `POST <BASE><Service>/<Method>` with positional-JSON
//! bodies. With a Bearer, send no `X-Goog-Api-Key` — the web app's key
//! belongs to another Google project and foyer rejects it (HTTP 400).

use home_store::{ErrorBody, Exit, Store};
use reqwest::blocking::{Client, ClientBuilder};
use serde_json::Value;

use crate::{reauth, Session};

pub const STRUCTURES: &str = "StructuresService";
pub const GET_HOME_GRAPH: &str = "GetHomeGraph";
const CONTENT_TYPE: &str = "application/json+protobuf";
const X_USER_AGENT: &str = "grpc-web-javascript/0.1";

pub fn client() -> Client {
    ClientBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("building the HTTP client")
}

/// The base ends at the package prefix (`…foyer.v1.`), so the path is
/// appended with no separator.
fn url_for(base: &str, service: &str, method: &str) -> String {
    format!("{base}{service}/{method}")
}

/// Call `Service/Method`; a 401/403 mints one fresh Bearer and retries once.
pub fn rpc(
    session: &mut Session,
    store: &Store,
    method: &str,
    body: &Value,
) -> Result<Value, (ErrorBody, Exit)> {
    let url = url_for(&session.cfg().foyer_url, STRUCTURES, method);
    let bearer = session.bearer(store)?;
    let (status, text) = post(session.client(), &url, body, &bearer)?;
    let (status, text) = if status == 401 || status == 403 {
        let bearer = session.refresh_bearer(store)?;
        post(session.client(), &url, body, &bearer)?
    } else {
        (status, text)
    };
    read_answer(status, &text)
}

fn post(
    client: &Client,
    url: &str,
    body: &Value,
    bearer: &str,
) -> Result<(u16, String), (ErrorBody, Exit)> {
    let resp = client
        .post(url)
        .bearer_auth(bearer)
        .header(reqwest::header::CONTENT_TYPE, CONTENT_TYPE)
        .header("X-User-Agent", X_USER_AGENT)
        .body(body.to_string())
        .send()
        .map_err(|e| {
            (
                ErrorBody::new("upstream", format!("foyer request: {}", e.without_url())),
                Exit::Upstream,
            )
        })?;
    let status = resp.status().as_u16();
    let text = resp.text().map_err(|e| {
        (
            ErrorBody::new(
                "upstream",
                format!("reading the foyer response: {}", e.without_url()),
            ),
            Exit::Upstream,
        )
    })?;
    Ok((status, text))
}

fn read_answer(status: u16, text: &str) -> Result<Value, (ErrorBody, Exit)> {
    match status {
        200..=299 => serde_json::from_str(text).map_err(|e| {
            (
                ErrorBody::new("upstream", format!("foyer returned non-JSON: {e}")),
                Exit::Upstream,
            )
        }),
        401 | 403 => Err(reauth(format!(
            "foyer rejected the credential (HTTP {status}); run `home gh login` again"
        ))),
        _ => Err((
            ErrorBody::new(
                "upstream",
                format!("foyer HTTP {status}: {}", snippet(text)),
            ),
            Exit::Upstream,
        )),
    }
}

fn snippet(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() > 300 {
        format!("{}…", t.chars().take(300).collect::<String>())
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_against_the_configured_base() {
        assert_eq!(
            url_for("http://127.0.0.1:1/$rpc/x.", STRUCTURES, GET_HOME_GRAPH),
            "http://127.0.0.1:1/$rpc/x.StructuresService/GetHomeGraph"
        );
    }

    #[test]
    fn a_refusal_is_reauth_and_a_server_error_is_upstream() {
        let err = read_answer(403, "").unwrap_err();
        assert_eq!(err.0.code, "reauth_required");
        assert_eq!(err.1.code(), 5);
        let err = read_answer(500, "boom").unwrap_err();
        assert_eq!(err.0.code, "upstream");
        assert_eq!(err.1.code(), 7);
    }
}
