# 05 — Tuya control and Telemetry

**What to build:** An agent turns Tuya plugs on and off and reads their Telemetry from any network through Cloud Control. Readings carry both the raw value and the scaled value with units. Transient cloud errors are retried, and dead sessions report `reauth_required`.

**Blocked by:** 04 — Tuya QR login, device list, and Local Key backup

**Status:** done (2026-11-16)

- [x] `home tuya on|off <device>` sends the switch command. Device resolution follows the same rules as ticket 03
- [x] `home tuya get <device>` prints all data points. Telemetry points appear as `{raw, scale, value, unit}`, with the scale taken from the Device's declared specification and never from hard-coded DP numbers
- [x] `home tuya set <device> <code> <value>` writes any raw data point
- [x] `1108` is retried up to twice. `sign invalid` and `1010` give exit 5 with `reauth_required` naming the Tuya Backend
- [x] Identical commands sent in quick succession are all sent (no silent 10-second drop)
- [x] An offline Device gives exit 4
- [x] `on`/`off`/`set` on a protected Device (ticket 02) gives exit 6 with a `protected` error and is never sent upstream (asserted on the fake: zero command requests). `get` and Telemetry still work
- [x] Calibration data points (`voltage_coe`, `electric_coe`, `power_coe`, `electricity_coe`) are reported separately from Telemetry, not as readings
- [x] Live checks against the real account target only the Home plug. Never send any write to LAB
- [x] Live guard test on the Home plug (Tuya ID: real ID, see the device table in the local, untracked `.scratch/smarthome-v1/validation.md`), same steps as ticket 03: protect, `home tuya off` gives exit 6, state unchanged on readback, remove protection, normal off/on works, leave it on
- [x] Only after both live guard tests (03 and 05) pass: `home protect` LAB on both Backends (Tuya and Google Home IDs: real IDs, see the device table in the local, untracked `.scratch/smarthome-v1/validation.md`), confirm `home protect --list` shows both, and confirm a write attempt on LAB gives exit 6 by checking the guard with the fake upstream, not the real plug
- [x] Black-box tests cover every criterion above against the fake Tuya upstream, including a Device whose declared scale disagrees with its raw values

## Offline implementation summary

Implemented the offline-verifiable part of ticket 05 (control plus Telemetry,
never against a real account): `home tuya on|off <device>` sends `switch_1`;
`get` reports data points with `{raw, scale, value, unit}` Telemetry and the
four calibration points separately; `set` writes one raw data point converted
by its declared type; `1108` is retried up to twice, `sign invalid` and
`1010` give exit 5 `reauth_required` for the Tuya Backend; repeated identical
commands are all sent; an offline Device gives exit 4; `on`/`off`/`set` on a
protected Device give exit 6 with zero command requests while `get` still
works. Test coverage: `cargo test --test tuya_05` 13/13,
`cargo test -p tuya-backend` 28/28, full workspace gate green. Committed as
`3f7ac31`. The live steps land in `build-notes-0304.md` under the exact
heading:

### Live steps (ticket 05, human-in-the-loop)

(Real Home/LAB IDs stay in the untracked `.scratch/smarthome-v1/validation.md`.)

## Live run (2026-11-16, Windows GNU build)

- `home tuya get` on the Home plug: Telemetry voltage raw 1052, scale 1, 105.2 V;
  energy raw 2, scale 3, 0.002 kWh; the four calibration points listed apart.
- Protected: `home tuya off` gave exit 6 `protected`; readback `switch_1` still true.
- After `--remove`: off and on both exit 0, readbacks false then true; left on.
- Both live guard tests (03 and this one) passed, then the second plug was added
  to the protected list on both Backends; `home protect --list` shows both entries.
- The write refusal for a protected Device is asserted by the black-box tests
  (`tests/tuya_05.rs`, `tests/gh_03.rs`: exit 6, zero command requests). A write
  naming the real second plug was not run, not even against a fake upstream: the
  project hook refuses such a command by design.
