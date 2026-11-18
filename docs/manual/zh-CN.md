# home 手册

[English](en.md) | 简体中文 | [日本語](ja.md)

这是 `home` 的中文手册。`home` 是一个 Rust 二进制程序和一个网页面板，用于
管理 Google Home 与 Tuya 设备。每条命令都向 stdout 写出一份 JSON 文档，退出
码说明执行结果。README 是概览；本页记录每条命令、选项和规则。

<a id='contents'></a>
## 目录

- [目录](#contents)
- [术语](#terms)
- [仓库结构](#repository-layout)
- [构建与测试](#build-and-test)
- [快速开始](#quick-start)
- [部署](#deploy)
- [Google Home 登录](#google-home-sign-in)
- [Google Home 设备](#google-home-devices)
- [Tuya 登录](#tuya-sign-in)
- [遥测](#telemetry)
- [数据库](#database)
- [退出码](#exit-codes)
- [受保护设备](#protected-devices)
- [上游覆盖](#upstream-overrides)
- [网页面板](#web-panel)
- [Docker](#docker)
- [安全](#security)
- [许可证](#license)

<a id='terms'></a>
## 术语

[`CONTEXT.md`](../../CONTEXT.md) 是这张表的来源。

| 术语 | 中文含义 | 英文含义 | 日本語 |
|---|---|---|---|
| 后端 | 工具用来访问设备的独立通道，有 Google Home 和 Tuya 两种，彼此不依赖。 | An independent channel the tool uses to reach Devices. There are two, Google Home and Tuya, and neither depends on the other. | バックエンド：ツールがデバイスに到達するための独立した経路。Google Home と Tuya の 2 つがあり、互いに依存しない。 |
| 设备 | 通过某个后端能够控制或读取的对象。同一个物理插座可能在两个后端中各显示为一个设备。 | Something that can be controlled or read, as seen through one Backend. One physical plug may appear as a Device in both Backends. | デバイス：あるバックエンドから見て制御または読み取りできる対象。1 つの物理プラグが両方のバックエンドにデバイスとして現れることがある。 |
| 受保护设备 | 在所有后端中都拒绝写入的设备，只有用户执行 `home protect --remove` 才能解除。读取状态和遥测仍然可用。保护在多次写入之间持续有效；两种明确的一次性覆盖各自允许恰好一次写入且不改变列表：命令行上的 `--force`，以及在网页面板中针对该次请求重新输入面板密码。两者都供人在键盘前使用；面板覆盖与登录共享同一套失败密码退避。敏感操作针对动作，保护针对设备，受保护设备上的敏感操作仍然需要 `--confirm`。 | A Device the tool refuses to write to through any Backend until the user removes the protection with `home protect --remove`. Reading its state and Telemetry still works. Protection stays active across writes; two explicit one-time overrides let exactly one write through without changing the list: `--force` on the CLI, and re-entering the panel password in the Web Panel for that one request. Both exist for the human at the keyboard, and the same failed-password backoff covers the panel override as the login. Unlike a Sensitive Action, which is about the action, protection is about the Device, and a Protected Device still needs `--confirm` for a Sensitive Action. | 保護デバイス：ユーザーが `home protect --remove` で解除するまで、どのバックエンドからも書き込みを拒否するデバイス。状態とテレメトリーの読み取りは引き続き可能。保護は書き込みをまたいで有効なまま。明示的な 1 回限りの上書きが 2 つあり、リストを変えずに 1 回だけ書き込みを通す：CLI の `--force` と、Web パネルでそのリクエストのためにパネルパスワードを再入力する方法。どちらもキーボードの前にいる人間のためのもので、パネルの上書きにはログインと同じ失敗パスワードのバックオフが適用される。センシティブ操作は動作について、保護はデバイスについての区別であり、保護デバイスでのセンシティブ操作には引き続き `--confirm` が必要。 |
| 敏感操作 | 开锁或撤防等削弱物理安全的动作。除非调用方明确确认，否则工具拒绝执行。 | An action that weakens physical security, such as unlocking a door or disarming an alarm. The tool refuses it unless the caller confirms explicitly. | センシティブ操作：ドアの解錠やアラームの解除など、物理的な安全性を弱める操作。呼び出し元が明示的に確認しない限り、ツールは拒否する。 |
| 遥测 | 设备产生的测量读数，如功率、电流、电压和累计电量。只有 Tuya 后端提供。 | Measured readings from a Device, such as power, current, voltage and accumulated energy. Only the Tuya Backend provides it. | テレメトリー：電力、電流、電圧、累積エネルギーなど、デバイスからの測定値。提供するのは Tuya バックエンドのみ。 |
| 本地密钥 | 允许对 Tuya 设备进行本地控制的设备专属密钥。工具持有后，设备在重置或重新配网之前无需 Tuya 云即可操作。 | The per-device secret that allows Local Control of a Tuya Device. Once the tool holds it, the Device can be operated without the Tuya cloud, until the Device is reset or re-paired. | ローカルキー：Tuya デバイスのローカル制御を可能にするデバイスごとの秘密。ツールが保持すれば、デバイスがリセットまたは再ペアリングされるまで Tuya クラウドなしで操作できる。 |
| 本地控制 | 在家庭网络中直接与设备通信，不经过厂商云。暂缓实现；Tuya 后端目前使用云端控制。 | Talking to a Device directly over the home network, with no vendor cloud involved. Deferred; the Tuya Backend currently uses Cloud Control. | ローカル制御：ベンダーのクラウドを介さず、家庭内ネットワークでデバイスと直接通信すること。延期されており、Tuya バックエンドは現在クラウド制御を使う。 |
| 云端控制 | 以用户的 Smart Life 账号登录，通过 Smart Life 云操作 Tuya 设备。可在任意网络中使用。 | Operating a Tuya Device through the Smart Life cloud, signed in as the user's Smart Life account. Works from any network. | クラウド制御：ユーザーの Smart Life アカウントでサインインし、Smart Life クラウド経由で Tuya デバイスを操作すること。どのネットワークからでも動作する。 |
| 供应商退出 | 在厂商云消失时让设备仍可使用。对 Tuya 来说，第一步是持有全部本地密钥。最后手段是刷入第三方固件，使设备变成另一个后端的设备。 | Keeping Devices usable if a vendor's cloud disappears. For Tuya, the first step is holding every Local Key. The last resort is reflashing a Device with third-party firmware, which turns it into a Device of a different Backend. | ベンダー出口：ベンダーのクラウドがなくなってもデバイスを使い続けられるようにすること。Tuya では、まずすべてのローカルキーを保持する。最後の手段はサードパーティ製ファームウェアを書き込み、別のバックエンドのデバイスにすること。 |
| 关联服务 | 接入 Google Home 的厂商云账号，例如用于 Tuya 设备的 Smart Life。Google Home 后端从不直接与关联服务通信。 | A vendor cloud account connected into Google Home, for example Smart Life for Tuya devices. The Google Home Backend never talks to a Linked Service directly. | リンクされたサービス：Google Home に接続されたベンダーのクラウドアカウント。Tuya デバイスに対する Smart Life など。Google Home バックエンドがリンクされたサービスと直接通信することはない。 |
| 网页面板 | 由 Docker 部署提供的浏览器界面，供人控制设备。 | A browser UI, served from the Docker deployment, for humans to control Devices. | Web パネル：Docker デプロイから提供されるブラウザ UI。人間がデバイスを操作するためのもの。 |
| 命令行 | 为智能体调用而设计的命令行界面，输出机器可读。 | The command-line interface, designed to be called by agents. Its output is machine-readable. | CLI：エージェントから呼び出されることを想定したコマンドラインインターフェース。出力は機械可読。 |
| 名称歧义 | 匹配到多个设备的设备名称。工具以退出码 3 拒绝并列出候选，而不是猜测。 | A Device name that matches more than one Device. The tool refuses it with exit 3 and lists the candidates instead of guessing. | 曖昧な名前：複数のデバイスに一致するデバイス名。ツールは推測せず、終了コード 3 で拒否して候補を列挙する。 |

<a id='repository-layout'></a>
## 仓库结构

- `crates/home-store` 是 SQLite 存储层，负责 JSON 与退出码契约，以及
  `guard`，即受保护设备唯一的写入闸门。
- `crates/ghome-backend` 是 Google Home 后端，分叉自
  `piekstra/google-home-cli` v0.10.0（MIT；上游文本保留在
  `crates/ghome-backend/LICENSE-ghome`）。它使用非官方 foyer API；见
  `docs/adr/0001-google-home-via-unofficial-foyer-api.md`。
- `crates/tuya-backend` 是 Tuya 后端，移植自 `lueur` 的 MIT 许可
  `tuya_cloud.rs`（上游文本保留在 `crates/tuya-backend/LICENSE-tuya`）。
- `src/main.rs` 是 `home` 二进制：命令行与 `serve`。
- `src/web/` 是面板：零构建的单文件 `index.html` 和 `login.html`。
- `tests/` 存放黑盒命令行与面板测试（`cli.rs`、`gh_03.rs`、`tuya_04.rs`、
  `tuya_05.rs`、`serve_06.rs`、`serve_07.rs`，以及 i18n 测试）。
- `docs/adr/` 存放架构决策；`docs/manual/` 存放本手册；
  `scripts/image-smoke.sh` 是容器冒烟检查。
- 两个后端从不互相依赖。Google Home 后端只向工作区其余部分暴露
  `list_devices`、`get_state` 和 `set_trait`，这是将来实现 Home MCP 的
  接缝。

<a id='build-and-test'></a>
## 构建与测试

在 WSL 中开发和测试，不使用 Windows 工具链：Arch Linux，`cargo` 来自发行版。
`Cargo.lock` 已提交，因此依赖版本固定；但这并不代表 crate 本身已经可用。

这里的网络可能可用：对 `https://index.crates.io/config.json` 和
`https://static.crates.io/crates/serde/serde-1.0.0.crate` 的 `curl` 探测曾
返回 2xx，因此在线构建可以成功。但在当前环境中，既定的构建方式仍是下面基于
缓存的方案，本手册对此不作掩饰：

- 使用默认 `CARGO_HOME`（`~/.cargo`）时，若 crate 缺失，
  `cargo build --offline` 会停在 `no matching package named 'rusqlite' found`；
  因此不带缓存的干净 `cargo build` 可能失败。
- 可用的配置把 `CARGO_HOME` 指向 Windows 主机的 cargo 缓存（在 WSL 中通过
  `/mnt/c` 读写），并以 `--offline` 构建：

```console
export CARGO_HOME=/mnt/c/Users/<you>/.cargo
export CARGO_TARGET_DIR=/home/<you>/work/target   # inside WSL, not on /mnt
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

把 `CARGO_TARGET_DIR` 指向 WSL 内的路径：在 Windows 挂载上构建很慢，而且
那里的 `target/` 之类很容易被误提交。同样的两个 export 加上 `--release` 即可
构建二进制。要摆脱主机缓存，可以在机器仍有网络时 vendor 依赖
（`cargo vendor`），或预先填充 WSL 自己的 registry 缓存；在此之前，这两个
export 是前提条件，而不是便利选项。网络不可信时请 vendor 依赖；之后的离线
构建不再需要其他任何东西。

发布构建是单个静态链接的 Linux 二进制（musl），因此不依赖主机的 glibc 版本。
`scripts/image-smoke.sh <tag>` 是容器冒烟检查；它在一次性容器中运行，从不
接触真实设备。

<a id='quick-start'></a>
## 快速开始

0.3.0 发布镜像为 `ghcr.io/wray-lee/smarthome:0.3.0`。先拉取一次；Compose 会
使用这个固定标签：

```console
docker pull ghcr.io/wray-lee/smarthome:0.3.0
```

在你运行 Compose 的位置旁编写这个 `compose.yaml`：

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

`name: home-data` 让卷名保持固定；没有它，Compose 会加上项目名前缀，下面
`docker volume create home-data` 的步骤就会准备好一个服务永远不会挂载的卷。

`HOME_PASSWORD` 是必填项：`${HOME_PASSWORD:?set HOME_PASSWORD}` 形式使
Compose 在未设置时报错停止。可以在 Compose 命令的环境中提供它，或在
`compose.yaml` 旁的 `.env` 文件中提供，该文件保持 600 模式并置于 git 之外：

```console
printf '%s\n' "HOME_PASSWORD='{panel_password}'" > .env
chmod 600 .env
docker compose up -d
```

在容器内把两个后端各登录一次：

```console
docker exec -it home home gh login --email you@example.com
docker exec -it home home tuya login
```

打开 http://127.0.0.1:8080，用同一个密码登录。新数据库没有任何设备受保护。
在首次写入前，为每个必须保持只读的设备加入保护：

```console
docker exec -i home home protect tuya:{device_id}
docker exec -i home home protect gh:{device_id}
```

只有 `home protect --remove` 能解除保护。密码文件变体、两种非交互式 Google
登录回退方案和健康检查见 [Docker](#docker)。

<a id='deploy'></a>
## 部署

有两种运行方式：在 Linux 主机上直接运行二进制，或在 [Docker](#docker) 中
运行容器。两者提供同一个面板，所有数据都在一个 SQLite 文件里。

### 二进制

发布构建是单个静态链接的 Linux 二进制（musl），因此不依赖主机的 glibc 版本。
复制到任何常见 Linux 发行版或 Alpine，直接运行即可。给它一个数据目录和一个
只有其用户能读的密码文件：

```console
install -Dm755 home /usr/local/bin/home
sudo install -d -m700 -o "$USER" /var/lib/home
install -m600 /dev/stdin /var/lib/home/password <<< '{panel_password}'
```

按此顺序把两个后端各登录一次（密码文件按进程读取，因此命令行与 `serve` 共享
同一个数据库）：

```console
home --db /var/lib/home/home.sqlite tuya login      # QR code, then scan it
echo '{gh_token}' | home --db /var/lib/home/home.sqlite gh login --email you@example.com
home --db /var/lib/home/home.sqlite protect tuya:{device_id}   # every Device that must stay read-only
```

主令牌以 `aas_et/` 开头。然后在你的 init 系统下运行面板，密码从文件读取，
因此不会出现在 `ps` 中：

```console
HOME_DB=/var/lib/home/home.sqlite \
HOME_HOST=127.0.0.1 HOME_PORT=8080 \
HOME_PASSWORD_FILE=/var/lib/home/password \
home serve
```

把 `HOME_HOST` 保持在回环地址，通过 Tailscale（或其他私有网络）访问面板。
它是带密码的纯 HTTP：不要暴露到互联网；如果确实在它前面加了 HTTPS，请加上
`HOME_COOKIE_SECURE=true`，让会话 cookie 带上 `Secure`。

离开前确认它能应答：

```console
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/login   # 200
```

整个 `/api/` 树（包括 `/api/health`）都在密码之后，因此那里的 401 表示
“还没有在任何地方登录”，而不是“服务挂了”：`GET /login` 才是无需认证的
存活检查。

### 升级

停止旧进程，替换二进制，再重新启动。数据库在打开时向前迁移
（`PRAGMA user_version`，当前为 2）；较旧的二进制会拒绝更新版本的数据库，
而不是半读取它，因此请在新版本首次启动前备份：

```console
sqlite3 /var/lib/home/home.sqlite ".backup '/var/backups/home-$(date +%F).sqlite'"
```

`home tuya keys > tuya-keys.json` 把本地密钥保存在数据库之外，作为供应商退出
的备份；Google 主令牌和 Tuya 会话只存在于数据库中。

<a id='google-home-sign-in'></a>
## Google Home 登录

凭据是 Google 主令牌（等同于账号密码）或一次性的浏览器 `oauth_token`。它按
顺序来自 `--token-file <PATH>`、`HOME_GH_TOKEN` 或 stdin 的一行；令牌本身绝不
进入 argv：

```console
home gh login --email you@example.com           # interactive: paste, press Enter
HOME_GH_TOKEN='{gh_token}' home gh login --email you@example.com
home gh login --email you@example.com --token-file /run/secrets/gh-token
home gh devices
```

当 stdin 是终端时，`home gh login` 在读取前把步骤打印到 stderr，并在你按下
Enter 后立即保存粘贴的那一行。没有终端时，它从 stdin 读取一行（管道有没有
结尾换行都可以），因此 `docker exec -i` 不再需要 EOF。

要获取 `oauth_token`，在桌面浏览器中打开
https://accounts.google.com/EmbeddedSetup，用拥有该家庭的 Google 账号登录。
通过“I agree”页面后，打开 DevTools > Application/Storage > Cookies（针对
accounts.google.com），复制 `oauth_token` cookie（以 `oauth2_4/` 开头）。它是
一次性的，几分钟内就会过期，所以请立即粘贴。已有的主令牌以 `aas_et/` 开头，
也可以用同样的方式粘贴。

两条命令都输出一份 JSON 文档。`home gh login` 把主令牌和固定的 Android id
存入 SQLite。

`home gh login` 不验证主令牌：任何以 `aas_et/` 开头（或先经交换的
`oauth2_4/`）的内容都会被保存，坏令牌会在第一次 `home gh devices` 时以
`reauth_required`（退出码 5）暴露。Google 拒绝的 `oauth_token` 同样给出
`reauth_required`，并提示重新获取：它是一次性的，几分钟内过期。登录从不调用
foyer，因此只在交换 `oauth_token` 时联系 Google。

Android id 与主令牌绑定（每个 Bearer 都是为它生成的），因此重复登录会复用
已存储的那个。`--android-id <ID>` 会替换它并丢弃缓存的 Bearer。`--help` 和
`--version` 把文本打印到 stderr，仍向 stdout 写一份 JSON 文档。

<a id='google-home-devices'></a>
## Google Home 设备

设备可以用精确 ID、`room/name` 或不带房间的名称指定；匹配不区分大小写，匹配
到多个设备的名称会以退出码 3 失败并列出候选，而不是猜测。

```console
home gh get den/lamp
home gh set den/lamp onOff false
home gh set den/lamp brightness 75
home gh set den/lamp color.colorTemperature 3000
home gh set den/lamp thermostat 21 --mode heat
home gh set den/lamp volume.currentVolume 30
```

`<trait>` 接受 `onOff`、`brightness`、`color.colorTemperature`、
`volume.currentVolume`、`lockUnlock.isLocked`、`armDisarm.isArmed` 和
`thermostat`。恒温器同时需要模式和设定点（`<value>` 可以是其中之一，另一个
由标志提供）。`home gh get` 在 `traits` 下打印当前 trait；`home gh set` 打印
结果状态以及 `changed`。

开锁或撤防是敏感操作：没有 `--confirm` 会以退出码 6 拒绝，且不向上游发送任何
trait 调用。受保护设备即使带 `--confirm` 也会以退出码 6 拒绝（只有 `--force`
允许一次写入；见[受保护设备](#protected-devices)）；读取仍然可用。通过上述
拒绝的 `set` 会再读一次设备（一次 `GetTraits`）以确认在线，然后才写入。

<a id='tuya-sign-in'></a>
## Tuya 登录

`home tuya login` 在终端（stderr）渲染二维码并轮询，直到 Smart Life 应用完成
扫码；随后令牌和区域端点存入 SQLite。它要求的 user code 位于应用中的
Me → Settings → Account and Security → User Code，也可以由
`HOME_TUYA_USER_CODE` 提供而不是 `--user-code`（它不是机密，但 argv 对其他
进程可见，因此优先使用环境变量）：

```console
home tuya login                    # or: HOME_TUYA_USER_CODE={user_code} home tuya login
home tuya devices
home tuya keys > ~/tuya-keys.json  # back this up, off this machine
```

`home tuya devices` 打印 ID、名称、在线状态、类别和后端，并保存它看到的每个
本地密钥。成功登录本身就会拉取一次设备列表并保存返回的每个本地密钥，因此仅
`home tuya login` 就完成了备份；`home tuya devices` 每次运行都会刷新它们。
`home tuya keys` 把它们导出为 JSON（`device_id`、`local_key`、
`first_seen_at`、`changed_at`），用于供应商退出。把该文件当作机密：它应进入
你的备份，绝不进入 git。密钥变更时，旧密钥连同被替换的时间保留在历史中。

扫码登录不需要 Tuya IoT 开发者平台。它已在非中国区账号上验证；在中国区账号
上，硬编码的登录主机据报告会失败。

<a id='telemetry'></a>
## 遥测

设备用精确 ID 或不带房间的名称指定，不区分大小写（Tuya 设备行没有 `room`，
所以这里不适用 `room/name`；精确 ID 优先于拼写相同的名称，重复名称会以退出
码 3 列出候选）。Tuya 设备行没有 `room` 字段。

```console
home tuya on {device}                       # switch_1 = true
home tuya off {device}                      # switch_1 = false
home tuya get {device}                      # data points + Telemetry + calibration
home tuya set {device} {code} {value}       # one raw data point, e.g. child_lock true
```

`on`/`off` 发送 `switch_1`；返回的是带 code 和新值的 `changed`。`set` 在声明
的规格中找到 code，并把值转换为声明类型（`Boolean` 接受 `true`/`false`，
`Integer` 接受十进制整数，其他类型保持字符串），写入原始值，且不回读：
`get` 才是读取路径。未知 code 或无法转换的值会以退出码 2 失败，且不发送任何
内容。

`get` 在 `data_points` 下按顺序打印设备状态的每个数据点，每项含 `code`、
`value`，以及规格声明时的 `type`。`telemetry` 以
`{raw, scale, value, unit}` 的形式保存测量读数，其中 `scale` 和 `unit` 来自
设备声明的规格（绝不来自硬编码的 DP 编号），`value` 为 `raw / 10^scale`
（`scale` 为 0 时就是 raw 本身）。一个数据点属于遥测，当且仅当规格为它提供了
`unit`、它不是设备的功能项、也不是校准项：`voltage_coe`、`electric_coe`、
`power_coe` 和 `electricity_coe`。这四个在 `calibration` 下以原始值报告，
从不作为读数。

受保护设备会被 `on`/`off`/`set` 在发送任何命令前以退出码 6 拒绝，除非该次
调用给出 `--force`；`get` 仍然可用。离线设备的所有命令都以退出码 4 失败。
如果会话失效（`sign invalid` 或 `1010`），退出码 5 会指明 Tuya 后端。瞬时的
`1108` 会在单个串行化客户端内最多重试两次；其他情况为退出码 7。

<a id='database'></a>
## 数据库

一个文件，路径来自 `--db` 或 `HOME_DB`，默认为
`$XDG_DATA_HOME/home/home.sqlite`。它是整个家庭和 Google 账号的总钥匙：丢失它
会泄露 Google 主令牌（等同于账号密码）和 Tuya 会话；把它恢复到新机器则两者
都会恢复。创建时仅所有者可访问（模式 600），采用版本化迁移，无应用层加密。
让它留在仓库之外，并有意备份。

通过 Tailscale 从屋外访问面板。面板没有其他远程访问路径，绝不能暴露到公网。

<a id='exit-codes'></a>
## 退出码

每条命令都向 stdout 写出一份 JSON 文档；退出码承载结果。

| Exit code | Meaning |
|---|---|
| 0 | ok |
| 2 | usage |
| 3 | ambiguous name |
| 4 | Device offline |
| 5 | `reauth_required` |
| 6 | Sensitive Action or protected Device refused |
| 7 | upstream error |

网页面板把这些映射为 HTTP 状态；见 [HTTP 状态表](#web-panel)。

<a id='protected-devices'></a>
## 受保护设备

`home protect google_home:{device_id}`（或 `gh:{device_id}`、
`tuya:{device_id}`）把设备加入受保护列表；`home protect --list` 显示该列表；
`home protect --remove` 是唯一的解除方式。每条写入路径都经过
`home_store::guard`，它在任何 trait RPC 之前以退出码 6 拒绝受保护设备
（Google Home 的名称查找已经执行）。默认没有任何设备受保护。

```console
home protect google_home:{device_id}
home protect gh:{device_id}
home protect tuya:{device_id}
home protect --list
home protect --remove tuya:{device_id}
```

两种明确的一次性覆盖允许单次写入通过而不改变列表；设备保持受保护：

- `--force`，用于 `home gh set` 以及 `home tuya on` / `off` / `set`。它与
  `--confirm` 是分开的：`--force` 接受对受保护设备的写入，`--confirm` 接受
  敏感操作，受保护的锁两者都需要。
- 在网页面板中为那一次请求重新输入面板密码。

两者都供键盘前的人使用。保护只以 `home protect --remove` 结束；见
`docs/adr/0002`。

<a id='upstream-overrides'></a>
## 上游覆盖

用于测试：`HOME_FOYER_URL`（foyer RPC 前缀）、`HOME_GPSOAUTH_URL`（Android
账号令牌端点）、`HOME_TUYA_LOGIN_URL`（扫码登录基地址）、
`HOME_TUYA_ENDPOINT`（区域 API 基地址；测试接受回环主机）、
`HOME_TUYA_USER_CODE`、`HOME_TUYA_POLL_MS` 和 `HOME_TUYA_POLL_ATTEMPTS`
（扫码轮询间隔与次数）、`HOME_TUYA_REFRESH_POLL_MS`（`home serve` 后台 Tuya
刷新的周期，默认 10000 ms，最小 20）。

其他变量在生效处记录：`HOME_GH_TOKEN` 见
[Google Home 登录](#google-home-sign-in)，`HOME_DB` 见[数据库](#database)，
`HOME_HOST`、`HOME_PORT`、`HOME_PASSWORD`、`HOME_PASSWORD_FILE` 和
`HOME_COOKIE_SECURE` 见[网页面板](#web-panel)。

<a id='web-panel'></a>
## 网页面板

`home serve` 绑定 `HOME_HOST`/`HOME_PORT`（或 `--host`/`--port`），没有密码时
拒绝启动（`HOME_PASSWORD`、`--password`，或
`HOME_PASSWORD_FILE`/`--password-file`，在 Docker 中优先使用后者，因为 argv
对其他进程可见；文件末尾的换行会被去掉）。它在 `/` 和 `/settings` 提供面板
（同一页面，两个视图之一），在 `/login` 提供登录表单（POST 设置
`HttpOnly; SameSite=Strict` 的 `home_session` cookie；当面板位于 HTTPS 反向
代理之后时，加上 `--secure-cookie` 或设置 `HOME_COOKIE_SECURE`，让 cookie
带上 `Secure`），并在 `/api/` 下提供 JSON API。未认证的 API 请求返回 401
JSON（`unauthorized`）；密码错误的 POST 返回 401。`GET /api/devices` 并排
返回两个后端，因此一个后端失效不会遮住另一个；`GET /api/health` 和
`GET /api/status` 报告面板健康状况。后台定时器在到期前刷新 Tuya 令牌，并先
持久化轮换后的刷新令牌，因此会话无需等待请求即可保持存活。

登录后，面板是一个包含两个视图的页面。页头带有设备与设置链接，以及刷新和
退出。链接是真实路由，因此新标签页或直接访问 `/settings` 都可用；在页面内部
它们切换而不重载，后退和前进也在两个视图间移动。设置视图会把地址改为
`/settings`，页面标题跟随视图。退出登录后，`GET /settings` 与 `GET /` 一样
返回登录表单，退出后也会回到那里。

设备视图同时显示两个后端，由三个标签页过滤：全部、Google Home 和 Tuya。
每个标签页显示该后端最近一次应答中的设备数量，并在后端当前不可用时显示一个
状态词：`sign in needed`、`offline` 或 `error`。没有已存凭据的后端会在自己的
标签页内说明下一步：Tuya 提供扫码登录按钮，Google Home 显示
`home gh login --email you@example.com` 命令和刷新按钮。轮询或刷新从不改变
已选标签页。视图打开时选中的标签页是设置中选定的那个，在你更改之前为
`All`。

设置包含三个保存在本浏览器中的选择，由一个保存按钮应用：

- 语言：自动、英语、简体中文或日语。自动取浏览器语言列表中第一个 `zh`、
  `ja` 或 `en` 条目，找不到则回退到英语。选定的语言也应用于登录表单，页面
  `lang` 属性随之变化，使屏幕阅读器和拼写检查保持一致。
- 默认标签页：视图打开时选中哪个设备标签页。它在下次访问前不会改变你正在
  看的标签页。
- 外观：自动跟随系统配色方案；浅色和深色固定它。登录表单跟随同一选择。

这些选择保存在本浏览器的 `localStorage`（`home_panel_settings`）中，因此
两台机器可以让同一面板呈现不同外观；离开视图时未保存的更改会被丢弃。这些
选择不会在服务器上存储任何内容。

每个设备行以 10 秒轮询读取状态和遥测。浏览器标签页隐藏时轮询暂停，标签页
再次可见时立即刷新一次，且绝不并发执行两次读取。读取从不把行清空：上一次
读数在下次读数到达前保持显示；已有值的失败读取会保留这些值，并标记为
`Updated HH:MM, may be out of date`，直到读取成功。

登录失败与受保护设备覆盖共享同一个失败密码节流。从第三次失败起，单个计数器
按指数退避（`2 << min(failures - 3, 5)` 秒）；被阻止的请求在比较密码前得到
`429 Too Many Requests` 和 `Retry-After`，因此等待期间输入正确密码也会被拒，
而任何正确密码都会重置计数器。面板仍应放在 Tailscale 之后的回环地址上，
绝不直接暴露。会话 id 是来自 OS CSPRNG 的 32 字节，cookie 和内存中的会话都
在 8 小时后过期；退出登录会删除 id。每个响应都带有
`X-Content-Type-Options: nosniff`、`X-Frame-Options: DENY`、
`frame-ancestors 'none'`、`Referrer-Policy: no-referrer` 和
`Cache-Control: no-store`。密码比较使用常数时间，并折入长度。面板把每个
服务器值渲染为文本；唯一的 `innerHTML` 是 Tuya 二维码 SVG，其标记由服务器
根据二维码编码器自行构建（只有坐标，没有请求文本）。没有 CSRF 令牌；
`SameSite=Strict` 才是阻止外部页面驱动 POST 的机制，因此 cookie 绝不能
放宽。

面板调用与命令行相同的命令层，并保留命令行的安全规则。受保护设备显示可见的
`protected` 标签，默认只读；从面板写入需要为该次请求重新输入面板密码，并
应用同样的密码节流。开锁或撤防要求与命令行相同的明确确认，受保护的锁两者
都需要。

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

没有会话时，每个 `/api/` 请求都会在上述任何结果之前返回 401。

<a id='docker'></a>
## Docker

一个镜像包含 `home` 二进制。运行时镜像基于 `alpine:3.21` 构建，携带静态链接
的 musl 二进制，因此镜像大小只是 Debian/glibc 构建的一小部分。SQLite 文件
位于挂载卷上，绝不烘焙进镜像；容器以非 root 的 `home` 用户（UID 10001）
运行；镜像在 `/usr/share/licenses/home/` 下附带 `LICENSE` 和
`THIRD_PARTY_NOTICES.md`。

0.3.0 发布镜像为 `ghcr.io/wray-lee/smarthome:0.3.0`。先拉取一次；Compose 会
使用这个固定标签：

```console
docker pull ghcr.io/wray-lee/smarthome:0.3.0
```

### 使用 Compose 运行

在你运行 Compose 的位置旁编写这个 `compose.yaml`：

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

`name: home-data` 让卷名保持固定；没有它，Compose 会加上项目名前缀，下面
`docker volume create home-data` 的步骤就会准备好一个服务永远不会挂载的卷。

`HOME_PASSWORD` 是必填项：`${HOME_PASSWORD:?set HOME_PASSWORD}` 形式使
Compose 在未设置时报错停止。可以在 Compose 命令的环境中提供它，或在
`compose.yaml` 旁的 `.env` 文件中提供，该文件保持 600 模式并置于 git 之外：

```console
printf '%s\n' "HOME_PASSWORD='{panel_password}'" > .env
chmod 600 .env
docker compose up -d
```

环境变量会进入容器，因此 `docker inspect home` 会显示密码。当这一点重要时，
原有的密码文件方式仍然可用：把 `home-password` 写入 `/data` 卷，属主为
UID 10001，模式 600（见下），然后把 `compose.yaml` 中的 `HOME_PASSWORD` 行
替换为：

```yaml
      HOME_PASSWORD_FILE: /data/home-password
```

服务使用 `container_name: home`，所以下面的 `docker exec` 命令都指向
`home`。端口只发布在 `127.0.0.1:8080`；容器本身绑定 `0.0.0.0`，因此正是
主机侧的回环绑定让面板不暴露在 LAN 上，与[网页面板](#web-panel)章节的规则
相同。

### 首次启动与密码文件

上面的 Compose 示例通过环境传递密码。要改用文件，请在首次启动前创建命名卷
并写入密码：

```console
docker volume create home-data
printf '%s\n' '{panel_password}' | docker run --rm -i -v home-data:/data \
  --entrypoint sh ghcr.io/wray-lee/smarthome:0.3.0 \
  -c 'umask 077; cat > /data/home-password'
```

容器以 UID 10001 运行，因此文件落到磁盘时属主为 10001、模式 600，密码绝不
会出现在 argv 或镜像中。绑定挂载的主机文件也可用
（`--mount type=bind,src=/path/home-password,dst=/run/secrets/home-password,readonly`
加 `-e HOME_PASSWORD_FILE=/run/secrets/home-password`），只要 UID 10001
能读取它。

在容器内把两个后端各登录一次。`docker exec -it` 给 Google 登录一个终端，
因此它会打印步骤并接受粘贴的令牌后按 Enter；两种非交互回退从环境或 `/data`
卷中的文件读取令牌（文件更安全，因为 `docker inspect` 会显示环境）：

```console
docker exec -it home home gh login --email you@example.com
docker exec -e HOME_GH_TOKEN='{oauth_token}' home home gh login --email you@example.com
printf '%s\n' '{gh_token}' | docker run --rm -i -v home-data:/data \
  --entrypoint sh ghcr.io/wray-lee/smarthome:0.3.0 \
  -c 'umask 077; cat > /data/gh-token'
docker exec home home gh login --email you@example.com --token-file /data/gh-token
docker exec -it -e HOME_TUYA_USER_CODE={user_code} home home tuya login
```

Tuya 二维码打印在 stderr；用 Smart Life 应用扫描，或改用面板中的 Tuya 二维码
对话框（同一登录）。user code 位于应用中的 Me → Settings → Account and
Security → User Code。

新数据库没有任何设备受保护。每个必须保持只读的设备都需要在首次写入前显式
保护：

```console
docker exec -i home home protect tuya:{device_id}
docker exec -i home home protect gh:{device_id}
```

为每个这样的设备添加一行，或者在启动容器前把已有的 `home.sqlite`（保护信息
就在其中）复制到卷里。全新卷里是空数据库，没有任何保护。

容器只发布到 `127.0.0.1`。通过 Tailscale（或其他私有网络）访问；绝不要直接
暴露端口。如果反向代理在其前面终止 TLS，在容器上设置
`HOME_COOKIE_SECURE=true`，让会话 cookie 带上 `Secure`。

确认它能应答：

```console
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/login   # 200
```

整个 `/api/` 树都需要登录，因此匿名 `GET /api/devices` 返回 401 是正常现象，
不是故障。

运行时镜像设置 `HOME_DB=/data/home.sqlite`、`HOME_HOST=0.0.0.0` 和
`HOME_PORT=8080`，因此无需其他配置。

### 升级现有部署

下面的 `home` 是旧容器的名称。先检查它，并复用已经持有 `home.sqlite` 的卷；
不要让新服务启动在全新的空卷上：

```console
docker inspect home --format '{{range .Mounts}}{{.Type}} {{if .Name}}{{.Name}}{{else}}{{.Source}}{{end}} -> {{.Destination}}{{"\n"}}{{end}}'
```

以 `/data` 结尾的那行给出的是 Docker 卷（`volume`）或主机路径（`bind`）。在
容器停止时备份数据库，然后删除旧容器，让新容器能接管其名称和回环端口：

```console
docker stop home
mkdir -p ./data-backup
docker cp home:/data/. ./data-backup/
docker rm home
```

当卷是 `home-data` 时，上面的 Compose 文件无需修改。当 inspect 打印的是另一
个卷名时，把该名称放进 `name:`，让 Compose 复用它而不是创建空的 `home-data`：

```yaml
volumes:
  home-data:
    name: the-name-from-inspect
```

当 `/data` 是绑定挂载时，在服务中使用主机路径：

```yaml
    volumes:
      - /the/host/path/from/inspect:/data
```

两个进程不能同时写一个 SQLite 文件，端口也只能属于其中一个，因此在
`docker compose up -d` 前停止旧容器。新版本打开数据库时会向前迁移；如果之后
旧二进制拒绝更新的模式，备份就是回退手段。

### 离线：加载随附镜像

对于无法访问 registry 的主机，在能拉取的机器上按其架构保存镜像，复制 tar 包
过去，加载它，并让同一个 Compose 文件指向已加载的标签，Compose 会直接使用
而不拉取：

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

inspect 输出必须是 `amd64` 或 `arm64` 并与主机匹配；架构不符的 tar 会在启动
时以 `exec format error` 失败。如果没有匹配主机的 tar，改为运行
`docker build -t home:0.3.0 .`；它会为本机架构生成相同的标签。

<a id='security'></a>
## 安全

- 把面板保持在回环地址（`HOME_HOST=127.0.0.1`，或 Compose 端口绑定
  `127.0.0.1:8080:8080`），通过 Tailscale 或其他私有网络访问。绝不暴露到
  公网。它是带密码的纯 HTTP；如果反向代理在前面终止 TLS，设置
  `HOME_COOKIE_SECURE=true`（或 `--secure-cookie`），让会话 cookie 带上
  `Secure`。
- 数据库就是总钥匙：它持有 Google 主令牌（等同于账号密码）和 Tuya 会话，
  创建时模式 600，必须留在仓库之外并进入你的备份。`home tuya keys` 导出的
  本地密钥同样敏感。
- 登录失败与受保护设备覆盖共享一个失败密码节流：从第三次失败起计数器按
  指数退避（`2 << min(failures - 3, 5)` 秒），被阻止的请求在比较密码前得到
  `429 Too Many Requests` 和 `Retry-After`，任何正确密码都会重置计数器。
- 会话 id 是来自 OS CSPRNG 的 32 字节；cookie 和内存会话在 8 小时后过期，
  退出登录会删除 id。每个响应都带有 `X-Content-Type-Options: nosniff`、
  `X-Frame-Options: DENY`、`frame-ancestors 'none'`、
  `Referrer-Policy: no-referrer` 和 `Cache-Control: no-store`。密码比较是
  常数时间并折入长度。面板把服务器值渲染为文本；唯一的 `innerHTML` 是
  服务器构建的 Tuya 二维码 SVG。没有 CSRF 令牌；`SameSite=Strict` 才是阻止
  外部页面驱动 POST 的机制，因此 cookie 绝不能放宽。
- 受保护设备经每条写入路径都只读，直到 `home protect --remove`。`--force`
  和面板密码重输是给键盘前的人用的一次性写入覆盖。敏感操作需要 `--confirm`；
  受保护的锁两者都需要。见[受保护设备](#protected-devices)。

<a id='license'></a>
## 许可证

MIT；版权为 2026 wray-lee。完整文本见 [LICENSE](../../LICENSE)。

`crates/ghome-backend` 分叉自 `piekstra/google-home-cli` v0.10.0，
`crates/tuya-backend` 移植了 `lueur` 的 `tuya_cloud.rs`；两个上游项目均为
MIT。它们的许可证全文收录在
[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md)，原始跟踪副本保留在
`crates/ghome-backend/LICENSE-ghome` 和 `crates/tuya-backend/LICENSE-tuya`。

容器镜像在 `/usr/share/licenses/home/` 下附带 `LICENSE` 和
`THIRD_PARTY_NOTICES.md`。