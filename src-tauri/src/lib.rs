use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
use russh::{
    client,
    keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    ChannelMsg, Disconnect,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SshTarget {
    id: String,
    #[allow(dead_code)]
    name: String,
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    password: Option<String>,
    private_key_path: Option<String>,
    passphrase: Option<String>,
    expected_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeSecret {
    password: Option<String>,
    passphrase: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CredentialUpdate {
    id: String,
    password: Option<String>,
    passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionReport {
    fingerprint: String,
    nvidia_smi_available: bool,
    nvitop_available: bool,
    hostname: String,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GpuProcess {
    gpu_uuid: String,
    pid: u32,
    process_name: String,
    used_memory: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    captured_at: u64,
    gpus: Vec<GpuMetric>,
    processes: Vec<GpuProcess>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorEvent {
    target_id: String,
    run_id: String,
    kind: String,
    snapshot: Option<Snapshot>,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalEvent {
    target_id: String,
    run_id: String,
    kind: String,
    data: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SshConfigHost {
    alias: String,
    host: String,
    port: u16,
    username: String,
    identity_file: Option<String>,
}

struct MonitorControl {
    run_id: String,
    stop: Arc<AtomicBool>,
}

enum TerminalCommand {
    Input(Vec<u8>),
    Resize(u32, u32),
    Stop,
}

struct TerminalControl {
    run_id: String,
    sender: mpsc::Sender<TerminalCommand>,
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
    fingerprint: String,
}

#[derive(Default)]
struct RuntimeState {
    monitors: Mutex<HashMap<String, MonitorControl>>,
    terminals: Mutex<HashMap<String, TerminalControl>>,
    credentials: Mutex<()>,
    credential_revision: AtomicU64,
}

impl Drop for RuntimeState {
    fn drop(&mut self) {
        if let Ok(monitors) = self.monitors.lock() {
            for control in monitors.values() {
                control.stop.store(true, Ordering::Relaxed);
            }
        }
        if let Ok(terminals) = self.terminals.lock() {
            for control in terminals.values() {
                let _ = control.sender.send(TerminalCommand::Stop);
            }
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(any())]
fn host_fingerprint(session: &Session) -> Result<String, String> {
    let (key, _) = session
        .host_key()
        .ok_or_else(|| "服务器没有返回主机密钥".to_string())?;
    let digest = Sha256::digest(key);
    Ok(format!("SHA256:{}", STANDARD_NO_PAD.encode(digest)))
}

#[cfg(any())]
fn connect_ssh(target: &SshTarget) -> Result<(Session, String), String> {
    let mut addresses = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map_err(|error| format!("无法解析主机地址：{error}"))?;
    let address = addresses
        .next()
        .ok_or_else(|| "没有找到可用的主机地址".to_string())?;
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|error| format!("SSH 连接失败：{error}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(15))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(15))).ok();

    let mut session = Session::new().map_err(|error| format!("无法创建 SSH 会话：{error}"))?;
    session.set_tcp_stream(tcp);
    session.set_timeout(15_000);
    session
        .handshake()
        .map_err(|error| format!("SSH 握手失败：{error}"))?;

    let fingerprint = host_fingerprint(&session)?;
    if let Some(expected) = target
        .expected_fingerprint
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        if expected != fingerprint {
            return Err(format!(
                "主机指纹已变化。已保存 {expected}，当前 {fingerprint}。为避免中间人攻击，连接已中止"
            ));
        }
    }

    match target.auth_method.as_str() {
        "password" => {
            let password = target
                .password
                .as_deref()
                .ok_or_else(|| "请输入 SSH 密码".to_string())?;
            session
                .userauth_password(&target.username, password)
                .map_err(|error| format!("密码认证失败：{error}"))?;
        }
        "privateKey" => {
            let key_path = target
                .private_key_path
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "请选择私钥文件".to_string())?;
            session
                .userauth_pubkey_file(
                    &target.username,
                    None,
                    Path::new(key_path),
                    target
                        .passphrase
                        .as_deref()
                        .filter(|value| !value.is_empty()),
                )
                .map_err(|error| format!("密钥认证失败：{error}"))?;
        }
        _ => return Err("不支持的认证方式".to_string()),
    }

    if !session.authenticated() {
        return Err("SSH 认证未通过".to_string());
    }
    session.keepalive_config(true, 20);
    Ok((session, fingerprint))
}

#[cfg(any())]
fn run_remote(session: &Session, command: &str) -> Result<String, String> {
    let mut channel = session
        .channel_session()
        .map_err(|error| format!("无法创建 SSH 通道：{error}"))?;
    channel
        .exec(command)
        .map_err(|error| format!("远程命令启动失败：{error}"))?;

    let mut stdout = String::new();
    let mut stderr = String::new();
    channel
        .read_to_string(&mut stdout)
        .map_err(|error| format!("读取远程输出失败：{error}"))?;
    channel
        .stderr()
        .read_to_string(&mut stderr)
        .map_err(|error| format!("读取远程错误输出失败：{error}"))?;
    channel.wait_close().ok();
    let status = channel.exit_status().unwrap_or(-1);
    if status != 0 {
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(format!("远程命令退出码 {status}：{detail}"));
    }
    Ok(stdout)
}

async fn connect_ssh_async(target: &SshTarget) -> Result<SshConnection, String> {
    let observed_fingerprint = Arc::new(Mutex::new(None));
    let handler = ClientHandler {
        expected_fingerprint: target
            .expected_fingerprint
            .clone()
            .filter(|value| !value.trim().is_empty()),
        observed_fingerprint: observed_fingerprint.clone(),
    };
    let config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(45)),
        keepalive_interval: Some(Duration::from_secs(10)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    };
    let connection = tokio::time::timeout(
        Duration::from_secs(15),
        client::connect(
            Arc::new(config),
            (target.host.as_str(), target.port),
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
        if let (Some(expected), Some(actual)) = (
            target.expected_fingerprint.as_deref(),
            observed.as_deref(),
        ) {
            if expected.trim() != actual {
                return format!(
                    "主机指纹已变化。已保存 {expected}，当前为 {actual}。为避免中间人攻击，连接已中止"
                );
            }
        }
        format!("SSH 连接失败：{error}")
    })?;

    let fingerprint = observed_fingerprint
        .lock()
        .map_err(|_| "无法读取 SSH 主机指纹".to_string())?
        .clone()
        .ok_or_else(|| "SSH 服务端没有返回主机指纹".to_string())?;

    let auth = match target.auth_method.as_str() {
        "password" => {
            let password = target
                .password
                .as_deref()
                .ok_or_else(|| "请输入 SSH 密码".to_string())?;
            handle
                .authenticate_password(target.username.clone(), password.to_string())
                .await
                .map_err(|error| format!("密码认证失败：{error}"))?
        }
        "privateKey" => {
            let key_path = target
                .private_key_path
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "请选择私钥文件".to_string())?;
            let key_path = expand_home(key_path);
            let key = keys::load_secret_key(
                Path::new(&key_path),
                target
                    .passphrase
                    .as_deref()
                    .filter(|value| !value.is_empty()),
            )
            .map_err(|error| format!("读取私钥失败：{error}"))?;
            let hash_alg = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|error| format!("协商 RSA 签名算法失败：{error}"))?
                .flatten();
            handle
                .authenticate_publickey(
                    target.username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                )
                .await
                .map_err(|error| format!("密钥认证失败：{error}"))?
        }
        _ => return Err("不支持的认证方式".to_string()),
    };

    if !auth.success() {
        return Err("SSH 认证未通过".to_string());
    }
    Ok(SshConnection {
        handle,
        fingerprint,
    })
}

async fn close_ssh_async(session: SshConnection) {
    let _ = session
        .handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await;
}

async fn run_remote_async(session: &mut SshConnection, command: &str) -> Result<String, String> {
    let mut channel = session
        .handle
        .channel_open_session()
        .await
        .map_err(|error| format!("无法创建 SSH 通道：{error}"))?;
    channel
        .exec(true, command.as_bytes())
        .await
        .map_err(|error| format!("远程命令启动失败：{error}"))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut status = None;
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    let _ = channel.close().await;
    let stdout = String::from_utf8_lossy(&stdout).to_string();
    let exit_code = status.unwrap_or(0);
    if exit_code != 0 {
        let detail = if stderr.is_empty() {
            stdout.trim().to_string()
        } else {
            String::from_utf8_lossy(&stderr).trim().to_string()
        };
        return Err(format!("远程命令退出码 {}：{detail}", exit_code));
    }
    Ok(stdout)
}

fn parse_number(value: Option<&&str>) -> Option<f64> {
    value
        .map(|item| item.trim())
        .filter(|item| !item.is_empty() && *item != "N/A" && *item != "[N/A]")
        .and_then(|item| item.parse::<f64>().ok())
}

fn parse_snapshot(output: &str) -> Result<Snapshot, String> {
    const MARKER: &str = "__GPU_WATCHER_PROCESSES__";
    let (gpu_text, process_text) = output.split_once(MARKER).unwrap_or((output, ""));
    let mut gpus = Vec::new();
    for line in gpu_text.lines().filter(|line| !line.trim().is_empty()) {
        let values: Vec<&str> = line.split(',').map(str::trim).collect();
        if values.len() < 15 {
            continue;
        }
        let index = values[0]
            .parse::<u32>()
            .map_err(|_| format!("无法解析 GPU 编号：{}", values[0]))?;
        gpus.push(GpuMetric {
            index,
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
        return Err("nvidia-smi 没有返回 GPU 数据".to_string());
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

async fn collect_snapshot(session: &mut SshConnection) -> Result<Snapshot, String> {
    let command = "LANG=C nvidia-smi --query-gpu=index,uuid,name,driver_version,pstate,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,fan.speed,clocks.current.graphics,clocks.current.memory --format=csv,noheader,nounits; printf '\\n__GPU_WATCHER_PROCESSES__\\n'; LANG=C nvidia-smi --query-compute-apps=gpu_uuid,pid,process_name,used_memory --format=csv,noheader,nounits 2>/dev/null || true";
    parse_snapshot(&run_remote_async(session, command).await?)
}

fn emit_monitor(
    app: &AppHandle,
    target_id: &str,
    run_id: &str,
    kind: &str,
    snapshot: Option<Snapshot>,
    message: Option<String>,
) {
    let _ = app.emit(
        "monitor-update",
        MonitorEvent {
            target_id: target_id.to_string(),
            run_id: run_id.to_string(),
            kind: kind.to_string(),
            snapshot,
            message,
        },
    );
}

fn emit_terminal(
    app: &AppHandle,
    target_id: &str,
    run_id: &str,
    kind: &str,
    data: Option<String>,
    message: Option<String>,
) {
    let _ = app.emit(
        "terminal-update",
        TerminalEvent {
            target_id: target_id.to_string(),
            run_id: run_id.to_string(),
            kind: kind.to_string(),
            data,
            message,
        },
    );
}

#[tauri::command]
async fn test_connection(target: SshTarget) -> Result<ConnectionReport, String> {
    // Testing is the trust-on-first-use step. Existing fingerprints are still enforced.
    let mut session = connect_ssh_async(&target).await?;
    let fingerprint = session.fingerprint.clone();
    let output = run_remote_async(
        &mut session,
        "printf '%s\\n' \"$(hostname 2>/dev/null || echo unknown)\"; command -v nvidia-smi >/dev/null && echo NVIDIA_SMI_OK || true; command -v nvitop >/dev/null && echo NVITOP_OK || true",
    )
    .await?;
    let mut lines = output.lines();
    let hostname = lines.next().unwrap_or("unknown").trim().to_string();
    close_ssh_async(session).await;
    Ok(ConnectionReport {
        fingerprint,
        nvidia_smi_available: output.contains("NVIDIA_SMI_OK"),
        nvitop_available: output.contains("NVITOP_OK"),
        hostname,
    })
}

#[tauri::command]
fn start_monitor(
    app: AppHandle,
    state: State<RuntimeState>,
    target: SshTarget,
    run_id: String,
) -> Result<(), String> {
    let target_id = target.id.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let mut monitors = state
        .monitors
        .lock()
        .map_err(|_| "监控状态锁已损坏".to_string())?;
    if let Some(previous) = monitors.insert(
        target_id.clone(),
        MonitorControl {
            run_id: run_id.clone(),
            stop: stop.clone(),
        },
    ) {
        previous.stop.store(true, Ordering::Relaxed);
    }
    drop(monitors);

    thread::spawn(move || {
        let mut reconnect_delay = Duration::from_secs(1);
        while !stop.load(Ordering::Relaxed) {
            emit_monitor(&app, &target_id, &run_id, "connecting", None, None);
            match tauri::async_runtime::block_on(connect_ssh_async(&target)) {
                Ok(mut session) => {
                    if stop.load(Ordering::Relaxed) {
                        tauri::async_runtime::block_on(close_ssh_async(session));
                        break;
                    }
                    emit_monitor(&app, &target_id, &run_id, "connected", None, None);
                    let mut received_snapshot = false;
                    while !stop.load(Ordering::Relaxed) {
                        match tauri::async_runtime::block_on(collect_snapshot(&mut session)) {
                            Ok(snapshot) => {
                                received_snapshot = true;
                                reconnect_delay = Duration::from_secs(1);
                                if !stop.load(Ordering::Relaxed) {
                                    emit_monitor(
                                        &app,
                                        &target_id,
                                        &run_id,
                                        "snapshot",
                                        Some(snapshot),
                                        None,
                                    );
                                }
                            }
                            Err(error) => {
                                if !stop.load(Ordering::Relaxed) {
                                    emit_monitor(
                                        &app,
                                        &target_id,
                                        &run_id,
                                        "error",
                                        None,
                                        Some(error),
                                    );
                                }
                                break;
                            }
                        }
                        for _ in 0..10 {
                            if stop.load(Ordering::Relaxed) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                    tauri::async_runtime::block_on(close_ssh_async(session));
                    if !received_snapshot {
                        reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(30));
                    }
                }
                Err(error) => {
                    if !stop.load(Ordering::Relaxed) {
                        emit_monitor(&app, &target_id, &run_id, "error", None, Some(error));
                    }
                    reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(30));
                }
            }

            let retry_ticks = reconnect_delay.as_millis().div_ceil(100) as usize;
            for _ in 0..retry_ticks {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
        emit_monitor(&app, &target_id, &run_id, "stopped", None, None);
        if let Ok(mut monitors) = app.state::<RuntimeState>().monitors.lock() {
            if monitors
                .get(&target_id)
                .is_some_and(|control| control.run_id == run_id)
            {
                monitors.remove(&target_id);
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn stop_monitor(
    state: State<RuntimeState>,
    target_id: String,
    run_id: String,
) -> Result<(), String> {
    let mut monitors = state
        .monitors
        .lock()
        .map_err(|_| "监控状态锁已损坏".to_string())?;
    if monitors
        .get(&target_id)
        .is_some_and(|control| control.run_id == run_id)
    {
        if let Some(control) = monitors.remove(&target_id) {
            control.stop.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

#[cfg(any())]
#[tauri::command]
fn start_terminal(
    app: AppHandle,
    state: State<RuntimeState>,
    target: SshTarget,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let target_id = target.id.clone();
    let (sender, receiver) = mpsc::channel::<TerminalCommand>();

    if let Some(previous) = state
        .terminals
        .lock()
        .map_err(|_| "终端状态锁已损坏".to_string())?
        .remove(&target_id)
    {
        let _ = previous.sender.send(TerminalCommand::Stop);
    }
    state
        .terminals
        .lock()
        .map_err(|_| "终端状态锁已损坏".to_string())?
        .insert(target_id.clone(), TerminalControl { sender });

    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            emit_terminal(&app, &target_id, "connecting", None, None);
            let (session, _) = connect_ssh(&target)?;
            let mut channel = session
                .channel_session()
                .map_err(|error| format!("无法创建终端通道：{error}"))?;
            channel
                .request_pty("xterm-256color", None, Some((cols, rows, 0, 0)))
                .map_err(|error| format!("无法申请远程 PTY：{error}"))?;
            channel
                .shell()
                .map_err(|error| format!("无法启动远程 Shell：{error}"))?;
            channel
                .write_all(b"export TERM=xterm-256color; clear; nvitop\n")
                .map_err(|error| format!("无法启动 nvitop：{error}"))?;
            channel.flush().ok();
            session.set_blocking(false);
            emit_terminal(&app, &target_id, "connected", None, None);

            let mut buffer = [0_u8; 16 * 1024];
            let mut pending_input = Vec::<u8>::new();
            'terminal: loop {
                while let Ok(command) = receiver.try_recv() {
                    match command {
                        TerminalCommand::Input(bytes) => pending_input.extend(bytes),
                        TerminalCommand::Resize(new_cols, new_rows) => {
                            let _ = channel.request_pty_size(new_cols, new_rows, None, None);
                        }
                        TerminalCommand::Stop => break 'terminal,
                    }
                }

                if !pending_input.is_empty() {
                    match channel.write(&pending_input) {
                        Ok(written) => {
                            pending_input.drain(..written);
                            let _ = channel.flush();
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                        Err(error) => return Err(format!("终端输入失败：{error}")),
                    }
                }

                match channel.read(&mut buffer) {
                    Ok(0) if channel.eof() => break,
                    Ok(0) => {}
                    Ok(read) => emit_terminal(
                        &app,
                        &target_id,
                        "output",
                        Some(STANDARD_NO_PAD.encode(&buffer[..read])),
                        None,
                    ),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                    Err(error) => return Err(format!("终端输出失败：{error}")),
                }
                if channel.eof() {
                    break;
                }
                thread::sleep(Duration::from_millis(12));
            }
            let _ = channel.close();
            Ok(())
        })();

        match result {
            Ok(()) => emit_terminal(&app, &target_id, "stopped", None, None),
            Err(error) => emit_terminal(&app, &target_id, "error", None, Some(error)),
        }
    });
    Ok(())
}

async fn run_terminal_async(
    app: &AppHandle,
    target_id: &str,
    run_id: &str,
    target: &SshTarget,
    receiver: &mpsc::Receiver<TerminalCommand>,
    cols: &mut u32,
    rows: &mut u32,
) -> Result<bool, String> {
    loop {
        match receiver.try_recv() {
            Ok(TerminalCommand::Resize(new_cols, new_rows)) => {
                *cols = new_cols;
                *rows = new_rows;
            }
            Ok(TerminalCommand::Input(_)) => {}
            Ok(TerminalCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                return Ok(true);
            }
            Err(mpsc::TryRecvError::Empty) => break,
        }
    }
    let session = connect_ssh_async(target).await?;
    let mut channel = session
        .handle
        .channel_open_session()
        .await
        .map_err(|error| format!("无法创建终端通道：{error}"))?;
    channel
        .request_pty(false, "xterm-256color", *cols, *rows, 0, 0, &[])
        .await
        .map_err(|error| format!("无法申请远程 PTY：{error}"))?;
    channel
        .request_shell(true)
        .await
        .map_err(|error| format!("无法启动远程 Shell：{error}"))?;
    channel
        .data_bytes(b"export TERM=xterm-256color; clear; nvitop\n".to_vec())
        .await
        .map_err(|error| format!("无法启动 nvitop：{error}"))?;
    emit_terminal(app, target_id, run_id, "connected", None, None);

    loop {
        loop {
            match receiver.try_recv() {
                Ok(TerminalCommand::Input(bytes)) => {
                    channel
                        .data_bytes(bytes)
                        .await
                        .map_err(|error| format!("终端输入失败：{error}"))?;
                }
                Ok(TerminalCommand::Resize(new_cols, new_rows)) => {
                    *cols = new_cols;
                    *rows = new_rows;
                    channel
                        .window_change(new_cols, new_rows, 0, 0)
                        .await
                        .map_err(|error| format!("终端调整大小失败：{error}"))?;
                }
                Ok(TerminalCommand::Stop) => {
                    let _ = channel.close().await;
                    close_ssh_async(session).await;
                    return Ok(true);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = channel.close().await;
                    close_ssh_async(session).await;
                    return Ok(true);
                }
            }
        }

        match tokio::time::timeout(Duration::from_millis(40), channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data }))
            | Ok(Some(ChannelMsg::ExtendedData { data, .. })) => {
                emit_terminal(
                    app,
                    target_id,
                    run_id,
                    "output",
                    Some(STANDARD_NO_PAD.encode(data.as_ref())),
                    None,
                );
            }
            Ok(Some(ChannelMsg::Eof | ChannelMsg::Close)) | Ok(None) => break,
            Ok(Some(_)) | Err(_) => {}
        }
    }

    let _ = channel.close().await;
    close_ssh_async(session).await;
    Ok(false)
}

fn wait_for_terminal_retry(
    receiver: &mpsc::Receiver<TerminalCommand>,
    cols: &mut u32,
    rows: &mut u32,
    delay: Duration,
) -> bool {
    let ticks = delay.as_millis().div_ceil(100) as usize;
    for _ in 0..ticks {
        loop {
            match receiver.try_recv() {
                Ok(TerminalCommand::Resize(new_cols, new_rows)) => {
                    *cols = new_cols;
                    *rows = new_rows;
                }
                Ok(TerminalCommand::Input(_)) => {}
                Ok(TerminalCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => return true,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

#[tauri::command]
fn start_terminal(
    app: AppHandle,
    state: State<RuntimeState>,
    target: SshTarget,
    run_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let target_id = target.id.clone();
    let (sender, receiver) = mpsc::channel::<TerminalCommand>();
    let mut terminals = state
        .terminals
        .lock()
        .map_err(|_| "终端状态锁已损坏".to_string())?;
    if let Some(previous) = terminals.insert(
        target_id.clone(),
        TerminalControl {
            run_id: run_id.clone(),
            sender,
        },
    ) {
        let _ = previous.sender.send(TerminalCommand::Stop);
    }
    drop(terminals);

    thread::spawn(move || {
        let mut cols = cols;
        let mut rows = rows;
        let mut reconnect_delay = Duration::from_secs(1);
        loop {
            emit_terminal(&app, &target_id, &run_id, "connecting", None, None);
            match tauri::async_runtime::block_on(run_terminal_async(
                &app, &target_id, &run_id, &target, &receiver, &mut cols, &mut rows,
            )) {
                Ok(true) => break,
                Ok(false) => {
                    reconnect_delay = Duration::from_secs(1);
                    emit_terminal(
                        &app,
                        &target_id,
                        &run_id,
                        "error",
                        None,
                        Some("SSH 终端连接已中断，正在自动重连".to_string()),
                    );
                }
                Err(error) => {
                    emit_terminal(&app, &target_id, &run_id, "error", None, Some(error));
                    reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(30));
                }
            }
            if wait_for_terminal_retry(&receiver, &mut cols, &mut rows, reconnect_delay) {
                break;
            }
        }
        emit_terminal(&app, &target_id, &run_id, "stopped", None, None);
        if let Ok(mut terminals) = app.state::<RuntimeState>().terminals.lock() {
            if terminals
                .get(&target_id)
                .is_some_and(|control| control.run_id == run_id)
            {
                terminals.remove(&target_id);
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn terminal_input(
    state: State<RuntimeState>,
    target_id: String,
    run_id: String,
    data: String,
) -> Result<(), String> {
    let terminals = state
        .terminals
        .lock()
        .map_err(|_| "终端状态锁已损坏".to_string())?;
    let control = terminals
        .get(&target_id)
        .ok_or_else(|| "终端尚未启动".to_string())?;
    if control.run_id != run_id {
        return Ok(());
    }
    control
        .sender
        .send(TerminalCommand::Input(data.into_bytes()))
        .map_err(|_| "终端连接已关闭".to_string())
}

#[tauri::command]
fn resize_terminal(
    state: State<RuntimeState>,
    target_id: String,
    run_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let terminals = state
        .terminals
        .lock()
        .map_err(|_| "终端状态锁已损坏".to_string())?;
    let control = terminals
        .get(&target_id)
        .ok_or_else(|| "终端尚未启动".to_string())?;
    if control.run_id != run_id {
        return Ok(());
    }
    control
        .sender
        .send(TerminalCommand::Resize(cols, rows))
        .map_err(|_| "终端连接已关闭".to_string())
}

#[tauri::command]
fn stop_terminal(
    state: State<RuntimeState>,
    target_id: String,
    run_id: String,
) -> Result<(), String> {
    let mut terminals = state
        .terminals
        .lock()
        .map_err(|_| "终端状态锁已损坏".to_string())?;
    if terminals
        .get(&target_id)
        .is_some_and(|control| control.run_id == run_id)
    {
        if let Some(control) = terminals.remove(&target_id) {
            let _ = control.sender.send(TerminalCommand::Stop);
        }
    }
    Ok(())
}

fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            return PathBuf::from(home).join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

fn secrets_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法定位应用数据目录：{error}"))?;
    fs::create_dir_all(&dir).map_err(|error| format!("无法创建应用数据目录：{error}"))?;
    Ok(dir.join(".env"))
}

fn escape_env(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn unescape_env(value: &str) -> String {
    let mut output = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            output.push(match character {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            output.push(character);
        }
    }
    if escaped {
        output.push('\\');
    }
    output
}

fn encode_env_id(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

fn decode_env_id(value: &str) -> Option<String> {
    if value.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    String::from_utf8(bytes).ok()
}

#[tauri::command]
fn load_credentials(app: AppHandle) -> Result<Vec<CredentialUpdate>, String> {
    let state = app.state::<RuntimeState>();
    let _guard = state
        .credentials
        .lock()
        .map_err(|_| "本地凭据锁已损坏".to_string())?;
    let path = secrets_path(&app)?;
    let backup = path.with_extension("env.bak");
    if !path.exists() && backup.exists() {
        fs::rename(&backup, &path).map_err(|error| format!("无法恢复本地凭据：{error}"))?;
    }
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).map_err(|error| format!("无法读取本地凭据：{error}"))?;
    let mut records: HashMap<String, RuntimeSecret> = HashMap::new();
    for line in content.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some((encoded_id, field)) = key
            .strip_prefix("NOVA_")
            .and_then(|key| key.rsplit_once('_'))
        else {
            continue;
        };
        let id = decode_env_id(encoded_id).unwrap_or_else(|| encoded_id.to_string());
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        let secret = records.entry(id).or_default();
        match field {
            "PASSWORD" => secret.password = Some(unescape_env(value)),
            "PASSPHRASE" => secret.passphrase = Some(unescape_env(value)),
            _ => {}
        }
    }
    Ok(records
        .into_iter()
        .map(|(id, secret)| CredentialUpdate {
            id,
            password: secret.password,
            passphrase: secret.passphrase,
        })
        .collect())
}

#[tauri::command]
fn save_credentials(
    app: AppHandle,
    credentials: Vec<CredentialUpdate>,
    revision: u64,
) -> Result<(), String> {
    let state = app.state::<RuntimeState>();
    let _guard = state
        .credentials
        .lock()
        .map_err(|_| "本地凭据锁已损坏".to_string())?;
    if revision <= state.credential_revision.load(Ordering::Acquire) {
        return Ok(());
    }
    let path = secrets_path(&app)?;
    let mut lines = vec!["# NOVA Watch local credentials".to_string()];
    for credential in credentials {
        let id = encode_env_id(&credential.id);
        if let Some(password) = credential.password.filter(|value| !value.is_empty()) {
            lines.push(format!(
                "NOVA_{}_PASSWORD=\"{}\"",
                id,
                escape_env(&password)
            ));
        }
        if let Some(passphrase) = credential.passphrase.filter(|value| !value.is_empty()) {
            lines.push(format!(
                "NOVA_{}_PASSPHRASE=\"{}\"",
                id,
                escape_env(&passphrase)
            ));
        }
    }
    let temporary = path.with_extension("env.tmp");
    let backup = path.with_extension("env.bak");
    fs::write(&temporary, format!("{}\n", lines.join("\n")))
        .map_err(|error| format!("无法保存本地凭据：{error}"))?;
    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| format!("无法清理凭据备份：{error}"))?;
    }
    if path.exists() {
        fs::rename(&path, &backup).map_err(|error| format!("无法备份本地凭据：{error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        if backup.exists() {
            let _ = fs::rename(&backup, &path);
        }
        return Err(format!("无法提交本地凭据：{error}"));
    }
    if backup.exists() {
        fs::remove_file(backup).map_err(|error| format!("无法清理凭据备份：{error}"))?;
    }
    state.credential_revision.store(revision, Ordering::Release);
    Ok(())
}

#[tauri::command]
fn read_ssh_config_hosts() -> Result<Vec<SshConfigHost>, String> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or_else(|| "无法定位用户目录".to_string())?;
    let config_path = PathBuf::from(home).join(".ssh").join("config");
    if !config_path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&config_path)
        .map_err(|error| format!("无法读取 {}：{error}", config_path.display()))?;

    #[derive(Default)]
    struct DraftHost {
        alias: String,
        host: Option<String>,
        port: Option<u16>,
        username: Option<String>,
        identity_file: Option<String>,
    }

    fn finish(draft: DraftHost, output: &mut Vec<SshConfigHost>) {
        if draft.alias.is_empty() || draft.alias.contains('*') || draft.alias.contains('?') {
            return;
        }
        output.push(SshConfigHost {
            host: draft.host.unwrap_or_else(|| draft.alias.clone()),
            alias: draft.alias,
            port: draft.port.unwrap_or(22),
            username: draft.username.unwrap_or_default(),
            identity_file: draft.identity_file.map(|path| expand_home(&path)),
        });
    }

    let mut hosts = Vec::new();
    let mut current = DraftHost::default();
    for raw_line in content.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("").to_ascii_lowercase();
        let value = parts.collect::<Vec<_>>().join(" ");
        match key.as_str() {
            "host" => {
                finish(current, &mut hosts);
                current = DraftHost {
                    alias: value.split_whitespace().next().unwrap_or("").to_string(),
                    ..DraftHost::default()
                };
            }
            "hostname" if !current.alias.is_empty() => current.host = Some(value),
            "port" if !current.alias.is_empty() => current.port = value.parse().ok(),
            "user" if !current.alias.is_empty() => current.username = Some(value),
            "identityfile" if !current.alias.is_empty() => current.identity_file = Some(value),
            _ => {}
        }
    }
    finish(current, &mut hosts);
    Ok(hosts)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(RuntimeState::default())
        .invoke_handler(tauri::generate_handler![
            test_connection,
            load_credentials,
            save_credentials,
            start_monitor,
            stop_monitor,
            start_terminal,
            terminal_input,
            resize_terminal,
            stop_terminal,
            read_ssh_config_hosts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running NOVA Watch");
}

#[cfg(test)]
mod tests {
    use super::{decode_env_id, encode_env_id, escape_env, unescape_env};

    #[test]
    fn credential_id_round_trip() {
        let id = "4d1c6b82-955d-45b4-a324-eaa5ddfb57be";
        assert_eq!(decode_env_id(&encode_env_id(id)).as_deref(), Some(id));
    }

    #[test]
    fn credential_value_round_trip() {
        for value in ["plain", "quote\"and\\slash", "line1\nline2", "trailing\\"] {
            assert_eq!(unescape_env(&escape_env(value)), value);
        }
    }
}
