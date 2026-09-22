# NOVA Watch

NOVA Watch 是一个基于 Tauri 2 + Vue 3 的桌面端 NVIDIA GPU 监控工具。它通过 SSH 连接远程 GPU 服务器，持续读取 `nvidia-smi` 的 GPU 与进程数据，并在本地提供主机管理、实时监控、会话统计和交互式 `nvitop` 终端。

项目同时提供一个可选的无界面采集器 `nova-collector`。采集器适合部署在中转机、监控机或服务器上，以 systemd 服务持续采集数据并通过 HTTP API 提供给其他系统使用。

## 功能

- 多台远程主机管理，可手动添加，也可从 `~/.ssh/config` 导入
- 支持 SSH 密码和 SSH 私钥认证
- 首次连接自动记录服务器指纹（TOFU），后续连接校验指纹变化
- 实时查看 GPU 型号、温度、利用率、显存、功耗、风扇和时钟等指标
- 查看 GPU 上运行的计算进程及显存占用
- 记录单次监控会话，展示利用率、显存和运行时长趋势
- 通过 SSH 终端启动远程 `nvitop`，并支持断线重连
- 本地保存主机配置；密码和私钥口令与主机配置分开保存
- 可选的无界面采集器，支持 NDJSON 原始采样文件和 HTTP API

## 工作方式

桌面端不要求远程服务器安装额外 Agent，只要 SSH 用户能够在非交互 Shell 中执行 `nvidia-smi` 即可。

```text
┌────────────────────┐       SSH        ┌──────────────────────────┐
│ NOVA Watch Desktop │ ───────────────▶ │ 远程 NVIDIA GPU 服务器   │
│ Tauri + Vue        │ ◀─────────────── │ nvidia-smi / nvitop      │
└────────────────────┘                  └──────────────────────────┘

可选：
┌────────────────────┐       SSH        ┌──────────────────────────┐
│ nova-collector     │ ───────────────▶ │ 远程 NVIDIA GPU 服务器   │
│ headless + HTTP API│                  │                          │
└────────────────────┘
```

## 环境要求

开发桌面端需要：

- Node.js 18 或更高版本
- npm
- Rust stable toolchain
- Tauri 2 的系统依赖（Windows、macOS、Linux 请参考 [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)）

被监控的远程服务器需要：

- 可访问的 SSH 服务
- 可用的 `nvidia-smi`
- 如需使用交互式终端，需要安装 `nvitop`

## 开发桌面端

在项目根目录执行：

```bash
npm install
npm run tauri dev
```

`npm run dev` 只启动 Vite 前端开发服务器；完整的 SSH、系统密钥和 Tauri 命令调用需要在 Tauri 桌面窗口中运行。

常用命令：

```bash
# 类型检查并构建前端资源
npm run build

# 构建 Tauri 安装包/可执行文件
npm run tauri build

# 仅预览 Vite 构建产物
npm run preview
```

启动应用后，点击添加主机，填写主机地址、SSH 端口、用户名和认证方式。使用私钥认证时，私钥路径默认是 `~/.ssh/id_ed25519`；首次连接成功后请核对并确认服务器指纹。

## 使用 `nova-collector`

采集器位于 [`collector/`](collector/)，它通过 SSH 连接目标 GPU 服务器，周期性执行 `nvidia-smi`，将原始采样保存为 NDJSON，并提供状态查询 API。

### 本地构建与运行

```bash
# 在项目根目录执行
cargo build --release --manifest-path collector/Cargo.toml

cd collector
cp .env.example .env
# 编辑 .env 中的目标主机、认证方式和数据目录
cargo run --release
```

Windows PowerShell 可以使用：

```powershell
Copy-Item .env.example .env
cargo run --release
```

### 主要配置

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `NOVA_TARGET_HOST` | 必填 | 目标 GPU 服务器地址 |
| `NOVA_TARGET_PORT` | `22` | SSH 端口 |
| `NOVA_TARGET_USER` | 必填 | SSH 用户名 |
| `NOVA_TARGET_AUTH` | `privateKey` | `privateKey` 或 `password` |
| `NOVA_TARGET_PRIVATE_KEY` | `~/.ssh/id_ed25519` | SSH 私钥路径 |
| `NOVA_TARGET_PASSWORD` | 空 | 密码认证时使用 |
| `NOVA_TARGET_PASSPHRASE` | 空 | 加密私钥的口令 |
| `NOVA_TARGET_FINGERPRINT` | 空 | 预置 SSH 指纹；为空时首次连接自动固定 |
| `NOVA_INTERVAL_MS` | `1000` | 采样间隔（毫秒） |
| `NOVA_RECONNECT_MAX_SECONDS` | `30` | 重连退避上限 |
| `NOVA_DATA_DIR` | `./data` | NDJSON 与指纹文件目录 |
| `NOVA_BIND` | `127.0.0.1:17841` | HTTP API 监听地址 |
| `NOVA_API_TOKEN` | 空 | 设置后 `/api/*` 需要 Bearer Token |
| `RUST_LOG` | `nova_collector=info` | 日志级别 |

完整示例见 [`collector/.env.example`](collector/.env.example)。

### HTTP API

采集器默认只监听回环地址：

```text
GET /health
GET /api/status
GET /api/session/summary
GET /api/session/samples
```

如果配置了 `NOVA_API_TOKEN`，访问 `/api/*` 时需要携带：

```bash
curl -H "Authorization: Bearer $NOVA_API_TOKEN" \
  http://127.0.0.1:17841/api/session/summary
```

`/api/session/samples` 返回 `application/x-ndjson`，内容对应当前采集会话的原始采样文件。

### systemd 部署

项目提供了示例 unit 文件 [`collector/nova-collector.service`](collector/nova-collector.service)。推荐的部署布局如下：

```text
/opt/nova-collector/nova-collector
/etc/nova-collector.env
/var/lib/nova-collector/
```

安装并启动服务：

```bash
sudo cp collector/nova-collector.service /etc/systemd/system/nova-collector.service
sudo systemctl daemon-reload
sudo systemctl enable --now nova-collector
sudo systemctl status nova-collector
journalctl -u nova-collector -f
```

使用 systemd 时，请在 `/etc/nova-collector.env` 中将 `NOVA_DATA_DIR` 设置为 `/var/lib/nova-collector`，并确认服务用户对该目录具有写权限。

## 数据与安全

- `.env`、运行数据、构建产物和依赖目录不会被提交，相关规则见 [`.gitignore`](.gitignore)。
- 桌面端只将主机连接信息保存在本地；密码和私钥口令写入 Tauri 应用数据目录下的本地凭据文件，不会上传到本项目或远程服务器。
- 首次 SSH 连接采用 TOFU 策略记录指纹；生产环境建议通过 `expectedFingerprint` 或 `NOVA_TARGET_FINGERPRINT` 预先固定指纹。
- 采集器默认绑定 `127.0.0.1`。如果必须监听 `0.0.0.0`，请同时配置 `NOVA_API_TOKEN`，并使用防火墙或反向代理限制来源。
- 不要将真实密码、私钥、API Token 或生产环境 `.env` 文件提交到 Git。

## 项目结构

```text
.
├── src/                       # Vue 3 前端界面
├── src-tauri/                 # Tauri 桌面端与 SSH 采集逻辑
├── collector/                 # 无界面 Rust 采集器与 HTTP API
├── package.json               # 前端与 Tauri 命令
└── vite.config.ts
```

## 验证

当前项目可使用以下命令进行基础验证：

```bash
npm run build
cargo check --locked --manifest-path src-tauri/Cargo.toml
cargo check --locked --manifest-path collector/Cargo.toml
```

## 许可证

本仓库当前未附带许可证文件。如需在其他项目中再发布或商用，请先联系仓库所有者确认授权条款。
