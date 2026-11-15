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
