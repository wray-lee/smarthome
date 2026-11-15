# home

Smart home devices from the terminal: one binary, a JSON document on stdout
for every command, and exit codes that say what happened.

Two independent Backends behind it:

- **Google Home** (`home gh …`) — operates any Device in your Google Home,
  through the unofficial foyer API. See `docs/adr/0001`.
- **Tuya** (`home tuya …`) — direct Tuya access for Telemetry. Not built yet.

## Layout

- `crates/home-store` — the SQLite storage layer, the JSON/exit-code
  contract, and the one write guard for protected Devices.
- `crates/ghome-backend` — the Google Home Backend, forked from
  `piekstra/google-home-cli` v0.10.0 (MIT; see `LICENSE-ghome` there).
- `src/main.rs` — the `home` binary.

## Build and test

Developed and tested in WSL with no Windows toolchain:

```console
cargo build
cargo test
cargo clippy --workspace --all-targets -- -D warnings
```

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

## The database

One file, path from `--db` or `HOME_DB`, default
`$XDG_DATA_HOME/home/home.sqlite`. It is the key to the whole home: created
owner-only (mode 600), versioned migrations, no application-level encryption.
Keep it out of the repo and back it up deliberately.

## Exit codes

0 ok; 2 usage; 3 ambiguous name; 4 Device offline; 5 `reauth_required`;
6 Sensitive Action or protected Device refused; 7 upstream error.

## Protected Devices

`home protect google_home:<id>` (or `gh:<id>`, `tuya:<id>`) adds a Device to
the protected list; `home protect --list` shows it; `home protect --remove`
is the only way out. Every write path goes through `home_store::guard`, which
refuses a protected Device with exit 6 before any upstream call. No flag
bypasses it. Nothing is protected by default.

## Upstream overrides

For tests: `HOME_FOYER_URL` (foyer RPC prefix), `HOME_GPSOAUTH_URL` (Android
account-token endpoint).
