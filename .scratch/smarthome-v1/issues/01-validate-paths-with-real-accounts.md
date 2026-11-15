# 01 — Validate both Backends with the user's real accounts

**What to build:** Nothing is written in this ticket. It proves that both Backends work with the user's real accounts before any code depends on them. It needs the user present for the browser login and the QR scan. Raw artifacts go to the host `tmp/`. A short report goes to `.scratch/smarthome-v1/validation.md`, and redacted response samples go to `.scratch/smarthome-v1/fixtures/` for the fake upstreams to replay.

**Blocked by:** None — can start immediately

**Status:** done (2026-11-16, see validation.md)

- [ ] Record whether the Google account is an @gmail.com address
- [ ] With upstream `ghome` (Windows release binary): `ghome auth login` succeeds, the device list returns more than 0 Devices, and the Tuya plug is among them
- [ ] Turn the Tuya plug on and off through `ghome`, and confirm the state reads back within 30 seconds
- [ ] Record whether the Smart Life account is in the China region
- [ ] The Smart Life QR login (official Python `tuya-device-sharing-sdk`, run in WSL `/tmp` and deleted afterwards) succeeds against the hard-coded login host
- [ ] The device list contains a `local_key` for the plug, and its status includes power/current/voltage/energy data points with their declared scale
- [ ] One cloud on/off command succeeds
- [ ] Redacted samples saved (tokens, keys, home address, and per-device local auth tokens removed): foyer GetHomeGraph/GetTraits/UpdateTraits, Tuya device list, specification, status, and command responses
- [ ] Fixed signing and sealing vectors produced by the Python SDK are saved for ticket 04
- [ ] If either path fails, the report says which step failed and which fallback applies (capture home.google.com traffic ourselves; for a China-region account, find the regional login host)
