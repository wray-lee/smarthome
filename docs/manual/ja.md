# home マニュアル

[English](en.md) | [简体中文](zh-CN.md) | 日本語

このページは `home` の日本語マニュアルです。`home` は Google Home と Tuya の
デバイスを扱う 1 つの Rust バイナリと Web パネルです。コマンドはどれも JSON
ドキュメントを 1 つ stdout に書き出し、終了コードが結果を表します。README は
概要で、このページにはすべてのコマンド、オプション、規則を記録しています。

<a id='contents'></a>
## 目次

- [目次](#contents)
- [用語](#terms)
- [リポジトリ構成](#repository-layout)
- [ビルドとテスト](#build-and-test)
- [クイックスタート](#quick-start)
- [デプロイ](#deploy)
- [Google Home のサインイン](#google-home-sign-in)
- [Google Home のデバイス](#google-home-devices)
- [Tuya のサインイン](#tuya-sign-in)
- [テレメトリー](#telemetry)
- [データベース](#database)
- [終了コード](#exit-codes)
- [保護デバイス](#protected-devices)
- [上流オーバーライド](#upstream-overrides)
- [Web パネル](#web-panel)
- [Docker](#docker)
- [セキュリティ](#security)
- [ライセンス](#license)

<a id='terms'></a>
## 用語

[`CONTEXT.md`](../../CONTEXT.md) がこの表の出所です。

| 用語 | 意味 | English | 中文 |
|---|---|---|---|
| バックエンド | ツールがデバイスに到達するために使う独立した経路。Google Home と Tuya の 2 つがあり、互いに依存しない。 | An independent channel the tool uses to reach Devices. There are two, Google Home and Tuya, and neither depends on the other. | 后端：工具用来访问设备的独立通道，有 Google Home 和 Tuya 两种，彼此不依赖。 |
| デバイス | あるバックエンドから見て制御または読み取りできる対象。1 つの物理プラグが両方のバックエンドにデバイスとして現れることがある。 | Something that can be controlled or read, as seen through one Backend. One physical plug may appear as a Device in both Backends. | 设备：通过某个后端能够控制或读取的对象。同一个物理插座可能在两个后端中各显示为一个设备。 |
| 保護デバイス | ユーザーが `home protect --remove` で保護を解除するまで、どのバックエンドからもツールが書き込みを拒否するデバイス。状態とテレメトリーの読み取りは引き続き可能。保護は書き込みをまたいで有効なまま。2 つの明示的な 1 回限りの上書きが、リストを変えずに 1 回だけ書き込みを通す。CLI の `--force` と、Web パネルでそのリクエストのためにパネルパスワードを再入力する方法である。どちらもキーボードの前にいる人間のためのもので、パネルの上書きにはログインと同じ失敗パスワードのバックオフが適用される。機微な操作が動作についての区別であるのに対し、保護はデバイスについての区別であり、保護デバイスでの機微な操作には引き続き `--confirm` が必要。 | A Device the tool refuses to write to through any Backend until the user removes the protection with `home protect --remove`. Reading its state and Telemetry still works. Protection stays active across writes; two explicit one-time overrides let exactly one write through without changing the list: `--force` on the CLI, and re-entering the panel password in the Web Panel for that one request. Both exist for the human at the keyboard, and the same failed-password backoff covers the panel override as the login. Unlike a Sensitive Action, which is about the action, protection is about the Device, and a Protected Device still needs `--confirm` for a Sensitive Action. | 受保护设备：在所有后端中都拒绝写入的设备，只有用户执行 `home protect --remove` 才能解除。读取状态和遥测仍然可用。保护在多次写入之间持续有效；两种明确的一次性覆盖各自允许恰好一次写入且不改变列表：CLI 上的 `--force`，以及在 Web 面板中针对该次请求重新输入面板密码。两者都供人在键盘前使用；面板覆盖与登录共享同一套失败密码退避。敏感操作针对动作，保护针对设备，受保护设备上的敏感操作仍然需要 `--confirm`。 |
| 機微な操作 | ドアの解錠やアラームの解除など、物理的な安全性を弱める操作。呼び出し元が明示的に確認しない限り、ツールは拒否する。 | An action that weakens physical security, such as unlocking a door or disarming an alarm. The tool refuses it unless the caller confirms explicitly. | 敏感操作：开锁或撤防等削弱物理安全的动作。除非调用方明确确认，否则工具拒绝执行。 |
| テレメトリー | 電力、電流、電圧、累積エネルギーなど、デバイスからの測定値。提供するのは Tuya バックエンドのみ。 | Measured readings from a Device, such as power, current, voltage and accumulated energy. Only the Tuya Backend provides it. | 遥测：设备产生的测量读数，如功率、电流、电压和累计电量。只有 Tuya 后端提供。 |
| ローカルキー | Tuya デバイスのローカル制御を可能にするデバイスごとの秘密。ツールが保持すれば、デバイスがリセットまたは再ペアリングされるまで Tuya クラウドなしで操作できる。 | The per-device secret that allows Local Control of a Tuya Device. Once the tool holds it, the Device can be operated without the Tuya cloud, until the Device is reset or re-paired. | 本地密钥：允许对 Tuya 设备进行本地控制的每设备密钥。工具持有后，设备在重置或重新配网之前无需 Tuya 云即可操作。 |
| ローカル制御 | ベンダーのクラウドを介さず、家庭内ネットワークでデバイスと直接通信すること。延期されており、Tuya バックエンドは現在クラウド制御を使う。 | Talking to a Device directly over the home network, with no vendor cloud involved. Deferred; the Tuya Backend currently uses Cloud Control. | 本地控制：在家庭网络中直接与设备通信，不经过厂商云。暂缓实现；Tuya 后端目前使用云端控制。 |
| クラウド制御 | ユーザーの Smart Life アカウントでサインインし、Smart Life クラウド経由で Tuya デバイスを操作すること。どのネットワークからでも動作する。 | Operating a Tuya Device through the Smart Life cloud, signed in as the user's Smart Life account. Works from any network. | 云端控制：以用户的 Smart Life 账号登录，通过 Smart Life 云操作 Tuya 设备。可在任意网络中使用。 |
| ベンダー退出 | ベンダーのクラウドがなくなってもデバイスを使い続けられるようにすること。Tuya では、まずすべてのローカルキーを保持することが第一歩。最後の手段はサードパーティ製ファームウェアをデバイスに書き込み、別のバックエンドのデバイスにすること。 | Keeping Devices usable if a vendor's cloud disappears. For Tuya, the first step is holding every Local Key. The last resort is reflashing a Device with third-party firmware, which turns it into a Device of a different Backend. | 厂商退出：在厂商云消失时让设备仍可使用。对 Tuya 来说，第一步是持有全部本地密钥。最后手段是刷入第三方固件，使设备变成另一个后端的设备。 |
| リンク済みサービス | Google Home に接続されたベンダーのクラウドアカウント。Tuya デバイスに対する Smart Life など。Google Home バックエンドがリンク済みサービスと直接通信することはない。 | A vendor cloud account connected into Google Home, for example Smart Life for Tuya devices. The Google Home Backend never talks to a Linked Service directly. | 关联服务：接入 Google Home 的厂商云账号，例如用于 Tuya 设备的 Smart Life。Google Home 后端从不直接与关联服务通信。 |
| Web パネル | Docker デプロイから提供されるブラウザー UI。人間がデバイスを操作するためのもの。 | A browser UI, served from the Docker deployment, for humans to control Devices. | Web 面板：由 Docker 部署提供的浏览器界面，供人控制设备。 |
| コマンドライン (CLI) | エージェントから呼び出されることを想定したコマンドラインインターフェース。出力は機械可読。 | The command-line interface, designed to be called by agents. Its output is machine-readable. | 命令行界面：为代理调用而设计，输出机器可读。 |
| 曖昧な名前 | 複数のデバイスに一致するデバイス名。ツールは推測せず、終了コード 3 で拒否して候補を列挙する。 | A Device name that matches more than one Device. The tool refuses it with exit 3 and lists the candidates instead of guessing. | 歧义名称：匹配到多个设备的设备名称。工具以退出码 3 拒绝并列出候选，而不是猜测。 |

<a id='repository-layout'></a>
## リポジトリ構成

- `crates/home-store` は SQLite のストレージ層であり、JSON と終了コードの
  契約、そして保護デバイスへの唯一の書き込みゲートである `guard` を担います。
- `crates/ghome-backend` は Google Home バックエンドで、
  `piekstra/google-home-cli` v0.10.0 からフォークしています（MIT。上流の
  テキストは `crates/ghome-backend/LICENSE-ghome` に残っています）。非公式の
  foyer API を使います。`docs/adr/0001-google-home-via-unofficial-foyer-api.md`
  を参照してください。
- `crates/tuya-backend` は Tuya バックエンドで、`lueur` の MIT ライセンスの
  `tuya_cloud.rs` を移植しています（上流のテキストは
  `crates/tuya-backend/LICENSE-tuya` に残っています）。
- `src/main.rs` は `home` バイナリです。コマンドライン (CLI) と `serve` を
  提供します。
- `src/web/` はパネルです。ビルド不要の単一ファイル `index.html` と
  `login.html` があります。
- `tests/` にはブラックボックスの CLI とパネルのテストがあります（`cli.rs`、
  `gh_03.rs`、`tuya_04.rs`、`tuya_05.rs`、`serve_06.rs`、`serve_07.rs`、
  および i18n テスト）。
- `docs/adr/` にはアーキテクチャ決定が、`docs/manual/` にはこのマニュアルが
  あります。`scripts/image-smoke.sh` はコンテナのスモークチェックです。
- 2 つのバックエンドは互いに依存しません。Google Home バックエンドはワーク
  スペースの他の部分に `list_devices`、`get_state`、`set_trait` だけを公開し、
  将来の Home MCP 実装の接点になります。

<a id='build-and-test'></a>
## ビルドとテスト

WSL で Windows のツールチェーンなしに開発とテストを行っています。Arch Linux
で、`cargo` はディストリビューションのものを使います。`Cargo.lock` はコミット
済みなので依存のバージョンは固定されていますが、それで crate 本体が入手できる
わけではありません。

ここのネットワークは利用できる場合があります。`curl` による
`https://index.crates.io/config.json` と
`https://static.crates.io/crates/serde/serde-1.0.0.crate` への確認は 2xx を
返したため、オンラインビルドが動く可能性はあります。それでもこの環境で確立
しているビルド方法は、以下のキャッシュを使うやり方であり、このマニュアルは
その点を曖昧にしません。

- 既定の `CARGO_HOME`（`~/.cargo`）では、crate が存在しない場合
  `cargo build --offline` が `no matching package named 'rusqlite' found` で
  止まります。したがって、キャッシュなしのクリーンな `cargo build` は失敗する
  ことがあります。
- 動作する構成では、`CARGO_HOME` を Windows ホストの cargo キャッシュ
  （WSL から `/mnt/c` 経由で読み書き）に向け、`--offline` でビルドします。

```console
export CARGO_HOME=/mnt/c/Users/<you>/.cargo
export CARGO_TARGET_DIR=/home/<you>/work/target   # inside WSL, not on /mnt
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

`CARGO_TARGET_DIR` は WSL 内のパスにしてください。Windows マウント上での
ビルドは遅く、そこにある `target/` などを誤ってコミットしやすいためです。
同じ 2 つの export と `--release` でバイナリをビルドできます。ホストの
キャッシュを使わない場合は、マシンがまだネットワークに接続できるうちに依存を
vendor する（`cargo vendor`）か、WSL 自身のレジストリキャッシュを事前に埋めて
ください。それまでは、この 2 つの export は利便性ではなく前提条件です。
ネットワークを信頼できない場合は依存を vendor してください。そうすれば
オフラインビルドに他に何も要りません。

リリースビルドは静的にリンクされた単一の Linux バイナリ（musl）なので、
ホストの glibc のバージョンに依存しません。`scripts/image-smoke.sh <tag>` は
コンテナのスモークチェックです。使い捨てのコンテナ内で実行され、実際の
デバイスに決して接続しません。

<a id='quick-start'></a>
## クイックスタート

0.3.0 リリースのイメージは `ghcr.io/wray-lee/smarthome:0.3.0` です。一度だけ
pull します。Compose はその固定タグを使います。

```console
docker pull ghcr.io/wray-lee/smarthome:0.3.0
```

Compose を実行する場所に次の `compose.yaml` を書きます。

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

`name: home-data` はボリューム名を固定します。これがないと Compose がプロジェクト名
を前置するため、後述の `docker volume create home-data` の手順で用意したボリュームを
サービスがマウントしません。

`HOME_PASSWORD` は必須です。`${HOME_PASSWORD:?set HOME_PASSWORD}` の形式に
より、未設定なら Compose はエラーで停止します。Compose コマンドの環境で指定
するか、`compose.yaml` と同じディレクトリに置く `.env` ファイルで指定して
ください。`.env` はモード 600 にし、git 管理外に保ちます。

```console
printf '%s\n' "HOME_PASSWORD='{panel_password}'" > .env
chmod 600 .env
docker compose up -d
```

両方のバックエンドに、コンテナ内で 1 回ずつサインインします。

```console
docker exec -it home home gh login --email you@example.com
docker exec -it home home tuya login
```

http://127.0.0.1:8080 を開き、同じパスワードでサインインします。新しいデータベース
では何も保護されていません。読み取り専用に保つデバイスはすべて、最初の書き込み前に
保護に追加します。

```console
docker exec -i home home protect tuya:{device_id}
docker exec -i home home protect gh:{device_id}
```

保護を解除できるのは `home protect --remove` だけです。パスワードファイルを
使う方法、Google の非対話サインイン 2 通り、ヘルスチェックは
[Docker](#docker) にあります。

<a id='deploy'></a>
## デプロイ

実行方法は 2 つあります。Linux ホスト上のバイナリか、[Docker](#docker) の
コンテナです。どちらも同じパネルを提供し、すべてを 1 つの SQLite ファイルに
保ちます。

### バイナリ

リリースビルドは静的にリンクされた単一の Linux バイナリ（musl）なので、
ホストの glibc のバージョンに依存しません。一般的な Linux ディストリビューションや
Alpine にコピーして、そのまま実行できます。データディレクトリと、そのユーザーだけが
読めるパスワードファイルを用意します。

```console
install -Dm755 home /usr/local/bin/home
sudo install -d -m700 -o "$USER" /var/lib/home
install -m600 /dev/stdin /var/lib/home/password <<< '{panel_password}'
```

両方のバックエンドに、この順で 1 回ずつサインインします（パスワードファイルは
プロセスごとに読まれるため、CLI と `serve` は同じデータベースを共有します）。

```console
home --db /var/lib/home/home.sqlite tuya login      # QR code, then scan it
echo '{gh_token}' | home --db /var/lib/home/home.sqlite gh login --email you@example.com
home --db /var/lib/home/home.sqlite protect tuya:{device_id}   # every Device that must stay read-only
```

マスタートークンは `aas_et/` で始まります。次に init システムの下でパネルを
起動します。パスワードはファイルから読むので、`ps` に現れることはありません。

```console
HOME_DB=/var/lib/home/home.sqlite \
HOME_HOST=127.0.0.1 HOME_PORT=8080 \
HOME_PASSWORD_FILE=/var/lib/home/password \
home serve
```

`HOME_HOST` はループバックに保ち、Tailscale（または別のプライベートネットワーク）経由で
パネルにアクセスしてください。パスワード付きの平文 HTTP なので、インターネットに公開
してはいけません。前に HTTPS を置く場合は、`HOME_COOKIE_SECURE=true` を追加して
セッション cookie に `Secure` を付けてください。

離れる前に、応答することを確認します。

```console
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/login   # 200
```

`/api/health` を含む `/api/` ツリー全体がパスワードの背後にあります。そのため
401 は「どこにもサインインしていない」を意味し、「停止している」ではありません。
認証不要の生存確認は `GET /login` です。

### アップグレード

古いプロセスを停止し、バイナリを置き換え、再度起動します。データベースは
オープン時に前方へマイグレーションされ（`PRAGMA user_version`、現在は 2）、
新しいデータベースは古いバイナリに中途半端に読まれるのではなく拒否されます。
そのため、新しいバージョンの初回起動前にバックアップしてください。

```console
sqlite3 /var/lib/home/home.sqlite ".backup '/var/backups/home-$(date +%F).sqlite'"
```

`home tuya keys > tuya-keys.json` はローカルキーをデータベースの外に保存し、
ベンダー退出のバックアップとします。Google のマスタートークンと Tuya の
セッションはデータベースにのみ存在します。

<a id='google-home-sign-in'></a>
## Google Home のサインイン

アカウントのメールアドレスは `--email` で渡すか、`HOME_GH_EMAIL` に設定します。
資格情報は Google のマスタートークン（アカウントのパスワードに相当）または
1 回限りのブラウザー `oauth_token` です。`--token-file <PATH>`、
`HOME_GH_TOKEN`、stdin の 1 行、の順で取得します。トークン自体は argv に
入りません。

```console
home gh login --email you@example.com           # interactive: paste, press Enter
HOME_GH_TOKEN='{gh_token}' home gh login --email you@example.com
home gh login --email you@example.com --token-file /run/secrets/gh-token
home gh devices
```

stdin が端末の場合、`home gh login` は読み取り前に手順を stderr に表示し、
Enter を押すと貼り付けた行をすぐに保存します。端末がない場合は stdin から 1 行
読みます（パイプは末尾の改行の有無を問いません）。そのため `docker exec -i` に
EOF は不要になりました。

`oauth_token` を取得するには、デスクトップブラウザーで
https://accounts.google.com/EmbeddedSetup を開き、家を所有する Google
アカウントでサインインします。「I agree」ページの後、DevTools >
Application/Storage > Cookies で accounts.google.com の Cookie を開き、
`oauth_token` Cookie をコピーします（`oauth2_4/` で始まります）。1 回限りで
数分で失効するので、すぐに貼り付けてください。既存のマスタートークンは
`aas_et/` で始まり、同じ方法で貼り付けられます。

両方のコマンドは JSON ドキュメントを 1 つ出力します。`home gh login` は
マスタートークンと固定の Android ID を SQLite に保存します。

`home gh login` はマスタートークンを検証しません。`aas_et/`（または先に交換
される `oauth2_4/`）で始まれば何でも保存され、不正なトークンは最初の
`home gh devices` で `reauth_required`（終了コード 5）として現れます。Google
が拒否した `oauth_token` も `reauth_required` になり、新しいものを取得するよう
ヒントが出ます。1 回限りで数分以内に失効するためです。ログインは foyer を
呼ばないので、Google に接続するのは `oauth_token` の交換時だけです。

Android ID はマスタートークンに結び付いており（すべての Bearer がそれに対して
発行されます）、再ログインでは保存済みのものが再利用されます。
`--android-id <ID>` はそれを置き換え、キャッシュされた Bearer を破棄します。
`--help` と `--version` はテキストを stderr に出力し、それでも JSON ドキュメント
を 1 つ stdout に書き出します。

<a id='google-home-devices'></a>
## Google Home のデバイス

デバイスは正確な ID、`room/name`、または名前だけでも指定できます。照合は
大文字小文字を区別せず、複数のデバイスに一致する名前は推測せずに終了コード 3
と候補一覧で失敗します。

```console
home gh get study/reading-lamp
home gh set study/reading-lamp onOff false
home gh set study/reading-lamp brightness 75
home gh set study/reading-lamp color.colorTemperature 3000
home gh set study/reading-lamp thermostat 21 --mode heat
home gh set study/reading-lamp volume.currentVolume 30
```

`<trait>` は `onOff`、`brightness`、`color.colorTemperature`、
`volume.currentVolume`、`lockUnlock.isLocked`、`armDisarm.isArmed`、
`thermostat` を受け付けます。サーモスタットにはモードと設定値の両方が必要です
（`<value>` はどちらでもよく、足りない側はフラグで指定します）。`home gh get`
は現在の trait を `traits` の下に出力し、`home gh set` は結果の状態と
`changed` を出力します。

ロックの解錠やアラームの解除は機微な操作です。`--confirm` がなければ終了
コード 6 で拒否され、trait 呼び出しは上流に送られません。保護デバイスは
`--confirm` があっても終了コード 6 で拒否されます（1 回だけ書き込みを通すのは
`--force` だけです。[保護デバイス](#protected-devices)を参照）。読み取りは
引き続き可能です。これらの拒否を通過した `set` は、デバイスがオンラインか
確認するためにもう一度読み取り（`GetTraits` 1 回）、それから書き込みます。

<a id='tuya-sign-in'></a>
## Tuya のサインイン

`home tuya login` は端末（stderr）に QR コードを描画し、Smart Life アプリが
スキャンするまでポーリングします。その後、トークンとリージョンのエンドポイント
が SQLite に保存されます。要求されるユーザーコードはアプリの Me → Settings →
Account and Security → User Code にあり、`--user-code` の代わりに
`HOME_TUYA_USER_CODE` から渡すこともできます（秘密ではありませんが、argv は
他のプロセスから見えるため、環境変数が推奨です）。

```console
home tuya login                    # or: HOME_TUYA_USER_CODE={user_code} home tuya login
home tuya devices
home tuya keys > ~/tuya-keys.json  # back this up, off this machine
```

`home tuya devices` は ID、名前、オンライン状態、カテゴリ、バックエンドを出力
し、目にするすべてのローカルキーを保存します。サインインに成功した時点ですで
にデバイス一覧を 1 回取得し、返されたすべてのローカルキーを保存するため、
`home tuya login` だけでバックアップになります。`home tuya devices` は実行の
たびにそれらを更新します。`home tuya keys` はベンダー退出のために JSON
（`device_id`、`local_key`、`first_seen_at`、`changed_at`）としてエクスポート
します。そのファイルは機密情報として扱ってください。バックアップに入れるもの
で、git に入れるものではありません。キーが変わった場合、古いキーは置き換え
られた時刻とともに履歴に残ります。

QR サインインに Tuya IoT 開発者プラットフォームは不要です。中国以外のアカウント
で検証済みです。中国リージョンのアカウントでは、ハードコードされたログイン
ホストが失敗したとの報告があります。

<a id='telemetry'></a>
## テレメトリー

1 つのデバイスを正確な ID または名前だけで指定します。大文字小文字は区別しません
（Tuya のデバイス行には `room` がないため、ここでは `room/name` は使えません。
正確な ID は、その ID を名前として綴った指定より優先され、重複する名前は候補
付きの終了コード 3 になります）。Tuya のデバイス行に `room` フィールドは
ありません。

```console
home tuya on {device}                       # switch_1 = true
home tuya off {device}                      # switch_1 = false
home tuya get {device}                      # data points + Telemetry + calibration
home tuya set {device} {code} {value}       # one raw data point, e.g. child_lock true
```

`on`/`off` は `switch_1` を送ります。応答はコードと新しい値を含む `changed`
です。`set` は宣言された仕様からコードを探し、値を宣言された型に変換して
（`Boolean` は `true`/`false`、`Integer` は 10 進整数、それ以外は文字列のまま）、
生の値を書き込み、再読み取りはしません。読み取り経路は `get` です。未知の
コードや変換できない値は終了コード 2 で失敗し、何も送信しません。

`get` は `data_points` の下にデバイスのステータスのすべてのポイントを順に出力
し、各ポイントに `code`、`value`、仕様が宣言していれば `type` を付けます。
`telemetry` には測定値を `{raw, scale, value, unit}` として保持します。`scale`
と `unit` はデバイスの宣言された仕様から取得し（ハードコードされた DP 番号から
は決して取得しません）、`value` は `raw / 10^scale` です（`scale` が 0 のときは
raw そのもの）。ポイントがテレメトリーになるのは、仕様が `unit` を与え、デバイスの
機能の 1 つではなく、キャリブレーションでもない場合です。キャリブレーションとは
`voltage_coe`、`electric_coe`、`power_coe`、`electricity_coe` です。
この 4 つは `calibration` の下に生の値として報告され、測定値としては扱われ
ません。

保護デバイスは、その 1 回の呼び出しに `--force` がなければ、コマンドが送信
される前に `on`/`off`/`set` から終了コード 6 で拒否されます。`get` は引き続き
動作します。オフラインのデバイスはすべてのコマンドが終了コード 4 で失敗します。
セッションが死んでいる場合（`sign invalid` または `1010`）、終了コード 5 が
Tuya バックエンドを示します。一時的な `1108` は 1 つの直列化されたクライアント
内で最大 2 回再試行され、それ以外は終了コード 7 です。

<a id='database'></a>
## データベース

1 つのファイルで、パスは `--db` または `HOME_DB` から取得し、既定は
`$XDG_DATA_HOME/home/home.sqlite` です。これは家全体と Google アカウントの
鍵です。失うと Google のマスタートークン（アカウントのパスワードに相当）と
Tuya のセッションが漏れ、新しいマシンに復元すると両方が復元されます。所有者
のみの権限（モード 600）で作成され、バージョン付きマイグレーションを使い、
アプリケーションレベルの暗号化はありません。リポジトリの外に保ち、意図的に
バックアップしてください。

パネルには家の外から Tailscale 経由でアクセスします。パネルに他のリモートアクセス
経路はなく、インターネットに公開してはいけません。

<a id='exit-codes'></a>
## 終了コード

コマンドはどれも JSON ドキュメントを 1 つ stdout に書き出し、結果は終了コード
が担います。

| Exit code | Meaning |
|---|---|
| 0 | ok |
| 2 | usage |
| 3 | ambiguous name |
| 4 | Device offline |
| 5 | `reauth_required` |
| 6 | Sensitive Action or protected Device refused |
| 7 | upstream error |

Web パネルはこれらを HTTP ステータスにマッピングします。
[HTTP ステータス表](#web-panel)を参照してください。

<a id='protected-devices'></a>
## 保護デバイス

`home protect google_home:{device_id}`（または `gh:{device_id}`、
`tuya:{device_id}`）はデバイスを保護リストに追加します。`home protect
--list` はそれを表示し、`home protect --remove` だけが解除方法です。すべての
書き込み経路は `home_store::guard` を通り、保護デバイスを trait RPC の前に
終了コード 6 で拒否します（Google Home の名前検索はすでに実行済みです）。既定
では何も保護されていません。

```console
home protect google_home:{device_id}
home protect gh:{device_id}
home protect tuya:{device_id}
home protect --list
home protect --remove tuya:{device_id}
```

2 つの明示的な 1 回限りの上書きが、リストを変えずに 1 回の書き込みを通します。
デバイスは保護されたままです。

- `home gh set` と `home tuya on` / `off` / `set` の `--force`。これは
  `--confirm` とは別です。`--force` は保護デバイスへの書き込みを許可し、
  `--confirm` は機微な操作を許可し、保護されたロックには両方が必要です。
- その 1 回のリクエストのために Web パネルでパネルパスワードを再入力する方法。

どちらもキーボードの前にいる人間のためのものです。保護が終わるのは
`home protect --remove` だけです。`docs/adr/0002` を参照してください。

<a id='upstream-overrides'></a>
## 上流オーバーライド

テスト用: `HOME_FOYER_URL`（foyer RPC のプレフィックス）、
`HOME_GPSOAUTH_URL`（Android アカウントトークンのエンドポイント）、
`HOME_TUYA_LOGIN_URL`（QR ログインのベース）、`HOME_TUYA_ENDPOINT`（リージョン
API のベース。テスト用にループバックホストも受け付けます）、
`HOME_TUYA_USER_CODE`、`HOME_TUYA_POLL_MS`、`HOME_TUYA_POLL_ATTEMPTS`（QR ポーリング
の間隔と回数）、`HOME_TUYA_REFRESH_POLL_MS`（`home serve` のバックグラウンド Tuya
リフレッシュの間隔、既定 10000 ms、最小 20）。

その他の変数は効果のある場所で説明しています。`HOME_GH_TOKEN` は
[Google Home のサインイン](#google-home-sign-in)、`HOME_DB` は
[データベース](#database)、`HOME_HOST`、`HOME_PORT`、`HOME_PASSWORD`、
`HOME_PASSWORD_FILE`、`HOME_COOKIE_SECURE` は [Web パネル](#web-panel)です。

<a id='web-panel'></a>
## Web パネル

`home serve` は `HOME_HOST`/`HOME_PORT`（または `--host`/`--port`）にバインド
し、パスワードなしでは起動を拒否します（`HOME_PASSWORD`、`--password`、または
`HOME_PASSWORD_FILE`/`--password-file`。argv は他のプロセスから見えるため
Docker ではファイルが推奨です。ファイル末尾の改行は取り除かれます）。`/` と
`/settings` にパネル（同じページで、どちらか一方のビュー）、`/login` にログイン
フォーム（POST は `HttpOnly; SameSite=Strict` の `home_session` cookie を設定
します。HTTPS リバースプロキシの背後にパネルを置く場合は `--secure-cookie` を
追加するか `HOME_COOKIE_SECURE` を設定して、cookie に `Secure` を付けてください）、
`/api/` 以下に JSON API を提供します。認証されていない API リクエストは 401
JSON（`unauthorized`）を返し、誤ったパスワードの POST も 401 を返します。
`GET /api/devices` は両方のバックエンドを並べて返すため、片方のバックエンドが
死んでいても他方を隠しません。`GET /api/health` と `GET /api/status` はパネル
の健全性を報告します。バックグラウンドのティッカーが期限切れ前に Tuya トークン
を更新し、ローテーションされたリフレッシュトークンを先に永続化するので、
リクエストを待たずにセッションが保たれます。

サインイン後、パネルは 2 つのビューを持つ 1 つのページです。ヘッダーには
デバイスと設定へのリンク、更新、サインアウトがあります。リンクは実際のルート
なので、新しいタブや `/settings` への直接アクセスが機能します。ページ内では
リロードなしで切り替わり、戻る/進むで 2 つのビューの間を移動します。設定は
アドレスを `/settings` にし、ページタイトルはビューに従います。サインアウトした
状態では、`GET /settings` は `GET /` とまったく同じくログインフォームを返し、
サインアウトするとそこに戻ります。

デバイスビューは両方のバックエンドを一度に表示し、All、Google Home、Tuya の
3 つのタブで絞り込みます。各タブは最新の応答からそのバックエンドのデバイス数を
示し、バックエンドを今すぐ使えないときは言葉で示します。`sign in needed`、
`offline`、`error` です。保存された資格情報がないバックエンドは、自分のタブ内
で次の手順を説明します。Tuya は QR サインインボタンを提供し、Google Home は
`home gh login --email you@example.com` コマンドと更新ボタンを表示します。
ポーリングや更新で選択中のタブが変わることはありません。ビューを開いたときに
選択されるタブは設定で決まり、変更するまで `All` です。

設定にはこのブラウザーに保存される 3 つの選択肢があり、1 つの保存ボタンで適用
されます。

- 言語: 自動、英語、中国語（簡体字）、日本語。自動はブラウザーの言語リストから
  最初の `zh`、`ja`、`en` の項目を選び、なければ英語にフォールバックします。
  選択した言語はログインフォームにも適用され、ページの `lang` 属性もそれに
  従うので、スクリーンリーダーとスペルチェックが一致します。
- 既定のタブ: ビューを開いたときに選択されるデバイスタブ。次に訪れるまで、
  いま見ているタブは変わりません。
- 外観: 自動はシステムの配色設定に従い、ライトとダークはそれを固定します。
  ログインフォームも同じ選択に従います。

選択肢はこのブラウザーの `localStorage`（`home_panel_settings`）に保存されるので、
2 台のマシンが同じパネルを異なる見た目で表示できます。保存されていない変更は
ビューを離れると破棄されます。これらの選択肢がサーバーに保存されることは
ありません。

各デバイス行は 10 秒のポーリングで状態とテレメトリーを読み取ります。ポーリング
はブラウザーのタブが隠れている間は一時停止し、タブが再び表示されるとすぐに 1 回
更新され、同時に 2 つの読み取りが走ることはありません。読み取りは行を空に
しません。次の読み取りが進行中も最後の値が残り、すでに値があった読み取りが
失敗した場合は値を保持して `Updated HH:MM, may be out of date` とマークし、
読み取りが成功するまで続きます。

ログインの失敗は保護デバイスの上書きと 1 つの失敗パスワードスロットルを共有
します。3 回目の失敗からは 1 つのカウンターが指数関数的にバックオフし
（`2 << min(failures - 3, 5)` 秒）、ブロックされたリクエストはパスワードを
比較する前に `Retry-After` 付きの `429 Too Many Requests` を受け取ります。
そのため待機中は正しいパスワードでも拒否され、正しいパスワードはカウンターを
リセットします。パネルはこれまでどおり Tailscale の背後のループバックに置くもの
で、直接公開してはいけません。セッション ID は OS の CSPRNG から 32 バイトで、
cookie とメモリ内セッションは 8 時間で失効し、サインアウトすると ID が削除
されます。すべての応答には `X-Content-Type-Options: nosniff`、
`X-Frame-Options: DENY`、`frame-ancestors 'none'`、
`Referrer-Policy: no-referrer`、`Cache-Control: no-store` が付きます。
パスワードの照合は定数時間で行い、長さも比較に組み込みます。パネルはサーバーの
値をすべてテキストとして描画します。唯一の `innerHTML` は Tuya の QR SVG で、
そのマークアップは QR エンコーダからサーバー自身が組み立てます（座標のみで、
リクエストのテキストは含みません）。CSRF トークンはありません。外部ページが
POST を駆動するのを防ぐのは `SameSite=Strict` なので、cookie を緩めてはいけません。

パネルは CLI と同じコマンド層を呼び出し、CLI の安全規則を保ちます。保護デバイス
は目に見える `protected` ラベルを表示し、既定では読み取り専用です。パネルから
の書き込みには、その 1 回のリクエストのためにパネルパスワードの再入力が必要で、
同じパスワードスロットルが適用されます。ロックの解錠やアラームの解除は CLI と
同じ明示的な確認を求め、保護されたロックには両方が必要です。

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

セッションがない場合、すべての `/api/` リクエストは上記のどれかに到達する前に
401 を返します。

<a id='docker'></a>
## Docker

1 つのイメージが `home` バイナリを保持します。ランタイムイメージは
`alpine:3.21` を基に構築され、静的リンクされた musl バイナリを同梱するので、
Debian/glibc ビルドに比べてはるかに小さいサイズです。SQLite ファイルはマウントされた
ボリュームに置かれ、イメージに焼き込まれることはありません。コンテナは
非 root の `home` ユーザー（UID 10001）で実行され、イメージは `LICENSE` と
`THIRD_PARTY_NOTICES.md` を `/usr/share/licenses/home/` に同梱します。

0.3.0 リリースのイメージは `ghcr.io/wray-lee/smarthome:0.3.0` です。一度だけ
pull します。Compose はその固定タグを使います。

```console
docker pull ghcr.io/wray-lee/smarthome:0.3.0
```

### Compose で実行

Compose を実行する場所に次の `compose.yaml` を書きます。

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

`name: home-data` はボリューム名を固定します。これがないと Compose がプロジェクト名
を前置するため、後述の `docker volume create home-data` の手順で用意したボリュームを
サービスがマウントしません。

`HOME_PASSWORD` は必須です。`${HOME_PASSWORD:?set HOME_PASSWORD}` の形式に
より、未設定なら Compose はエラーで停止します。Compose コマンドの環境で指定
するか、`compose.yaml` と同じディレクトリに置く `.env` ファイルで指定して
ください。`.env` はモード 600 にし、git 管理外に保ちます。

```console
printf '%s\n' "HOME_PASSWORD='{panel_password}'" > .env
chmod 600 .env
docker compose up -d
```

環境はコンテナに届くため、`docker inspect home` でパスワードが見えます。それが
問題になる場合は既存のパスワードファイル方式が引き続き使えます。`/data` ボリュームに
UID 10001 所有、モード 600 で `home-password` を書き（下記参照）、
`compose.yaml` の `HOME_PASSWORD` 行を次に置き換えます。

```yaml
      HOME_PASSWORD_FILE: /data/home-password
```

サービスは `container_name: home` を使うので、以下の `docker exec` コマンドは
`home` を指定します。ポートは `127.0.0.1:8080` にのみ公開されます。コンテナ
自体は `0.0.0.0` にバインドするため、パネルを LAN の外に保つのはホスト側の
ループバックバインドです。これは [Web パネル](#web-panel)の節と同じ規則です。

### 初回起動とパスワードファイル

上の Compose の例はパスワードを環境で渡します。ファイルを使うには、最初の
起動前に名前付きボリュームを作成し、パスワードを書き込みます。

```console
docker volume create home-data
printf '%s\n' '{panel_password}' | docker run --rm -i -v home-data:/data \
  --entrypoint sh ghcr.io/wray-lee/smarthome:0.3.0 \
  -c 'umask 077; cat > /data/home-password'
```

コンテナは UID 10001 で実行されるので、ファイルは 10001 所有、モード 600 で
ディスクに置かれ、パスワードが argv やイメージに現れることはありません。
ホストのファイルをバインドマウントする方法も使えます
（`--mount type=bind,src=/path/home-password,dst=/run/secrets/home-password,readonly`
と `-e HOME_PASSWORD_FILE=/run/secrets/home-password`）。UID 10001 が読める限り
有効です。

両方のバックエンドにコンテナ内で 1 回ずつサインインします。`docker exec -it`
は Google ログインに端末を与えるので、手順が表示され、貼り付けたトークンと
Enter を受け付けます。2 つの非対話の代替手段はトークンを環境か `/data` ボリューム内
のファイルから読みます（`docker inspect` が環境を見せるため、ファイルの方が安全
です）。

```console
docker exec -it home home gh login --email you@example.com
docker exec -e HOME_GH_TOKEN='{oauth_token}' home home gh login --email you@example.com
printf '%s\n' '{gh_token}' | docker run --rm -i -v home-data:/data \
  --entrypoint sh ghcr.io/wray-lee/smarthome:0.3.0 \
  -c 'umask 077; cat > /data/gh-token'
docker exec home home gh login --email you@example.com --token-file /data/gh-token
docker exec -it -e HOME_TUYA_USER_CODE={user_code} home home tuya login
```

Tuya の QR コードは stderr に出力されます。Smart Life アプリでスキャンするか、
パネルの Tuya QR ダイアログを使ってください（同じログインです）。ユーザーコード
はアプリの Me → Settings → Account and Security → User Code にあります。

新しいデータベースでは何も保護されていません。読み取り専用に保つデバイスは
すべて、最初の書き込み前に明示的な保護が必要です。

```console
docker exec -i home home protect tuya:{device_id}
docker exec -i home home protect gh:{device_id}
```

そのようなデバイスごとに 1 行追加するか、保護がすでに入っている既存の
`home.sqlite` をコンテナ起動前にボリュームへコピーします。新しいボリュームには
空のデータベースがあり、何も保護されていません。

コンテナは `127.0.0.1` にのみ公開されます。Tailscale（または別の
プライベートネットワーク）経由でアクセスし、ポートを直接公開しないでください。
リバースプロキシが前に立って TLS を終端する場合は、コンテナに
`HOME_COOKIE_SECURE=true` を設定してセッション cookie に `Secure` を付けて
ください。

応答を確認します。

```console
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/login   # 200
```

`/api/` ツリー全体にログインが必要なので、匿名の `GET /api/devices` が 401 を
返すのは正常で、失敗ではありません。

ランタイムイメージは `HOME_DB=/data/home.sqlite`、`HOME_HOST=0.0.0.0`、
`HOME_PORT=8080` を設定するので、他の設定は不要です。

### 既存デプロイのアップグレード

下の `home` は古いコンテナの名前です。まず inspect し、すでに `home.sqlite` を
持つボリュームを再利用してください。新しいサービスを空のボリュームで起動させて
はいけません。

```console
docker inspect home --format '{{range .Mounts}}{{.Type}} {{if .Name}}{{.Name}}{{else}}{{.Source}}{{end}} -> {{.Destination}}{{"\n"}}{{end}}'
```

`/data` で終わる行が、Docker ボリューム（`volume`）かホストパス（`bind`）の
どちらかを示します。コンテナを停止した状態でデータベースをバックアップし、
古いコンテナを削除し、新しいコンテナがその名前とループバックポートを引き継げる
ようにします。

```console
docker stop home
mkdir -p ./data-backup
docker cp home:/data/. ./data-backup/
docker rm home
```

ボリュームが `home-data` の場合、上の Compose ファイルは変更不要です。inspect
が別のボリューム名を出力した場合は、その名前を `name:` に入れてください。
Compose は空の `home-data` を作らずにそれを再利用します。

```yaml
volumes:
  home-data:
    name: the-name-from-inspect
```

`/data` がバインドマウントの場合、サービスのボリュームにホストパスを使います。

```yaml
    volumes:
      - /the/host/path/from/inspect:/data
```

2 つのプロセスが同じ SQLite ファイルに同時に書き込むことはできず、ポートも
片方にしか属せないので、`docker compose up -d` の前に古いコンテナを停止して
ください。新しいバージョンがデータベースを開くときに前方へマイグレーション
します。古いバイナリが新しいスキーマを後で拒否する場合、バックアップが戻り道
です。

### オフライン: 同梱イメージの読み込み

レジストリにアクセスできないホストでは、pull できるマシンでそのアーキテクチャの
イメージを保存し、tar をコピーして読み込み、同じ Compose ファイルを読み込んだ
タグに向けます。Compose は pull せずにそれを使います。

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

inspect の出力は `amd64` か `arm64` で、ホストと一致しなければなりません。
アーキテクチャが違う tar は起動時に `exec format error` で失敗します。ホストに
合う tar がない場合は、代わりに `docker build -t home:0.3.0 .` を実行して
ください。ネイティブアーキテクチャ用に同じタグを生成します。

<a id='security'></a>
## セキュリティ

- パネルはループバック（`HOME_HOST=127.0.0.1`、または Compose のポートバインド
  `127.0.0.1:8080:8080`）に保ち、Tailscale または別のプライベートネットワーク
  経由でアクセスします。インターネットに公開してはいけません。パスワード付きの
  平文 HTTP です。リバースプロキシが前に立って TLS を終端する場合は、
  `HOME_COOKIE_SECURE=true`（または `--secure-cookie`）を設定してセッション
  cookie に `Secure` を付けてください。
- データベースはマスターキーです。Google のマスタートークン（アカウントの
  パスワードに相当）と Tuya のセッションを保持し、モード 600 で作成され、
  リポジトリの外かつバックアップの中に置く必要があります。`home tuya keys` は
  同じく機密性の高いローカルキーをエクスポートします。
- ログインの失敗と保護デバイスの上書きは 1 つの失敗パスワードスロットルを共有
  します。3 回目の失敗からカウンターは指数関数的にバックオフし
  （`2 << min(failures - 3, 5)` 秒）、ブロックされたリクエストはパスワードを
  比較する前に `Retry-After` 付きの `429 Too Many Requests` を受け取り、正しい
  パスワードはカウンターをリセットします。
- セッション ID は OS の CSPRNG から 32 バイトです。cookie とメモリ内セッション
  は 8 時間で失効し、サインアウトすると ID が削除されます。すべての応答には
  `X-Content-Type-Options: nosniff`、`X-Frame-Options: DENY`、
  `frame-ancestors 'none'`、`Referrer-Policy: no-referrer`、
  `Cache-Control: no-store` が付きます。パスワードの照合は定数時間で行い、
  長さも比較に組み込みます。パネルはサーバーの値をテキストとして描画し、唯一の
  `innerHTML` はサーバーが組み立てる Tuya の QR SVG です。CSRF トークンは
  ありません。外部ページが POST を駆動するのを防ぐのは `SameSite=Strict` なので、
  cookie を緩めてはいけません。
- 保護デバイスは `home protect --remove` まで、すべての書き込み経路で
  読み取り専用です。`--force` とパネルのパスワード再入力は、キーボードの前にいる人間の
  ための 1 回限りの上書きです。機微な操作には `--confirm` が必要で、保護された
  ロックには両方が必要です。[保護デバイス](#protected-devices)を参照してください。

<a id='license'></a>
## ライセンス

MIT。著作権は 2026 wray-lee です。全文は [LICENSE](../../LICENSE) にあります。

`crates/ghome-backend` は `piekstra/google-home-cli` v0.10.0 のフォークで、
`crates/tuya-backend` は `lueur` の `tuya_cloud.rs` を移植しています。両方の
上流プロジェクトは MIT です。ライセンス全文は
[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md) に収録され、元の追跡
コピーは `crates/ghome-backend/LICENSE-ghome` と
`crates/tuya-backend/LICENSE-tuya` に残っています。

コンテナイメージは `LICENSE` と `THIRD_PARTY_NOTICES.md` を
`/usr/share/licenses/home/` に同梱します。