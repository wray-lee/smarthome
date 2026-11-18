# home

[English](README.md) | 简体中文 | [日本語](README.ja.md)

[![CI](https://github.com/wray-lee/smarthome/actions/workflows/ci.yml/badge.svg)](https://github.com/wray-lee/smarthome/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`home` 用一个 Rust 二进制程序和一个网页面板管理 Google Home 与 Tuya 设备：
每条命令都向 stdout 写出一份 JSON 文档，退出码说明执行结果。

![面板](docs/images/panel-1280-light.png)

[390 px 宽度、深色主题下的同一个面板](docs/images/panel-390-dark.png)

## 功能

一个二进制程序背后是两个相互独立的后端：

- **Google Home**（`home gh`）通过非官方 foyer API 操作 Google Home 中的任意
  设备，代码分叉自 `piekstra/google-home-cli` v0.10.0。它向工作区其余部分
  只暴露 `list_devices`、`get_state` 和 `set_trait`，这也是将来实现
  Home MCP 的接缝。
- **Tuya**（`home tuya`）直接与 Tuya 通信：扫码登录、设备列表、本地密钥备份、
  云端控制与遥测。

两个后端是彼此独立的 crate，从不互相依赖。同一个物理插座可能在两个后端中各
显示为一个设备，名称与能力都不相同。命令行是为智能体和脚本设计的；网页面板
把同一套命令层交给人使用，安全规则完全一致。

[手册术语表](docs/manual/zh-CN.md#terms)界定了后端、设备、受保护设备、敏感
操作、遥测、本地密钥等术语。

## 快速开始

0.3.0 发布镜像为 `ghcr.io/wray-lee/smarthome:0.3.0`。编写如下
`compose.yaml`：

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

`HOME_PASSWORD` 是必填项。把它放进 `compose.yaml` 同目录下的 `.env` 文件，
权限设为 600，且不要提交到 git：

```dotenv
HOME_PASSWORD='{panel_password}'
```

然后启动容器：

```console
chmod 600 .env
docker compose up -d
```

在容器内把两个后端各登录一次：

```console
docker exec -it home home gh login --email you@example.com
docker exec -it home home tuya login
```

打开 http://127.0.0.1:8080，用同一个密码登录。

默认情况下没有任何设备受保护。每个必须保持只读的设备都要用
`docker exec -i home home protect tuya:{device_id}`（或把同一命令中的前缀换成
`gh:{device_id}`）显式加入保护；只有 `home protect --remove` 能解除保护。

[手册快速开始](docs/manual/zh-CN.md#quick-start)补充了密码文件流程、两种非
交互式登录回退方案和健康检查。

## 从源码构建

在 Linux 或 WSL 中构建。发布二进制采用静态链接（musl），因此可以在任何常见
Linux 发行版或 Alpine 上运行：

```console
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

[手册的构建与测试章节](docs/manual/zh-CN.md#build-and-test)涵盖 WSL 下
`CARGO_HOME` 与 `CARGO_TARGET_DIR` 的配置、离线构建与 vendor，以及
`scripts/image-smoke.sh`。

## 文档

[手册](docs/manual/zh-CN.md#contents)是二进制、后端和面板的完整参考：

- [术语](docs/manual/zh-CN.md#terms)和
  [仓库结构](docs/manual/zh-CN.md#repository-layout)
- [构建与测试](docs/manual/zh-CN.md#build-and-test)和
  [部署](docs/manual/zh-CN.md#deploy)
- [Google Home 登录](docs/manual/zh-CN.md#google-home-sign-in)和
  [Google Home 设备](docs/manual/zh-CN.md#google-home-devices)
- [Tuya 登录](docs/manual/zh-CN.md#tuya-sign-in)和
  [遥测](docs/manual/zh-CN.md#telemetry)
- [数据库](docs/manual/zh-CN.md#database)、
  [退出码](docs/manual/zh-CN.md#exit-codes)、
  [受保护设备](docs/manual/zh-CN.md#protected-devices)和
  [上游覆盖](docs/manual/zh-CN.md#upstream-overrides)
- [网页面板](docs/manual/zh-CN.md#web-panel)、
  [Docker](docs/manual/zh-CN.md#docker)和
  [安全](docs/manual/zh-CN.md#security)

## 安全

- 受保护设备经任何后端都只读，直到执行 `home protect --remove`。默认没有
  任何设备受保护；每个不允许写入的设备都需要显式保护。命令行上的
  `--force` 和面板的密码重输是给键盘前的人用的一次性写入覆盖，不适用于
  智能体。
- 把面板绑定到回环地址，并通过 Tailscale 或其他私有网络访问。绝不要把它
  暴露到互联网。如果反向代理负责 TLS 终止，设置
  `HOME_COOKIE_SECURE=true`，让会话 cookie 带上 `Secure`。
- SQLite 数据库是整个家庭和 Google 账号的总钥匙，`home tuya keys` 会导出
  本地密钥。两者都应按机密对待，留在 git 之外，并有意备份。

[手册安全章节](docs/manual/zh-CN.md#security)给出完整规则；
[受保护设备](docs/manual/zh-CN.md#protected-devices)说明覆盖的细节。

## 许可证

MIT，版权所有 2026 wray-lee；完整文本见 [LICENSE](LICENSE)。

`crates/ghome-backend` 分叉自
[piekstra/google-home-cli](https://github.com/piekstra/google-home-cli)
v0.10.0，`crates/tuya-backend` 移植了 `lueur` 的 `tuya_cloud.rs`。
两个上游项目均为 MIT。它们的许可证全文收录在
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)；原始跟踪副本保留在
`crates/ghome-backend/LICENSE-ghome` 和 `crates/tuya-backend/LICENSE-tuya`；
容器镜像在 `/usr/share/licenses/home/` 下附带 `LICENSE` 和
`THIRD_PARTY_NOTICES.md`。参见
[手册许可证章节](docs/manual/zh-CN.md#license)。