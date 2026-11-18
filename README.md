# home

English | [简体中文](README.zh-CN.md) | [日本語](README.ja.md)

[![CI](https://github.com/wray-lee/smarthome/actions/workflows/ci.yml/badge.svg)](https://github.com/wray-lee/smarthome/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`home` controls Google Home and Tuya devices from one Rust binary and a web
panel: every command writes one JSON document to stdout, and its exit code
says what happened.

![panel](docs/images/panel-1280-light.png)

[The same panel at 390 px in the dark theme](docs/images/panel-390-dark.png)

## What it does

Two independent Backends behind one binary:

- **Google Home** (`home gh`) operates any Device in your Google Home through
  the unofficial foyer API, forked from `piekstra/google-home-cli` v0.10.0. It
  exposes `list_devices`, `get_state` and `set_trait` to the rest of the
  workspace, the seam for a future Home MCP implementation.
- **Tuya** (`home tuya`) talks to Tuya directly: QR sign-in, device list, Local
  Key backup, Cloud Control and Telemetry.

The two Backends are independent crates and never depend on each other. One
physical plug can appear as a Device in both, under different names and with
different capabilities. The CLI is built for agents and scripts; the Web Panel
is the same command layer for humans, with the same safety rules.

The Web Panel is a zero-build single page with All, Google Home and Tuya tabs,
a Settings view for the UI language (English, Simplified Chinese or Japanese),
the default tab and the theme, and a visible `protected` label on read-only
Devices.

The [manual terms table](docs/manual/en.md#terms) pins down Backend, Device,
Protected Device, Sensitive Action, Telemetry, Local Key and the other domain
words.

## Quick start

The 0.3.0 release image is `ghcr.io/wray-lee/smarthome:0.3.0`, published for
amd64 and arm64. Write this `compose.yaml`:

```yaml
services:
  home:
    image: ghcr.io/wray-lee/smarthome:0.3.0
    container_name: home
    ports:
      - "127.0.0.1:8080:8080"
    environment:
      HOME_PASSWORD: ${HOME_PASSWORD:?set HOME_PASSWORD}
    volumes:
      - home-data:/data
    restart: unless-stopped

volumes:
  home-data:
    name: home-data
```

`HOME_PASSWORD` is required. Put it in a `.env` file next to `compose.yaml`,
mode 600 and out of git:

```dotenv
HOME_PASSWORD='{panel_password}'
```

Then start the container:

```console
chmod 600 .env
docker compose up -d
```

Sign both Backends in once inside the container:

```console
docker exec -it home home gh login --email you@example.com
docker exec -it home home tuya login
```

Open http://127.0.0.1:8080 and sign in with the same password.

Nothing is protected by default. Add every Device that must stay read-only
with `docker exec -i home home protect tuya:{device_id}` (or the same command
with `gh:{device_id}`); only `home protect --remove` takes the protection
away.

The [manual quick start](docs/manual/en.md#quick-start) adds the password file
flow, both non-interactive sign-in fallbacks and the health checks.

## Build from source

Build on Linux or in WSL. The release binary is statically linked (musl), so
it runs on any common Linux distribution or Alpine:

```console
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The [manual build and test section](docs/manual/en.md#build-and-test) covers
the WSL `CARGO_HOME` and `CARGO_TARGET_DIR` setup, offline builds and
vendoring, and `scripts/image-smoke.sh`.

## Documentation

The [English manual](docs/manual/en.md#contents) is the reference for the
binary, the Backends and the panel:

- [Terms](docs/manual/en.md#terms) and the
  [repository layout](docs/manual/en.md#repository-layout)
- [Build and test](docs/manual/en.md#build-and-test) and
  [deploy](docs/manual/en.md#deploy)
- [Google Home sign-in](docs/manual/en.md#google-home-sign-in) and
  [Google Home Devices](docs/manual/en.md#google-home-devices)
- [Tuya sign-in](docs/manual/en.md#tuya-sign-in) and
  [Telemetry](docs/manual/en.md#telemetry)
- [Database](docs/manual/en.md#database),
  [exit codes](docs/manual/en.md#exit-codes),
  [Protected Devices](docs/manual/en.md#protected-devices) and
  [upstream overrides](docs/manual/en.md#upstream-overrides)
- [Web Panel](docs/manual/en.md#web-panel), [Docker](docs/manual/en.md#docker)
  and [security](docs/manual/en.md#security)

## Safety

- A Protected Device is read-only through every Backend until
  `home protect --remove`. Nothing is protected by default; every Device that
  must not be written needs explicit protection. `--force` on the CLI and the
  panel password re-entry are one-write overrides for the human at the
  keyboard, not for agents.
- Bind the panel to loopback and reach it over Tailscale or another private
  network. Never expose it to the internet. If a reverse proxy terminates TLS,
  set `HOME_COOKIE_SECURE=true` so the session cookie carries `Secure`.
- The SQLite database is the key to the whole home and the Google account, and
  `home tuya keys` exports the Local Keys. Treat both as secrets, keep them out
  of git and back them up deliberately.

The [manual security section](docs/manual/en.md#security) has the full rules;
[Protected Devices](docs/manual/en.md#protected-devices) has the override
details.

## License

MIT, copyright 2026 wray-lee; the full text is in [LICENSE](LICENSE).

`crates/ghome-backend` is a fork of
[piekstra/google-home-cli](https://github.com/piekstra/google-home-cli)
v0.10.0, and `crates/tuya-backend` ports `lueur`'s `tuya_cloud.rs`. Both
upstream projects are MIT. Their license texts are reproduced in full in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), the original tracked copies
stay at `crates/ghome-backend/LICENSE-ghome` and
`crates/tuya-backend/LICENSE-tuya`, and the container image ships `LICENSE`
and `THIRD_PARTY_NOTICES.md` under `/usr/share/licenses/home/`. See the
[manual license section](docs/manual/en.md#license).