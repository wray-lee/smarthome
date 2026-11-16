# home

Smart home devices from the terminal: one binary, a JSON document on stdout
for every command, and exit codes that say what happened.

Two independent Backends behind it:

- **Google Home** (`home gh …`) — operates any Device in your Google Home,
  through the unofficial foyer API. See `docs/adr/0001`.
- **Tuya** (`home tuya …`) — direct Tuya access (QR login, device list, Local
  Key backup, Cloud Control, Telemetry).

The two Backends are independent crates and never depend on each other. The
Google Home Backend exposes only `list_devices`, `get_state` and `set_trait`
to the rest of the workspace — the seam for a future Home MCP implementation.

## Layout

- `crates/home-store` — the SQLite storage layer, the JSON/exit-code
  contract, and the one write guard for protected Devices.
- `crates/ghome-backend` — the Google Home Backend, forked from
  `piekstra/google-home-cli` v0.10.0 (MIT; see `LICENSE-ghome` there).
- `crates/tuya-backend` — the Tuya Backend, ported from `lueur`'s MIT
  `tuya_cloud.rs` (see `LICENSE-tuya` there).
- `src/main.rs` — the `home` binary.

## Build and test

Developed and tested in WSL with no Windows toolchain: Arch Linux, user
`wray`, `cargo` from the distro. `Cargo.lock` is committed, so dependency
versions are fixed; that does not make the crates themselves available.

This WSL has no outbound network (crates.io and `static.crates.io` are
unreachable). A clean `cargo build` therefore fails, and the README does not
pretend otherwise:

- with the default `CARGO_HOME` (`~/.cargo`), `cargo build --offline` stops at
  `no matching package named 'rusqlite' found`; without `--offline` it hangs
  updating the crates.io index, each attempt waiting 30 s and retrying (still
  going after 130 s) because `static.crates.io` cannot be reached.
- the working setup points `CARGO_HOME` at the Windows host's cargo cache
  (read and written from WSL through `/mnt/c`):

```console
export CARGO_HOME=/mnt/c/Users/Wray/.cargo
export CARGO_TARGET_DIR=/home/wray/work/smarthome/target   # inside WSL, not on /mnt
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Point `CARGO_TARGET_DIR` at a path inside WSL: building onto `/mnt/f` is slow
and cases like `target/` there are easy to commit by accident. The same two
exports plus `--release` build the binary. To drop the host cache, vendor the
dependencies (`cargo vendor`) or pre-fill WSL's own registry cache while the
machine still has a network; until then, those `CARGO_HOME`/`CARGO_TARGET_DIR`
exports are a prerequisite, not a convenience.

## Sign in and list Devices

The credential is the Google master token (equivalent to the account
password) or a one-time browser `oauth_token`. It goes to stdin, never argv:

```console
home gh login --email you@example.com
home gh devices
```

Both commands print one JSON document. `home gh login` stores the master
token and the fixed Android id in SQLite.

`home gh login` does not verify the master token: anything that starts with
`aas_et/` (or `oauth2_4/`, which is exchanged first) is stored, and a bad
token surfaces at the first `home gh devices` as `reauth_required` (exit 5).
Login never calls foyer, so it contacts Google only for the `oauth_token`
exchange.

The Android id is bound to the master token (every Bearer is minted for it),
so a repeated login reuses the stored one. `--android_id <ID>` replaces it
and drops the cached Bearer. `--help` and `--version` print their text on
stderr and still write one JSON document to stdout.

## Read and operate Google Home Devices

A Device can be named by exact ID, by `room/name`, or by bare name; matching
is case-insensitive, and a name that matches more than one Device fails with
exit 3 and the candidates instead of guessing.

```console
home gh get 客厅/lamp
home gh set 客厅/lamp onOff false
home gh set 客厅/lamp brightness 75
home gh set 客厅/lamp color.colorTemperature 3000
home gh set 客厅/lamp thermostat 21 --mode heat
home gh set 客厅/lamp volume.currentVolume 30
```

`<trait>` accepts `onOff`, `brightness`, `color.colorTemperature`,
`volume.currentVolume`, `lockUnlock.isLocked`, `armDisarm.isArmed` and
`thermostat`. A thermostat needs both a mode and a setpoint (`<value>` may be
either, the flags supply the other). `home gh get` prints the current traits
under `traits`; `home gh set` prints the resulting state plus `changed`.

Unlocking a lock or disarming an alarm is a Sensitive Action: without
`--confirm` it is refused with exit 6 and no trait call is sent upstream. A
protected Device is refused with exit 6 even with `--confirm`; reading it
still works. A `set` that passes those refusals reads the Device once more
(one `GetTraits`) to check it is online, and only then writes.

## Sign in to Tuya and back up Local Keys

`home tuya login` renders a QR code in the terminal (stderr) and polls until
the Smart Life app scans it; the tokens and the regional endpoint are then
stored in SQLite. The user code it asks for is in the app under Me →
Settings → Account and Security → User Code, and can come from
`HOME_TUYA_USER_CODE` instead of `--user-code` (it is not a secret, but argv
is visible to other processes, so the env var is preferred):

```console
home tuya login                    # or: HOME_TUYA_USER_CODE=… home tuya login
home tuya devices
home tuya keys > ~/tuya-keys.json  # back this up, off this machine
```

`home tuya devices` prints ID, name, online state, category and Backend, and
saves every Local Key it sees. A successful sign-in already fetches one
device list and saves every Local Key it returns, so `home tuya login` alone
backs them up; `home tuya devices` refreshes them on every run.
`home tuya keys` exports them as JSON
(`device_id`, `local_key`, `first_seen_at`, `changed_at`) for the Vendor Exit.
Treat that file as a secret: it belongs in your backups, never in git. A
changed key keeps the old one in history with the time it was replaced.

The QR sign-in needs no Tuya IoT developer platform. It was validated on a
non-China account; on a China-region account the hard-coded login host has
reportedly failed.

## Control and read Telemetry

One Device is named by exact ID or by bare name, case-insensitive (a Tuya
Device row carries no `room`, so `room/name` does not apply here; an exact ID
wins over a name that spells it, and a repeated name is exit 3 with the
candidates). Tuya Device rows have no `room` field.

```console
home tuya on <device>                       # switch_1 = true
home tuya off <device>                      # switch_1 = false
home tuya get <device>                      # data points + Telemetry + calibration
home tuya set <device> <code> <value>       # one raw data point, e.g. child_lock true
```

`on`/`off` send `switch_1`; the answer is `changed` with the code and the new
value. `set` finds the code in the declared specification and converts the
value to its declared type (`Boolean` accepts `true`/`false`, `Integer`
accepts a decimal integer, anything else stays a string), writes the raw
value, and does not re-read: `get` is the read path. An unknown code or a
value that does not convert fails with exit 2 and sends nothing.

`get` prints under `data_points` every point of the Device's status, in order,
each with `code`, `value` and, when the specification declares one, `type`.
`telemetry` holds the measured readings as `{raw, scale, value, unit}` where
`scale` and `unit` come from the Device's declared specification (never from
hard-coded DP numbers) and `value` is `raw / 10^scale` (raw itself when
`scale` is 0). A point is Telemetry when the specification gives it a `unit`,
it is not one of the Device's functions, and it is not calibration:
`voltage_coe`, `electric_coe`, `power_coe` and `electricity_coe`. Those four
are reported under `calibration`, raw values, never as readings.

A protected Device is refused by `on`/`off`/`set` with exit 6 before any
command is sent; `get` still works. An offline Device fails every command
with exit 4. If the session is dead (`sign invalid` or `1010`), exit 5 names
the Tuya Backend. A transient `1108` is retried up to twice inside the one
serialised client; anything else is exit 7.

## The database

One file, path from `--db` or `HOME_DB`, default
`$XDG_DATA_HOME/home/home.sqlite`. It is the key to the whole home and the
Google account: losing it leaks the Google master token (equivalent to the
account password) and the Tuya sessions, and restoring it onto a new machine
restores both. Created owner-only (mode 600), versioned migrations, no
application-level encryption. Keep it out of the repo and back it up
deliberately.

Reach the panel from outside your house through Tailscale. The panel has no
other remote access path and must never be exposed to the public internet.

## Exit codes

0 ok; 2 usage; 3 ambiguous name; 4 Device offline; 5 `reauth_required`;
6 Sensitive Action or protected Device refused; 7 upstream error.

## Protected Devices

`home protect google_home:<id>` (or `gh:<id>`, `tuya:<id>`) adds a Device to
the protected list; `home protect --list` shows it; `home protect --remove`
is the only way out. Every write path goes through `home_store::guard`, which
refuses a protected Device with exit 6 before any trait RPC (the Google Home
name lookup has already run). No flag bypasses it. Nothing is protected by
default.

## Upstream overrides

For tests: `HOME_FOYER_URL` (foyer RPC prefix), `HOME_GPSOAUTH_URL` (Android
account-token endpoint), `HOME_TUYA_LOGIN_URL` (QR login base),
`HOME_TUYA_ENDPOINT` (regional API base; a loopback host is accepted for
tests), `HOME_TUYA_USER_CODE`, `HOME_TUYA_POLL_MS` and
`HOME_TUYA_POLL_ATTEMPTS` (QR poll interval and count).

## Web Panel (`home serve`)

`home serve` binds `HOME_HOST`/`HOME_PORT` (or `--host`/`--port`) and refuses
to start without a password (`HOME_PASSWORD`, `--password`, or
`HOME_PASSWORD_FILE`/`--password-file`, preferred for Docker because argv is
visible to other processes). It serves the panel at `/`, the login form at
`/login` (POST sets an `HttpOnly; SameSite=Lax` `home_session` cookie; add
`--secure-cookie` or set `HOME_COOKIE_SECURE` when the panel sits behind an
HTTPS reverse proxy so the cookie carries `Secure`), and a JSON API under
`/api/`. Unauthenticated API requests return 401 JSON (`unauthorized`); a
wrong password POST returns 401. `GET /api/devices` returns both Backends
side by side, so one dead Backend does not hide the other; `GET /api/health`
and `GET /api/status` report panel health. A background ticker refreshes the
Tuya token before expiry and persists the rotated refresh token first, so the
session stays alive without waiting for a request.

The panel calls the same command layer as the CLI and keeps the CLI safety
rules. A protected Device shows a visible `protected` label (text, not color
alone) and no on/off control, and its write endpoints return exit code 6.
Unlocking a lock or disarming an alarm asks for the same explicit
confirmation as the CLI.

| CLI exit code | HTTP status | Meaning |
|---|---|---|
| 0 | 200 | ok |
| 2 | 400 | usage |
| 3 | 409 | ambiguous name |
| 4 | 503 | device offline |
| 5 | 424 | `reauth_required` |
| 6 | 403 | Sensitive Action or protected Device refused |
| 7 | 502 | upstream error |
| not authenticated | 401 | `unauthorized` |

Without a session, every `/api/` request returns 401 before any of the above
is reached.

## Docker

One image holds the `home` binary. The SQLite file lives on a mounted
volume, never baked into the image, and the container runs as the
non-root `home` user (UID 10001). Build both architectures with
`docker buildx`:

```console
docker buildx build --platform linux/amd64,linux/arm64 -t home:latest .
```

Run the panel with the data directory on a volume and the password in a
secret file (or `HOME_PASSWORD`):

```console
docker run -d --name home -p 8080:8080 \
  -v home-data:/data \
  -e HOME_PASSWORD_FILE=/run/secrets/home-password \
  --secret home-password \
  --restart unless-stopped home:latest
```

The runtime image sets `HOME_DB=/data/home.sqlite`, `HOME_HOST=0.0.0.0` and
`HOME_PORT=8080`, so no other configuration is needed.

Before the first write on any new database, re-add the protection:
`home protect gh:<id>` and `home protect tuya:<id>` for every protected
Device, or copy an existing `home.sqlite` (where the protection already
lives on both Backends) into the volume instead. A fresh volume holds an
empty database with nothing protected.
