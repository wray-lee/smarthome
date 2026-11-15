# 03 — Google Home read and write with safety rules

**What to build:** An agent reads and sets traits on any Google Home Device, referring to it by ID or by "room/name". Ambiguous names fail with a list of candidates, Sensitive Actions need `--confirm`, and expired credentials report `reauth_required` instead of failing obscurely.

**Blocked by:** 02 — Project skeleton plus Google Home sign-in and device list

**Status:** in-progress (awaiting live test)

- [x] `home gh get <device>` prints the current trait state
- [x] `home gh set <device> <trait> <value>` works for on/off, brightness, color temperature, thermostat mode plus setpoint (sent together), and volume
- [x] Resolution order: exact ID, then "room/name", then bare name. Multiple matches give exit 3 with a JSON candidate list
- [x] Unlock and disarm without `--confirm` give exit 6 and are never sent upstream (asserted on the fake)
- [x] On 401/403 the Bearer is minted again once and the call retried. A second failure gives exit 5 with `reauth_required` naming the Google Home Backend
- [x] An offline Device gives exit 4
- [x] The Google Home Backend exposes only list / get state / set trait to the rest of the workspace (the seam for a future Home MCP implementation)
- [x] Any `set` on a protected Device (ticket 02) gives exit 6 with a `protected` error and is never sent upstream, even with `--confirm` (asserted on the fake: zero UpdateTraits requests). Reading its state still works
- [x] Black-box tests cover every criterion above against the fake foyer
- [ ] Live checks against the real account target only the Home plug. Never send any write to LAB
- [ ] Live guard test on the Home plug (Google Home ID: real ID, see the device table in the local, untracked `.scratch/smarthome-v1/validation.md`): record its state, `home protect` it, `home gh set … onOff false` gives exit 6, a later read shows the state unchanged, `home protect --remove` it, then one normal off/on round trip succeeds. Leave it on at the end

Offline work is done (2026-11-16, commits in
`.scratch/smarthome-v1/build-notes-0304.md` `## final integration`);
`cargo test --test gh_03` 17/17, `cargo test -p ghome-backend` 46/46, full
workspace gates green. The two unchecked items need the real account and are
listed in `.scratch/smarthome-v1/build-notes-0304.md` (`## live steps`).
Real IDs come from `validation.md`, never from this file.
