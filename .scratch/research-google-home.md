# web-app

## home.google.com — web app capability, internal API, auth, and OSS landscape (research as of 2026-11-15)

### 1. Does the web app control devices, or only cameras/automations? (VERIFIED)
- **It does control devices, but only a subset.** Google Home for web (`home.google.com`) has a **Devices** tab between Cameras and Automations. It shipped with cameras (live view, event timeline, clips, two-way talk, PiP) and routines plus the **script editor** (YAML, Public Preview), then added device controls.
- Controls arrived first through Public Preview (**2025-09-08**), then rolled out widely (**2025-09-23**): **lights (on/off + brightness drag, no color picker), Nest thermostats (temp/mode), plugs, smart locks, blinds**. Speakers, displays and Chromecasts still show tiles that say "Controls for this device are not yet supported." Google's own help page says device controls are "limited and vary by device; use the mobile app for full controls."
- The **UI is not the API's limit**. The backend enumerates and controls everything in the home graph (see §3).
- Sources: [support.google.com/googlehome/answer/9241220](https://support.google.com/googlehome/answer/9241220?hl=en), [9to5google 2025-09-08](https://9to5google.com/2025/09/08/google-home-web-devices/), [9to5google 2025-09-23](https://9to5google.com/2025/09/23/google-home-web-device-controls/), [howtogeek](https://www.howtogeek.com/google-home-web-app-device-controls/).

### 2. Internal API (VERIFIED — it is neither batchexecute nor binary gRPC-web)
Reverse-engineered and **live-captured from home.google.com on 2026-07-15/16** by the `ericmigi/googlehome-mcp` author; cross-confirmed by three other independent projects.

- **Host (web):** `https://googlehomefoyer-pa.clients6.google.com/$rpc/google.internal.home.foyer.v1.<Service>/<Method>` — POST.
- **Host (app / native gRPC):** `googlehomefoyer-pa.googleapis.com:443`, standard gRPC with `access_token_call_credentials` ([Drakulix/googlehome](https://github.com/Drakulix/googlehome/blob/master/custom_components/googlehome/__init__.py), archived 2021, 29★; community DNS errors reference this host).
- **Wire format:** `Content-Type: application/json+protobuf`, `X-User-Agent: grpc-web-javascript/0.1`. Bodies are **positional JSON arrays** — protobuf field numbers as array indices with `null` holes. Scalar wrapper: number at index 1 `[null,42]`, string/enum at 2 `[null,null,"heat"]`, bool at 3 `[null,null,null,1]`. Response may carry an XSSI guard `)]}'` to strip. **Plain `serde_json` + one POST per call — no codegen, no gRPC, no protoc needed.**
- **Services/methods seen:** `StructuresService/GetHomeGraph`, `HomeControlService/GetTraits`, `HomeControlService/UpdateTraits`, `AutomationService/ListAutomations`, `AutomationService/ExecuteAutomation`, `EntitlementService/ListEntitlementRecordsForStructure`, `UsersService/{GetUserPreferences,GetNestAccountLinkState}`. `grep`-able proto also in [KapJI/ghome-foyer-api](https://github.com/KapJI/ghome-foyer-api) (`HomeControlService.GetAssistantRoutines`, `HomeDevicesService`) and [homebridge-nest-accfactory foyer.proto](https://github.com/n0rt0nthec4t/homebridge-nest-accfactory/blob/main/src/protobuf/googlehome/foyer.proto).
- **Protocol doc:** https://github.com/ericmigi/googlehome-mcp/blob/main/docs/PROTOCOL.md and `docs/TRAITS.md`.

### 3. Auth (VERIFIED)
Two paths; only the second matters for a CLI:
- **Browser/cookie:** `Authorization: SAPISIDHASH <ts>_<sha1(ts+" "+SAPISID+" "+"https://home.google.com")>` + `SAPISID1PHASH` + `SAPISID3PHASH`, plus `X-Goog-Api-Key: {google_web_api_key}`, `X-Goog-AuthUser: 0`, optional rotating `X-Server-Token` and `x-foyer-client-environment`.
- **Non-browser (recommended):** one-time `oauth_token` cookie (`oauth2_4/…`) from `accounts.google.com/EmbeddedSetup` → gpsoauth `ac2dm` exchange at `https://android.clients.google.com/auth` → long-lived **master token `aas_et/…`** → `getAuthToken` → 1-hour **Bearer**.
  - Working triple (live-verified 2026-07-16): `app=com.google.android.apps.chromecast.app`, `client_sig=24bb24c05e47e0aefa68a58a766179d9b613a600`, scope `oauth2:https://www.googleapis.com/auth/homegraph`.
  - **Gotcha:** with a Bearer, send **no** `X-Goog-Api-Key` — mixing the web key with the Android-project Bearer returns HTTP 400 `CONSUMER_INVALID` ("API Key and the authentication credential are from different projects"). The PROTOCOL.md text contradicts itself here; the shipped client (`js/src/foyer-client.ts`, `post()`) sends only `Authorization` + `X-User-Agent` + `Content-Type`. Treat the code as ground truth and test `GetTraits` first.

### 4. What it can control (VERIFIED per `TRAITS.md`, live on 2026-07-16)
| Capability | Trait.field | Status |
|---|---|---|
| on/off | `onOff.onOff` | verified |
| brightness | `brightness.brightness` 0–100 | verified |
| color temp | `color.colorTemperature` (K) | verified |
| volume / mute | `volume.currentVolume` / `isMuted` | verified / inferred |
| lock / unlock | `lockUnlock.isLocked` (+ `pin` challenge) | verified |
| thermostat | `temperatureSetting.mode` + `thermostatTemperatureSetpoint` (display unit, mode required) | verified |
| media play/pause/stop | `mediaState.playbackState` | verified |
| routines | `AutomationService.ExecuteAutomation` (flag=1 only) | verified |
| RGB / HSV color, next/prev track | `spectrumHsv`/`spectrumRgb`, `transportControl` | **not** verified |

Device coverage is the **whole home graph**, including cloud-to-cloud third-party devices (a Tuya plug linked via Smart Life appears with `agentId`/`partnerDeviceId` and `action.devices.types.*` type strings). Test account had ~71 devices / a dozen rooms. One caveat from `PROTOCOL.md`: physical actuators are async — commands return fast, `GetTraits` lags 10–30 s, and lock `challenge: "pinNeeded"` needs a resend with the PIN.

### 5. OSS projects (all checked 2026-11-15 via GitHub API)
| Repo | Lang | Last push | ★ | Notes |
|---|---|---|---|---|
| [coredevices/pebble-plugins/googlehome](https://github.com/coredevices/pebble-plugins) | TS | 2026-11-09 | 6 | Most complete; Kotlin original; perms model |
| [ericmigi/googlehome-mcp](https://github.com/ericmigi/googlehome-mcp) | TS | 2026-08-29 | 11 | Cleanest protocol docs + codec, MIT-ish (no license file) |
| [ericmigi/ha-googlehome-foyer](https://github.com/ericmigi/ha-googlehome-foyer) | Python | 2026-11-12 | 0 | `api.py` = 1-file reference impl of auth + codec; MIT |
| [KapJI/ghome-foyer-api](https://github.com/KapJI/ghome-foyer-api) | Python | 2026-11-11 | 3 | `.proto` stubs, actively CI-bumped |
| [n0rt0nthec4t/homebridge-nest-accfactory](https://github.com/n0rt0nthec4t/homebridge-nest-accfactory) | JS | 2026-11-14 | 55 | Full `foyer.proto` incl. camera `JoinStream` (WebRTC) |
| [rithvikvibhu/GHLocalApi](https://github.com/rithvikvibhu/GHLocalApi) | docs | 2026-04-13 | 468 | **Different API** — local port 8443 on speakers, LAN only |

**No Rust implementation exists.** GitHub search `foyer google home language:rust` = 0; crates.io has only `googleapis-tonic-google-home-graph-v1` (the *official* HomeGraph API, not foyer). Nothing for gpsoauth either.

### 6. Official paths (for comparison — VERIFIED)
- **Home APIs (Android/iOS SDK):** no web, REST, or server-side surface; needs Cloud project + OAuth consent. Developer Console "not yet available for registration."
- **Google Home MCP** — launched **2026-09-16** early access, **still early access 2026-11-15**. Endpoint `https://home.googleapis.com/mcp` (Streamable HTTP, JSON-RPC), OAuth scope `https://www.googleapis.com/auth/home.platform.v2`. Tools: `list_homes`, `list_home_resources`, `list_home_states`, `list_home_history`, `run_home_actions`, `list_automations`, `save_automation`. ~60 traits. **Gates: active Google Home Premium Advanced subscription ($20/mo US), Google Cloud project, "access approval", OAuth Web client.** Unlocking doors / disarming blocked; rate-limited. ([docs](https://developers.home.google.com/mcp/home), [release notes](https://developers.home.google.com/mcp/home/release-notes), [TechCrunch 2026-09-16](https://techcrunch.com/2026/09/16/your-ai-agents-can-now-control-your-google-home-devices/))

### 7. ToS / ban risk
- **VERIFIED text:** Google APIs ToS (last modified **2021-11-09**, §4a) — "Reverse engineer or attempt to extract the source code from any API or any related software". Google general ToS — "reverse engineering our services or underlying technology… to extract trade secrets"; "using automated means to access content… in violation of the machine-readable instructions."
- **INFERENCE:** foyer is an undocumented internal endpoint, so using it is at best outside the ToS letter; the reverse-engineering clause is a plausible hook for action.
- **VERIFIED absence of evidence:** I found **no** credible report of an account ban for foyer use. The one "account got suspended" comment is an unverified gist anecdote and the causal link isn't established. Real-world breakage in 2024 was Google **restricting the `homegraph` OAuth scope to verified partners** and killing the password-login script path — which is why every current tool uses the `oauth_token` cookie → master-token bridge instead ([HA issue #977](https://github.com/leikoilja/ha-google-home/issues/977), opened 2025-10-21, "Google removed support for master-token API endpoint since early 2024", fixed by browser sign-in in v1.14.0 on 2026-10-17).
- **Real risk, not ban risk:** the master token is password-equivalent and never expires until password/2FA change; the `GetHomeGraph` response also contains **each device's local auth token plus home address/coordinates** — don't log or ship it.

### 8. Verdict for our use case
**Foyer is the right transport for the CLI.** It is the only free, no-approval, no-subscription path that sees *all* devices including Tuya-via-Smart-Life, and it needs no gRPC stack in Rust: HTTPS POST + `serde_json` arrays + a gpsoauth form POST. Budget ~400–600 lines (auth, 5 RPCs, codec) — the existing Python/TS implementations are each ~700 lines total. Nothing to port from a nonexistent Rust crate; port from `ha-googlehome-foyer/api.py`, it's the smallest complete reference.

**Skip the cookie/SAPISIDHASH path entirely.** Master token + Bearer is cookie-free, headless, Docker-friendly, ARM-friendly.

**One design constraint worth deciding now:** put foyer behind a `Transport` trait with `list()/state()/set()`, so if Google MCP ever goes GA and you get Premium Advanced, you swap one impl instead of rewriting the CLI. Do not build that abstraction further than one trait.

**Push-back (ponytail ultra):** (a) If the actual goal in q1 is a Tuya plug you already own, the shortest path is Tuya's own local/Cloud API and Google Home is a 10x-larger detour — but since the goal is "a Google Home CLI so *any* linked vendor works," foyer is correct. (b) The Docker web panel does not need its own backend design: `ghome --json` plus a static page that fetches it, both in the same tiny axum binary, is enough — do not build a second API surface. (c) Expect breakage: no SLA, and the last format change (early 2024) took the ecosystem months to recover.

**Known gotchas to encode before writing code:** `)]}'` prefix stripping; Bearer without `X-Goog-Api-Key`; thermostat needs `mode` in the same call as setpoint; lock unlock returns a `pinNeeded` challenge; state reads lag 10–30 s on physical devices, so the web panel must poll, not assume.

Skipped: I did not verify the foyer `clients6` host against live traffic (no account here) — that claim rests on one repo's 2026-07 captures, though three other projects independently confirm the `googleapis` host and the JSON+protobuf codec. Risk: the endpoint can change without notice, and the reverse-engineering clause in Google's API ToS is a plausible basis for action even though no ban reports exist.

---

# assistant-sdk

## Google Assistant SDK `textQuery` (Embedded Assistant gRPC) — status for a Rust "Google Home CLI"

### Verdict up front
**Still functional as of 2026-11, but it is the wrong foundation for your CLI.** It is a voice-assistant RPC ("send words, get audio back"), not a device API: no structured state, no device list, text answers are mostly gone, non-commercial only, and Google has already silently degraded it once (Oct 2025). For "list + control all Google Home devices incl. cloud-to-cloud Tuya", the correct API now exists: **Google Home MCP server** (`https://home.googleapis.com/mcp`, early access since 2026-09-16), which has explicit `list_homes` / `list_home_resources` / `list_home_states` / `run_home_actions` tools. It is plain HTTP JSON-RPC + OAuth — trivially implementable in Rust with `reqwest`, no gRPC/protobuf needed.

---

### VERIFIED FACTS

**1. The service itself has not been shut down (evidence 2026-11-15)**
- Official SDK landing + overview pages are still live, present tense, and carry **no** shutdown/deprecation notice for the SDK; the only deprecation banner is "Conversational Actions were deprecated on June 13, 2023."
  https://developers.google.com/assistant/sdk · https://developers.google.com/assistant/sdk/overview
- Release notes have **zero entries after 2019-06-28** — the service is effectively frozen/unmaintained, not retired.
  https://developers.google.com/assistant/sdk/release-notes
- Google's official Python samples repo was archived **2023-10-19**: https://github.com/googlesamples/assistant-sdk-python
- `assistant-relay` (Node wrapper) was archived by its author **2025-08-28**: https://github.com/greghesp/assistant-relay

**2. It still works in practice — the strongest evidence is Home Assistant**
- `google_assistant_sdk` is still core, quality scale **platinum**, dependency `gassist-text==0.1.0`, last touched **2026-11-09** (commit "Add action enums to google_assistant_sdk", PR #190330). `gh api` on `homeassistant/core`, checked 2026-11-15.
- Its client library `tronikos/gassist_text` had commits **2026-11-12** and is not archived — someone is actively keeping the text path alive: https://github.com/tronikos/gassist_text
- HA docs still document setup + `send_text_command`: https://www.home-assistant.io/integrations/google_assistant_sdk

**3. Auth mechanism (confirmed from source, not just docs)**
- OAuth 2.0 **Web application** client for the HA flow; a second **Desktop app** client is required for "personal results" / stable responses. Docs: "Text responses are no longer returned by the Google Assistant SDK API."
- Implementation reads `TextAssistantAsync(credentials, language_code, audio_out=...)` with `device_model_id` defaulting to the literal string `"default"` — i.e. **the current working path registers no device model at all**. Source: `homeassistant/components/google_assistant_sdk/helpers.py` + `gassist_text/textinput.py` (`device_model_id: str = "default"`), endpoint `embeddedassistant.googleapis.com`, gRPC deadline 185 s.
- Device-model registration in the Actions Console is reported broken since the Conversational Actions sunset (HA docs issue #30172, reports 2024-11-20/26, closed "not planned" 2025-08-17; https://github.com/Melvin-Abraham/Google-Assistant-Unofficial-Desktop-Client/issues/958, 2025-03-09: "no link for device registration"). Yet Google's SDK docs still tell you to register a model: https://developers.google.com/assistant/sdk/guides/service/python/embed/register-device

**4. It cannot read device state**
- `textQuery` is `AssistRequest.text_query` → streamed `AssistResponse` containing `audio_out` + `dialog_state_out.supplemental_display_text` (+ optional `screen_out` HTML). There is **no** device-list or state field in the proto. Confirmed by reading the vendored `embedded_assistant.proto` in gassist_text.
- Since Oct 2025 Google often returns empty text for smart-home commands, and custom-integration author's README states: "Due to a recent change by Google (October 2025)" responses go empty; fix is Desktop-app credentials and/or scraping `screen_out` HTML. https://github.com/tronikos/google_assistant_sdk_custom · https://github.com/tronikos/google_assistant_sdk_custom/issues/36
- Corroborating report 2025-10-29 (HA #155395): command reached Google (visible in My Activity) but no expected response came back. https://github.com/home-assistant/core/issues/155395

**5. What it can control**
- Anything Assistant's own NLU can reach **through your account**, including cloud-to-cloud linked devices — so yes, a Smart Life/Tuya plug linked into Google Home works, and LAN topology is irrelevant (command path: your box → Google cloud → vendor cloud → plug). That answers your q2 for this path.
- Control is **natural-language only**, per-device by name as Google knows it ("turn on the desk plug"). No traits, no structured params, no enumeration. Device/trait coverage = whatever Assistant exposes; you never see a trait list.
- Known broken: most media playback commands, routines, voice-match-dependent actions, multiple Google accounts, room-specific broadcasts.

**6. ToS / ban risk**
- Explicitly non-commercial: "It's available for experimental and non-commercial uses only." / "You can't launch commercial devices that integrate with the Google Assistant SDK." https://developers.google.com/assistant/sdk/support
- Practical risk: **low-to-moderate, mostly "breaks silently" not "banned"**. It is a first-party documented API; HA's integration is used by ~2.2% of installs. No published quota page exists (`/reference/service-api-quotas` and `/reference/device-registration/register-device` both 404 now), so rate limits are undocumented.
- Account-side risk: every command is attributed to your Google account and logged in My Activity; personal-results mode requires trusting local tokens.

**7. The Assistant→Gemini transition: not the cause of SDK breakage (yet)**
- Assistant on phones/tablets/Wear OS/headphones/Android Auto was removed starting **2026-09-04**, completed ~**2026-09-28**: https://9to5google.com/2026/08/04/google-assistant-september-2026-shutdown/ · https://9to5google.com/2026/09/28/google-assistant-gemini-android/
- Speakers/displays are being replaced by **Gemini for Home** (early access US 2025-10-28, wider 2026, Premium subscription); "You can't switch back to Google Assistant once you've migrated": https://blog.google/products-and-platforms/devices/google-nest/gemini-for-home-launch/ · https://support.google.com/googlehome/thread/429303319
- Wikipedia (2026-08-29): Assistant "still runs on … smart home devices that have not yet made the planned transition to Gemini." https://en.wikipedia.org/wiki/Google_Assistant
- **No Google document links the mobile shutdown to the SDK service.** Developer docs are simply stale and silent.

**8. Google Home MCP — the API you actually want (2026-09-16)**
- Server: `https://home.googleapis.com/mcp`, Streamable HTTP, OAuth scope `https://www.googleapis.com/auth/home.platform.v2`.
- Tools: `list_homes`, `list_home_resources` (devices, areas, traits, attributes, **command schemas**), `list_home_states` (current connectivity + trait states), `run_home_actions` (structured commands with params, `challengeResponse` pin/ack/passphrase), `list_home_history`, `list_automations`, `save_automation`.
- `run_home_actions` is a plain JSON-RPC POST with a Bearer token — the example cURL on the page is directly translatable to `reqwest`. https://developers.home.google.com/mcp/home · https://developers.home.google.com/reference/home/mcp/tools_list/run_home_actions.md.txt
- Supported traits list is Matter-shaped (`OnOff`, `LevelControl`, `ColorControl`, `Thermostat`, `TemperatureControl`, `DoorLock`, `WindowCovering`, `MediaPlayback`, `ValveConfigurationAndControl`, ~85 traits): https://developers.home.google.com/mcp/home/supported-traits.md.txt
- Limits: rate-limited (numbers unpublished), unlocking doors / disarming security blocked, creating automations via MCP not supported, "higher-than-expected latency", experimental traits unstable. https://aicybr.com/blog/google-home-mcp-ai-agents-early-access-guide
- Requires **Google Home Premium Advanced + early access, English, US** (Premium Advanced = subscription tier). https://support.google.com/googlehome/thread/467705013

**9. Google Home APIs (Android/iOS SDKs)**
- Device/Structure/Commissioning/Automation APIs, public developer beta since **2025-01-07** (Android) and iOS beta **2025-04**; mobile SDKs only, no REST. Not usable from a Rust CLI/Docker container. https://developers.googleblog.com/en/build-the-future-of-home-with-google-home-apis/ · https://developers.home.google.com/newsletters/2025/04

---

### INFERENCE (not verified)
- Device-model registration is probably not required for the text path anymore, because HA passes `"default"` and still works; the `device_model_id` field appears to be tolerated-but-ignored. Do not spend time on Actions Console registration — the console is view-only per https://developers.home.google.com/cloud-to-cloud/project/migration ("All smart home projects in the Actions Console are now view-only").
- Assistant SDK will keep working on speakers/displays only until your region's Gemini for Home migration completes; after that the Assistant backend may disappear with little warning. The Oct 2025 empty-response change is consistent with that trajectory.
- The 2026-09-28 mobile shutdown likely **did not** touch the SDK (no flood of breakage reports; gassist_text commits continue), but nobody has published a definitive statement either way.
- Home MCP covers Tuya/Smart Life plugs because they are Works with Google Home cloud-to-cloud devices and appear in `list_home_resources` with Matter-style traits. Unverified for your specific account.
- A Rust CLI against the Assistant SDK needs: `tonic` + generated `v1alpha2` proto (copy from `gassist_text/src/google/assistant/embedded/v1alpha2/embedded_assistant.proto` — Apache-2.0, already vendored there) + OAuth desktop-client loopback flow + refresh token. Roughly a day of work, and it gives you a 1-bit-or-audio control channel. Against Home MCP it's `reqwest` + token + 4 JSON-RPC methods + real state and a trait list — less work, more capability.

### Recommendation for the Rust project
1. **Primary backend: Home MCP** (`home.googleapis.com/mcp`) — device list + state + structured commands, matches your q1 goal exactly, and works for cloud-to-cloud Tuya with no LAN dependency. Cost: your own Google Home Premium Advanced + US early access.
2. **Optional fallback: Assistant SDK `textQuery`** only for accounts without MCP access, as a "shout at Google" control channel (no state, text may be empty, scrape `supplemental_display_text`/`screen_out`).
3. Don't build the assistant-relay/device-model registration path — the console is view-only and the maintainers both gave up (relay archived 2025-08-28).

Skipped: didn't test either API against a live Google account (no credentials here), didn't check non-English Home MCP availability beyond the "English/US" wording, and didn't verify the rumored shared-quota unauthenticated path for the separate Developer MCP server. Risk: if Home MCP is still early-access-gated in your region, your CLI's only working backend is the fragile, non-commercial Assistant text path.

---

# official-apis

# Google Home 官方 API 调研（2026-11-15）

## 结论先行（verdict）

**可以，但只有一条官方路：Home MCP**（`https://home.googleapis.com/mcp`，2026-09 早期访问）。它是 Google 官方托管的 **streamable-HTTP MCP / JSON-RPC 服务**，带 OAuth bearer，服务端（你的 CLI / Docker 容器）可直接调用，能列出并控制 Google Home 里的设备，包括第三方 cloud-to-cloud 接入的设备（后台会"直接查询 partner cloud"）。这正好符合"为 Google Home 做 CLI、让其它厂商设备也能用"的目标。

三道门（截至今日均未放宽，均为 verified）：
1. 账号需 **Google Home Premium Advanced 订阅**（US $20/月），早期访问**仅限美国、英文**。
2. Google Cloud 项目需 **enable Home API** + 官方所说 "obtaining access approval"（文档没写申请流程，第三方实测多为自动放行，未验证）。
3. OAuth scope `https://www.googleapis.com/auth/home.platform.v2` —— 官方 sample 文档明说 "Many features … are gated by the `home.platform.v2` scope and **require partner approval** to access"。

若你在非美区 / 无 Premium Advanced：**官方 API 全部不可用于控制"别人家的"设备**，只剩 Matter（设备本身必须是 Matter）或非官方路线。

---

## 1) Google Home APIs（Device & Structure, Automation）

**状态：GA（2026-07/08）** — 主源：Android release notes 显示 `Public Beta 1.9.1 (2026-06-26)` → `GA 1.10 (2026-08-03)`；iOS `Public Beta 1.9.1` → `GA 1.10 (2026-07-30)`。此前 2025-01-07 Android 公测。
https://developers.home.google.com/apis/android/release-notes · https://developers.home.google.com/apis/ios/release-notes

| 项 | 事实（verified） | 来源 |
|---|---|---|
| 平台 | **仅 Android（Kotlin）/ iOS（Swift）SDK**，属 Google Play services（`com.google.android.gms:play-services-home`）。**没有 web SDK、没有 REST、没有服务端 SDK** | https://developers.home.google.com/apis （"Kotlin on Android and Swift on iOS"）；google-home org 只有 android/ios sample（2026-11-03 更新）https://github.com/google-home |
| 设备覆盖 | 明确不止 Matter："The device types presented in the Home APIs are a unification of Matter and Cloud-to-cloud data models"，"from Google Nest or 3rd party manufacturers" | https://developers.home.google.com/apis/android/data-model |
| 能控什么 | traits/attributes/events/commands，含 `OnOff`、`Brightness`、`Volume` 等；device type 列表含 `On/Off Plug-in Unit`、`Dimmable Plug-In Unit`、Light、Thermostat 等 100+ 类型 | https://developers.home.google.com/apis/android/supported-device-types |
| 鉴权 | OAuth 2.0，用户在 Google 授权页按 structure 授权，可只授权部分 device type（敏感类型可逐设备选择） | https://developers.home.google.com/apis/android/permissions |
| 个人可用性 | **未验证应用限 100 个 OAuth 测试用户**；要对外发布需 Developer Console 注册 + device type 审批 + OAuth 品牌验证；而 **"The Google Home Developer Console is not yet available for registration."** | https://developers.home.google.com/apis/android/overview · /get-started |
| 配额 | 30,000 QPM / Cloud project | https://developers.home.google.com/apis/ios/quota-management |

**对 CLI 的意义**：这是手机 App SDK，`home.googleapis.com` 上我实测 REST 探测结果——`/$discovery/rest?version=v1` 返回 403 "unregistered callers"，`/v1`、`/v2`、`/v1/structures` 全 404（2026-11-15 直接从本机 curl 探测）。即：**没有可用的官方 REST 面**，除了 `/mcp`。

## 2) Smart Device Management API（SDM）—— Nest only

- 只覆盖 Nest Thermostat / Cam / Doorbell / Hub Max，**不含 Tuya**。
- REST（`smartdevicemanagement.googleapis.com/v1`），`enterprises.devices` / `structures`，OAuth2 + Pub/Sub 事件。
- **个人开发者可注册，一次 $5 不可退**；需 consumer Google 账号；"This applies to all users, both individual users and those looking to create a commercial offering."
- 条款：**"for your personal use only"**、禁止商业用途（Device Access Sandbox ToS §3.1）。
- 时效：Google 托管 Pub/Sub topic 停用，**2026-11-30 前**必须迁到自建 topic（也就是两周后）。

https://developers.google.com/nest/device-access · /registration · /tos

## 3) Home Graph API —— 第三方查不到别人接入的设备

- 纯 REST（`homegraph.googleapis.com/v1`），service account 鉴权，scope `…/auth/homegraph`。
- 所有调用都要 `agentUserId`：**"The third-party user's identity is passed in via the `agentUserId`"**，`devices:query` 返回 "the given set of the third-party user's devices"，即**你自己那套 cloud-to-cloud Action 同步过的设备**。
- 它是"给 Google 上报你自己的设备"的回调面，不是家庭总账本。**推断（有强证据，未实证）**：无法用它读别的厂商（Tuya/Smart Life）经 Works with Google Home 链入的设备 —— 这是 Google 设计上的集成边界。
- 另有账号绑定迁移公告：第三方账号绑定全部移入 Google Home app，**2026-10-30 前**可能需要改集成（日期已过）。

https://developers.home.google.com/reference/home-graph/rest · /rest/v1/devices/query

## 4) 其它官方路线

- **Matter**：只能控 Matter 设备。Tuya 的普通 Wi-Fi 插座经 Smart Life 云链入 Google Home **不是** Matter 设备，Matter 控制器（如 Rust `rs-matter`，已支持 controller/commissioner，声明支持 spec 1.6 且测过 Google Home）看不到它。
- **Local Home SDK**：给"自己的 cloud-to-cloud 集成"加局域网执行路径，是**被 Google 调用**的 fulfillment，不是控制器；未见弃用公告（reference 页仅列 ColorSpectrum/ColorTemperature 两个 trait 弃用）。
- **Gemini for Home**：面向厂商/服务商的合作项目（2026-05-21 开放第三方伙伴、camera embedded SDK），非个人开发者接口。
- **Google Home MCP（关键）**：官方文档 https://developers.home.google.com/mcp/home · 参考 https://developers.home.google.com/reference/home/mcp
  - 端点 `https://home.googleapis.com/mcp`，transport `streamable-http`，scope `home.platform.v2`。我实测：`initialize` 无鉴权 200（`serverInfo: StatelessServer / ESF`），`tools/call` 无 token 401；**`tools/list` 无鉴权即可读**（2026-11-15 本机 curl）。
  - 工具（实测 tools/list）：`list_homes`、`list_home_resources`、`list_home_states`、`list_home_history`、`run_home_actions`、`list_automations`、`save_automation`、`submit_feedback`。
  - 控制格式：`{"id":"device@…","type":"…","command":"OnOff.On"}`；开关类有 `OnOff`/`Brightness`。
  - **支持第三方云设备（强证据）**：`list_home_states` 的 `freshness` 有 `MOST_FRESH` = "Bypasses caches and **forces a direct query to the partner cloud**"；`DEFAULT` 也是"decide whether to call through to the partner or use home graph data"。即 Home Graph 里聚合了各 partner cloud 的设备。
  - 官方口径覆盖面：newsletter "all of the devices and event history in the Google Home ecosystem"；TechCrunch 写 "any device in the Google Home ecosystem … Works with Google Home"；Tuya case study："once added to Google Home, these devices can become available to any app that uses the Home APIs"。
  - 限制：Premium Advanced 订阅 + 美国英文早期访问；Test 模式应用**每 7 天要重新授权**；门锁类禁止；有 rate limit（未公开数值）；已知 bug：Nest Aware Plus 也能进。
  - CLI 友好度：官方给出的回调示例含 **OpenClaw 的 loopback URI `http://127.0.0.1:8989/oauth/callback`**（对齐 `mcp-remote`），说明 loopback OAuth 是被认可的客户端形态 —— 一个 Rust CLI 自己起本地回调页接 code 即可，不需要 SDK。

## ToS / 封号风险

- Google APIs ToS："You will only access (or attempt to access) an API **by the means described in the documentation** of that API"；禁止 reverse engineer。→ **走 Home MCP 的 OAuth 是文档内行为，合规**。
- Home 平台侧：cloud-to-cloud 文档**未禁止**个人/爱好项目；但要求隐私政策可公开访问、集成名不得暗示 "personal use purpose"。
- 真风险在非官方路线：`rithvikvibhu/GHLocalApi`（音箱 8443 本地 API，需 cast-local-authorization-token）与 `glocaltokens`（master token，2026-10 仍在维护）—— 用的是账号密码/master token，属未文档化接口，违反上述 ToS 条款，理论上有账号风控风险；且它们只管音箱/显示器的本地能力，**控不了 Tuya 插座**。

---

## 事实 vs 推断

**Verified（有主源/实测）**：Home APIs 只有 Android/iOS SDK、GA 时间、Developer Console 未开放注册、100 测试用户上限、`home.platform.v2` 需 partner approval；SDM 仅 Nest、$5、personal use only；Home Graph 需 agentUserId 且只返回该集成用户的设备；Home MCP 端点/工具/scope/`MOST_FRESH→partner cloud`/7 天重授权/Premium Advanced/美国英文；`home.googleapis.com` 无公开 REST（本机 curl 2026-11-15）。

**Inference（未实证）**：
- Home MCP 的 `list_home_resources` 是否**逐台**列出 Works with Google Home 的每一类第三方设备（比如 Tuya 插座的具体 trait 是否完整暴露）。文档只说 "all devices"，无清单；**这是唯一需要在真实账号上验证的点**。
- "access approval" 是否真有人工审批队列。文档把它列进 setup 概览但步骤里没有，第三方记录也没遇到。
- 非美区账号是否被硬拦（订阅可买 20 国，但 MCP 公告只写 US English）。

**对你那套 Rust CLI + Docker + 网页的落地建议（最小路径）**：Home MCP 就是 HTTP JSON-RPC，Rust 侧用 `reqwest` 直接 `initialize` → `tools/list` → `tools/call`，OAuth 走 loopback 授权码流程拿 refresh token；不需要官方 SDK、不需要 Docker 里跑 Android 模拟器。备选兜底：Matter-only（设备换 Matter 插座，本地直控，`rs-matter`），或直接接 Tuya 云（但那就回到了你明确说不要的 Tuya 路线）。

跳过/未做：没用 omnisearch（直接 curl 探测官方端点比二手结果更硬）；未在真实 Premium Advanced 账号上验证 `list_home_resources` 对 Tuya 插座的逐台可见性 —— 这是整套方案唯一的真未知，且它决定"能不能做"，建议第一步就拿一个美区 Advanced 账号跑一次 `tools/list` + `list_home_resources`。风险：Home MCP 仍是 Early Access，接口/配额可随时变；`home.platform.v2` 的 partner approval 随时可能收紧到个人不可用。

---

# unofficial-tokens

# 未公开/私有 token 路径（gpsoauth → foyer）调研报告 — 2026-11-15

> 范围：通过 gpsoauth 主令牌（master token）、oAuth `oauth_token` cookie 换取 Google Home App 私有 API 的访问能力；glocaltokens；HA 社区项目；Rust crate 现状。取证日期均在正文标注。

## 0. 结论先行

**技术上完全可行，且已经有一个 Rust 实现（MIT）**：`piekstra/google-home-cli`（二进制 `ghome`，v0.10.0，2026-11-04）。它做的正是我们想做的事：gpsoauth cookie 换 master token → 铸 foyer Bearer → `GetHomeGraph` / `GetTraits` / `UpdateTraits` 列设备并控制，包括通过 Smart Life 关联进 Google Home 的 Tuya 设备（证据见 §3）。**但：无 aarch64-linux release 资产，且令牌只存 OS keychain（Docker 里没有 keychain）。**

另外：官方终于有了服务端路线 —— **Home MCP**（`https://home.googleapis.com/mcp`，streamable-http + OAuth，2026-09-16 早访问），但门槛是 Google Home Premium **Advanced** 订阅 + 云项目 + 审批，且未明确覆盖 Tuya。

**建议**：CLI 阶段直接 `cargo install --git` 用它（或 fork 补 arm）验证通路；不要从零重写 foyer codec。Docker/web 阶段再决定「打补丁绕 keychain」还是「自写 ~400 行最小客户端」。

---

## 1. 认证机制（VERIFIED）

### 1.1 令牌链

```
浏览器登录 accounts.google.com/EmbeddedSetup
        │  人工复制 oauth_token cookie  (oauth2_4/…, 一次性, 分钟级有效)
        ▼
POST https://android.clients.google.com/auth          ← "gpsoauth" 协议，表单 POST，
   service=ac2dm, Token=<oauth_token>,                 User-Agent: GoogleAuth/1.4
   client_sig=38918a453d07199354f8b19af05ec6562ced5788,  回复是换行 Key=Value
   droidguard_results=dummy123, androidId=<固定的随机ID>
        │
        ▼  master token  aas_et/…     ← 长期有效（改密码/Google 安全撤销才失效）
POST 同上
   EncryptedPasswd=<master token 原样>   ← 注意：不加密，直接当密码字段传
   service=oauth2:https://www.googleapis.com/auth/homegraph
   app=com.google.android.apps.chromecast.app
   client_sig=24bb24c05e47e0aefa68a58a766179d9b613a600
        │
        ▼  Bearer  ya29.…   ← ~1 小时
POST https://googlehomefoyer-pa.clients6.google.com/$rpc/google.internal.home.foyer.v1.<Service>/<Method>
   Authorization: Bearer <ya29…>       ← 绝不能同时带 X-Goog-Api-Key，否则 400 CONSUMER_INVALID
   Content-Type: application/json+protobuf
   X-User-Agent: grpc-web-javascript/0.1
        │  请求/响应 = 位置式 JSON 数组（protobuf 字段号 N → 下标 N-1，缺省 null）
        ▼
   StructuresService/GetHomeGraph、HomeControlService/{GetTraits,UpdateTraits}、
   AutomationService/{ListAutomations,ExecuteAutomation}
```

关键点（都来自实测文档，非猜测）：

- **RSA 加密只用于已经死掉的「密码登录」**。cookie→master token 与 master→Bearer 两步都是纯表单 POST，`EncryptedPasswd` 直接塞 `aas_et/…` 明文。→ **Rust 实现不需要任何 crypto crate**，reqwest 足矣。来源：[googlehome-mcp PROTOCOL.md](https://github.com/ericmigi/googlehome-mcp/blob/main/docs/PROTOCOL.md)（实测 2026-07-16）、[ghome docs/api.md](https://github.com/piekstra/google-home-cli/blob/main/docs/api.md)。
- **`androidId` 必须在换 token 与铸 Bearer 之间保持一致**（ghome 首次登录生成后存 config 文件，非机密）。
- **gpsoauth 的 TLS 有怪癖**：必须保留 session ticket、不能发 ALPN，否则 Google 返回 403/BadAuthentication（[gpsoauth 源码注释](https://github.com/simon-weber/gpsoauth)，2026-11-10 commit）。rustls 默认不发 ALPN，ghome 实测可用（2026-09）。
- **必须用 Gmail 地址账号**：Workspace / 非 Gmail 地址能拿到 master token 但 `GetHomeGraph` 返回 0 设备（[ha-google-home#1071](https://github.com/leikoilja/ha-google-home/issues/1071)，2026-10-16）。
- 换 token 后 Google 会给账号发「新设备已登录」安全提醒 —— 预期行为，不是被入侵。

### 1.2 本地 vs 云端（关系到「不在同一局域网」）

- **云端（本文主题）**：控制走 Google 云，插座在家、CLI 在任意网络都可以。这是 Tuya 这类云对云设备的**唯一**可控路径。
- **本地**：`glocaltokens` 从 foyer 的 `dev[27] local_auth_token` 取每设备本地令牌，直连 `https://<ip>:8443`（[ha-google-home 配置文档](https://ha-google-home.pages.dev/configuration/)）。**要求同局域网 + 每 ~1 天刷新令牌**，且 glocaltokens 明确警告「登录太频繁 Google 会把你已登录的 Android 设备踢下线」（[glocaltokens README](https://github.com/leikoilja/glocaltokens)）。→ 对「跨网络 + Tuya」目标用不上，不必做。

---

## 2. 项目与状态（VERIFIED，含日期）

| 项目 | 语言 | 最新 | 日期 | 状态 | 说明 |
|---|---|---|---|---|---|
| [piekstra/google-home-cli](https://github.com/piekstra/google-home-cli) (`ghome`) | **Rust** | v0.10.0 | 2026-11-04 | **可用** | MIT；完整 foyer 客户端 + 房间/例程/审计/本地 Cast 播报 + MCP server。实测 2026-09-08/09/14 真实账号（91 设备） |
| [KapJI/ghome-foyer-api](https://github.com/KapJI/ghome-foyer-api) | Python | v3.1.0 | 2026-10-25（commit 至 2026-11-11） | **可用** | 从 APK 提取的 foyer `api.proto`（21 KB）。v3.0.0 修过真实响应字段映射 → 说明布局确实变过 |
| [leikoilja/glocaltokens](https://github.com/leikoilja/glocaltokens) | Python | v2.0.0 | 2026-11-02 | **可用（仅本地令牌）** | MIT；`oauth_token=` 参数走浏览器流程 |
| [leikoilja/ha-google-home](https://github.com/leikoilja/ha-google-home) | Python | v1.17.0 | 2026-11-06 | **可用** | 只做音箱/闹钟/蓝牙在场，**不控灯插座** |
| [ericmigi/ha-googlehome-foyer](https://github.com/ericmigi/ha-googlehome-foyer) | Python | — | 2026-11-12 | 可用，0 star | 云端控灯/锁/温控/媒体 |
| [ericmigi/googlehome-mcp](https://github.com/ericmigi/googlehome-mcp) | TS | — | 2026-08-29 | 可用 | 协议文档最全（PROTOCOL/TRAITS），活体验证 2026-07-16 |
| [simon-weber/gpsoauth](https://github.com/simon-weber/gpsoauth) | Python | 3.0.0 | 2026-11-05 | 活跃 | 参考实现；README 明确弃用密码登录 |
| `gpsoauth` **Rust crate** | — | — | — | **不存在** | crates.io 查询 `{"errors":[{"detail":"crate gpsoauth does not exist"}]}`（2026-11-15） |
| [g2p/gpsoauth-rs](https://github.com/g2p/gpsoauth-rs) | Rust | 0.1.0 | **2018-09-28** | **死** | hyper 0.12，未发布 crates.io，只实现密码流（已无用）|
| `rust_cast` | Rust | 0.21.0 | 2025-12-30 | 可用 | 16.9 万下载；Cast 局域网播报可用 |

**已死的路径**（VERIFIED，有日期）：

- `perform_master_login`（邮箱+密码/app password）自 2024 起对多数账号失效，返回 `BadAuthentication` / `NeedsBrowser` / `MissingDroidguard`。[gpsoauth#70](https://github.com/simon-weber/gpsoauth/issues/70)（open，2024-10-14 → 2026-02-24 仍有人复现）、[#81](https://github.com/simon-weber/gpsoauth/issues/81)（2026-03-05）、[glocaltokens#581](https://github.com/leikoilja/glocaltokens/issues/581)（2026-10-16 被维护者以「Google 拒绝密码/应用密码，本库无法改变」关为重复）。任何 2025 年后的教程教你填密码的都是错的。
- 云端「广播/播报」路径被 Google 拒绝：mesh `SendCommands` 一律 gRPC status 13，客户端在 Play services 的运行时下发模块里，无法复现（ghome 实测 2026-09-09）。替代：本地 Cast + Google Translate TTS（`ghome announce` 已实现）。
- 通过 foyer RPC 解绑厂商集成：`SetupService/UnlinkApplication` 只服务媒体应用，做不到（2026-09-14 反编译结论）。

---

## 3. 能控的设备与 trait（VERIFIED，活体验证）

`GetHomeGraph` 返回**账号下全部 home / room / device**，按 `agentId`（厂商的 Google Cloud 项目号）区分来源 —— 这就是「一个 CLI 管所有厂商设备」的依据。ghome 实测 2026-09-08：单 home 91 设备，`resp[8]` 里带各 partner 标签。

**Tuya/Smart Life 是否在列：是（间接但强证据）** —— ghome ROADMAP 记录「在 Home App 里 unlink 后 `devices sync`，**Tuya** 和 Yale 的设备即消失」（2026-09）。说明 C2C 关联设备确实出现在 foyer home graph 里，并可用同一 `UpdateTraits` 控制（Google 侧统一走 `action.devices.traits.*`，厂商无关）。

已验证可写的 trait（[TRAITS.md](https://github.com/ericmigi/googlehome-mcp/blob/main/docs/TRAITS.md)，2026-07-16；ghome 2026-09-09 复验）：

| 能力 | trait.field | 值编码 | 状态 |
|---|---|---|---|
| 开/关 | `onOff.onOff` | `[null,null,null,0\|1]` | ✅ 实测（插座/灯/开关） |
| 亮度 | `brightness.brightness` | `[null,0..100]` | ✅ |
| 色温 | `color.colorTemperature` | `[null,<K>]` | ✅ |
| RGB | `color.colorRGB` | `[null,0xRRGGBB]` | ✅ 实测（ghome 2026-09-09） |
| 音量/静音 | `volume.currentVolume` / `isMuted` | — | ✅ / ⚠️ 推断 |
| 播放/暂停/停止 | `mediaState.playbackState` | 字符串 | ✅ |
| 上锁/解锁 | `lockUnlock.isLocked` + `pin` | — | ✅（PIN 挑战两步式；解锁慢 10–30 s） |
| 温控设定/模式 | `temperatureSetting` | `mode` 必须与 `thermostatTemperatureSetpoint` **同时**发，且用显示单位 | ✅ |
| 例程 | `AutomationService/{List,Execute}Automation` | — | ✅（还能建/改/删 YAML 脚本） |

不可用：读播放状态回传、RGB 之外的 spectrum 写、锁的 PIN 管理 UI、门铃/摄像头。

---

## 4. 会坏的地方（运维清单）

1. `oauth_token` **一次性**且分钟级过期 → 换失败必须重新取 cookie。
2. Bearer ~1 h；**401/403 先重铸一次**，仍失败 = master token 被撤销（改密码/安全撤销）→ 重新登录。
3. 账号必须是 Gmail 地址。
4. 私有 API 无契约：ghome-foyer-api v3.0.0（2026-10）、v3.1.0（2026-10-25）都在修真实响应差异 → 字段映射会漂。
5. 单次 `GetHomeGraph` ~100 KB / 70 设备，无缓存；轮询要自己限速（官方 Home MCP 明确「enforces rate limits」但未公布数值）。
6. `GetTraits` 写入后立刻读会读到旧值，需等 ~1.5 s / 4.5 s 再确认（ghome 的实现细节）。

---

## 5. ToS / 封号风险（证据分级）

**VERIFIED**
- Google 通用服务条款（生效 **2026-07-30**）「Don't abuse our services」禁止：`bypassing our systems or protective measures`、`using automated means to access content from any of our services`、`reverse engineering our services`（[policies.google.com/terms](https://policies.google.com/terms)）。
- Google **APIs** 服务条款（最后修改 2021-11-09）：「You will only access (or attempt to access) an API by the means described in the documentation of that API」（[developers.google.com/terms](https://developers.google.com/terms)）。foyer 没有文档，严格讲这条无从遵守。
- **没有找到任何一例「因使用 Google Home 私有 API 而被封号」的公开报告**（检索 gpsoauth / glocaltokens / ha-google-home issue tracker、Reddit、Google 论坛，2024–2026）。gpsoauth / gkeepapi / gmusicapi 生态已存在 10+ 年，Google 的惯常反应是**坏掉流程 / 撤销 token**，而非封号。
- 唯一相关的账号处置传闻：ha-google-home#977 评论（2026-02）有人新建账号后「the account got suspended of course」，发生在 app password 反复失败语境，**原因未证实**；以及 Gemini/Antigravity 用第三方 OAuth 客户端被封（不同服务，2026，Google 明确援引 ToS）——这类是「白嫖付费 AI 配额」，与智能家居不同。
- glocaltokens 的历史警告：登录过于频繁会让 Google 踢掉你已登录的 Android 设备（副作用，非封号）。

**INFERENCE（我的判断，非事实）**
- 风险等级：**低—中**。低在「无先例、Google 长期默许」；中在「明面上确实踩了 ToS 的 automated access / bypass 两条，理论上可被单方面处置，且无申诉路径」。
- 缓解：用**专用小号**加入同一个 Home（Home App 支持多成员），主号不受影响；失败时只是这个号被吊销 token/停用。这是 gkeepapi 系项目的通行做法。
- 不要做的事：高频轮询（分钟级全量刷 home graph）、自动化「设备被踢就重登」循环 —— 这两件事最容易触发风控。

---

## 6. 官方替代路线（必须知道，可能直接改变方案）

| 路线 | 平台 | 日期/状态 | 门槛 | 能否给 CLI/Docker |
|---|---|---|---|---|
| **Home APIs SDK** | 仅 Android(Kotlin)/iOS(Swift) | **GA 1.11.1，2026-10-22**（Android+iOS 均 GA） | 移动 App | ❌ 无服务端/REST/SDK |
| **Home MCP** | 服务端 `https://home.googleapis.com/mcp`，streamable-http | 早访问 2026-09-16 | Google Home Premium **Advanced**（$20/月）+ Cloud 项目 OAuth(Web client, scope `home.platform.v2`) + **审批**；报道称美区英文 | ✅ CLI 只需当 MCP 客户端；但需审批、订阅，且**未说明是否覆盖 Tuya 等 C2C 设备**，禁解锁等敏感动作 |

来源：[Android release notes](https://developers.home.google.com/apis/android/release-notes)、[Home MCP 文档](https://developers.home.google.com/mcp/home.md.txt)、[unite.ai 2026-09](https://www.unite.ai/google-opens-home-mcp-early-access-to-ai-agents-for-smart-home-control/)。Android sample app 文档还提到 `home.platform.v2` 多数能力 **需要 partner approval**。

→ 官方路线今天是「订阅 + 审批 + 不确定覆盖 Tuya」，无法作为我们 CLI 的基础；但值得**留一个 MCP 后端插槽**（Home MCP 与 `ghome mcp` 都是 MCP，客户端代码可复用）。

---

## 7. 给本项目的裁决

1. **不要从零写 foyer 客户端。** 协议是位置式 JSON 数组 + 无契约字段，`ghome`/`ghome-foyer-api`/`googlehome-mcp` 已经把坑踩完且带 MIT。重写的唯一理由是许可证/可控性 —— 用 fork 更便宜。
2. **CLI 阶段（本机、arm 支持）**：`git clone piekstra/google-home-cli && cargo build --release`（rust 1.89+）。release 只有 `aarch64-apple-darwin`，**没有 aarch64-linux** —— 但既然要本地编译，交叉到 `aarch64-unknown-linux-musl` 只是加 target，不需要改代码。auth 用 `ghome auth login`（浏览器取 cookie）。
3. **Docker/web 阶段的真障碍不是 arm，是密钥存储**：ghome 通过 `pk-cli-secrets` 只写 OS keychain（macOS Keychain / Windows CredMgr / Linux Secret Service）。容器里没有 —— 两条路：(a) 给上游提 PR 加 `GHOME_MASTER_TOKEN` env / 文件后端（最省事，符合其既有 codec）；(b) 自写 web 层：只需 cookie 交换 + Bearer 铸造 + `GetHomeGraph`/`GetTraits`/`UpdateTraits` 三个 RPC，**无 crypto、无 proto codegen**，`reqwest + serde_json + axum` 约 400–600 行即可，`ghome` 的 `docs/api.md` 就是规格书。
4. **Tuya 侧什么都不用做**：设备已在 Google Home 图里，`onOff`/`brightness` 与其他厂商同构。但**厂商特有功能（电量统计、Tuya 场景、定时）拿不到** —— 那是 Tuya 私有 API 的事，与「CLI 控 Google Home」目标无关，明确不做。
5. **跨网络**：走云端 foyer，无 LAN 依赖。本地 8443 路径只在「同局域网 + 想要零延迟/断网可用」时才值得做，当前目标不需要。

### 推荐的最小验收探针（先证明通路，再写代码）
```bash
ghome auth login                     # 浏览器取 oauth_token
ghome devices list --status --json   # 应看到 Tuya 插座，带 agent_id / online
ghome devices set "<Tuya 插座名>" --on   # 2 秒后 ghome devices state 应回读为 on
```
三条都过 = 通路与权限确认，之后所有工程量都在 UI/部署层。

---

## 来源

- https://github.com/piekstra/google-home-cli ｜ https://github.com/piekstra/google-home-cli/blob/main/docs/api.md ｜ ROADMAP.md
- https://github.com/KapJI/ghome-foyer-api ｜ https://github.com/ericmigi/googlehome-mcp ｜ docs/PROTOCOL.md ｜ docs/TRAITS.md
- https://github.com/leikoilja/glocaltokens ｜ https://github.com/leikoilja/ha-google-home ｜ https://ha-google-home.pages.dev/configuration/
- https://github.com/simon-weber/gpsoauth ｜ issues #70 #81 ｜ https://github.com/simon-weber/gpsoauth/issues/70
- https://github.com/g2p/gpsoauth-rs ｜ https://crates.io/crates/rust_cast
- https://developers.home.google.com/apis/android/release-notes ｜ https://developers.home.google.com/mcp/home.md.txt ｜ https://developers.home.google.com/apis
- https://policies.google.com/terms ｜ https://developers.google.com/terms
- https://www.unite.ai/google-opens-home-mcp-early-access-to-ai-agents-for-smart-home-control/ ｜ https://developers.googleblog.com/en/build-the-future-of-home-with-google-home-apis/

---

*没做的检查：没有拿到真实 Google 账号实测（本报告全部基于他人 2026-07/09 的活体验证记录 + 我今天的仓库/API 日期核对）；没有验证 Home MCP 在中国大陆/非美区账号的可申请性；没有查 ghome 在 aarch64-linux-musl 上是否真能编过（只确认依赖不含平台限定项）。风险：master token 等同账号密码，Docker 版一旦落盘必须按密钥管理；私有 API 无契约，随时可能被 Google 改字段或关停。*