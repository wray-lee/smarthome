# 04 — Tuya QR login, device list, and Local Key backup

**What to build:** The user runs `home tuya login`, scans a QR code shown in the terminal with the Smart Life app, and is signed in without the Tuya IoT developer platform. `home tuya devices` lists Tuya Devices. Every Local Key is saved on each sign-in and refresh, and `home tuya keys` exports them for the Vendor Exit backup.

**Blocked by:** 01 — Validate both Backends with the user's real accounts; 02 — Project skeleton plus Google Home sign-in and device list

**Status:** done (2026-11-16)

CLI wiring (`home tuya login|devices|keys`) is in (commits in
`.scratch/smarthome-v1/build-notes-0304.md` `## final integration`).
A successful sign-in now fetches one device list and saves every Local Key,
so `home tuya login` alone backs the keys up; a failed fetch is reported on
stderr and still leaves the sign-in done (exit 0). All offline criteria pass
— `cargo test --test tuya_04` 17/17, `cargo test -p tuya-backend` 23/23,
full workspace gates green. The one
manual step still open is the real Smart Life QR scan against the real
account (see `.scratch/smarthome-v1/build-notes-0304.md`, `## Live steps (tickets 03/04, human-in-the-loop)`).
No write command exists in this ticket; never send any write to LAB.

- [x] The Tuya Backend crate is ported from `lueur`'s MIT `tuya_cloud.rs`, with attribution, and does not depend on the Google Home Backend crate
- [x] `home tuya login` renders the QR code in the terminal, polls until it is scanned, and stores the tokens plus the regional endpoint in SQLite
- [x] All Tuya calls go through one serialised client. Every rotated refresh token is persisted before it is used
- [x] `home tuya devices` prints ID, name, online state, category, and Backend
- [x] Local Keys are stored per Device. A changed key is kept with the time it changed
- [x] `home tuya keys` prints all Local Keys as JSON
- [x] Unit test: request signing and AES-GCM sealing match the fixed Python SDK vectors from ticket 01
- [x] Black-box tests against a fake Tuya upstream: login poll, device list, Local Key saving, key change tracking, refresh-token rotation persisted
