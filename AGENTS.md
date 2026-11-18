# AGENTS.md

`home` is one Rust binary and a Web Panel for Google Home and Tuya Devices.
Every command writes one JSON document to stdout, and the exit code says what
happened. Domain terms live in `CONTEXT.md`; decisions in `docs/adr/`.

## Layout

- `crates/home-store` - SQLite store, the JSON and exit-code contract, and
  `guard`, the one write gate.
- `crates/ghome-backend` - Google Home through the unofficial foyer API
  (ADR 0001).
- `crates/tuya-backend` - Tuya through Smart Life QR login and Cloud Control.
- `src/main.rs` - the `home` binary: CLI and `serve`.
- `src/web/` - the panel: zero-build single-file `index.html` and `login.html`.
- `tests/` - black-box CLI and panel tests (`cli.rs`, `gh_03.rs`, `tuya_04.rs`,
  `tuya_05.rs`, `serve_06.rs`, `serve_07.rs`).
- `docs/adr/` - architecture decisions.
- `docs/manual/` - the manual in English, Simplified Chinese and Japanese.
- `docs/images/` - the panel screenshots used by the READMEs.
- `scripts/image-smoke.sh` - container smoke check.

The two Backends never depend on each other.

## Build and test

Run everything on Linux or in WSL. Point `CARGO_HOME` at your host's cargo
cache (for example `/mnt/c/Users/<you>/.cargo`) and `CARGO_TARGET_DIR` at a
Linux-native path (for example `/home/<you>/work/target`); a target directory
on a Windows mount is slow.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
scripts/image-smoke.sh <image_tag>
```

`scripts/check-docs.sh` checks the six README and manual files for consistency; CI runs it after the tests.

## Engineering rules

Test-driven. For every behaviour change, write the black-box test first and
watch it fail (RED), make it pass (GREEN), then refactor. A change without its
test is unfinished.

Before editing `src/web/*`, query the `modern-web-guidance` skill (`search`,
then `retrieve`). Keep the panel zero-build and single-file: no external
assets, CSP-compliant, WCAG AA, interactive targets at least 44px, no em
dashes.

Exit codes: `0` ok, `2` usage, `3` ambiguous, `4` offline, `5`
reauth_required, `6` refused (Sensitive Action or Protected Device), `7`
upstream. `CONTEXT.md` defines the terms.

## Safety

- Never write to, or run a live test against, a Protected Device. That holds
  for the one-time overrides too: an agent never passes `--force` and never
  enters the panel password for one. The overrides exist for the human at the
  keyboard. Protection ends only with `home protect --remove`.
- Run live hardware tests only against the Devices the user designated as
  safe, and never against a protected or otherwise excluded Device.
- Never modify or bypass `.claude/hooks/`.
- Never commit secrets: real Device IDs, email addresses, `local_key`, tokens,
  or `home.sqlite`. The SQLite file holds the master key to the home and the
  Google account.
- Before committing, read `git config user.name` and `git config user.email`
  and use the configured identity. Never fabricate an email address; if git
  has none configured, ask the user.
