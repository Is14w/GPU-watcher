use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, Response, StatusCode},
    routing::get,
    Json, Router,
};
use russh::{
    client,
    keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    ChannelMsg, Disconnect,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    net::TcpListener,
    sync::RwLock,
};
use tokio_util::io::ReaderStream;
use tracing::{error, info, warn};

/// 远端快照流的结束边界标记。
const SNAPSHOT_END_MARKER: &str = "__NOVA_SNAPSHOT_END__";

/// 采集流的空闲看门狗：超过该时长没有任何数据就重建连接。
const STATS_STREAM_IDLE_LIMIT: Duration = Duration::from_secs(25);

const STDERR_TAIL_LIMIT: usize = 600;

/// 长连接采集命令：在远端持续输出带边界标记的快照流，
/// 整个采集会话只发起这一次 exec 请求。
fn stats_stream_command(interval: Duration) -> String {
    format!(
        "command -v nvidia-smi >/dev/null 2>&1 || {{ printf 'nvidia-smi not found on remote host\\n' >&2; exit 127; }}; export LANG=C LC_ALL=C; while :; do nvidia-smi --query-gpu=index,uuid,name,driver_version,pstate,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,fan.speed,clocks.current.graphics,clocks.current.memory --format=csv,noheader,nounits; printf '\\n__NOVA_PROCESSES__\\n'; nvidia-smi --query-compute-apps=gpu_uuid,pid,process_name,used_memory --format=csv,noheader,nounits 2>/dev/null || true; printf '__NOVA_SNAPSHOT_END__\\n'; sleep {:.3}; done",
        interval.as_secs_f64()
    )
}

#[derive(Clone)]
struct CollectorConfig {
    bind: SocketAddr,
    interval: Duration,
    reconnect_max: Duration,
    data_dir: PathBuf,
    api_token: Option<String>,
    target: TargetConfig,
}

#[derive(Clone)]
struct TargetConfig {
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    password: Option<String>,
    private_key_path: Option<String>,
    passphrase: Option<String>,
    expected_fingerprint: Option<String>,
}

#[derive(Clone)]
struct SshTarget {
    config: TargetConfig,
    pinned_fingerprint: Arc<Mutex<Option<String>>>,
    fingerprint_file: PathBuf,
}

struct ClientHandler {
    expected_fingerprint: Option<String>,
    observed_fingerprint: Arc<Mutex<Option<String>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();
        if let Ok(mut observed) = self.observed_fingerprint.lock() {
            *observed = Some(fingerprint.clone());
        }
        Ok(self
            .expected_fingerprint
            .as_deref()
            .map(|expected| expected.trim() == fingerprint)
            .unwrap_or(true))
    }
}

struct SshConnection {
    handle: client::Handle<ClientHandler>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GpuMetric {
    index: u32,
    uuid: String,
    name: String,
    driver_version: String,
    pstate: String,
    temperature: Option<f64>,
    gpu_utilization: Option<f64>,
    memory_utilization: Option<f64>,
    memory_used: Option<f64>,
    memory_total: Option<f64>,
    power_draw: Option<f64>,
    power_limit: Option<f64>,
    fan_speed: Option<f64>,
    graphics_clock: Option<f64>,
    memory_clock: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GpuProcess {
    gpu_uuid: String,
    pid: u32,
    process_name: String,
    used_memory: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    captured_at: u64,
    gpus: Vec<GpuMetric>,
    processes: Vec<GpuProcess>,
}

#[derive(Default)]
struct GpuAggregate {
    index: u32,
    name: String,
    samples: u64,
    utilization_sum: f64,
    utilization_peak: f64,
    active_samples: u64,
    memory_sum: f64,
}

#[derive(Default)]
struct Aggregate {
    first_sample_at: Option<u64>,
    last_sample_at: Option<u64>,
    successful_samples: u64,
    failed_attempts: u64,
    utilization_sum: f64,
    utilization_peak: f64,
    active_samples: u64,
    memory_sum: f64,
    gpus: HashMap<String, GpuAggregate>,
}

struct Runtime {
    status: String,
    connected_at: Option<u64>,
    last_success_at: Option<u64>,
    reconnect_attempt: u64,
    last_error: Option<String>,
    latest: Option<Snapshot>,
    aggregate: Aggregate,
}

#[derive(Clone)]
struct AppState {
    runtime: Arc<RwLock<Runtime>>,
    started_at: u64,
    target_label: String,
    session_file: PathBuf,
    api_token: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthPayload {
    status: String,
    started_at: u64,
    uptime_ms: u64,
    last_success_at: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusPayload {
    status: String,
    target: String,
    started_at: u64,
    uptime_ms: u64,
    connected_at: Option<u64>,
    last_success_at: Option<u64>,
    reconnect_attempt: u64,
    last_error: Option<String>,
    session_file: String,
    latest: Option<Snapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GpuSummary {
    index: u32,
    uuid: String,
    name: String,
    samples: u64,
    average_utilization: f64,
    peak_utilization: f64,
    active_time_percent: f64,
    average_memory_percent: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryPayload {
    started_at: u64,
    uptime_ms: u64,
    first_sample_at: Option<u64>,
    last_sample_at: Option<u64>,
    successful_samples: u64,
    failed_attempts: u64,
    average_utilization: f64,
    peak_utilization: f64,
    active_time_percent: f64,
    average_memory_percent: f64,
    gpus: Vec<GpuSummary>,
}

impl CollectorConfig {
    fn from_env() -> Result<Self, String> {
        let host = required_env("NOVA_TARGET_HOST")?;
        let port = parse_env("NOVA_TARGET_PORT", 22_u16)?;
        let username = required_env("NOVA_TARGET_USER")?;
        let auth_method = optional_env("NOVA_TARGET_AUTH").unwrap_or_else(|| "privateKey".into());
        if auth_method != "privateKey" && auth_method != "password" {
            return Err("NOVA_TARGET_AUTH 只能是 privateKey 或 password".into());
        }
        let password = optional_env("NOVA_TARGET_PASSWORD");
        if auth_method == "password" && password.is_none() {
            return Err("密码认证需要设置 NOVA_TARGET_PASSWORD".into());
        }
        let private_key_path = if auth_method == "privateKey" {
            Some(
                optional_env("NOVA_TARGET_PRIVATE_KEY")
                    .unwrap_or_else(|| "~/.ssh/id_ed25519".into()),
            )
        } else {
            None
        };
        let bind = optional_env("NOVA_BIND")
            .unwrap_or_else(|| "127.0.0.1:17841".into())
            .parse::<SocketAddr>()
            .map_err(|error| format!("NOVA_BIND 无效：{error}"))?;
        let interval_ms = parse_env("NOVA_INTERVAL_MS", 1_000_u64)?.max(250);
        let reconnect_max_seconds = parse_env("NOVA_RECONNECT_MAX_SECONDS", 30_u64)?.max(1);

        Ok(Self {
            bind,
            interval: Duration::from_millis(interval_ms),
            reconnect_max: Duration::from_secs(reconnect_max_seconds),
            data_dir: PathBuf::from(
                optional_env("NOVA_DATA_DIR").unwrap_or_else(|| "./data".into()),
            ),
            api_token: optional_env("NOVA_API_TOKEN"),
            target: TargetConfig {
                host,
                port,
                username,
                auth_method,
                password,
                private_key_path,
                passphrase: optional_env("NOVA_TARGET_PASSPHRASE"),
                expected_fingerprint: optional_env("NOVA_TARGET_FINGERPRINT"),
            },
        })
    }
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn required_env(name: &str) -> Result<String, String> {
    optional_env(name).ok_or_else(|| format!("缺少环境变量 {name}"))
}

fn parse_env<T>(name: &str, default: T) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match optional_env(name) {
        Some(value) => value
            .parse::<T>()
            .map_err(|error| format!("{name} 无效：{error}")),
        None => Ok(default),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}

async fn connect_ssh(target: &SshTarget) -> Result<SshConnection, String> {
    let expected_fingerprint = target
        .pinned_fingerprint
        .lock()
        .map_err(|_| "SSH 指纹锁已损坏".to_string())?
        .clone();
    let observed_fingerprint = Arc::new(Mutex::new(None));
    let handler = ClientHandler {
        expected_fingerprint: expected_fingerprint.clone(),
        observed_fingerprint: observed_fingerprint.clone(),
    };
    let ssh_config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(45)),
        keepalive_interval: Some(Duration::from_secs(10)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    };
    let connection = tokio::time::timeout(
        Duration::from_secs(15),
        client::connect(
            Arc::new(ssh_config),
            (target.config.host.as_str(), target.config.port),
            handler,
        ),
    )
    .await
    .map_err(|_| "SSH 连接超时".to_string())?;
    let mut handle = connection.map_err(|error| {
        let observed = observed_fingerprint
            .lock()
            .ok()
            .and_then(|value| value.clone());
        if let (Some(expected), Some(actual)) =
            (expected_fingerprint.as_deref(), observed.as_deref())
        {
            if expected.trim() != actual {
                return format!("SSH 主机指纹变化：已固定 {expected}，当前 {actual}");
            }
        }
        format!("SSH 连接失败：{error}")
    })?;
    let fingerprint = observed_fingerprint
        .lock()
        .map_err(|_| "无法读取 SSH 主机指纹".to_string())?
        .clone()
        .ok_or_else(|| "SSH 服务端没有返回主机指纹".to_string())?;

    let auth = match target.config.auth_method.as_str() {
        "password" => handle
            .authenticate_password(
                target.config.username.clone(),
                target.config.password.clone().unwrap_or_default(),
            )
            .await
            .map_err(|error| format!("SSH 密码认证失败：{error}"))?,
        "privateKey" => {
            let key_path = expand_home(
                target
                    .config
                    .private_key_path
                    .as_deref()
                    .unwrap_or("~/.ssh/id_ed25519"),
            );
            let key =
                keys::load_secret_key(Path::new(&key_path), target.config.passphrase.as_deref())
                    .map_err(|error| {
                        format!("读取 SSH 私钥 {} 失败：{error}", key_path.display())
                    })?;
            let hash_alg = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|error| format!("协商 RSA 签名算法失败：{error}"))?
                .flatten();
            handle
                .authenticate_publickey(
                    target.config.username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                )
                .await
                .map_err(|error| format!("SSH 私钥认证失败：{error}"))?
        }
        _ => return Err("不支持的 SSH 认证方式".into()),
    };
    if !auth.success() {
        return Err("SSH 认证未通过".into());
    }

    let should_persist = {
        let mut pinned = target
            .pinned_fingerprint
            .lock()
            .map_err(|_| "SSH 指纹锁已损坏".to_string())?;
        if pinned.is_none() {
            *pinned = Some(fingerprint.clone());
            true
        } else {
            false
        }
    };
    if should_persist {
        fs::write(&target.fingerprint_file, format!("{fingerprint}\n"))
            .await
            .map_err(|error| format!("保存 SSH 指纹失败：{error}"))?;
        info!(fingerprint = %fingerprint, "已固定目标服务器 SSH 指纹");
    }

    Ok(SshConnection { handle })
}

async fn close_ssh(connection: SshConnection) {
    let _ = connection
        .handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await;
}

fn parse_number(value: Option<&&str>) -> Option<f64> {
    value
        .map(|item| item.trim())
        .filter(|item| !item.is_empty() && *item != "N/A" && *item != "[N/A]")
        .and_then(|item| item.parse::<f64>().ok())
}

fn parse_snapshot(output: &str) -> Result<Snapshot, String> {
    const MARKER: &str = "__NOVA_PROCESSES__";
    let (gpu_text, process_text) = output.split_once(MARKER).unwrap_or((output, ""));
    let mut gpus = Vec::new();
    for line in gpu_text.lines().filter(|line| !line.trim().is_empty()) {
        let values: Vec<&str> = line.split(',').map(str::trim).collect();
        if values.len() < 15 {
            continue;
        }
        gpus.push(GpuMetric {
            index: values[0]
                .parse::<u32>()
                .map_err(|_| format!("无法解析 GPU 编号：{}", values[0]))?,
            uuid: values[1].to_string(),
            name: values[2].to_string(),
            driver_version: values[3].to_string(),
            pstate: values[4].to_string(),
            temperature: parse_number(values.get(5)),
            gpu_utilization: parse_number(values.get(6)),
            memory_utilization: parse_number(values.get(7)),
            memory_used: parse_number(values.get(8)),
            memory_total: parse_number(values.get(9)),
            power_draw: parse_number(values.get(10)),
            power_limit: parse_number(values.get(11)),
            fan_speed: parse_number(values.get(12)),
            graphics_clock: parse_number(values.get(13)),
            memory_clock: parse_number(values.get(14)),
        });
    }
    if gpus.is_empty() {
        return Err("目标服务器的 nvidia-smi 没有返回 GPU 数据".into());
    }

    let mut processes = Vec::new();
    for line in process_text.lines().filter(|line| !line.trim().is_empty()) {
        let values: Vec<&str> = line.split(',').map(str::trim).collect();
        if values.len() < 4 {
            continue;
        }
        if let Ok(pid) = values[1].parse::<u32>() {
            processes.push(GpuProcess {
                gpu_uuid: values[0].to_string(),
                pid,
                process_name: values[2].to_string(),
                used_memory: parse_number(values.get(3)),
            });
        }
    }
    Ok(Snapshot {
        captured_at: now_millis(),
        gpus,
        processes,
    })
}

/// 把远程采集流的字节增量拼接为完整的快照文本块。
#[derive(Default)]
struct SnapshotAssembler {
    pending: Vec<u8>,
    block: String,
}

impl SnapshotAssembler {
    /// 喂入一段通道数据，返回这段数据补齐的全部完整快照块。
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut blocks = Vec::new();
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = String::from_utf8_lossy(&self.pending[..position]).to_string();
            self.pending.drain(..=position);
            let line = line.trim_end_matches('\r');
            if line == SNAPSHOT_END_MARKER {
                let block = std::mem::take(&mut self.block);
                if !block.trim().is_empty() {
                    blocks.push(block);
                }
            } else {
                self.block.push_str(line);
                self.block.push('\n');
            }
        }
        blocks
    }
}

fn append_stderr_tail(tail: &mut String, data: &[u8]) {
    tail.push_str(&String::from_utf8_lossy(data));
    if tail.len() > STDERR_TAIL_LIMIT {
        let mut cut = tail.len() - STDERR_TAIL_LIMIT;
        while !tail.is_char_boundary(cut) {
            cut += 1;
        }
        tail.drain(..cut);
    }
}

fn stream_ended_message(stderr_tail: &str) -> String {
    let detail = stderr_tail.trim();
    if detail.is_empty() {
        "远程采集流已结束".to_string()
    } else {
        let end = detail
            .char_indices()
            .nth(300)
            .map_or(detail.len(), |(index, _)| index);
        format!("远程采集流已结束：{}", &detail[..end])
    }
}

/// 与桌面端一致的长连接采集：连接后只 exec 一次远程循环命令，
/// 之后持续读取带边界标记的快照流并逐块解析，不再周期性发起 SSH 请求。
async fn run_stats_stream(
    state: &AppState,
    connection: &mut SshConnection,
    interval: Duration,
    received_snapshot: &mut bool,
) -> Result<(), String> {
    let mut channel = connection
        .handle
        .channel_open_session()
        .await
        .map_err(|error| format!("无法创建 SSH 通道：{error}"))?;
    channel
        .exec(true, stats_stream_command(interval).as_bytes())
        .await
        .map_err(|error| format!("远程命令启动失败：{error}"))?;

    let mut assembler = SnapshotAssembler::default();
    let mut stderr_tail = String::new();
    let mut idle = Duration::ZERO;
    loop {
        // wait() 底层是 tokio mpsc 的 recv()，被超时取消不会丢失通道消息。
        match tokio::time::timeout(Duration::from_millis(100), channel.wait()).await {
            Err(_) => {
                idle += Duration::from_millis(100);
                if idle >= STATS_STREAM_IDLE_LIMIT {
                    let _ = channel.close().await;
                    return Err(format!(
                        "采集流超过 {} 秒没有新数据",
                        STATS_STREAM_IDLE_LIMIT.as_secs()
                    ));
                }
            }
            Ok(None) => {
                let _ = channel.close().await;
                return Err(stream_ended_message(&stderr_tail));
            }
            Ok(Some(ChannelMsg::Data { data })) => {
                idle = Duration::ZERO;
                for block in assembler.push(&data) {
                    let snapshot = parse_snapshot(&block).map_err(|error| {
                        let detail = stderr_tail.trim();
                        if detail.is_empty() {
                            error
                        } else {
                            format!("{error}：{detail}")
                        }
                    })?;
                    *received_snapshot = true;
                    match append_snapshot(&state.session_file, &snapshot).await {
                        Ok(()) => {
                            let mut runtime = state.runtime.write().await;
                            record_snapshot(&mut runtime, snapshot);
                        }
                        Err(message) => {
                            error!(error = %message, "原始采样持久化失败，将继续尝试");
                            set_failure(state, message, "degraded").await;
                        }
                    }
                }
            }
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => {
                idle = Duration::ZERO;
                append_stderr_tail(&mut stderr_tail, &data);
            }
            Ok(Some(ChannelMsg::Eof | ChannelMsg::Close)) => {
                let _ = channel.close().await;
                return Err(stream_ended_message(&stderr_tail));
            }
            Ok(Some(_)) => {
                idle = Duration::ZERO;
            }
        }
    }
}

async fn append_snapshot(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let mut line =
        serde_json::to_vec(snapshot).map_err(|error| format!("序列化采样失败：{error}"))?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|error| format!("打开原始采样文件失败：{error}"))?;
    file.write_all(&line)
        .await
        .map_err(|error| format!("写入原始采样失败：{error}"))?;
    file.flush()
        .await
        .map_err(|error| format!("刷新原始采样失败：{error}"))
}

fn record_snapshot(runtime: &mut Runtime, snapshot: Snapshot) {
    let captured_at = snapshot.captured_at;
    let utilization = snapshot
        .gpus
        .iter()
        .map(|gpu| gpu.gpu_utilization.unwrap_or(0.0))
        .sum::<f64>()
        / snapshot.gpus.len() as f64;
    let memory_used = snapshot
        .gpus
        .iter()
        .map(|gpu| gpu.memory_used.unwrap_or(0.0))
        .sum::<f64>();
    let memory_total = snapshot
        .gpus
        .iter()
        .map(|gpu| gpu.memory_total.unwrap_or(0.0))
        .sum::<f64>();
    let memory_percent = if memory_total > 0.0 {
        memory_used / memory_total * 100.0
    } else {
        0.0
    };

    let aggregate = &mut runtime.aggregate;
    aggregate.first_sample_at.get_or_insert(captured_at);
    aggregate.last_sample_at = Some(captured_at);
    aggregate.successful_samples += 1;
    aggregate.utilization_sum += utilization;
    aggregate.utilization_peak = aggregate.utilization_peak.max(utilization);
    aggregate.active_samples += u64::from(utilization >= 5.0);
    aggregate.memory_sum += memory_percent;

    for gpu in &snapshot.gpus {
        let memory_percent = match (gpu.memory_used, gpu.memory_total) {
            (Some(used), Some(total)) if total > 0.0 => used / total * 100.0,
            _ => 0.0,
        };
        let utilization = gpu.gpu_utilization.unwrap_or(0.0);
        let item = aggregate
            .gpus
            .entry(gpu.uuid.clone())
            .or_insert_with(|| GpuAggregate {
                index: gpu.index,
                name: gpu.name.clone(),
                ..Default::default()
            });
        item.samples += 1;
        item.utilization_sum += utilization;
        item.utilization_peak = item.utilization_peak.max(utilization);
        item.active_samples += u64::from(utilization >= 5.0);
        item.memory_sum += memory_percent;
    }

    runtime.latest = Some(snapshot);
    runtime.last_success_at = Some(captured_at);
    runtime.status = "connected".into();
    runtime.last_error = None;
}

async fn set_failure(state: &AppState, message: String, status: &str) {
    let mut runtime = state.runtime.write().await;
    runtime.status = status.into();
    runtime.last_error = Some(message);
    runtime.aggregate.failed_attempts += 1;
}

async fn collector_loop(state: AppState, config: CollectorConfig, target: SshTarget) {
    let mut reconnect_delay = Duration::from_secs(1);
    loop {
        {
            let mut runtime = state.runtime.write().await;
            runtime.status = "connecting".into();
            runtime.reconnect_attempt += 1;
        }
        info!(target = %state.target_label, "正在连接目标 GPU 服务器");
        match connect_ssh(&target).await {
            Ok(mut connection) => {
                {
                    let mut runtime = state.runtime.write().await;
                    runtime.status = "connected".into();
                    runtime.connected_at = Some(now_millis());
                    runtime.last_error = None;
                }
                info!(target = %state.target_label, "SSH 已连接，启动流式采集");
                let mut received_snapshot = false;
                if let Err(message) = run_stats_stream(
                    &state,
                    &mut connection,
                    config.interval,
                    &mut received_snapshot,
                )
                .await
                {
                    warn!(error = %message, "SSH 采集流中断，将重建连接");
                    set_failure(&state, message, "reconnecting").await;
                }
                close_ssh(connection).await;
                if received_snapshot {
                    reconnect_delay = Duration::from_secs(1);
                } else {
                    reconnect_delay = (reconnect_delay * 2).min(config.reconnect_max);
                }
            }
            Err(message) => {
                warn!(error = %message, "SSH 连接失败，将继续重试");
                set_failure(&state, message, "reconnecting").await;
                reconnect_delay = (reconnect_delay * 2).min(config.reconnect_max);
            }
        }
        tokio::time::sleep(reconnect_delay).await;
    }
}

fn authorized(headers: &HeaderMap, state: &AppState) -> Result<(), StatusCode> {
    let Some(token) = state.api_token.as_deref() else {
        return Ok(());
    };
    let expected = format!("Bearer {token}");
    match headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        Some(actual) if actual == expected => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

async fn health(State(state): State<AppState>) -> Json<HealthPayload> {
    let runtime = state.runtime.read().await;
    Json(HealthPayload {
        status: runtime.status.clone(),
        started_at: state.started_at,
        uptime_ms: now_millis().saturating_sub(state.started_at),
        last_success_at: runtime.last_success_at,
    })
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StatusPayload>, StatusCode> {
    authorized(&headers, &state)?;
    let runtime = state.runtime.read().await;
    Ok(Json(StatusPayload {
        status: runtime.status.clone(),
        target: state.target_label.clone(),
        started_at: state.started_at,
        uptime_ms: now_millis().saturating_sub(state.started_at),
        connected_at: runtime.connected_at,
        last_success_at: runtime.last_success_at,
        reconnect_attempt: runtime.reconnect_attempt,
        last_error: runtime.last_error.clone(),
        session_file: state.session_file.display().to_string(),
        latest: runtime.latest.clone(),
    }))
}

async fn summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SummaryPayload>, StatusCode> {
    authorized(&headers, &state)?;
    let runtime = state.runtime.read().await;
    let aggregate = &runtime.aggregate;
    let sample_count = aggregate.successful_samples as f64;
    let mut gpus = aggregate
        .gpus
        .iter()
        .map(|(uuid, gpu)| {
            let samples = gpu.samples as f64;
            GpuSummary {
                index: gpu.index,
                uuid: uuid.clone(),
                name: gpu.name.clone(),
                samples: gpu.samples,
                average_utilization: divide(gpu.utilization_sum, samples),
                peak_utilization: gpu.utilization_peak,
                active_time_percent: divide(gpu.active_samples as f64 * 100.0, samples),
                average_memory_percent: divide(gpu.memory_sum, samples),
            }
        })
        .collect::<Vec<_>>();
    gpus.sort_by_key(|gpu| gpu.index);
    Ok(Json(SummaryPayload {
        started_at: state.started_at,
        uptime_ms: now_millis().saturating_sub(state.started_at),
        first_sample_at: aggregate.first_sample_at,
        last_sample_at: aggregate.last_sample_at,
        successful_samples: aggregate.successful_samples,
        failed_attempts: aggregate.failed_attempts,
        average_utilization: divide(aggregate.utilization_sum, sample_count),
        peak_utilization: aggregate.utilization_peak,
        active_time_percent: divide(aggregate.active_samples as f64 * 100.0, sample_count),
        average_memory_percent: divide(aggregate.memory_sum, sample_count),
        gpus,
    }))
}

fn divide(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        0.0
    }
}

async fn samples(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response<Body>, StatusCode> {
    authorized(&headers, &state)?;
    let body = match fs::File::open(&state.session_file).await {
        Ok(file) => Body::from_stream(ReaderStream::new(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Body::empty(),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nova_collector=info".into()),
        )
        .init();

    let config = CollectorConfig::from_env()
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    fs::create_dir_all(&config.data_dir).await?;
    let started_at = now_millis();
    let session_file = config.data_dir.join(format!("session-{started_at}.ndjson"));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&session_file)
        .await?;

    let fingerprint_file = config.data_dir.join(format!(
        "known-host-{}-{}-{}.txt",
        safe_component(&config.target.username),
        safe_component(&config.target.host),
        config.target.port
    ));
    let saved_fingerprint = if config.target.expected_fingerprint.is_some() {
        config.target.expected_fingerprint.clone()
    } else {
        fs::read_to_string(&fingerprint_file)
            .await
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let target = SshTarget {
        config: config.target.clone(),
        pinned_fingerprint: Arc::new(Mutex::new(saved_fingerprint)),
        fingerprint_file,
    };
    let target_label = format!(
        "{}@{}:{}",
        config.target.username, config.target.host, config.target.port
    );
    let state = AppState {
        runtime: Arc::new(RwLock::new(Runtime {
            status: "starting".into(),
            connected_at: None,
            last_success_at: None,
            reconnect_attempt: 0,
            last_error: None,
            latest: None,
            aggregate: Aggregate::default(),
        })),
        started_at,
        target_label,
        session_file,
        api_token: config.api_token.clone(),
    };

    if !config.bind.ip().is_loopback() && config.api_token.is_none() {
        warn!(bind = %config.bind, "API 正在非回环地址监听且未配置 NOVA_API_TOKEN");
    }
    info!(bind = %config.bind, target = %state.target_label, file = %state.session_file.display(), "NOVA Collector 已启动");

    let collector = tokio::spawn(collector_loop(state.clone(), config.clone(), target));
    let app = Router::new()
        .route("/", get(|| async { "NOVA Collector" }))
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/api/session/summary", get(summary))
        .route("/api/session/samples", get(samples))
        .with_state(state);
    let listener = TcpListener::bind(config.bind).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    collector.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gpu_and_process_rows() {
        let output = "0, GPU-1, NVIDIA RTX, 555.1, P2, 61, 72, 41, 2048, 24576, 180, 300, 45, 1900, 9000\n__NOVA_PROCESSES__\nGPU-1, 123, python, 1024\n";
        let snapshot = parse_snapshot(output).expect("snapshot should parse");
        assert_eq!(snapshot.gpus.len(), 1);
        assert_eq!(snapshot.gpus[0].gpu_utilization, Some(72.0));
        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].pid, 123);
    }

    #[test]
    fn snapshot_assembler_joins_chunks_into_blocks() {
        let mut assembler = SnapshotAssembler::default();
        assert!(assembler
            .push(b"0, GPU-1, NVIDIA RTX, 555.1, P2, 61, 72, 41, 2048, 24576, 180, 300, 45, 1900, 9000\r\n__NOVA_PRO")
            .is_empty());
        let blocks = assembler.push(
            b"CESSES__\nGPU-1, 123, python, 1024\n__NOVA_SNAPSHOT_END__\n0, GPU-1, NVIDIA RTX",
        );
        assert_eq!(blocks.len(), 1);
        let snapshot = parse_snapshot(&blocks[0]).expect("snapshot should parse");
        assert_eq!(snapshot.gpus.len(), 1);
        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].pid, 123);
    }

    #[test]
    fn aggregate_keeps_complete_statistics() {
        let mut runtime = Runtime {
            status: "connected".into(),
            connected_at: None,
            last_success_at: None,
            reconnect_attempt: 0,
            last_error: None,
            latest: None,
            aggregate: Aggregate::default(),
        };
        for utilization in [10.0, 80.0] {
            record_snapshot(
                &mut runtime,
                Snapshot {
                    captured_at: utilization as u64,
                    gpus: vec![GpuMetric {
                        index: 0,
                        uuid: "GPU-1".into(),
                        name: "RTX".into(),
                        driver_version: "555".into(),
                        pstate: "P2".into(),
                        temperature: None,
                        gpu_utilization: Some(utilization),
                        memory_utilization: None,
                        memory_used: Some(5.0),
                        memory_total: Some(10.0),
                        power_draw: None,
                        power_limit: None,
                        fan_speed: None,
                        graphics_clock: None,
                        memory_clock: None,
                    }],
                    processes: Vec::new(),
                },
            );
        }
        assert_eq!(runtime.aggregate.successful_samples, 2);
        assert_eq!(runtime.aggregate.utilization_sum / 2.0, 45.0);
        assert_eq!(runtime.aggregate.utilization_peak, 80.0);
        assert_eq!(runtime.aggregate.active_samples, 2);
        assert_eq!(runtime.aggregate.memory_sum / 2.0, 50.0);
    }
}
