# NOVA Collector

这是 NOVA Watch 的无界面单目标采集服务。服务部署在监控机或中转服务器上，通过 SSH 访问目标 GPU 服务器，并在远端执行 `nvidia-smi`。采集使用单条 SSH 长连接：连接后只发起一次远程循环命令，之后持续读取带边界标记的快照流，不在每个采样周期重复发起 SSH 请求。采集服务本身不需要 NVIDIA 驱动，也不需要部署在 GPU 机器上。

## 行为

- SSH 每 10 秒发送保活，连续 3 次无响应后重建连接；
- 采集流超过 25 秒没有任何新数据时重建连接；
- 建连、认证、采集或网络临时失败后无限重试，退避上限由 `NOVA_RECONNECT_MAX_SECONDS` 控制；
- 第一次未配置指纹时采用 TOFU，将服务器指纹写入数据目录，之后严格校验；
- 每次服务进程启动创建一个 `session-<时间戳>.ndjson`，逐行保存完整原始快照；
- HTTP API 提供连接状态、最新快照、本次进程运行总结和完整 NDJSON；
- 密码、私钥口令只从本机 `.env` / systemd 环境文件读取，不写入采样文件或日志。

## 构建

在项目根目录执行：

```bash
cargo build --release --manifest-path collector/Cargo.toml
```

产物位于：

```text
collector/target/release/nova-collector
```

## 本地运行

```bash
cd collector
cp .env.example .env
# 编辑 .env
cargo run --release
```

目标机需允许 SSH 登录，并可在非交互 Shell 中执行 `nvidia-smi`。私钥路径相对于运行服务的系统用户；默认是 `~/.ssh/id_ed25519`。

## API

```text
GET /health
GET /api/status
GET /api/session/summary
GET /api/session/samples
```

设置了 `NOVA_API_TOKEN` 时：

```bash
curl -H "Authorization: Bearer $NOVA_API_TOKEN" \
  http://127.0.0.1:17841/api/session/summary
```

`/api/session/samples` 返回 `application/x-ndjson`，内容与本次启动对应的原始文件完全一致。默认只监听回环地址；若改为 `0.0.0.0`，应同时设置 API token，并通过防火墙或反向代理限制访问。

## systemd

推荐布局：

```text
/opt/nova-collector/nova-collector
/etc/nova-collector.env
/var/lib/nova-collector/
```

将示例 unit 放到 `/etc/systemd/system/nova-collector.service`，并在环境文件中设置：

```dotenv
NOVA_DATA_DIR=/var/lib/nova-collector
```

然后执行：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now nova-collector
sudo systemctl status nova-collector
journalctl -u nova-collector -f
```
