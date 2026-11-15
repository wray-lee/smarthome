# 02 — Project skeleton plus Google Home sign-in and device list

**What to build:** The user runs `home gh login` once, then `home gh devices` prints every Google Home Device as one JSON document. Behind that: the Rust workspace, the fork of `piekstra/google-home-cli` with keychain storage replaced by the SQLite file, the shared JSON output and exit-code contract, configurable upstream base URLs, and the black-box test harness (binary + `wiremock` + temporary SQLite) that every later ticket uses.

**Blocked by:** 01 — Validate both Backends with the user's real accounts

**Status:** done (2026-11-16)

- [x] One workspace builds in WSL. The Google Home Backend crate and the shared storage/output layer are separate, and the binary is `home`
- [x] The fork keeps the upstream MIT license and attribution
- [x] The SQLite file sits at a configurable path, is created with mode 600, and has a versioned migration
- [x] `home gh login` stores the master token and the fixed `androidId` in SQLite
- [x] `home gh devices` prints ID, name, room, type, traits, and Backend for each Device, and caches the list in SQLite
- [x] stdout is always exactly one JSON document, including errors. Exit codes follow the spec (0/2/3/4/5/6/7)
- [x] Every upstream base URL can be overridden by configuration
- [x] Black-box tests: the device list against a fake foyer replaying the ticket 01 fixtures, and a usage error giving exit code 2 with JSON
- [x] Bearer requests send no API key
- [x] Protected Devices: SQLite holds a protected list keyed by Backend + Device ID. `home protect <device>` adds, `home protect --list` shows, `home protect --remove <device>` is the only way out. Every write path (both Backends, CLI and later the Web Panel) goes through one shared guard that refuses a protected Device with exit 6 before any upstream call. No flag bypasses it
- [x] Nothing is seeded as protected. LAB gets locked only after the guard passes live tests on the Home plug (end of ticket 05). Until then the never-write-LAB rule is enforced by the agents themselves
- [x] Black-box test: protect, list, and remove round-trip against a temporary SQLite file
