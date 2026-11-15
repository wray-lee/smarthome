# tinytuya

## tinytuya (jasonacox/tinytuya) — facts

**Version:** 1.21.1, uploaded 2026-11-12 (PyPI JSON API; `pip index versions` confirms). Requires Python >=3.10, deps `requests`, `colorama`, `cryptography`. Prior: 1.21.0 2026-11-12, 1.20.0 2026-07-06, 1.19.0 2026-06-29, 1.18.0 2026-04-13, 1.17.5 2026-02-21. Source read is the actual 1.21.1 wheel downloaded from PyPI (all claims below verified by reading/executing that wheel, not docs).

### 1. CLI commands that exist (verified from `tinytuya/__main__.py`, `cli.py`, and by running them)

| command | what it does |
|---|---|
| `wizard` | fetches devices + local_key from Tuya IoT Cloud, writes `devices.json` / `tuya-raw.json` / `tinytuya.json` |
| `scan` | UDP broadcast + optional `-force` subnet scan; prints Address / Device ID / Version |
| `devices` | scan everything in `devices.json`, show status |
| `snapshot` | poll devices in `snapshot.json` |
| `json` | same, but stdout is a single JSON doc |
| `list` / `list --json` | dump `devices.json` |
| `on` / `off` | `--dps N` (default 1), `--id` or `--name`, `--key`, `--ip`, `--version` |
| `set` | `--dps N --value V` (value JSON-parsed: `true`, `123`, `"text"`) |
| `get` | full status JSON, or one plain value with `--dps N` |
| `monitor` | persistent connection, prints async pushes |
| `version`, `help` | |

`on/off/set/get/monitor/list` were added in **v1.17.5 (2026-02-21)** and **v1.18.0** (`monitor`) per [RELEASE.md](https://github.com/jasonacox/tinytuya/blob/master/RELEASE.md) — i.e. they are ~9 months old, not ancient.

Output formats, tested against a fake `devices.json` with unreachable IPs:
- `list` → ASCII table. `list --json` → array of `{name,id,key,ip,version}` (key is prefixed, `last_ip` wins over `ip`).
- `json` → `{"timestamp": <float>, "devices":[{id,gwId,ip,version,name,key,mac, "dps": {...} | "error":"No Response"}]}` — good for agents; exit code 0 even when all devices are unreachable.
- `get --dps 19` → bare scalar (`print(json.dumps(value))`), `get` → raw device JSON `{"devId":...,"dps":{...}}`.
- Errors: `Error 905: Network Error: Device Unreachable` — **printed to stdout, exit 1** (verified; stderr stays empty). So an agent must check exit code, not just JSON-parse stdout.
- No `cloud` CLI subcommand, and no `status` command (`get` is status).

Latency caveat (measured): a `get` against an offline device took **45 s** before erroring — default `connection_retry_limit=5`, 5 s timeout, 5 s delay. Not usable for an agent loop without tuning (the CLI has no timeout flag; `-maxtime` only affects scanning).

### 2. Wizard and local_key — yes, it requires the Tuya IoT developer platform

`wizard` builds `tinytuya.Cloud(**config)` and calls `getdevices()`. Keys come only from the cloud. `wizard -dry-run` (`nocloud=True`) skips the cloud but then just reuses an existing `devices.json`.

**The well-known failure:** Tuya's IoT Core is a free trial (starts ~1 month). When it lapses:
- **28841002** — "No permissions. Your subscription to cloud development plan has expired." (primary code; [Tuya global error codes](https://developer.tuya.com/en/docs/iot/error-code?id=K989ruxx88swc), [tinytuya #660](https://github.com/jasonacox/tinytuya/issues/660) 2025-11-09)
- **28841102** "API subscription expired", **28841004** "Trial Edition quota is exhausted", **1106** "permission deny" (app account not linked), **28841107** "No permission. The data center is suspended." ([TuyaAPI/cli #169](https://github.com/TuyaAPI/cli/issues/169))
- The literal word "suspend" appears in Tuya's quota-warning email: *"Trial remaining usage is 0.00%. Once the quota limit is exceeded, the service will be suspended."* ([tinytuya #230](https://github.com/jasonacox/tinytuya/issues/230))

tinytuya's wizard already detects this: it greps the error for `permission`/`1010` and prints *"Hint: This may indicate your Tuya IoT subscription has expired. Visit https://iot.tuya.com to check and renew your IoT Core service."* (wizard.py lines 194-214).

**Renewal** (README "RENEWAL", verified in the 1.21.1 metadata): Cloud → Project Management → Upgrade IoT Core Plan → Trial edition ($0.00) → Buy Now → then **"Extend trial period"** (1/3/6 months + a questionnaire), approval usually minutes-to-2 business days. [Localtuya docs](https://xzetsubou.github.io/hass-localtuya/cloud_api/) give the same path: Cloud > Cloud Services > IoT Core > My Subscriptions > Extend Trial Period.

**Key fact for your architecture:** local control does not need the cloud. Only *fetching the local_key* does. Once keys are in `devices.json`, LAN control works forever (until the device is re-paired, which rotates the key). So a one-time painful setup, then nothing.

**No-developer-account alternative** (inference confirmed by source reading of the tool, not by running it): [vineetchoudhary/tuya-local-key](https://github.com/vineetchoudhary/tuya-local-key) does QR login through Tuya's official `tuya-device-sharing-sdk` using Home Assistant's app registration, outputs TinyTuya setup values, has a Docker image + CLI. Same approach discussed for tuya-local ([discussion #6068](https://github.com/make-all/tuya-local/discussions/6068)). Maintainer explicitly won't bundle secrets extraction but points users at such tools ([#249](https://github.com/jasonacox/tinytuya/issues/249), comment 2023-07-20; on 2026-03-02 he redirected to the Device Compatibility FAQ).

### 3. Cloud mode (`tinytuya.Cloud`)

`Cloud(apiRegion, apiKey, apiSecret, apiDeviceID)` → OAuth `https://openapi.tuyaus.com/v1.0/token?grant_type=1`, HMAC-SHA256 signing (new post-2021 algorithm, and it sorts query keys before signing), regions `cn/us/us-e/eu/eu-w/in/sg`. Methods: `getdevices`, `getstatus`, `getfunctions`, `getproperties`, `getdps`, `sendcommand`, `getdevicelog` (up to 1 week of logs), `getconnectstatus`, `getmapping`. Every call needs a live IoT Core subscription and consumes quota (README warns: one free trial = very limited API calls). **No CLI wraps any of this.**

### 4. Protocol versions and power DP mapping

Claims and verified support for **3.1, 3.2, 3.3, 3.4, 3.5** (header.py has `PROTOCOL_VERSION_BYTES_34/35`; `set_version` handles 3.2 as device22; 3.5 needs AES-GCM, so `pyaes` won't work). v3.4/3.5 use per-session key negotiation; v1.21.1 fixed a bug where repeated failed key attempts could lock devices until power-cycled (RELEASE.md).

Power DPs (README DPS table, v3.3 plug):
| DP | code | unit | scale |
|---|---|---|---|
| 17 | add_ele (electricity) | kWh | ÷1000 per [Tasmota TuyaMCU](https://tasmota.github.io/docs/TuyaMCU/) ("Scale3"); README table says 0-50000 kWh |
| 18 | cur_current | mA | none |
| 19 | cur_power | W | ÷10 (scale 1) |
| 20 | cur_voltage | V | ÷10 (scale 1) |
| 22-25 | voltage/current/power/electricity coeff. | — | calibration |

Note "Some 3.3 energy plugs use the 3.1 plug DPs" (DP 4 current mA, 5 power W, 6 voltage V) — so hardcoding 17-20 breaks on some units.

tinyutuya itself **does not normalize any of this**: `status()` returns raw DPS. Only `tinytuya/Contrib/SocketDevice.py` hardcodes `1/18/19/20` with `/10` on 19 and 20. DP 17 semantics are contested even upstream — tuya-local's maintainer calls `add_ele` unsuitable as an energy sensor ([#6427](https://github.com/make-all/tuya-local/issues/6427)); other sources describe it as incremental-since-last-report. **Inference:** treat DP 17 as device-specific and validate against a known load.

Also relevant to you: `tinytuya/server/` exists — a beta GET-only HTTP API + web panel on :8888 (`/devices`, `/status/{id}`, `/turnon/{id}/{sw}`, `/set/{id}/{key}/{value}`, `/sync`, `/offline`), Docker image `jasonacox/tinytuya` is a **multi-arch OCI index with linux/amd64 and linux/arm64** (verified via Docker Hub registry API). No auth, no database, no history. It got a path-traversal fix in v1.21.0 (`/../tinytuya.json` could leak keys).

## Verdict

tinytuya covers **essentially all of the read/control surface you described, today, with zero code**: list, status, on/off, per-DP read/write, JSON output, and raw power DPs. What it does not cover, and what your two projects actually are:

| your need | tinytuya as-is |
|---|---|
| list devices, status, on/off | covered (`list --json`, `get`, `on`, `off`) |
| power/current/voltage/energy | raw DPS only, no scaling, no field names → you write a 30-line mapping anyway |
| JSON for agents | mostly (`get` is raw device JSON; errors go to stdout with exit 1) |
| not on the same LAN | **not covered locally** — cloud `getstatus/sendcommand` exists in the Python API but no CLI, and needs live IoT Core quota |
| history / database / web panel arm64+amd64 | not covered (server/ is beta, no DB, no auth) |
| getting local_key | still requires IoT Core trial (the exact thing that broke for you), unless you use the QR-login route |

So "tinytuya already did it" is true for the hard part — **the LAN protocol, crypto, discovery, and reconnect logic** — and false for the product part (persistence, normalized metrics, non-LAN access, Docker panel). Two practical consequences: (a) don't write the Tuya crypto yourself; [rustuya 0.4.7](https://crates.io/crates/rustuya) (2026-11-14, sans-I/O core for v3.1-3.5 + tokio driver with discovery, auto-version, reconnect backoff, `watch_status`) is the Rust equivalent, and is explicitly scoped as "the layer tinytuya builds on"; (b) whether the panel can work off-LAN is the one architectural decision that changes everything — if the box with the panel is not on the plug's LAN, none of this helps and you'd be on cloud quota forever; if it is (or via WireGuard/Tailscale), local-only is strictly better.

Skipped: did not run against a real plug (no hardware), did not check `tuya-rs`/`rust-tuyapi` internals, did not verify the QR-login tool by running it, and did not confirm DP 17's real scale on your plugs. Risk: your local_key must be fetched once from an IoT Core project in Trial — if Tuya declines the extension, the QR-login route is the only path, and re-pairing any plug silently invalidates its key.

---

# sharing-sdk

## tuya-device-sharing-sdk — verified findings (all read from source, 2026-11-15)

**Repo/version.** `tuya/tuya-device-sharing-sdk`, MIT, latest tag **0.2.15 released 2026-08-20** (commit `170cfd9`), PyPI also 0.2.15 (Aug 20, 2026). Declared deps are only `paho-mqtt` + `requests`, but `customerapi.py` imports `cryptography` without declaring it (open issue [#83](https://github.com/tuya/tuya-device-sharing-sdk/issues/83), 2026-10-29) — install it yourself. Home Assistant's official Tuya integration pins `tuya-device-sharing-sdk==0.2.15` ([manifest.json](https://github.com/home-assistant/core/blob/dev/homeassistant/components/tuya/manifest.json)). It has been the HA login path since 2024.2 ([PR #109155 merged 2024-01-31](https://github.com/home-assistant/core/pull/109155), [release notes](https://github.com/home-assistant/home-assistant.io/blob/current/source/_posts/2024-02-07-release-20242.markdown)).

### 1. Login flow (verified: `tuya_sharing/user.py`, `customerapi.py`)

- `POST https://apigw.iotbing.com/v1.0/m/life/home-assistant/qrcode/tokens?clientid=<id>&usercode=<code>&schema=<schema>` → `result.qrcode` token (`user.py:19`).
- QR payload is the literal string `tuyaSmart--qrLogin?token=<token>` (HA `config_flow.py:85`).
- Poll `GET .../qrcode/tokens/{token}?clientid=&usercode=`; success returns `access_token`, `refresh_token`, `expire_time`, `uid`, `t`, `terminal_id`, `endpoint` (HA `config_flow.py:123-134`; `tuya_local/cloud.py:100-110`). `endpoint` is the account's regional gateway (e.g. `https://apigw.tuyaeu.com`), and every later call goes there.
- `client_id = HA_3y9q4ak7g4ephrvke`, `schema = haauthorize` — HA's public app registration, published in HA `const.py:34-35` and make-all/tuya-local `const.py:22-23`. Not a secret; the QR scan is the authorisation.
- The login host is hard-coded to `apigw.iotbing.com` regardless of region (`user.py:9`). Refresh: `GET /v1.0/m/token/{refresh_token}` (`customerapi.py:143`), and the response carries a **new** refresh token that invalidates the old one — you must persist it (HA does via `SharingTokenListener.update_token`; same warning in [open-tuya `account.ts`](https://github.com/cheziyi/open-tuya) lines 70-71).

### 2. Device list does include `local_key` — yes

`CustomerDevice` declares `local_key: str` (`device.py:75`), and devices come from `CustomerDevice(**item)` over the raw `GET /v1.0/m/life/ha/home/devices?homeId=` payload (`device.py:107-121`), so the field is in the raw JSON. Confirmed independently by three clients parsing that exact endpoint: [LscSync](https://github.com/jonag972/LscSync) Rust `pub struct CloudDevice { … pub local_key: String … }` (`crates/cloud/src/lib.rs:49`), the Swift client (`item["local_key"] as? String`), and make-all/tuya-local [`cloud.py:148`](https://github.com/make-all/tuya-local/blob/main/custom_components/tuya_local/cloud.py). Caveats: Bluetooth-only devices return no key (tuya-local-key issue #3 / troubleshooting), and the field can be absent for some devices, hence the defensive `hasattr(device, "local_key")` in tuya-local.

Bonus for your plan: the same payload gives `local_strategy` (dpId → status code + `valueConvert` + `supportLocal`), and `GET /v1.0/m/life/ha/{dev}/dp-report-types` labels energy DPs `sum` (incremental) vs `minux` (cumulative) (`device.py:201`) — that's exactly what decides how you store `add_ele`.

### 3. Commands, status, power DPs, push — all cloud, all cross-LAN

- Commands: `POST /v1.1/m/thing/{device_id}/commands` with `{"commands":[{"code":…,"value":…}]}` (`device.py:213`). Errors raise `ApiRequestException`; note `DeviceRepository.send_commands` wraps this in a `Filter(10)` that **silently drops a command identical to the previous one for the same device within 10 s** (`device.py:212, 227-261`) — a trap for an agent CLI, and it returns `None` even on success.
- Status: snapshotted from the device list (`status` array of `{code,value}`) plus `GET /v1.1/m/life/{dev}/specifications` and `GET /v1.0/m/life/devices/{dev}/status` (`dpStatusRelationDTOS`).
- Push: `POST /v1.0/m/life/ha/access/config` returns MQTT url/credentials/expiry plus topics; subscribes the owner topic and per-device `…/pen` (local-capable) or `…/sta` (`mq.py:52, 106-111`). Frames are `{"protocol":4,"data":{"devId":…,"status":[{"dpId":105,"t":…,"value":39788}]}}` then mapped to DP codes via `local_strategy`. So `cur_power` / `cur_voltage` / `cur_current` / `add_ele` arrive as live push — no polling, no LAN. Real-time cross-LAN control confirmed.
- Power values are **raw integers**; the scale lives in `status_range[code].values` (`scale`) and HA applies `value / 10**scale` in `tuya-device-handlers/type_information.py:311-313`. That scale is sometimes wrong in the cloud (e.g. quirk `cz_dbieebqj8teqxt1w`: declared `scale=0`, device reports deci-watts) — so hard-code nothing, and store both raw and scaled. 0.2.15 also adds decoding of electricity-meter frames V0/V1/V2 into voltage/current/active/reactive/apparent power/power factor (`db_v1_params.py`, PR #76).
- Known gaps (all open, 2026): some "Smart Meter" products return empty specs and empty DP relations over this API while MQTT raw-dpId reports work ([#81](https://github.com/tuya/tuya-device-sharing-sdk/issues/81), 2026-10-06); some devices deliver no MQTT at all ([#85](https://github.com/tuya/tuya-device-sharing-sdk/issues/85), 2026-11-08); some writes accepted by the developer OpenAPI are rejected here with `2008` ([#84](https://github.com/tuya/tuya-device-sharing-sdk/issues/84), 2026-11-03); ~1 in 20-25 requests returns transient `1108 uri path invalid` and must be retried ([#77](https://github.com/tuya/tuya-device-sharing-sdk/issues/77)/[#78](https://github.com/tuya/tuya-device-sharing-sdk/issues/78), 2026-09-27).

### 4. Token lifetime and the real "auth" problem

Access-token TTL is 7200 s (2 h) on EU per logged timings in [#75](https://github.com/tuya/tuya-device-sharing-sdk/issues/75) (2026-08-15 comment). Refresh is lazy only: `refresh_access_token_if_need()` fires only when the token is within 60 s of expiry (`customerapi.py:138`). Two open defects matter for a long-running Docker panel:
- [#75](https://github.com/tuya/tuya-device-sharing-sdk/issues/75) (open, 2026-08-06) + [HA #178281](https://github.com/home-assistant/core/issues/178281) (open, 2026-08-05): roughly every 2-3 h the cloud answers `network error:(-9999999) sign invalid`, the refresh swallows it (`customerapi.py:158`), and HA demands manual QR re-link. Fix proposed in [PR #74](https://github.com/tuya/tuya-device-sharing-sdk/pull/74) — **still unmerged** as of 2026-11-15.
- [#82](https://github.com/tuya/tuya-device-sharing-sdk/issues/82) (open, 2026-10-14): concurrent requests can sign with a stale token. Mitigation: one `Manager` instance behind a lock, serialise all calls — what tuya-local-key does — plus persist rotated refresh tokens and treat `sign invalid` / `1010` as "re-auth required" and surface it, not swallow it.

### 5. No "suspend"/trial issue — this path never touches the developer platform

Every call goes to `apigw.*` with the HA client_id and app tokens. There is no Access ID/secret, no `openapi.*`, no IoT Core subscription. The trial errors belong to the other route: `28841002` "subscription to cloud development plan has expired" ([tinytuya #660](https://github.com/jasonacox/tinytuya/issues/660), 2025-11-09) and `28841107` "The data center is suspended" (community reports, no official Tuya doc). *Inference:* your "suspend" error is one of those two from iot.tuya.com — neither can occur here. Remaining constraints are different: the account must be a Smart Life / Tuya Smart app account (OEM-app accounts cannot use this QR flow, [#43](https://github.com/tuya/tuya-device-sharing-sdk/issues/43), 2025-11-29), and [#61](https://github.com/tuya/tuya-device-sharing-sdk/issues/61) (2026-02-19) reports only one HA registration active at a time.

### 6. Existing tools

- **[vineetchoudhary/tuya-local-key](https://github.com/vineetchoudhary/tuya-local-key) v4.1.0**, Apache-2.0, 2026: CLI + Flask web UI, Docker images `linux/amd64,linux/arm64` on GHCR, QR login with HA's client_id, exports `local_key`, every DP value, LAN scan for IP/protocol, key-rotation detection, session cache. This is ~80% of the panel you described.
- **[tz-yang/tuya-local-key](https://github.com/tz-yang/tuya-local-key)**: same technique, simpler.
- **[make-all/tuya-local `cloud.py`](https://github.com/make-all/tuya-local/blob/main/custom_components/tuya_local/cloud.py)**: uses `Manager` only to fetch `device.local_key`; README notes the token expires after a few hours and is not saved.
- **tinytuya is NOT this.** Current 1.21.1 (2026-11-12) has no QR/sharing code (grep: zero hits); its `wizard` still needs Access ID/secret and an IoT Core trial it documents renewing monthly ([README](https://github.com/jasonacox/tinytuya)). So the answer to "tinytuya 似乎已经实现了？" is no — tinytuya is excellent for the LAN path, not the account-login path.
- Rust: no crate wraps this API. Closest reusable code is [LscSync's `crates/cloud/src/lib.rs`](https://github.com/jonag972/LscSync) (MIT, Nov 2026): QR login, token refresh, device list with `local_key`, capabilities — the whole signing scheme (MD5, AES-GCM, HMAC-SHA256) in ~250 lines of Rust, a direct port reference.

### Verdict

**Yes — it replaces the IoT developer platform entirely, for both jobs.** Key extraction: `local_key` is in the device list, no developer account, no trial to renew. Cross-LAN control and metering: on/off plus `cur_power`/`cur_voltage`/`cur_current`/`add_ele` work over the cloud with MQTT push, from any network. The one thing it does not replace is the developer *Statistics* API (`/v1.0/devices/{id}/statistics/...`) if you later want cloud-side historical energy charts — this SDK has no statistics endpoint; you accumulate history yourself from MQTT, which fits your database plan.

Implications for the build: single serialised `Manager`, persist the rotated refresh token, classify `sign invalid`/`1010` as "re-auth needed" in JSON output, retry `1108` twice, do not trust DP `scale`, and dedupe the SDK's 10-second command filter yourself. For Rust, port LscSync's signing code rather than inventing it; keep the Python SDK as the reference oracle.

Skipped: I did not run a live login against a real Smart Life account, so token TTL (7200 s) and CN-region behaviour of the hard-coded `apigw.iotbing.com` login host rest on third-party logs plus one 2024 DNS report ([HA #119380](https://github.com/home-assistant/core/issues/119380)); risk to know: if your account is CN-region and the QR request fails, that fixed login host is the first suspect.

---

# rust

## Rust ecosystem for Tuya — survey (checked 2026-11-15)

### Local-protocol crates on crates.io

Checked via the crates.io API and the code of every crate below (not just READMEs).

| Crate | Latest / date | Protocols | Discovery | Cloud | Status |
|---|---|---|---|---|---|
| **[rustuya](https://crates.io/crates/rustuya)** 0.4.7 (`rustuya-core` + `rustuya-tokio`) | 0.4.7, **2026-11-14** | **3.1 / 3.2 / 3.3 / 3.4 / 3.5** | UDP 6666 + 6667 + 7000 (v3.5 dialect), active probe + passive, multi-interface | none (by design) | Very active, 1 author, MIT |
| [tuya-rs](https://crates.io/crates/tuya-rs) 0.2.1 (bennesp/robottino-rs) | 0.2.1, **2026-03-05** | 3.3 only | 6666 + 6667 | OEM Mobile API (`cloud` feature) | Semi-active (repo 2026-06-18) |
| [rust-tuyapi](https://crates.io/crates/rust-tuyapi) 0.9.1 | 0.9.1, **2024-10-18** | 3.1 / 3.3 | **none** (TCP/UDP transport only) | none | Effectively dormant; last merged PR 2026-10-20 |
| [rust-async-tuyapi](https://crates.io/crates/rust-async-tuyapi) 1.2.0 | 1.2.0, **2025-11-28** | 3.1 / 3.3 / **3.4** (session key, incl. the 0x00-first-byte quirk guard) | **none** | none | Quiet since Nov 2025 |
| Silenttttttt/tuya-lan-rs | GitHub only | 3.4 / 3.5 | none | none | **Not on crates.io**; lamp-specific toy API |
| [tuya-interpreter](https://crates.io/crates/tuya-interpreter) 0.1.1 | 0.1.1, 2026-11-10 | n/a — DPS→semantic model | n/a | n/a | New, same author as rustuya |

`tuyapi-rs`, `tuyapi`, `rust-tuya` **do not exist** on crates.io (404 on the API).

### rustuya 0.4.7 — what's actually in it (read the source)

- Version profile table in `rustuya-core/src/version.rs`: v3.3 = 55AA+CRC32, AES-128-ECB, header **after** encrypt; v3.4 = 55AA+**HMAC-SHA256**, header **before** encrypt, session key; v3.5 = **6699 frames, AES-128-GCM** with the frame header as AAD, header inside the GCM plaintext.
- `session.rs`: full tinytuya-compatible 3-step `SessKeyNegStart/Resp/Finish`, HMAC verify, session key = `AES-ECB(local_key, local^remote)` for 3.4 and GCM-ciphertext-of-xor for 3.5.
- `command.rs`: v3.4/v3.5 `Control`→`ControlNew` (0x0d) remap, `DpQuery`→`DpQueryNew` (0x10). `version.rs` documents that device22 is *not* auto-detected (explicit opt-in; v3.2 always device22) — a deliberate, documented limitation vs tinytuya's `"data unvalid"` heuristic.
- `discovery.rs`: 6666/6667/7000, source-address spoof rejection on by default, `local_ips` for multi-homed hosts, TTL cache.
- Driver (`rustuya-tokio`, 3,111 LOC): reconnect backoff w/ jitter, heartbeat, idle-liveness, `ConnectLimiter` for fleets, sub-devices, `MultiListener`, `watch_status`/`watch_error`/`watch_failure`. Commands are **fire-and-forget** — replies arrive on a broadcast listener (the LAN protocol has no request/response token).
- Validation: 97 unit tests in core, 22 integration test files, a **tinytuya parity fixture** generated from tinytuya 1.18.1 (`rustuya-core/tests/fixtures/tinytuya_payloads.json`), and CI against `tuyamock` 0.0.6 as an independent wire oracle. docs.rs build for 0.4.7: **successful**.
- I compile-checked a realistic snippet (discovery scan + 3.4 connect + query + set DP + close) against 0.4.7 — **passes** (`cargo check`, GNU toolchain).

Weak signals: 6 stars, 3,106 total downloads, single author (3735943886), and a `claude` contributor + `.github/workflows/claude.yml` (AI-assisted, hand-curated changelog, shipping near-daily). One real third-party consumer: [lueur](https://github.com/Alegzandr/lueur) pins `rustuya 0.4.6` in production for bulbs. Python bindings stayed on 0.3.x (PyPI `rustuya` = 0.3.0rc9, 2026-06-28).

### Rust implementation of the Smart Life / sharing cloud API

**No published crate.** What exists:

- **`tuya-rs` `cloud` feature** = OEM Mobile **email/password** API, signed with credentials hardcoded from a specific APK (`com.groupeseb.ext.xplorer` / Rowenta vacuum, per its README). It does return `localKey` in `list_devices()`, but it authenticates against *that OEM app's* realm — **not Smart Life**. Not usable for your account.
- **Two working Rust ports of the official device-sharing API, embedded in apps, not libraries** — copy-in references:
  - [lueur `tuya_cloud.rs`](https://github.com/Alegzandr/lueur/blob/main/crates/lueur-shell/src/tuya_cloud.rs) — **527 lines, MIT**, pushed 2026-11-13. QR token POST + poll → session; `X-sign` HMAC header signing; request/response payloads **AES-GCM sealed**; `/v1.0/m/life/users/homes`, `/v1.0/m/life/ha/home/devices`, `/v1.0/m/life/devices/{id}/status`; extracts `local_key` + DP code↔id map.
  - [Domus `tuya/api.rs`](https://github.com/aiyu-ayaan/Domus/blob/main/backend/src/integrations/tuya/api.rs) — **313 + 200 lines, MIT**, pushed 2026-11-05. Same API, same `HA_3y9q4ak7g4ephrvke` client id / `haauthorize` schema.
- Upstream reference: [tuya-device-sharing-sdk](https://github.com/tuya/tuya-device-sharing-sdk) **0.2.15 (2026-08-20)**, the exact Python package HA's Tuya integration pins; and [tuyawizard](https://github.com/3735943886/tuyawizard) 0.1.10 (MIT) = QR login CLI + device list + local keys + LAN scan, by the rustuya author.

**Correction to a premise:** tinytuya itself does **not** implement the Smart Life QR login. Its `Cloud.py` (944 lines) is only the *developer OpenAPI* (`apiKey`/`apiSecret`/`apiDeviceID`), and `wizard.py` still prompts for those. What tinytuya "already did" is the local protocol. The QR-login path is `tuya-device-sharing-sdk` / `tuyawizard`.

### Effort if you wrote the protocol yourself

tinytuya's protocol+device stack = **2,384 lines / ~93 KB**: `XenonDevice.py` 1,445, `core.py` 269, `crypto_helper.py` 214, `Device.py` 198, `message_helper.py` 184, `udp_helper.py` 44, `header.py` 30. (`scanner.py` is another 2,378 but mostly interactive UI/wizard.) rustuya's equivalent is ~7,966 lines of Rust. A hand-rolled 3.3/3.4/3.5 + discovery + reconnect is realistically **1,200–1,800 lines to first light, 3–5 focused days to trustworthy** — and the cost isn't the happy path, it's: 3.4's header-before-encrypt asymmetry, 3.5's AAD = frame header + fresh IV per frame, the 0x00-first-byte session key bug (device rejects with "Error 914"), device22's `"data unvalid"` retry, and the ~4 response-format variants (retcode/version-header present or absent).

### Verdict

1. **Use `rustuya` 0.4.7 for the local protocol. Do not write it.** It already has 3.3/3.4/3.5, discovery on all three ports, reconnect/heartbeat/limiter, and tinytuya-parity + tuyamock tests — i.e. all of the expensive parts, verified by source reading and a compile check. API shape fits your CLI: `Discovery::scan()` for `list`, `dev.query()` + `watch_status()`/`listener()` for `status`/power DPs, `set_value(dp, v)` for on/off.
2. **Accept its two gaps explicitly** (both are design choices, not bugs): no per-device-type DP mapping → you map DP ids yourself per plug model (power/current/voltage/energy are commonly 19/18/20/17-ish but **product-specific**; fetch the schema once via cloud `specifications`/`status` and store it — your SQLite plan is exactly the right place). And no cloud.
3. **Cloud: vendor, don't invent.** There is no crate. Copy ~500 lines from `lueur` (MIT) or `Domus` (MIT) for QR login + sealed requests, or shell out to `tuyawizard` once to capture local keys. Note this is the *same* API Home Assistant uses, so it is the well-trodden path — and it sidesteps the Tuya IoT developer-platform "suspend"/trial-expiry wall you hit entirely (no developer project needed at all).
4. **Pushback on scope:** since the agent client may not be on the plug's LAN, the local path covers only the always-on Docker panel at home; the CLI-anywhere case *requires* the cloud path. Don't build the CLI's remote mode on local protocol. Conversely, don't build a cloud client for the panel — it's on-LAN, local is faster and needs no token refresh. One binary, two backends, chosen at runtime.

Risk to know: rustuya is a 6-star, one-author, AI-assisted project shipping daily; pin `=0.4.7`, keep the local-key/DP mapping in your own code so a swap to `tuya-rs` (3.3-only) or a hand-rolled codec stays cheap. Also: your Windows box has **no MSVC linker or Windows SDK** — a plain `cargo build` fails on this machine; build in Docker (which you need for arm64/amd64 anyway) or install VS Build Tools.

Skipped: I did not test against real hardware (no LAN/plug here), did not audit rustuya's HMAC/tag handling line-by-line against tinytuya's fixtures myself, and did not dig into Tuya's current cloud-project suspension policy beyond noting it's the known trial-expiry behavior.