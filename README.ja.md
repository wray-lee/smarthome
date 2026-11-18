# home

[English](README.md) | [简体中文](README.zh-CN.md) | 日本語

[![CI](https://github.com/wray-lee/smarthome/actions/workflows/ci.yml/badge.svg)](https://github.com/wray-lee/smarthome/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`home` は Google Home と Tuya のデバイスを、1 つの Rust バイナリと Web パネルから
操作します。コマンドはどれも JSON ドキュメントを 1 つ stdout に書き出し、
終了コードが結果を表します。

![パネル](docs/images/panel-1280-light.png)

[ダークテーマで幅 390 px の同じパネル](docs/images/panel-390-dark.png)

## 主な機能

1 つのバイナリの背後に、互いに独立した 2 つのバックエンドがあります。

- **Google Home**（`home gh`）は非公式の foyer API を通じて、Google Home 内の
  任意のデバイスを操作します。コードは `piekstra/google-home-cli` v0.10.0 から
  フォークしています。ワークスペースの他の部分には `list_devices`、`get_state`、
  `set_trait` だけを公開しており、将来 Home MCP を実装する際の接点になります。
- **Tuya**（`home tuya`）は Tuya と直接やり取りします。QR サインイン、デバイス
  一覧、ローカルキーのバックアップ、クラウド制御、テレメトリーを担います。

2 つのバックエンドは独立した crate で、互いに依存しません。1 つの物理プラグが、
異なる名前と能力を持つデバイスとして両方のバックエンドに現れることもあります。
コマンドライン (CLI) はエージェントとスクリプト向けに作られており、Web パネルは
同じコマンド層を人間向けに提供します。安全規則も共通です。

[マニュアルの用語表](docs/manual/ja.md#terms)が、バックエンド、デバイス、
保護デバイス、機微な操作、テレメトリー、ローカルキーなどの用語を定義しています。

## クイックスタート

0.3.0 リリースのイメージは `ghcr.io/wray-lee/smarthome:0.3.0` です。次の
`compose.yaml` を書きます。

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

`HOME_PASSWORD` は必須です。`compose.yaml` と同じディレクトリに置く `.env`
ファイルに入れ、モード 600、git 管理外にしてください。

```dotenv
HOME_PASSWORD='{panel_password}'
```

そしてコンテナを起動します。

```console
chmod 600 .env
docker compose up -d
```

コンテナ内で両方のバックエンドに 1 回ずつサインインします。

```console
docker exec -it home home gh login --email you@example.com
docker exec -it home home tuya login
```

http://127.0.0.1:8080 を開き、同じパスワードでサインインします。

既定では何も保護されていません。読み取り専用に保つデバイスはすべて
`docker exec -i home home protect tuya:{device_id}` で保護に追加します。
`gh:{device_id}` を使う場合もコマンドは同じです。保護を解除できるのは
`home protect --remove` だけです。

[マニュアルのクイックスタート](docs/manual/ja.md#quick-start)には、パスワードファイルを
使う手順、非対話のサインイン方法 2 通り、ヘルスチェックを載せています。

## ソースからビルド

Linux または WSL でビルドします。リリースバイナリは静的リンク（musl）なので、
一般的な Linux ディストリビューションでも Alpine でもそのまま動きます。

```console
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

[マニュアルのビルドとテスト](docs/manual/ja.md#build-and-test)では、WSL での
`CARGO_HOME` と `CARGO_TARGET_DIR` の設定、オフラインビルドと vendoring、
`scripts/image-smoke.sh` を扱っています。

## ドキュメント

[マニュアル](docs/manual/ja.md#contents)は、バイナリ、バックエンド、パネルの
リファレンスです。

- [用語](docs/manual/ja.md#terms)と
  [リポジトリ構成](docs/manual/ja.md#repository-layout)
- [ビルドとテスト](docs/manual/ja.md#build-and-test)と
  [デプロイ](docs/manual/ja.md#deploy)
- [Google Home のサインイン](docs/manual/ja.md#google-home-sign-in)と
  [Google Home のデバイス](docs/manual/ja.md#google-home-devices)
- [Tuya のサインイン](docs/manual/ja.md#tuya-sign-in)と
  [テレメトリー](docs/manual/ja.md#telemetry)
- [データベース](docs/manual/ja.md#database)、
  [終了コード](docs/manual/ja.md#exit-codes)、
  [保護デバイス](docs/manual/ja.md#protected-devices)、
  [上流オーバーライド](docs/manual/ja.md#upstream-overrides)
- [Web パネル](docs/manual/ja.md#web-panel)、
  [Docker](docs/manual/ja.md#docker)、
  [セキュリティ](docs/manual/ja.md#security)

## 安全

- 保護デバイスは、`home protect --remove` を実行するまで、どのバックエンドからも
  読み取り専用です。既定では何も保護されていません。書き込みを許してはならない
  デバイスは、明示的に保護する必要があります。コマンドラインの `--force` と
  Web パネルでのパスワード再入力は、キーボードの前にいる人間のための 1 回限りの
  上書きであり、エージェント向けではありません。
- パネルはループバックにバインドし、Tailscale などのプライベートネットワーク
  経由でアクセスしてください。インターネットに公開してはいけません。リバース
  プロキシが TLS 終端を担う場合は、`HOME_COOKIE_SECURE=true` を設定して、
  セッション cookie に `Secure` を付けてください。
- SQLite データベースは家全体と Google アカウントの鍵であり、`home tuya keys`
  はローカルキーをエクスポートします。どちらも機密情報として扱い、git に含めず、
  意図的にバックアップしてください。

[マニュアルのセキュリティ](docs/manual/ja.md#security)に完全な規則があり、
[保護デバイス](docs/manual/ja.md#protected-devices)に上書きの詳細があります。

## ライセンス

MIT、著作権 2026 wray-lee。全文は [LICENSE](LICENSE) にあります。

`crates/ghome-backend` は
[piekstra/google-home-cli](https://github.com/piekstra/google-home-cli)
v0.10.0 のフォークで、`crates/tuya-backend` は `lueur` の `tuya_cloud.rs` を
移植しています。両方の上流プロジェクトは MIT です。ライセンス全文は
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) に収録され、元の追跡コピーは
`crates/ghome-backend/LICENSE-ghome` と `crates/tuya-backend/LICENSE-tuya` に
残っています。コンテナイメージは `LICENSE` と `THIRD_PARTY_NOTICES.md` を
`/usr/share/licenses/home/` に同梱します。
[マニュアルのライセンス](docs/manual/ja.md#license)も参照してください。