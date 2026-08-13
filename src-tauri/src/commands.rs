use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use crate::state::{self, AppState, EnvInfo, ServerStatus, Settings};

/// 可克隆的服务器共享状态快照（只含 Arc 字段），用于把阻塞工作丢到 spawn_blocking 线程。
pub struct Shared {
    pub pid: Arc<Mutex<Option<u32>>>,
    pub stop: Arc<AtomicBool>,
    pub url: Arc<Mutex<Option<String>>>,
    pub logs: Arc<Mutex<VecDeque<String>>>,
    pub settings: Arc<Mutex<Settings>>,
}

impl Shared {
    pub fn from_state(st: &AppState) -> Self {
        Self {
            pid: Arc::clone(&st.pid),
            stop: Arc::clone(&st.stop_requested),
            url: Arc::clone(&st.server_url),
            logs: Arc::clone(&st.logs),
            settings: Arc::clone(&st.settings),
        }
    }
}

#[tauri::command]
pub async fn detect_env() -> EnvInfo {
    tauri::async_runtime::spawn_blocking(state::detect_env)
        .await
        .unwrap_or_default()
}

fn install_node_inner(app: &AppHandle, shared: &Shared) -> Result<EnvInfo, String> {
    state::push_log(
        &shared.logs,
        "[系统] 开始安装 Node.js LTS（winget，可能弹出 UAC 授权窗口，请点击“是”）",
    );
    let result = state::run_streaming(
        app,
        "winget",
        &[
            "install",
            "OpenJS.NodeJS.LTS",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ],
    );
    if let Err(e) = result {
        state::push_log(&shared.logs, &format!("[系统] winget 安装失败：{e}"));
        return Err("Node.js 安装失败。请尝试手动到 https://nodejs.org 下载 LTS 版安装。".to_string());
    }
    state::push_log(&shared.logs, "[系统] Node.js 安装完成，正在重新检测");
    Ok(state::detect_env())
}

#[tauri::command]
pub async fn install_node(app: AppHandle, st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || install_node_inner(&app, &shared))
        .await
        .map_err(|e| e.to_string())?
}

fn install_dsh_inner(app: &AppHandle, shared: &Shared, spec: &str) -> Result<EnvInfo, String> {
    let env = state::detect_env();
    let node = env
        .node_path
        .clone()
        .ok_or("未检测到 Node.js，请先安装 Node.js。".to_string())?;
    let cli = state::npm_cli(&node).ok_or("未找到 npm，请确认 Node.js 安装完整。".to_string())?;
    let install_prefix = env
        .install_prefix
        .clone()
        .ok_or("无法确定安装目录，请先安装 Node.js。".to_string())?;

    let registry = {
        let s = shared.settings.lock().unwrap();
        s.registry.trim().to_string()
    };

    let mut args: Vec<String> = vec![
        cli.to_string_lossy().to_string(),
        "install".to_string(),
        "-g".to_string(),
        "--prefix".to_string(),
        install_prefix.clone(),
        spec.to_string(),
        "--no-audit".to_string(),
        "--no-fund".to_string(),
    ];
    if !registry.is_empty() {
        args.push("--registry".to_string());
        args.push(registry);
    }

    state::push_log(
        &shared.logs,
        &format!("[系统] 开始安装 {spec} 到 {install_prefix}"),
    );
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    state::run_streaming(app, &node, &arg_refs)?;
    state::push_log(&shared.logs, "[系统] 安装完成，正在校验");
    Ok(state::detect_env())
}

#[tauri::command]
pub async fn install_dsh(app: AppHandle, st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || install_dsh_inner(&app, &shared, "@deepseek-ai/dsh"))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn update_dsh(app: AppHandle, st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || {
        install_dsh_inner(&app, &shared, "@deepseek-ai/dsh@latest")
    })
    .await
    .map_err(|e| e.to_string())?
}

fn verify_dsh_inner() -> Result<String, String> {
    let env = state::detect_env();
    let node = env.node_path.clone().ok_or("未检测到 Node.js。".to_string())?;
    let bin = state::resolve_dsh_bin(&env).ok_or("未安装 DeepSeek Harness。".to_string())?;
    let version = state::run_output(&node, &[bin.to_str().unwrap_or(""), "--version"])?;
    state::run_output(&node, &[bin.to_str().unwrap_or(""), "--dump-default-config"])?;
    Ok(format!("DeepSeek Harness {version} 安装正常，可正常启动"))
}

#[tauri::command]
pub async fn verify_dsh() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(verify_dsh_inner)
        .await
        .map_err(|e| e.to_string())?
}

pub fn start_server_impl(app: &AppHandle, shared: &Shared) -> Result<ServerStatus, String> {
    if shared.pid.lock().unwrap().is_some() {
        return Err("服务器已在运行".to_string());
    }

    let (port, workspace) = {
        let s = shared.settings.lock().unwrap();
        (s.port, s.workspace_dir.clone())
    };

    // 端口上可能已有外部启动的服务器，避免重复启动
    let probe = format!("http://127.0.0.1:{port}");
    if state::health(&probe) {
        return Err(format!(
            "端口 {port} 已有服务器在运行（可能由外部启动），无需重复启动"
        ));
    }

    let env = state::detect_env();
    if !env.dsh_installed {
        return Err("尚未安装 DeepSeek Harness，请先点击“安装/更新”。".to_string());
    }
    let node = env
        .node_path
        .clone()
        .ok_or("未检测到 Node.js，请先安装。".to_string())?;
    let bin = state::resolve_dsh_bin(&env).ok_or("找不到 dsh 入口文件，请重新安装 Harness。".to_string())?;

    shared.stop.store(false, Ordering::SeqCst);
    *shared.url.lock().unwrap() = None;

    let mut cmd = std::process::Command::new(&node);
    cmd.arg(bin.to_string_lossy().to_string())
        .arg("web")
        .arg("--port")
        .arg(port.to_string());
    if let Some(ws) = workspace.filter(|w| !w.trim().is_empty()) {
        cmd.current_dir(&ws);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    let mut child = cmd.spawn().map_err(|e| format!("启动失败：{e}"))?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    *shared.pid.lock().unwrap() = Some(pid);
    state::refresh_tray(app, true);
    state::push_log(
        &shared.logs,
        &format!("[系统] 正在启动 dsh web --port {port}（pid {pid}）"),
    );

    let mut streams: Vec<Box<dyn std::io::Read + Send>> = Vec::new();
    if let Some(s) = stdout {
        streams.push(Box::new(s));
    }
    if let Some(s) = stderr {
        streams.push(Box::new(s));
    }
    for stream in streams {
        let app = app.clone();
        let logs = Arc::clone(&shared.logs);
        let url = Arc::clone(&shared.url);
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(stream).lines() {
                match line {
                    Ok(line) => {
                        state::push_log(&logs, &line);
                        let _ = app.emit("log", &line);
                        if let Some(u) = state::extract_url(&line) {
                            let mut slot = url.lock().unwrap();
                            if slot.is_none() {
                                *slot = Some(u.clone());
                                drop(slot);
                                let _ = app.emit("server-url", &u);
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    let app2 = app.clone();
    let pid_slot = Arc::clone(&shared.pid);
    let url_slot = Arc::clone(&shared.url);
    let stop = Arc::clone(&shared.stop);
    let logs = Arc::clone(&shared.logs);
    std::thread::spawn(move || {
        let result = child.wait();
        let was_stopped = stop.load(Ordering::SeqCst);
        *url_slot.lock().unwrap() = None;
        let mut p = pid_slot.lock().unwrap();
        if *p == Some(pid) {
            *p = None;
        }
        drop(p);
        if !was_stopped {
            let code = result.as_ref().ok().and_then(|s| s.code()).unwrap_or(-1);
            state::refresh_tray(&app2, false);
            state::push_log(&logs, &format!("[系统] 服务器进程已退出（退出码 {code}）"));
            let _ = app2.emit("server-exited", code);
        }
    });

    Ok(state::status_from(&shared.pid, &shared.url, port))
}

#[tauri::command]
pub async fn start_server(app: AppHandle, st: State<'_, AppState>) -> Result<ServerStatus, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || start_server_impl(&app, &shared))
        .await
        .map_err(|e| e.to_string())?
}

fn stop_server_inner(app: &AppHandle, shared: &Shared) -> Result<ServerStatus, String> {
    shared.stop.store(true, Ordering::SeqCst);
    let managed_pid = *shared.pid.lock().unwrap();
    let target_pid = if let Some(pid) = managed_pid {
        Some(pid)
    } else {
        // 外部启动的服务器：按端口定位进程
        let port = shared.settings.lock().unwrap().port;
        state::find_pid_on_port(port)
    };

    if let Some(pid) = target_pid {
        state::push_log(&shared.logs, &format!("[系统] 正在停止服务器（pid {pid}）"));
        let _ = state::run_output("taskkill", &["/PID", &pid.to_string(), "/T", "/F"]);
    } else {
        state::push_log(&shared.logs, "[系统] 未找到正在监听该端口的进程");
    }

    *shared.pid.lock().unwrap() = None;
    *shared.url.lock().unwrap() = None;
    state::refresh_tray(app, false);
    Ok(ServerStatus {
        phase: "stopped".to_string(),
        url: None,
        pid: None,
    })
}

#[tauri::command]
pub async fn stop_server(app: AppHandle, st: State<'_, AppState>) -> Result<ServerStatus, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || stop_server_inner(&app, &shared))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn server_status(st: State<'_, AppState>) -> Result<ServerStatus, String> {
    let shared = Shared::from_state(&st);
    let port = shared.settings.lock().unwrap().port;
    tauri::async_runtime::spawn_blocking(move || state::status_from(&shared.pid, &shared.url, port))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_logs(st: State<AppState>) -> Vec<String> {
    st.logs.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
pub fn get_settings(st: State<AppState>) -> Settings {
    st.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    st: State<AppState>,
    value: Settings,
) -> Result<Settings, String> {
    {
        let mut s = st.settings.lock().unwrap();
        *s = value;
    }
    let s = st.settings.lock().unwrap().clone();
    state::save_settings(&app, &s)?;
    Ok(s)
}

#[tauri::command]
pub fn set_autostart(
    app: AppHandle,
    st: State<AppState>,
    enabled: bool,
) -> Result<bool, String> {
    state::set_autostart(enabled)?;
    st.settings.lock().unwrap().autostart = enabled;
    let s = st.settings.lock().unwrap().clone();
    state::save_settings(&app, &s)?;
    Ok(enabled)
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if url.is_empty() {
        return Err("没有可打开的地址".to_string());
    }
    #[cfg(windows)]
    std::process::Command::new("explorer")
        .arg(&url)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
