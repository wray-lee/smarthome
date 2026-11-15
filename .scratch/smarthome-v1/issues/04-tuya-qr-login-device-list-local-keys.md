# 04 — Tuya QR login, device list, and Local Key backup

**What to build:** The user runs `home tuya login`, scans a QR code shown in the terminal with the Smart Life app, and is signed in without the Tuya IoT developer platform. `home tuya devices` lists Tuya Devices. Every Local Key is saved on each sign-in and refresh, and `home tuya keys` exports them for the Vendor Exit backup.

**Blocked by:** 01 — Validate both Backends with the user's real accounts; 02 — Project skeleton plus Google Home sign-in and device list

**Status:** ready-for-agent

- [ ] The Tuya Backend crate is ported from `lueur`'s MIT `tuya_cloud.rs`, with attribution, and does not depend on the Google Home Backend crate
- [ ] `home tuya login` renders the QR code in the terminal, polls until it is scanned, and stores the tokens plus the regional endpoint in SQLite
- [ ] All Tuya calls go through one serialised client. Every rotated refresh token is persisted before it is used
- [ ] `home tuya devices` prints ID, name, online state, category, and Backend
- [ ] Local Keys are stored per Device. A changed key is kept with the time it changed
- [ ] `home tuya keys` prints all Local Keys as JSON
- [ ] Unit test: request signing and AES-GCM sealing match the fixed Python SDK vectors from ticket 01
- [ ] Black-box tests against a fake Tuya upstream: login poll, device list, Local Key saving, key change tracking, refresh-token rotation persisted
