//! The foyer RPC client: `POST <BASE><Service>/<Method>` with positional-JSON
//! bodies. With a Bearer, send no `X-Goog-Api-Key` — the web app's key
//! belongs to another Google project and foyer rejects it (HTTP 400).
//!
//! One HTTP client serves both foyer and the account-token endpoint, and it
//! follows redirects only within the original origin: a 307/308 to another
//! host would re-send a body that holds a Bearer or a master token.

use std::time::Duration;

use home_store::{ErrorBody, Exit, Store};
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::redirect;
use reqwest::Url;
use serde_json::Value;

use crate::{reauth, Session};

pub const STRUCTURES: &str = "StructuresService";
pub const GET_HOME_GRAPH: &str = "GetHomeGraph";
pub const HOME_CONTROL: &str = "HomeControlService";
const CONTENT_TYPE: &str = "application/json+protobuf";
const X_USER_AGENT: &str = "grpc-web-javascript/0.1";
/// The same ceiling reqwest's default policy uses.
const MAX_REDIRECTS: usize = 10;

/// Redirects stay on the origin of the first request. A redirect anywhere
/// else is an error, and the client sends nothing to the other host.
pub fn same_host_redirects() -> redirect::Policy {
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

/// The builder every upstream request goes through.
pub fn client_builder() -> ClientBuilder {
    ClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .redirect(same_host_redirects())
}

pub fn client() -> Client {
    client_builder().build().expect("building the HTTP client")
}

/// A reqwest error with its causes appended. `Display` alone drops the
/// policy's own message ("another host"), and so would the user.
pub(crate) fn describe(e: reqwest::Error) -> String {
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

/// The base ends at the package prefix (`…foyer.v1.`), so the path is
/// appended with no separator.
fn url_for(base: &str, service: &str, method: &str) -> String {
    format!("{base}{service}/{method}")
}

/// One `Service/Method` call with a Bearer, no session and no retry: the
/// tests drive the client through it; [`rpc`] adds the bearer refresh.
#[cfg(test)]
fn call(
    client: &Client,
    base: &str,
    bearer: &str,
    method: &str,
    body: &Value,
) -> Result<Value, (ErrorBody, Exit)> {
    let url = url_for(base, STRUCTURES, method);
    let (status, text) = post(client, &url, body, bearer)?;
    read_answer(status, &text)
}

/// Call `Service/Method`; a 401/403 mints one fresh Bearer and retries once.
pub fn rpc(
    session: &mut Session,
    store: &Store,
    method: &str,
    body: &Value,
) -> Result<Value, (ErrorBody, Exit)> {
    rpc_service(session, store, STRUCTURES, method, body)
}

/// [`rpc`] against any service: `StructuresService` for the home graph,
/// `HomeControlService` for traits.
pub fn rpc_service(
    session: &mut Session,
    store: &Store,
    service: &str,
    method: &str,
    body: &Value,
) -> Result<Value, (ErrorBody, Exit)> {
    let url = url_for(&session.cfg().foyer_url, service, method);
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
                ErrorBody::new("upstream", format!("foyer request: {}", describe(e))),
                Exit::Upstream,
            )
        })?;
    let status = resp.status().as_u16();
    let text = resp.text().map_err(|e| {
        (
            ErrorBody::new(
                "upstream",
                format!("reading the foyer response: {}", describe(e)),
            ),
            Exit::Upstream,
        )
    })?;
    Ok((status, text))
}

/// Some clients6 endpoints prepend the XSSI guard `)]}'` on its own line;
/// strip it before parsing, exactly as upstream does.
fn strip_xssi(body: &str) -> &str {
    let t = body.trim_start();
    match t.strip_prefix(")]}'") {
        Some(rest) => rest.trim_start_matches(['\r', '\n']),
        None => body,
    }
}

fn read_answer(status: u16, text: &str) -> Result<Value, (ErrorBody, Exit)> {
    match status {
        200..=299 => serde_json::from_str(strip_xssi(text)).map_err(|e| {
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
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// A one-answer HTTP stub: its base URL, a connection counter and the raw
    /// bytes of every request, so a test can prove that the host a redirect
    /// points at was never contacted.
    pub(crate) struct Stub {
        pub url: String,
        pub hits: Arc<AtomicUsize>,
        pub requests: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    pub(crate) fn stub(answer: String) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = hits.clone();
        let recorded = requests.clone();
        let _ = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let request = read_request(&mut stream);
                seen.fetch_add(1, Ordering::SeqCst);
                recorded.lock().unwrap().push(request);
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
            }
        });
        Stub {
            url,
            hits,
            requests,
        }
    }

    pub(crate) fn ok_body(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    pub(crate) fn redirect_to(place: &str) -> String {
        format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {place}\r\nConnection: close\r\n\
             Content-Length: 0\r\n\r\n"
        )
    }

    /// Read one request: headers, then as much of the `Content-Length` body as
    /// the client sent, so the stub answers only after the request is complete.
    fn read_request(stream: &mut TcpStream) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(head_end) = find(&buf, b"\r\n\r\n") {
                        let length = content_length(&buf[..head_end]);
                        if buf.len() >= head_end + 4 + length {
                            break;
                        }
                    } else if buf.len() > 64 * 1024 {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        buf
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn content_length(head: &[u8]) -> usize {
        String::from_utf8_lossy(head)
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .unwrap_or(0)
    }

    #[test]
    fn paths_resolve_against_the_configured_base() {
        assert_eq!(
            url_for("http://127.0.0.1:1/$rpc/x.", STRUCTURES, GET_HOME_GRAPH),
            "http://127.0.0.1:1/$rpc/x.StructuresService/GetHomeGraph"
        );
    }

    #[test]
    fn the_xssi_guard_line_is_stripped() {
        assert_eq!(strip_xssi(")]}'\n[1,2]"), "[1,2]");
        assert_eq!(strip_xssi(")]}'\r\n[1,2]"), "[1,2]");
        assert_eq!(strip_xssi(" \n)]}'\n[1,2]"), "[1,2]");
        assert_eq!(strip_xssi("[1,2]"), "[1,2]");
        assert_eq!(
            read_answer(200, ")]}'\n[1,2]").unwrap(),
            json!([1, 2]),
            "a guarded answer parses after the guard line"
        );
        assert_eq!(read_answer(200, "[1,2]").unwrap(), json!([1, 2]));
    }

    #[test]
    fn a_guarded_answer_that_is_not_json_is_still_an_upstream_error() {
        let err = read_answer(200, ")]}'\nnot json").unwrap_err();
        assert_eq!(err.0.code, "upstream");
        assert_eq!(err.1.code(), 7);
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

    #[test]
    fn a_redirect_to_another_host_is_refused_and_never_contacted() {
        let elsewhere = stub(ok_body("[]"));
        let first = stub(redirect_to(&format!("{}/elsewhere", elsewhere.url)));
        let client = client();
        let (body, exit) = call(
            &client,
            &format!("{}/", first.url),
            "dummy-bearer",
            GET_HOME_GRAPH,
            &json!([]),
        )
        .unwrap_err();
        assert_eq!(exit.code(), 7);
        assert!(
            body.message.contains("another host"),
            "the refusal must say why: {}",
            body.message
        );
        assert_eq!(
            elsewhere.hits.load(Ordering::SeqCst),
            0,
            "no request may reach the second host"
        );
        assert_eq!(
            first.hits.load(Ordering::SeqCst),
            1,
            "the first host was hit"
        );
    }

    /// A redirect to the same host but another port is another origin too.
    #[test]
    fn a_redirect_to_the_same_host_on_another_port_is_refused() {
        let elsewhere = stub(ok_body("[]"));
        let port = elsewhere.url.rsplit(':').next().unwrap();
        let first = stub(redirect_to(&format!("http://localhost:{port}/x")));
        let client = client();
        let (body, exit) = call(
            &client,
            &format!("{}/", first.url),
            "dummy-bearer",
            GET_HOME_GRAPH,
            &json!([]),
        )
        .unwrap_err();
        assert_eq!(exit.code(), 7);
        assert!(body.message.contains("another host"), "{}", body.message);
        assert_eq!(elsewhere.hits.load(Ordering::SeqCst), 0);
    }

    /// Same port, other host name: the stub answers on `127.0.0.1:port` and
    /// redirects to `localhost:port`. Both resolve to the same machine, but
    /// the origin differs, so the hop is refused and the stub is hit once.
    #[test]
    fn a_redirect_to_the_same_port_under_another_host_name_is_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let answer = redirect_to(&format!("http://localhost:{port}/x"));
        let _ = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = read_request(&mut stream);
                seen.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
            }
        });
        let url = format!("http://127.0.0.1:{port}/");
        let target = Url::parse(&format!("http://localhost:{port}/x")).unwrap();
        let source = Url::parse(&url).unwrap();
        assert_eq!(
            source.port_or_known_default(),
            target.port_or_known_default(),
            "the ports match, so only the host name differs"
        );
        let client = client();
        let (body, exit) =
            call(&client, &url, "dummy-bearer", GET_HOME_GRAPH, &json!([])).unwrap_err();
        assert_eq!(exit.code(), 7);
        assert!(body.message.contains("another host"), "{}", body.message);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "the redirect target is never requested"
        );
    }

    #[test]
    fn a_redirect_within_the_origin_is_followed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let finals = Arc::new(AtomicUsize::new(0));
        let reached = finals.clone();
        let _ = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let request = read_request(&mut stream);
                let answer = if request.starts_with(b"POST /first ") {
                    redirect_to("/second")
                } else {
                    reached.fetch_add(1, Ordering::SeqCst);
                    ok_body("[]")
                };
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
            }
        });
        let client = client();
        let (status, text) = post(&client, &format!("{base}/first"), &json!([]), "dummy").unwrap();
        assert_eq!(status, 200);
        assert_eq!(text, "[]");
        assert_eq!(
            finals.load(Ordering::SeqCst),
            1,
            "the same origin is followed"
        );
    }

    #[test]
    fn a_redirect_loop_on_one_origin_stops() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let answering = base.clone();
        let _ = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = read_request(&mut stream);
                let _ = stream.write_all(redirect_to(&format!("{answering}/loop")).as_bytes());
                let _ = stream.flush();
            }
        });
        let client = client();
        let (body, exit) = call(
            &client,
            &format!("{base}/"),
            "dummy-bearer",
            GET_HOME_GRAPH,
            &json!([]),
        )
        .unwrap_err();
        assert_eq!(exit.code(), 7);
        assert!(
            body.message.contains("more than 10 redirects"),
            "{}",
            body.message
        );
    }
}
