# home manual

[English](en.md) | [简体中文](zh-CN.md) | [日本語](ja.md)

This is the English manual for `home`, one Rust binary and a Web Panel for
Google Home and Tuya Devices. Every command writes one JSON document to
stdout, and the exit code says what happened. The README is the overview;
this page records every command, option and rule.

<a id='contents'></a>
## Contents

- [Contents](#contents)
- [Terms](#terms)
- [Repository layout](#repository-layout)
- [Build and test](#build-and-test)
- [Quick start](#quick-start)
- [Deploy](#deploy)
- [Google Home sign-in](#google-home-sign-in)
- [Google Home Devices](#google-home-devices)
- [Tuya sign-in](#tuya-sign-in)
- [Telemetry](#telemetry)
- [Database](#database)
- [Exit codes](#exit-codes)
- [Protected Devices](#protected-devices)
- [Upstream overrides](#upstream-overrides)
- [Web Panel](#web-panel)
- [Docker](#docker)
- [Security](#security)
- [License](#license)

<a id='terms'></a>
## Terms

[`CONTEXT.md`](../../CONTEXT.md) is the source of this table.

| Term | English meaning | 中文 | 日本語 |
|---|---|---|---|
| Backend | An independent channel the tool uses to reach Devices. There are two, Google Home and Tuya, and neither depends on the other. | 后端：工具用来访问设备的独立通道，有 Google Home 和 Tuya 两种，彼此不依赖。 | バックエンド：ツールがデバイスに到達するための独立した経路。Google Home と Tuya の 2 つがあり、互いに依存しない。 |
| Device | Something that can be controlled or read, as seen through one Backend. One physical plug may appear as a Device in both Backends. | 设备：通过某个后端能够控制或读取的对象。同一个物理插座可能在两个后端中各显示为一个设备。 | デバイス：あるバックエンドから見て制御または読み取りできる対象。1 つの物理プラグが両方のバックエンドにデバイスとして現れることがある。 |
| Protected Device | A Device the tool refuses to write to through any Backend until the user removes the protection with `home protect --remove`. Reading its state and Telemetry still works. Protection stays active across writes; two explicit one-time overrides let exactly one write through without changing the list: `--force` on the CLI, and re-entering the panel password in the Web Panel for that one request. Both exist for the human at the keyboard, and the same failed-password backoff covers the panel override as the login. Unlike a Sensitive Action, which is about the action, protection is about the Device, and a Protected Device still needs `--confirm` for a Sensitive Action. | 受保护设备：在所有后端中都拒绝写入的设备，只有用户执行 `home protect --remove` 才能解除。读取状态和遥测仍然可用。保护在多次写入之间持续有效；两种明确的一次性覆盖各自允许恰好一次写入且不改变列表：CLI 上的 `--force`，以及在 Web 面板中针对该次请求重新输入面板密码。两者都供人在键盘前使用；面板覆盖与登录共享同一套失败密码退避。敏感操作针对动作，保护针对设备，受保护设备上的敏感操作仍然需要 `--confirm`。 | 保護デバイス：ユーザーが `home protect --remove` で解除するまで、どのバックエンドからも書き込みを拒否するデバイス。状態とテレメトリーの読み取りは引き続き可能。保護は書き込みをまたいで有効なまま。明示的な 1 回限りの上書きが 2 つあり、リストを変えずに 1 回だけ書き込みを通す：CLI の `--force` と、Web パネルでそのリクエストのためにパネルパスワードを再入力する方法。どちらもキーボードの前にいる人間のためのもので、パネルの上書きにはログインと同じ失敗パスワードのバックオフが適用される。機微な操作は動作について、保護はデバイスについての区別であり、保護デバイスでの機微な操作には引き続き `--confirm` が必要。 |
| Sensitive Action | An action that weakens physical security, such as unlocking a door or disarming an alarm. The tool refuses it unless the caller confirms explicitly. | 敏感操作：开锁或撤防等削弱物理安全的动作。除非调用方明确确认，否则工具拒绝执行。 | 機微な操作：ドアの解錠やアラームの解除など、物理的な安全性を弱める操作。呼び出し元が明示的に確認しない限り、ツールは拒否する。 |
| Telemetry | Measured readings from a Device, such as power, current, voltage and accumulated energy. Only the Tuya Backend provides it. | 遥测：设备产生的测量读数，如功率、电流、电压和累计电量。只有 Tuya 后端提供。 | テレメトリー：電力、電流、電圧、累積エネルギーなど、デバイスからの測定値。提供するのは Tuya バックエンドのみ。 |
| Local Key | The per-device secret that allows Local Control of a Tuya Device. Once the tool holds it, the Device can be operated without the Tuya cloud, until the Device is reset or re-paired. | 本地密钥：允许对 Tuya 设备进行本地控制的每设备密钥。工具持有后，设备在重置或重新配网之前无需 Tuya 云即可操作。 | ローカルキー：Tuya デバイスのローカル制御を可能にするデバイスごとの秘密。ツールが保持すれば、デバイスがリセットまたは再ペアリングされるまで Tuya クラウドなしで操作できる。 |
| Local Control | Talking to a Device directly over the home network, with no vendor cloud involved. Deferred; the Tuya Backend currently uses Cloud Control. | 本地控制：在家庭网络中直接与设备通信，不经过厂商云。暂缓实现；Tuya 后端目前使用云端控制。 | ローカル制御：ベンダーのクラウドを介さず、家庭内ネットワークでデバイスと直接通信すること。延期されており、Tuya バックエンドは現在クラウド制御を使う。 |
| Cloud Control | Operating a Tuya Device through the Smart Life cloud, signed in as the user's Smart Life account. Works from any network. | 云端控制：以用户的 Smart Life 账号登录，通过 Smart Life 云操作 Tuya 设备。可在任意网络中使用。 | クラウド制御：ユーザーの Smart Life アカウントでサインインし、Smart Life クラウド経由で Tuya デバイスを操作すること。どのネットワークからでも動作する。 |
| Vendor Exit | Keeping Devices usable if a vendor's cloud disappears. For Tuya, the first step is holding every Local Key. The last resort is reflashing a Device with third-party firmware, which turns it into a Device of a different Backend. | 厂商退出：在厂商云消失时让设备仍可使用。对 Tuya 来说，第一步是持有全部本地密钥。最后手段是刷入第三方固件，使设备变成另一个后端的设备。 | ベンダー出口：ベンダーのクラウドがなくなってもデバイスを使い続けられるようにすること。Tuya では、まずすべてのローカルキーを保持する。最後の手段はサードパーティ製ファームウェアを書き込み、別のバックエンドのデバイスにすること。 |
| Linked Service | A vendor cloud account connected into Google Home, for example Smart Life for Tuya devices. The Google Home Backend never talks to a Linked Service directly. | 关联服务：接入 Google Home 的厂商云账号，例如用于 Tuya 设备的 Smart Life。Google Home 后端从不直接与关联服务通信。 | リンク済みサービス：Google Home に接続されたベンダーのクラウドアカウント。Tuya デバイスに対する Smart Life など。Google Home バックエンドがリンク済みサービスと直接通信することはない。 |
| Web Panel | A browser UI, served from the Docker deployment, for humans to control Devices. | Web 面板：由 Docker 部署提供的浏览器界面，供人控制设备。 | Web パネル：Docker デプロイから提供されるブラウザ UI。人間がデバイスを操作するためのもの。 |
| CLI | The command-line interface, designed to be called by agents. Its output is machine-readable. | 命令行界面：为代理调用而设计，输出机器可读。 | CLI：エージェントから呼び出されることを想定したコマンドラインインターフェース。出力は機械可読。 |
| Ambiguous name | A Device name that matches more than one Device. The tool refuses it with exit 3 and lists the candidates instead of guessing. | 歧义名称：匹配到多个设备的设备名称。工具以退出码 3 拒绝并列出候选，而不是猜测。 | 曖昧な名前：複数のデバイスに一致するデバイス名。ツールは推測せず、終了コード 3 で拒否して候補を列挙する。 |

<a id='repository-layout'></a>
## Repository layout

- `crates/home-store` is the SQLite storage layer, the JSON and exit-code
  contract, and `guard`, the one write gate for Protected Devices.
- `crates/ghome-backend` is the Google Home Backend, forked from
  `piekstra/google-home-cli` v0.10.0 (MIT; the upstream text stays at
  `crates/ghome-backend/LICENSE-ghome`). It uses the unofficial foyer API;
  see `docs/adr/0001-google-home-via-unofficial-foyer-api.md`.
- `crates/tuya-backend` is the Tuya Backend, ported from `lueur`'s MIT
  `tuya_cloud.rs` (the upstream text stays at
  `crates/tuya-backend/LICENSE-tuya`).
- `src/main.rs` is the `home` binary: the CLI and `serve`.
- `src/web/` is the panel: zero-build single-file `index.html` and
  `login.html`.
- `tests/` holds the black-box CLI and panel tests (`cli.rs`, `gh_03.rs`,
  `tuya_04.rs`, `tuya_05.rs`, `serve_06.rs`, `serve_07.rs`, plus the i18n
  tests).
- `docs/adr/` holds the architecture decisions; `docs/manual/` holds this
  manual; `scripts/image-smoke.sh` is the container smoke check.
- The two Backends never depend on each other. The Google Home Backend
  exposes only `list_devices`, `get_state` and `set_trait` to the rest of the
  workspace, the seam for a future Home MCP implementation.

<a id='build-and-test'></a>
## Build and test

Developed and tested in WSL with no Windows toolchain: Arch Linux, `cargo`
from the distro. `Cargo.lock` is committed, so dependency versions are fixed;
that does not make the crates themselves available.

The network here may be available: `curl` probes of
`https://index.crates.io/config.json` and
`https://static.crates.io/crates/serde/serde-1.0.0.crate` have returned 2xx,
so an online build can work. The established way to build in this environment
is still the cached one below, and this manual does not pretend otherwise:

- with the default `CARGO_HOME` (`~/.cargo`), `cargo build --offline` stops at
  `no matching package named 'rusqlite' found` when the crates are absent; a
  clean `cargo build` without the cache can therefore fail.
- the working setup points `CARGO_HOME` at the Windows host's cargo cache
  (read and written from WSL through `/mnt/c`) and builds `--offline`:

```console
export CARGO_HOME=/mnt/c/Users/<you>/.cargo
export CARGO_TARGET_DIR=/home/<you>/work/target   # inside WSL, not on /mnt
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Point `CARGO_TARGET_DIR` at a path inside WSL: building onto a Windows mount
is slow, and cases like `target/` there are easy to commit by accident. The
same two exports plus `--release` build the binary. To drop the host cache,
vendor the dependencies (`cargo vendor`) or pre-fill WSL's own registry cache
while the machine still has a network; until then, those two exports are a
prerequisite, not a convenience. Vendor the dependencies when the network
cannot be trusted; the offline build then needs nothing else.

The release build is a single statically linked Linux binary (musl), so it
does not depend on the host's glibc version. `scripts/image-smoke.sh <tag>` is
the container smoke check; it runs in throwaway containers and never contacts
a real Device.

<a id='quick-start'></a>
## Quick start

The 0.3.0 release image is `ghcr.io/wray-lee/smarthome:0.3.0`. Pull it once;
Compose runs that fixed tag:

```console
docker pull ghcr.io/wray-lee/smarthome:0.3.0
```

Write this `compose.yaml` next to where you run Compose:

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

`name: home-data` keeps the volume name fixed; without it Compose prefixes
the project name, and the `docker volume create home-data` steps below would
prepare a volume the service never mounts.

`HOME_PASSWORD` is required: the `${HOME_PASSWORD:?set HOME_PASSWORD}` form
makes Compose stop with an error when it is unset. Provide it in the
environment of the Compose command, or in a `.env` file next to
`compose.yaml` that you keep mode 600 and out of git:

```console
printf '%s\n' "HOME_PASSWORD='{panel_password}'" > .env
chmod 600 .env
docker compose up -d
```

Sign both Backends in once, inside the container:

```console
docker exec -it home home gh login --email you@example.com
docker exec -it home home tuya login
```

Open http://127.0.0.1:8080 and sign in with the same password. A new database
has nothing protected. Add every Device that must stay read-only before the
first write:

```console
docker exec -i home home protect tuya:{device_id}
docker exec -i home home protect gh:{device_id}
```

Only `home protect --remove` takes the protection away. The password file
variant, both non-interactive Google sign-in fallbacks and the health checks
are in [Docker](#docker).

<a id='deploy'></a>
## Deploy

Two ways to run it: the binary on a Linux host, or the container in
[Docker](#docker). Both serve the same panel and keep everything in one
SQLite file.

### Binary

The release build is a single statically linked Linux binary (musl), so it
does not depend on the host's glibc version. Copy it to any common Linux
distribution, or to Alpine, and run it as is. Give it a data directory and a
password file only its user can read:

```console
install -Dm755 home /usr/local/bin/home
sudo install -d -m700 -o "$USER" /var/lib/home
install -m600 /dev/stdin /var/lib/home/password <<< '{panel_password}'
```

Sign both Backends in once, in this order (the password file is read per
process, so the CLI and `serve` share the same database):

```console
home --db /var/lib/home/home.sqlite tuya login      # QR code, then scan it
echo '{gh_token}' | home --db /var/lib/home/home.sqlite gh login --email you@example.com
home --db /var/lib/home/home.sqlite protect tuya:{device_id}   # every Device that must stay read-only
```

The master token starts with `aas_et/`. Then run the panel under your init
system, with the password from the file so it never shows in `ps`:

```console
HOME_DB=/var/lib/home/home.sqlite \
HOME_HOST=127.0.0.1 HOME_PORT=8080 \
HOME_PASSWORD_FILE=/var/lib/home/password \
home serve
```

Keep `HOME_HOST` on the loopback and reach the panel through Tailscale (or
another private network). It is plain HTTP with a password: do not expose it
to the internet, and if you do put HTTPS in front of it, add
`HOME_COOKIE_SECURE=true` so the session cookie carries `Secure`.

Check it answers before you walk away:

```console
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/login   # 200
```

The whole `/api/` tree, `/api/health` included, is behind the password, so a
401 there means "still signed in nowhere", not "down": `GET /login` is the
unauthenticated liveness check.

### Upgrade

Stop the old process, replace the binary, start it again. The database is
migrated forward on open (`PRAGMA user_version`, currently 2), and a newer
database is refused by an older binary rather than half-read, so back it up
before the first start of a new version:

```console
sqlite3 /var/lib/home/home.sqlite ".backup '/var/backups/home-$(date +%F).sqlite'"
```

`home tuya keys > tuya-keys.json` keeps the Local Keys outside the database
as the Vendor Exit backup; the Google master token and Tuya sessions live
only in the database.

<a id='google-home-sign-in'></a>
## Google Home sign-in

The account email comes from `--email` or `HOME_GH_EMAIL`. The credential is
the Google master token (equivalent to the account password) or a one-time
browser `oauth_token`. It comes from `--token-file <PATH>`, `HOME_GH_TOKEN`,
or one line on stdin, in that order; the token itself never goes in argv:

```console
home gh login --email you@example.com           # interactive: paste, press Enter
HOME_GH_TOKEN='{gh_token}' home gh login --email you@example.com
home gh login --email you@example.com --token-file /run/secrets/gh-token
home gh devices
```

With a terminal on stdin, `home gh login` prints the steps to stderr before
reading and stores the pasted line as soon as you press Enter. Without one it
reads a single line from stdin (a pipe works with or without a trailing
newline), so `docker exec -i` no longer needs EOF.

To get an `oauth_token`, open https://accounts.google.com/EmbeddedSetup in a
desktop browser and sign in with the Google account that owns the home. After
the "I agree" page, open DevTools > Application/Storage > Cookies for
accounts.google.com and copy the `oauth_token` cookie (it starts with
`oauth2_4/`). It is single-use and expires in minutes, so paste it right away.
An existing master token starts with `aas_et/` and can be pasted the same way.

Both commands print one JSON document. `home gh login` stores the master
token and the fixed Android id in SQLite.

`home gh login` does not verify the master token: anything that starts with
`aas_et/` (or `oauth2_4/`, which is exchanged first) is stored, and a bad
token surfaces at the first `home gh devices` as `reauth_required` (exit 5).
An `oauth_token` that Google rejects gives `reauth_required` too, with a hint
to fetch a fresh one: it is single-use and expires within minutes. Login never
calls foyer, so it contacts Google only for the `oauth_token` exchange.

The Android id is bound to the master token (every Bearer is minted for it),
so a repeated login reuses the stored one. `--android-id <ID>` replaces it
and drops the cached Bearer. `--help` and `--version` print their text on
stderr and still write one JSON document to stdout.

<a id='google-home-devices'></a>
## Google Home Devices

A Device can be named by exact ID, by `room/name`, or by bare name; matching
is case-insensitive, and a name that matches more than one Device fails with
exit 3 and the candidates instead of guessing.

```console
home gh get study/reading-lamp
home gh set study/reading-lamp onOff false
home gh set study/reading-lamp brightness 75
home gh set study/reading-lamp color.colorTemperature 3000
home gh set study/reading-lamp thermostat 21 --mode heat
home gh set study/reading-lamp volume.currentVolume 30
```

`<trait>` accepts `onOff`, `brightness`, `color.colorTemperature`,
`volume.currentVolume`, `lockUnlock.isLocked`, `armDisarm.isArmed` and
`thermostat`. A thermostat needs both a mode and a setpoint (`<value>` may be
either, the flags supply the other). `home gh get` prints the current traits
under `traits`; `home gh set` prints the resulting state plus `changed`.

Unlocking a lock or disarming an alarm is a Sensitive Action: without
`--confirm` it is refused with exit 6 and no trait call is sent upstream. A
Protected Device is refused with exit 6 even with `--confirm` (only `--force`
lets one write through; see [Protected Devices](#protected-devices)); reading
it still works. A `set` that passes those refusals reads the Device once more
(one `GetTraits`) to check it is online, and only then writes.

<a id='tuya-sign-in'></a>
## Tuya sign-in

`home tuya login` renders a QR code in the terminal (stderr) and polls until
the Smart Life app scans it; the tokens and the regional endpoint are then
stored in SQLite. The user code it asks for is in the app under Me →
Settings → Account and Security → User Code, and can come from
`HOME_TUYA_USER_CODE` instead of `--user-code` (it is not a secret, but argv
is visible to other processes, so the env var is preferred):

```console
home tuya login                    # or: HOME_TUYA_USER_CODE={user_code} home tuya login
home tuya devices
home tuya keys > ~/tuya-keys.json  # back this up, off this machine
```

`home tuya devices` prints ID, name, online state, category and Backend, and
saves every Local Key it sees. A successful sign-in already fetches one
device list and saves every Local Key it returns, so `home tuya login` alone
backs them up; `home tuya devices` refreshes them on every run.
`home tuya keys` exports them as JSON (`device_id`, `local_key`,
`first_seen_at`, `changed_at`) for the Vendor Exit. Treat that file as a
secret: it belongs in your backups, never in git. A changed key keeps the old
one in history with the time it was replaced.

The QR sign-in needs no Tuya IoT developer platform. It was validated on a
non-China account; on a China-region account the hard-coded login host has
reportedly failed.

<a id='telemetry'></a>
## Telemetry

One Device is named by exact ID or by bare name, case-insensitive (a Tuya
Device row carries no `room`, so `room/name` does not apply here; an exact ID
wins over a name that spells it, and a repeated name is exit 3 with the
candidates). Tuya Device rows have no `room` field.

```console
home tuya on {device}                       # switch_1 = true
home tuya off {device}                      # switch_1 = false
home tuya get {device}                      # data points + Telemetry + calibration
home tuya set {device} {code} {value}       # one raw data point, e.g. child_lock true
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
command is sent unless `--force` is given for that one call; `get` still
works. An offline Device fails every command with exit 4. If the session is
dead (`sign invalid` or `1010`), exit 5 names the Tuya Backend. A transient
`1108` is retried up to twice inside the one serialised client; anything else
is exit 7.

<a id='database'></a>
## Database

One file, path from `--db` or `HOME_DB`, default
`$XDG_DATA_HOME/home/home.sqlite`. It is the key to the whole home and the
Google account: losing it leaks the Google master token (equivalent to the
account password) and the Tuya sessions, and restoring it onto a new machine
restores both. Created owner-only (mode 600), versioned migrations, no
application-level encryption. Keep it out of the repo and back it up
deliberately.

Reach the panel from outside your house through Tailscale. The panel has no
other remote access path and must never be exposed to the public internet.

<a id='exit-codes'></a>
## Exit codes

Every command writes one JSON document to stdout; the exit code carries the
outcome.

| Exit code | Meaning |
|---|---|
| 0 | ok |
| 2 | usage |
| 3 | ambiguous name |
| 4 | Device offline |
| 5 | `reauth_required` |
| 6 | Sensitive Action or protected Device refused |
| 7 | upstream error |

The Web Panel maps these to HTTP statuses; see the [HTTP status
table](#web-panel).

<a id='protected-devices'></a>
## Protected Devices

`home protect google_home:{device_id}` (or `gh:{device_id}`,
`tuya:{device_id}`) adds a Device to the protected list; `home protect
--list` shows it; `home protect --remove` is the only way out. Every write
path goes through `home_store::guard`, which refuses a protected Device with
exit 6 before any trait RPC (the Google Home name lookup has already run).
Nothing is protected by default.

```console
home protect google_home:{device_id}
home protect gh:{device_id}
home protect tuya:{device_id}
home protect --list
home protect --remove tuya:{device_id}
```

Two explicit one-time overrides let a single write through without changing
the list; the Device stays protected:

- `--force` on `home gh set` and on `home tuya on` / `off` / `set`. It is
  separate from `--confirm`: `--force` accepts a write to a Protected Device,
  `--confirm` accepts a Sensitive Action, and a protected lock needs both.
- Re-entering the panel password in the Web Panel for that one request.

Both are for the human at the keyboard. Protection ends only with
`home protect --remove`; see `docs/adr/0002`.

<a id='upstream-overrides'></a>
## Upstream overrides

For tests: `HOME_FOYER_URL` (foyer RPC prefix), `HOME_GPSOAUTH_URL` (Android
account-token endpoint), `HOME_TUYA_LOGIN_URL` (QR login base),
`HOME_TUYA_ENDPOINT` (regional API base; a loopback host is accepted for
tests), `HOME_TUYA_USER_CODE`, `HOME_TUYA_POLL_MS` and
`HOME_TUYA_POLL_ATTEMPTS` (QR poll interval and count),
`HOME_TUYA_REFRESH_POLL_MS` (the `home serve` background Tuya refresh tick,
default 10000 ms, minimum 20).

The other variables are documented where they take effect: `HOME_GH_TOKEN` in
[Google Home sign-in](#google-home-sign-in), `HOME_DB` in
[Database](#database), and `HOME_HOST`, `HOME_PORT`, `HOME_PASSWORD`,
`HOME_PASSWORD_FILE` and `HOME_COOKIE_SECURE` in [Web Panel](#web-panel).

<a id='web-panel'></a>
## Web Panel

`home serve` binds `HOME_HOST`/`HOME_PORT` (or `--host`/`--port`) and refuses
to start without a password (`HOME_PASSWORD`, `--password`, or
`HOME_PASSWORD_FILE`/`--password-file`, preferred for Docker because argv is
visible to other processes; the file's trailing newline is trimmed). It
serves the panel at `/` and `/settings` (the same page, one view or the
other), the login form at `/login` (POST sets an `HttpOnly; SameSite=Strict`
`home_session` cookie; add `--secure-cookie` or set `HOME_COOKIE_SECURE` when
the panel sits behind an HTTPS reverse proxy so the cookie carries `Secure`),
and a JSON API under `/api/`. Unauthenticated API requests return 401 JSON
(`unauthorized`); a wrong password POST returns 401. `GET /api/devices`
returns both Backends side by side, so one dead Backend does not hide the
other; `GET /api/health` and `GET /api/status` report panel health. A
background ticker refreshes the Tuya token before expiry and persists the
rotated refresh token first, so the session stays alive without waiting for a
request.

Signed in, the panel is one page with two views. The header carries Devices
and Settings links plus Refresh and Sign out. The links are real routes, so a
new tab or a direct visit to `/settings` works; inside the page they switch
without a reload, and Back and Forward move between the two views. Settings
sets the address to `/settings` and the page title follows the view. Signed
out, `GET /settings` answers with the login form exactly like `GET /`, and
signing out returns there.

The Devices view shows both Backends at once, filtered by three tabs: All,
Google Home and Tuya. Each tab carries that Backend's Device count from the
latest answer, and a word when the Backend cannot be used right now: `sign
in needed`, `offline` or `error`. A Backend with no stored credential
explains the next step inside its own tab: Tuya offers the QR sign-in button,
Google Home shows the `home gh login --email you@example.com` command with a
Refresh button. A poll or a Refresh never moves the selected tab. The tab
selected when the view opens is the one set in Settings, `All` until you
change it.

Settings holds three choices that stay in this browser, applied with one Save
button:

- Language: Auto, English, Chinese (Simplified) or Japanese. Auto takes the
  first `zh`, `ja` or `en` entry of the browser's language list and falls
  back to English. A chosen language also applies to the login form, and the
  page `lang` attribute follows it so screen readers and spell check agree.
- Default tab: which Devices tab is selected when the view opens. It does
  not change the tab you are looking at until the next visit.
- Appearance: Auto follows the system colour scheme; Light and Dark pin it.
  The login form follows the same choice.

The choices are kept in this browser's `localStorage`
(`home_panel_settings`), so two machines can show the same panel differently,
and an unsaved change is dropped when you leave the view. Nothing about
these choices is stored on the server.

Each Device row reads its state and Telemetry on a 10 s poll. The poll pauses
while the browser tab is hidden, refreshes once as soon as the tab is visible
again, and never runs two reads at the same time. A read never blanks the
row: the last reading stays while the next one is on the way, and a failed
read that already had values keeps them and marks them `Updated HH:MM, may be
out of date` until a read succeeds.

Login failures share one failed-password throttle with the protected-device
override. From the third failure a single counter backs off exponentially
(`2 << min(failures - 3, 5)` seconds); a blocked request gets `429 Too Many
Requests` with `Retry-After` before its password is compared, so a right
password during the wait is refused too, and any right password resets the
counter. The panel still belongs on loopback behind Tailscale, never exposed
directly. Session ids are 32 bytes from the OS CSPRNG, and both the cookie
and the in-memory session expire after 8 hours; signing out deletes the id.
Every response carries `X-Content-Type-Options: nosniff`, `X-Frame-Options:
DENY`, `frame-ancestors 'none'`, `Referrer-Policy: no-referrer` and
`Cache-Control: no-store`. The password check compares in constant time and
folds the length in. The panel renders every server value as text; the one
`innerHTML` is the Tuya QR SVG, whose markup the server builds itself from the
QR encoder (coordinates only, no request text). There is no CSRF token;
`SameSite=Strict` is what keeps a foreign page from driving a POST, which is
why the cookie must never be relaxed.

The panel calls the same command layer as the CLI and keeps the CLI safety
rules. A protected Device shows a visible `protected` label and is read-only
by default; a write from the panel needs the panel password re-entered for
that one request, and the same password throttle applies. Unlocking a lock or
disarming an alarm asks for the same explicit confirmation as the CLI, and a
protected lock needs both.

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
| many failed passwords | 429 | `rate_limited`, with `Retry-After` |

Without a session, every `/api/` request returns 401 before any of the above
is reached.

<a id='docker'></a>
## Docker

One image holds the `home` binary. The runtime image is built on `alpine:3.21`
and carries the statically linked musl binary, so the image is a fraction of
the Debian/glibc build's size. The SQLite file lives on a mounted volume,
never baked into the image, the container runs as the non-root `home` user
(UID 10001), and the image ships `LICENSE` and `THIRD_PARTY_NOTICES.md` under
`/usr/share/licenses/home/`.

The 0.3.0 release image is `ghcr.io/wray-lee/smarthome:0.3.0`. Pull it once;
Compose runs that fixed tag:

```console
docker pull ghcr.io/wray-lee/smarthome:0.3.0
```

### Run with Compose

Write this `compose.yaml` next to where you run Compose:

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

`name: home-data` keeps the volume name fixed; without it Compose prefixes
the project name, and the `docker volume create home-data` steps below would
prepare a volume the service never mounts.

`HOME_PASSWORD` is required: the `${HOME_PASSWORD:?set HOME_PASSWORD}` form
makes Compose stop with an error when it is unset. Provide it in the
environment of the Compose command, or in a `.env` file next to
`compose.yaml` that you keep mode 600 and out of git:

```console
printf '%s\n' "HOME_PASSWORD='{panel_password}'" > .env
chmod 600 .env
docker compose up -d
```

The environment reaches the container, so `docker inspect home` shows the
password. The existing password file method still works when that matters:
write `home-password` into the `/data` volume owned by UID 10001 with mode
600 (see below), then replace the `HOME_PASSWORD` line in `compose.yaml`
with:

```yaml
      HOME_PASSWORD_FILE: /data/home-password
```

The service uses `container_name: home`, so the `docker exec` commands below
address `home`. The port is published only on `127.0.0.1:8080`; the container
itself binds `0.0.0.0`, so the host-side loopback binding is what keeps the
panel off the LAN, the same rule as the [Web Panel](#web-panel) section.

### First start and the password file

The Compose example above passes the password in the environment. To use the
file instead, create the named volume and write the password before the first
start:

```console
docker volume create home-data
printf '%s\n' '{panel_password}' | docker run --rm -i -v home-data:/data \
  --entrypoint sh ghcr.io/wray-lee/smarthome:0.3.0 \
  -c 'umask 077; cat > /data/home-password'
```

The container runs as UID 10001, so the file lands on disk owned by 10001
with mode 600, and the password never appears in argv or in the image. A
bind-mounted host file also works
(`--mount type=bind,src=/path/home-password,dst=/run/secrets/home-password,readonly`
plus `-e HOME_PASSWORD_FILE=/run/secrets/home-password`) as long as UID 10001
can read it.

Sign both Backends in once, inside the container. `docker exec -it` gives the
Google login a terminal, so it prints the steps and accepts the pasted token
followed by Enter; the two non-interactive fallbacks read the token from the
environment or from a file in the `/data` volume (the file is the safer one,
because `docker inspect` shows the environment):

```console
docker exec -it home home gh login --email you@example.com
docker exec -e HOME_GH_TOKEN='{oauth_token}' home home gh login --email you@example.com
printf '%s\n' '{gh_token}' | docker run --rm -i -v home-data:/data \
  --entrypoint sh ghcr.io/wray-lee/smarthome:0.3.0 \
  -c 'umask 077; cat > /data/gh-token'
docker exec home home gh login --email you@example.com --token-file /data/gh-token
docker exec -it -e HOME_TUYA_USER_CODE={user_code} home home tuya login
```

The Tuya QR code is printed on stderr; scan it with the Smart Life app, or
use the Tuya QR dialog in the panel instead (same login). The user code is
in the app under Me → Settings → Account and Security → User Code.

A new database has nothing protected. Every Device that must stay read-only
needs an explicit protection before the first write:

```console
docker exec -i home home protect tuya:{device_id}
docker exec -i home home protect gh:{device_id}
```

Add one line for every such Device, or copy an existing `home.sqlite` (where
the protection already lives) into the volume before starting the container.
A fresh volume holds an empty database with nothing protected.

The container publishes only to `127.0.0.1`. Reach it over Tailscale (or
another private network); never expose the port directly. If a reverse proxy
terminates TLS in front of it, set `HOME_COOKIE_SECURE=true` on the container
so the session cookie carries `Secure`.

Check it answers:

```console
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/login   # 200
```

The whole `/api/` tree needs a login, so an anonymous `GET /api/devices`
returning 401 is normal, not a failure.

The runtime image sets `HOME_DB=/data/home.sqlite`, `HOME_HOST=0.0.0.0` and
`HOME_PORT=8080`, so no other configuration is needed.

### Upgrade an existing deployment

`home` below is the old container's name. Inspect it first and reuse the
volume that already holds `home.sqlite`; do not let the new service start on
a fresh, empty one:

```console
docker inspect home --format '{{range .Mounts}}{{.Type}} {{if .Name}}{{.Name}}{{else}}{{.Source}}{{end}} -> {{.Destination}}{{"\n"}}{{end}}'
```

The line ending in `/data` names either a Docker volume (`volume`) or a host
path (`bind`). Back up the database while the container is stopped, then
remove the old container so the new one can take its name and the loopback
port:

```console
docker stop home
mkdir -p ./data-backup
docker cp home:/data/. ./data-backup/
docker rm home
```

When the volume is `home-data`, the Compose file above needs no change. When
inspect printed another volume name, put that name in `name:` instead, so
Compose reuses it rather than creating an empty `home-data`:

```yaml
volumes:
  home-data:
    name: the-name-from-inspect
```

When `/data` is a bind mount, use the host path in the service:

```yaml
    volumes:
      - /the/host/path/from/inspect:/data
```

Two processes must not write one SQLite file at the same time, and the port
can belong to only one of them, so stop the old container before
`docker compose up -d`. The database migrates forward when a new version
opens it; the backup is the way back if an older binary later refuses the
newer schema.

### Offline: load the shipped image

For a host without registry access, save the image for its architecture on a
machine that can pull, copy the tarball over, load it, and point the same
Compose file at the loaded tag, which Compose uses without pulling:

```console
docker pull --platform linux/arm64 ghcr.io/wray-lee/smarthome:0.3.0
docker tag ghcr.io/wray-lee/smarthome:0.3.0 home:0.3.0
docker save home:0.3.0 -o home-0.3.0-arm64.tar
# on the offline host:
docker load -i home-0.3.0-arm64.tar
docker image inspect home:0.3.0 --format '{{.Architecture}}'
```

```yaml
services:
  home:
    image: home:0.3.0
```

The inspect output must be `amd64` or `arm64` and match the host; a tar for
the wrong architecture fails at start with `exec format error`. If no tar
matches the host, run `docker build -t home:0.3.0 .` instead; it produces the
same tag for the native architecture.

<a id='security'></a>
## Security

- Keep the panel on loopback (`HOME_HOST=127.0.0.1`, or the Compose port
  binding `127.0.0.1:8080:8080`) and reach it over Tailscale or another
  private network. Never expose it to the public internet. It is plain HTTP
  with a password; if a reverse proxy terminates TLS in front of it, set
  `HOME_COOKIE_SECURE=true` (or `--secure-cookie`) so the session cookie
  carries `Secure`.
- The database is the master key: it holds the Google master token
  (equivalent to the account password) and the Tuya sessions, is created mode
  600, and must stay out of the repo and in your backups. `home tuya keys`
  exports the Local Keys, which are equally sensitive.
- Login failures and the protected-device override share one failed-password
  throttle: from the third failure the counter backs off exponentially
  (`2 << min(failures - 3, 5)` seconds), a blocked request gets `429 Too Many
  Requests` with `Retry-After` before the password is compared, and any right
  password resets the counter.
- Session ids are 32 bytes from the OS CSPRNG; the cookie and the in-memory
  session expire after 8 hours, and signing out deletes the id. Every
  response carries `X-Content-Type-Options: nosniff`, `X-Frame-Options:
  DENY`, `frame-ancestors 'none'`, `Referrer-Policy: no-referrer` and
  `Cache-Control: no-store`. The password check is constant time and folds
  the length in. The panel renders server values as text; the only
  `innerHTML` is the server-built Tuya QR SVG. There is no CSRF token;
  `SameSite=Strict` is what blocks a foreign page from driving a POST, so the
  cookie must never be relaxed.
- A Protected Device is read-only through every write path until
  `home protect --remove`. `--force` and the panel password re-entry are
  one-write overrides for the human at the keyboard. A Sensitive Action needs
  `--confirm`; a protected lock needs both. See
  [Protected Devices](#protected-devices).

<a id='license'></a>
## License

MIT; the copyright is 2026 wray-lee. The full text is in
[LICENSE](../../LICENSE).

`crates/ghome-backend` is a fork of `piekstra/google-home-cli` v0.10.0, and
`crates/tuya-backend` ports `lueur`'s `tuya_cloud.rs`; both upstream projects
are MIT. Their license texts are reproduced in full in
[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md), and the original
tracked copies stay at `crates/ghome-backend/LICENSE-ghome` and
`crates/tuya-backend/LICENSE-tuya`.

The container image ships `LICENSE` and `THIRD_PARTY_NOTICES.md` under
`/usr/share/licenses/home/`.