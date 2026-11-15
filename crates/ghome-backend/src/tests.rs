//! The three operating functions against a scripted fake foyer: what goes on
//! the wire, and which refusal happens before it.

use super::*;
use serde_json::json;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const GRAPH_PATH: &str = "/$rpc/google.internal.home.foyer.v1.StructuresService/GetHomeGraph";
const GET_PATH: &str = "/$rpc/google.internal.home.foyer.v1.HomeControlService/GetTraits";
const UPDATE_PATH: &str = "/$rpc/google.internal.home.foyer.v1.HomeControlService/UpdateTraits";
const AUTH_PATH: &str = "/auth";

fn bw(v: bool) -> Value {
    json!([null, null, null, if v { 1 } else { 0 }])
}

fn answer(id: &str, traits: Value) -> String {
    json!([[[[id], traits]]]).to_string()
}

fn on_off_state(id: &str, online: i64, on: bool) -> String {
    answer(
        id,
        json!([
            ["deviceStatus", [["online", [null, null, null, online]]]],
            ["onOff", [["onOff", bw(on)]]]
        ]),
    )
}

/// Two homes: one Lamp in 主卧, a second Lamp and a lock in Office.
fn graph() -> String {
    json!([
        "<num>",
        [
            [
                "home-1",
                "home",
                [],
                [],
                null,
                [[
                    "room-bed",
                    null,
                    "主卧",
                    ["MASTER_BEDROOM"],
                    [[["dup-1", ["agent-a", "p-1"]]]]
                ]],
                [[
                    ["dup-1", ["agent-a", "p-1"]],
                    null,
                    null,
                    "Lamp",
                    null,
                    "action.devices.types.LIGHT",
                    [
                        "action.devices.traits.OnOff",
                        "action.devices.traits.Brightness"
                    ]
                ]]
            ],
            [
                "home-2",
                "office",
                [],
                [],
                null,
                [[
                    "room-office",
                    null,
                    "Office",
                    ["OFFICE"],
                    [
                        [["dup-2", ["agent-b", "p-2"]]],
                        [["lock-1", ["agent-c", "p-3"]]]
                    ]
                ]],
                [
                    [
                        ["dup-2", ["agent-b", "p-2"]],
                        null,
                        null,
                        "Lamp",
                        null,
                        "action.devices.types.LIGHT",
                        ["action.devices.traits.OnOff"]
                    ],
                    [
                        ["lock-1", ["agent-c", "p-3"]],
                        null,
                        null,
                        "Front Door",
                        null,
                        "action.devices.types.LOCK",
                        [
                            "action.devices.traits.OnOff",
                            "action.devices.traits.LockUnlock"
                        ]
                    ]
                ]
            ]
        ]
    ])
    .to_string()
}

struct Route {
    path: String,
    replies: Vec<(u16, String)>,
    hits: usize,
}

/// A multi-route HTTP fake: every path answers from its own reply list (the
/// last reply repeating) and every request is recorded.
struct Fake {
    base: String,
    calls: Arc<Mutex<Vec<(String, String)>>>,
    routes: Arc<Mutex<Vec<Route>>>,
}

impl Fake {
    fn start(routes: Vec<(&str, Vec<(u16, String)>)>) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let calls: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let routes = Arc::new(Mutex::new(
            routes
                .into_iter()
                .map(|(path, replies)| Route {
                    path: path.to_string(),
                    replies,
                    hits: 0,
                })
                .collect::<Vec<_>>(),
        ));
        let table = routes.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (path, body) = read_request(&mut stream);
                recorded.lock().unwrap().push((path.clone(), body));
                let mut table = table.lock().unwrap();
                let Some(route) = table.iter_mut().find(|r| r.path == path) else {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                    );
                    continue;
                };
                let (status, body) = route.replies[route.hits.min(route.replies.len() - 1)].clone();
                route.hits += 1;
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    _ => "Error",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nConnection: close\r\n\
                     Content-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Fake {
            base,
            calls,
            routes,
        }
    }

    fn session(&self, store: &Store) -> Session {
        Credential {
            master: "aas_et/dummy-master".into(),
            android_id: "0123456789abcdef".into(),
            email: "user@example.com".into(),
            bearer: Some("cached-bearer".into()),
            bearer_expires_at: Some(now_unix() + 3600),
        }
        .save(store)
        .unwrap();
        let cfg = config::Config {
            foyer_url: format!("{}/$rpc/google.internal.home.foyer.v1.", self.base),
            gpsoauth_url: format!("{}{AUTH_PATH}", self.base),
        };
        Session::open(store, cfg).unwrap()
    }

    fn hits(&self, path: &str) -> usize {
        self.routes
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.path == path)
            .map(|r| r.hits)
            .unwrap_or(0)
    }

    fn bodies(&self, path: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, b)| b.clone())
            .collect()
    }

    /// Every path the fake saw, in order, including unmatched ones.
    fn paths(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _)| p.clone())
            .collect()
    }
}

fn read_request(stream: &mut TcpStream) -> (String, String) {
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
    let head_end = find(&buf, b"\r\n\r\n").unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("")
        .to_string();
    let body = String::from_utf8_lossy(&buf[(head_end + 4).min(buf.len())..]).to_string();
    (path, body)
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

fn happy_foyer(get: &str, update: &str) -> Fake {
    Fake::start(vec![
        (GRAPH_PATH, vec![(200, graph())]),
        (GET_PATH, vec![(200, get.to_string())]),
        (UPDATE_PATH, vec![(200, update.to_string())]),
    ])
}

#[test]
fn get_state_reads_the_traits_and_keeps_the_kind_field() {
    let foyer = happy_foyer(&on_off_state("dup-2", 1, true), "[]");
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    let doc = get_state(&mut session, &store, "dup-2").unwrap();
    assert_eq!(doc["id"], "dup-2");
    assert_eq!(doc["name"], "Lamp");
    assert_eq!(doc["room"], "Office");
    assert_eq!(doc["kind"], "action.devices.types.LIGHT");
    assert_eq!(doc["backend"], "google_home");
    assert_eq!(doc["online"], true);
    assert_eq!(doc["traits"]["onOff"]["onOff"], true);
    assert!(doc.get("type").is_none(), "the metadata field is kind");
    assert_eq!(foyer.bodies(GET_PATH), vec![r#"[[["dup-2"]]]"#.to_string()]);
    // The credential was cached, so no Bearer was minted.
    assert_eq!(foyer.hits(AUTH_PATH), 0);
}

#[test]
fn room_and_name_resolve_before_a_bare_name() {
    // The fake answers with a fixed id, so each resolution gets its own fake.
    let foyer = happy_foyer(&on_off_state("dup-1", 1, false), "[]");
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    assert_eq!(
        get_state(&mut session, &store, "主卧/Lamp").unwrap()["id"],
        "dup-1"
    );
    let foyer = happy_foyer(&on_off_state("dup-2", 1, false), "[]");
    let mut session = foyer.session(&store);
    assert_eq!(
        get_state(&mut session, &store, "office/lamp").unwrap()["id"],
        "dup-2"
    );
    // Two Devices are named Lamp, so the bare name alone is ambiguous.
    let (body, exit) = get_state(&mut session, &store, "Lamp").unwrap_err();
    assert_eq!(exit.code(), 3);
    assert_eq!(body.code, "ambiguous");
    assert_eq!(body.backend, Some("google_home"));
    assert_eq!(body.candidates.len(), 2);
    let first: Value = serde_json::from_str(&body.candidates[0]).unwrap();
    assert_eq!(first["id"], "dup-1");
    assert_eq!(first["name"], "Lamp");
    assert_eq!(first["room"], "主卧");
    assert_eq!(first["kind"], "action.devices.types.LIGHT");
}

#[test]
fn an_offline_device_is_exit_four_on_get() {
    let foyer = happy_foyer(&on_off_state("dup-2", 0, true), "[]");
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    let (body, exit) = get_state(&mut session, &store, "dup-2").unwrap_err();
    assert_eq!(exit.code(), 4);
    assert_eq!(body.code, "device_offline");
    assert_eq!(body.backend, Some("google_home"));
}

#[test]
fn set_trait_sends_the_partner_ids_and_reports_the_change() {
    let foyer = happy_foyer(
        &on_off_state("dup-2", 1, true),
        &on_off_state("dup-2", 1, false),
    );
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    let doc = set_trait(
        &mut session,
        &store,
        "office/Lamp",
        TraitChange::OnOff(false),
        false,
    )
    .unwrap();
    assert_eq!(doc["id"], "dup-2");
    assert_eq!(doc["kind"], "action.devices.types.LIGHT");
    assert_eq!(doc["changed"]["trait"], "onOff");
    assert_eq!(doc["changed"]["value"], false);
    assert_eq!(
        serde_json::from_str::<Value>(&foyer.bodies(UPDATE_PATH)[0]).unwrap(),
        json!([[[
            ["dup-2", ["agent-b", "p-2"]],
            [["onOff", [["onOff", [null, null, null, 0]]]]]
        ]]])
    );
}

#[test]
fn a_protected_device_is_refused_even_with_confirm_and_only_the_graph_is_read() {
    // The fake serves the graph, GetTraits and UpdateTraits, so an absent
    // request below can only mean the guard stopped the call, not that a
    // route was missing. Protecting first is what makes this the guard path:
    // resolution needs the graph, and `set_trait` returns at `guard::require`
    // right after it. Without that guard the same call would go on to send
    // the offline-check `GetTraits` and then `UpdateTraits`, so the counts
    // below distinguish the two behaviours. The Device is reported offline in
    // the fake's `GetTraits` answer: if the guard ever moved behind the
    // offline check, this test would see exit 4 `device_offline` and a
    // non-zero `GetTraits` instead of exit 6.
    let foyer = happy_foyer(&on_off_state("dup-2", 0, true), "[]");
    let store = Store::open_in_memory().unwrap();
    store.protect(BACKEND, "dup-2").unwrap();
    let mut session = foyer.session(&store);
    let (body, exit) = set_trait(
        &mut session,
        &store,
        "dup-2",
        TraitChange::OnOff(false),
        true,
    )
    .unwrap_err();
    assert_eq!(exit.code(), 6);
    assert_eq!(body.code, "protected");
    assert_eq!(body.backend, Some("google_home"));
    // The only upstream call of any kind is the name-resolution graph read:
    // no trait RPC at all, so no offline check and no write.
    assert_eq!(
        foyer.paths(),
        vec![GRAPH_PATH.to_string()],
        "only GetHomeGraph may be sent before the guard refuses"
    );
    assert_eq!(
        foyer.hits(GET_PATH),
        0,
        "the guard stops before any trait RPC (no offline check)"
    );
    assert_eq!(foyer.hits(UPDATE_PATH), 0, "no write may be sent");
}

#[test]
fn unlock_and_disarm_need_confirm_and_send_nothing_without_it() {
    let foyer = happy_foyer(&on_off_state("lock-1", 1, true), "[]");
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    let (body, exit) = set_trait(
        &mut session,
        &store,
        "lock-1",
        TraitChange::Lock(false),
        false,
    )
    .unwrap_err();
    assert_eq!(exit.code(), 6);
    assert_eq!(body.code, "sensitive_action");
    assert_eq!(body.backend, Some("google_home"));
    assert_eq!(foyer.hits(UPDATE_PATH), 0);
    // The guard passed (the Device is not protected), so the stop is the
    // Sensitive Action itself: no trait RPC, offline check included, was
    // sent, and the only upstream call is the name-resolution graph read.
    assert_eq!(
        foyer.paths(),
        vec![GRAPH_PATH.to_string()],
        "only GetHomeGraph may be sent before the Sensitive Action refusal"
    );
    assert_eq!(foyer.hits(GET_PATH), 0, "no state read before the refusal");
    assert!(body.message.contains("unlocking"), "{}", body.message);
}

#[test]
fn confirming_an_unlock_sends_is_locked_false() {
    let foyer = happy_foyer(
        &on_off_state("lock-1", 1, true),
        &on_off_state("lock-1", 1, false),
    );
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    set_trait(
        &mut session,
        &store,
        "lock-1",
        TraitChange::Lock(false),
        true,
    )
    .unwrap();
    let sent: Value = serde_json::from_str(&foyer.bodies(UPDATE_PATH)[0]).unwrap();
    assert_eq!(
        sent[0][0][1],
        json!([["lockUnlock", [["isLocked", [null, null, null, 0]]]]])
    );
}

#[test]
fn a_trait_the_device_does_not_advertise_is_usage() {
    let foyer = happy_foyer(&on_off_state("dup-2", 1, true), "[]");
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    // dup-2 advertises OnOff only, not Brightness.
    let (body, exit) = set_trait(
        &mut session,
        &store,
        "dup-2",
        TraitChange::Brightness(40),
        false,
    )
    .unwrap_err();
    assert_eq!(exit.code(), 2);
    assert_eq!(body.code, "unsupported_trait");
    assert_eq!(foyer.hits(UPDATE_PATH), 0);
}

#[test]
fn an_offline_device_refuses_a_set_with_exit_four() {
    let foyer = happy_foyer(
        &on_off_state("dup-2", 0, true),
        &on_off_state("dup-2", 0, false),
    );
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    let (body, exit) = set_trait(
        &mut session,
        &store,
        "dup-2",
        TraitChange::OnOff(false),
        false,
    )
    .unwrap_err();
    assert_eq!(exit.code(), 4);
    assert_eq!(body.code, "device_offline");
    assert_eq!(foyer.hits(UPDATE_PATH), 0);
}

#[test]
fn a_rejected_bearer_is_minted_once_and_the_call_retried() {
    let foyer = Fake::start(vec![
        (GRAPH_PATH, vec![(401, "nope".to_string()), (200, graph())]),
        (GET_PATH, vec![(200, on_off_state("dup-2", 1, true))]),
        (
            AUTH_PATH,
            vec![(200, "Auth=fresh-bearer\nExpiry=9999999999\n".to_string())],
        ),
    ]);
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    let doc = get_state(&mut session, &store, "dup-2").unwrap();
    assert_eq!(doc["online"], true);
    assert_eq!(foyer.hits(GRAPH_PATH), 2, "one retry after the 401");
    assert_eq!(foyer.hits(AUTH_PATH), 1, "exactly one fresh Bearer");
    let form = &foyer.bodies(AUTH_PATH)[0];
    assert!(
        form.contains("EncryptedPasswd") && form.contains("aas_et%2Fdummy-master"),
        "the mint re-sent the stored master token: {form}"
    );
}

#[test]
fn a_second_rejection_is_reauth_naming_the_google_home_backend() {
    let foyer = Fake::start(vec![
        (GRAPH_PATH, vec![(403, "nope".to_string())]),
        (
            AUTH_PATH,
            vec![(200, "Auth=fresh-bearer\nExpiry=9999999999\n".to_string())],
        ),
    ]);
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    let (body, exit) = get_state(&mut session, &store, "dup-2").unwrap_err();
    assert_eq!(exit.code(), 5);
    assert_eq!(body.code, "reauth_required");
    assert_eq!(body.backend, Some("google_home"));
    assert!(
        body.message.contains("Google Home Backend"),
        "{}",
        body.message
    );
    assert_eq!(foyer.hits(AUTH_PATH), 1);
    assert_eq!(foyer.hits(GRAPH_PATH), 2, "the retry happened once");
}

#[test]
fn list_devices_hides_the_partner_ids_from_the_json() {
    let foyer = Fake::start(vec![(GRAPH_PATH, vec![(200, graph())])]);
    let store = Store::open_in_memory().unwrap();
    let mut session = foyer.session(&store);
    let devices = list_devices(&mut session, &store).unwrap();
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[0].id, "dup-1");
    assert_eq!(devices[0].room.as_deref(), Some("主卧"));
    assert_eq!(devices[0].agent_id.as_deref(), Some("agent-a"));
    assert_eq!(devices[0].partner_device_id.as_deref(), Some("p-1"));
    let json = serde_json::to_string(&devices[0]).unwrap();
    assert!(!json.contains("agent-a"), "{json}");
    assert!(!json.contains("\"p-1\""), "{json}");
    assert!(
        json.contains("\"kind\":\"action.devices.types.LIGHT\""),
        "{json}"
    );
    assert!(!json.contains("\"type\""), "{json}");
}
