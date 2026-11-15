# Smarthome v1 — Spec

**Status:** ready-for-agent

## Problem Statement

The user owns Tuya smart plugs and other devices linked into Google Home. The Tuya IoT developer platform keeps failing them (auth problems, "suspend" errors from an expired IoT Core trial). Google Home can operate devices but shows no Telemetry (power, energy). There is no reliable, machine-readable way for an agent to list, read, and operate these Devices. The user also worries the Tuya vendor may disappear and wants an exit path kept open.

## Solution

One tool with two independent Backends:

- **Google Home Backend**: operates any Device in the user's Google Home, whatever vendor made it, through the unofficial foyer API (see ADR 0001).
- **Tuya Backend**: signs in with the Smart Life app's QR login (no developer platform), operates Tuya Devices through Cloud Control from any network, reads Telemetry, and stores every Local Key for a future Vendor Exit.

Agents use a CLI with JSON output. Humans use a Web Panel served from a Docker image that runs on linux amd64 and arm64.

## User Stories

1. As an agent, I want to list every Google Home Device with its ID, name, room, type, and supported traits, so that I know what I can operate.
2. As an agent, I want to read a Google Home Device's current trait state, so that I can answer questions about the home.
3. As an agent, I want to set a trait value on a Google Home Device (on/off, brightness, color temperature, thermostat mode and setpoint, volume), so that I can operate any vendor's Device.
4. As an agent, I want to refer to a Device by ID or by "room/name", so that I don't need to look up IDs first.
5. As an agent, I want an ambiguous name to fail with a JSON list of candidates instead of a guess, so that I never operate the wrong Device.
6. As an agent, I want Sensitive Actions (unlock, disarm) refused unless I pass an explicit confirmation flag, so that a misunderstood instruction cannot open the door.
7. As an agent, I want distinct exit codes for "device offline", "re-auth required", "Sensitive Action refused", "ambiguous name", and "upstream error", so that I can decide what to do without parsing prose.
8. As an agent, I want every command's stdout to be one JSON document, including errors, so that I can parse results reliably.
9. As an agent, when a Backend's credentials are no longer valid, I want a `reauth_required` result telling me which Backend, so that I can notify the user instead of retrying.
10. As the user, I want to sign in to Google Home once through a browser and have the tool keep working afterwards, so that I don't log in repeatedly.
11. As the user, I want to sign in to Tuya by scanning a QR code with the Smart Life app, so that I never need the Tuya IoT developer platform.
12. As the user, I want the Tuya QR code shown in the terminal, so that I can sign in from the CLI.
13. As an agent, I want to list Tuya Devices with ID, name, online state, and category, so that I can find plugs.
14. As an agent, I want to turn a Tuya plug on or off from any network, so that control does not depend on being at home.
15. As an agent, I want a Tuya plug's Telemetry (power, current, voltage, energy) with both the raw value and the scaled value with units, so that I get usable numbers even when a device declares the wrong scale.
16. As an agent, I want to read and write any raw Tuya data point by code, so that non-plug Tuya Devices are not locked out.
17. As the user, I want every Tuya Device's Local Key saved whenever I sign in, so that I keep a Vendor Exit if the Tuya cloud disappears.
18. As the user, I want to export all saved Local Keys as JSON, so that I can back them up off the machine.
19. As the user, I want transient Tuya cloud errors retried automatically, so that occasional flakiness does not surface as failures.
20. As the user, I want rotated Tuya refresh tokens persisted every time, so that the session survives restarts.
21. As the user, I want all credentials and caches in one SQLite file, so that backup and Docker mounting are one file.
22. As the user, I want the SQLite file created with owner-only permissions, so that other local users cannot read my keys.
23. As the user, I want to run the tool in Docker on a linux arm64 machine, so that it can live on a small always-on box.
24. As the user, I want the Web Panel protected by a password, so that anyone on the network cannot operate my home.
25. As the user, I want the Web Panel to show Devices from both Backends, labelled by Backend, so that I can see everything in one place.
26. As the user, I want controls on the Web Panel generated from each Device's traits, so that new device types work without new UI code.
27. As the user, I want the Web Panel to show my action immediately and then correct itself from the real state, so that slow devices still feel responsive.
28. As the user, I want the Web Panel to poll only while it is in the foreground, at most every 10 seconds, so that the tool does not trigger rate limits.
29. As the user, I want the Web Panel to show a fresh Tuya QR code when Tuya needs re-auth, so that I can recover by scanning once.
30. As the user, I want the Web Panel to tell me when Google Home needs re-auth and how to sign in again, so that I am not left guessing.
31. As the user, I want the Web Panel to show Telemetry for Tuya plugs, so that I can see power use at a glance.
32. As a developer, I want the Google Home Backend behind one narrow interface, so that it can be swapped for Google's official Home MCP server later.

## Implementation Decisions

- **Workspace**: one Rust workspace. A Google Home Backend crate, a Tuya Backend crate, a shared storage/output layer, and one binary named `home` with subcommands per Backend (`home gh …`, `home tuya …`) plus `home serve` for the Web Panel. The two Backend crates never depend on each other.
- **Google Home Backend**: forked from `piekstra/google-home-cli` (MIT). Its auth chain (browser `oauth_token` cookie, then gpsoauth master token, then a 1-hour Bearer) and its foyer codec (positional JSON arrays) are kept. Its OS-keychain token storage is replaced with SQLite. Bearer requests send no API key. On 401/403, mint a new Bearer once; if that fails, report `reauth_required`. The fixed `androidId` is stored with the master token.
- **Google Home interface**: list Devices, get state, set trait. This is the only seam the CLI and Web Panel use, so a Home MCP implementation can replace foyer.
- **Name resolution**: exact ID match first, then "room/name", then bare name. More than one match is an error that lists the candidates.
- **Sensitive Actions**: unlocking a lock or disarming security is refused without `--confirm`. The Web Panel asks the user for the same confirmation.
- **Tuya Backend**: ported from the MIT `tuya_cloud.rs` in `lueur`, using Home Assistant's public client id and schema. The official Python `tuya-device-sharing-sdk` is the reference. QR login uses the hard-coded login host, and later calls go to the regional endpoint the login returns. All calls are serialised through one client. Every rotated refresh token is persisted immediately. `sign invalid` and `1010` mean re-auth required. `1108` is retried up to twice. The SDK's silent drop of repeated commands within 10 seconds is not reproduced.
- **Telemetry**: values are reported as `{raw, scale, value, unit}`. The scale comes from the Device's declared specification and is never hard-coded to DP numbers.
- **Local Keys**: saved for every Device that has one on each sign-in and device refresh. A key that changes is recorded with the time it changed. `home tuya keys` exports them.
- **Storage**: one SQLite file at a configurable path, created with mode 600. Tables cover Backend credentials, the Device cache per Backend, and Tuya Local Keys. Schema migrations are versioned from the start. No application-level encryption.
- **CLI contract**: stdout is always exactly one JSON document. Exit codes: 0 ok; 2 usage; 3 ambiguous name; 4 Device offline; 5 `reauth_required`; 6 Sensitive Action refused; 7 upstream error.
- **Upstream overrides**: every upstream base URL (foyer, gpsoauth, Tuya login host, Tuya regional endpoint) can be overridden by configuration. This exists for testing.
- **Web Panel**: served by the same binary through `home serve`. Its HTTP endpoints call the same command layer as the CLI and return the same JSON. Password auth comes from a configured secret, with a session cookie after login. It is meant for LAN use; remote access goes through Tailscale. The front end is a small static page with no build step.
- **Build and deploy**: developed and tested in WSL. A multi-stage Dockerfile builds with `docker buildx` for linux/amd64 and linux/arm64. The SQLite file lives on a mounted volume and is never baked into the image.

## Testing Decisions

- Good tests check external behaviour only: run the `home` binary and assert its stdout JSON and exit code. Never assert on internal functions.
- **Primary seam**: black-box CLI tests. Upstreams are faked with `wiremock`, replaying response shapes recorded from the reference implementations and from the validation ticket. Each test gets a temporary SQLite file.
- Covered through this seam: device listing, name ambiguity, Sensitive Action refusal, the `reauth_required` exit code and Bearer re-mint, Telemetry scaling (raw and scaled), `1108` retry, refresh-token rotation persistence, Local Key saving, export, and change tracking.
- **Extra seam, Tuya signing only**: one unit test checks the request signing and AES-GCM sealing against fixed vectors produced by the official Python SDK, because the fake upstream cannot catch a wrong signature.
- **Web Panel**: one smoke test. Without the password it returns 401. With the password, the device list returns the same JSON as the CLI.
- **Real accounts**: not automated. The validation ticket is the manual acceptance, and its results are recorded under `.scratch/`.
- The repo has no prior art yet. The tests from the forked `ghome` are the starting point for the Google Home Backend.

## Out of Scope

- History, charts, and any background collector (v2)
- Tuya MQTT push (v2)
- Local Control of Tuya Devices and reflashing firmware (Vendor Exit, deferred; `rustuya` is the chosen library when it happens)
- Merging the same physical plug across the two Backends
- Google Home Home MCP implementation (the interface is ready; there is no implementation)
- Application-level encryption of credentials
- Public internet exposure of the Web Panel
- A native Windows build

## Further Notes

- Records: `validation.md` (ticket 01 report) and `tools/` are local and untracked on purpose — they name the real device IDs and the real account, and no real ID goes into git. Tickets refer to the device table in `validation.md` instead of spelling IDs out.
- The facts that are still unknown: whether the user's Smart Life account is in the China region (the hard-coded QR login host has reportedly failed there), and whether the Google account is an @gmail.com address (Workspace accounts see 0 Devices through foyer). The validation ticket settles both.
- Foyer is undocumented and its fields have changed before. If it breaks, we capture traffic from home.google.com ourselves (ADR 0001).
- The Google master token is equivalent to the account password, and the user chose to use their main account.
- Research: `.scratch/research-google-home.md`, `.scratch/research-tuya.md`. Glossary: `CONTEXT.md`.
