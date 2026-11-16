# 06 — Multi-arch Docker image and minimal Web Panel

**What to build:** The user deploys one Docker image on a linux arm64 or amd64 box, opens the Web Panel, enters a password, sees Devices from both Backends labelled by Backend, and turns them on and off. The panel's HTTP endpoints call the same command layer as the CLI and return the same JSON.

**Blocked by:** 03 — Google Home read and write with safety rules; 05 — Tuya control and Telemetry

**Status:** done

- [x] `home serve` serves the panel and its JSON endpoints from the same binary
- [x] Password from a configured secret, then a session cookie. Every endpoint without a session returns 401
- [x] The panel lists Devices from both Backends with a Backend label and an on/off control. It is a static page with no front-end build step
- [x] Protected Devices show their state with a visible "protected" label (text, not color alone) and no on/off control. Their write endpoints return the same exit-6 JSON error
- [x] Sensitive Actions require the same explicit confirmation as the CLI
- [x] While `home serve` runs, it keeps the Tuya session alive without waiting for a request: a background task refreshes the access token shortly before it expires and persists the rotated refresh token first (same path as ticket 04, same serial lock). A failed refresh is logged and shown in the panel as `reauth_required` for the Tuya Backend. Black-box test with a fake upstream and a short `expireTime`: a refresh happens with zero client requests
- [x] A multi-stage Dockerfile builds with `docker buildx` for linux/amd64 and linux/arm64. The SQLite file lives on a mounted volume and is never baked into the image
- [x] Smoke test: 401 without the password, and with it the device list matches the CLI's JSON
- [x] README states that the SQLite file is the key to the whole home and the Google account, and that remote access goes through Tailscale
- [x] WSL build caches and temporary files are deleted when the ticket ends; images go to the host `tmp/` only if they must be kept
