use std::collections::VecDeque;
use std::path::Path;
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

    fn node_dir(&self) -> Option<String> {
        self.settings.lock().unwrap().node_dir.clone()
    }

    fn registry(&self) -> String {
        self.settings.lock().unwrap().registry.trim().to_string()
    }
}

#[tauri::command]
pub async fn detect_env(st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let node_dir = st.settings.lock().unwrap().node_dir.clone();
    tauri::async_runtime::spawn_blocking(move || state::detect_env(node_dir.as_deref()))
        .await
        .map_err(|e| e.to_string())
}

fn winget_node(app: &AppHandle, shared: &Shared, upgrade: bool) -> Result<EnvInfo, String> {
    let label = if upgrade { "升级" } else { "安装" };
    state::push_log(
        &shared.logs,
        &format!("[系统] 开始通过 winget {label} Node.js LTS（可能弹出 UAC 授权窗口，请点击“是”）"),
    );
    let _ = app.emit("install-stage", "install");
    let args: Vec<&str> = if upgrade {
        vec![
            "upgrade",
            "--id",
            "OpenJS.NodeJS.LTS",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--exact",
        ]
    } else {
        vec![
            "install",
            "OpenJS.NodeJS.LTS",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ]
    };
    let result = state::run_streaming(app, "winget", &args);
    if let Err(e) = result {
        state::push_log(&shared.logs, &format!("[系统] winget {label}失败：{e}"));
        let _ = app.emit("install-stage", "error");
        return Err(format!(
            "Node.js {label}失败。可尝试「自定义安装」或到 https://nodejs.org 手动下载 LTS 版。"
        ));
    }
    state::push_log(&shared.logs, &format!("[系统] Node.js {label}完成，正在重新检测"));
    let node_dir = shared.node_dir();
    Ok(state::detect_env(node_dir.as_deref()))
}

#[tauri::command]
pub async fn install_node(app: AppHandle, st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || winget_node(&app, &shared, false))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn upgrade_node(app: AppHandle, st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || winget_node(&app, &shared, true))
        .await
        .map_err(|e| e.to_string())?
}

fn install_node_nvm_inner(app: &AppHandle, shared: &Shared) -> Result<EnvInfo, String> {
    let nvm = state::detect_nvm().ok_or("未检测到 nvm，无法使用 nvm 安装 Node.js。")?;
    let version = state::latest_node_lts_major22().unwrap_or_else(|| "22.19.0".to_string());
    state::push_log(
        &shared.logs,
        &format!("[系统] 检测到 nvm，开始安装 Node.js {version}"),
    );
    let _ = app.emit("install-stage", "install");
    state::run_streaming(app, &nvm, &["install", &version])?;
    let _ = app.emit("install-stage", "use");
    state::push_log(
        &shared.logs,
        &format!("[系统] 正在切换 Node 版本（nvm use {version}，若需要管理员权限请确认）"),
    );
    state::run_streaming(app, &nvm, &["use", &version])?;
    state::push_log(&shared.logs, "[系统] nvm 安装并切换完成，正在重新检测");
    let node_dir = shared.node_dir();
    Ok(state::detect_env(node_dir.as_deref()))
}

#[tauri::command]
pub async fn install_node_nvm(app: AppHandle, st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || install_node_nvm_inner(&app, &shared))
        .await
        .map_err(|e| e.to_string())?
}

fn install_node_portable_inner(
    app: &AppHandle,
    shared: &Shared,
    dir: String,
) -> Result<EnvInfo, String> {
    let dir = dir.trim().to_string();
    if dir.is_empty() {
        return Err("请选择 Node.js 安装目录".to_string());
    }
    let version = state::latest_node_lts_major22().unwrap_or_else(|| "22.19.0".to_string());
    let registry = shared.registry();
    let base = state::node_dist_base(&registry);
    let url = format!("{base}/v{version}/node-v{version}-win-x64.zip");
    let zip_path = std::env::temp_dir().join(format!("node-v{version}-win-x64.zip"));

    let _ = app.emit("install-stage", "download");
    state::push_log(
        &shared.logs,
        &format!("[系统] 正在下载 Node.js {version}（{url}）"),
    );
    state::download_file(&url, &zip_path)?;

    let _ = app.emit("install-stage", "extract");
    state::push_log(&shared.logs, &format!("[系统] 下载完成，正在解压到 {dir}"));
    state::extract_zip(&zip_path, Path::new(&dir))?;
    let _ = std::fs::remove_file(&zip_path);

    {
        let mut s = shared.settings.lock().unwrap();
        s.node_dir = Some(dir.clone());
    }
    let s = shared.settings.lock().unwrap().clone();
    state::save_settings(app, &s)?;

    state::push_log(&shared.logs, "[系统] 便携版 Node.js 安装完成，正在重新检测");
    Ok(state::detect_env(Some(&dir)))
}

#[tauri::command]
pub async fn install_node_portable(
    app: AppHandle,
    st: State<'_, AppState>,
    dir: String,
) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || install_node_portable_inner(&app, &shared, dir))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pick_node_dir(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .and_then(|fp| fp.into_path().ok().map(|p| p.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| e.to_string())
}

fn install_dsh_inner(app: &AppHandle, shared: &Shared, spec: &str) -> Result<EnvInfo, String> {
    let node_dir = shared.node_dir();
    let env = state::detect_env(node_dir.as_deref());
    let node = env
        .node_path
        .clone()
        .ok_or("未检测到 Node.js，请先安装 Node.js。".to_string())?;
    let cli = state::npm_cli(&node).ok_or("未找到 npm，请确认 Node.js 安装完整。".to_string())?;
    let install_prefix = env
        .install_prefix
        .clone()
        .ok_or("无法确定安装目录，请先安装 Node.js。".to_string())?;

    // 应用目录专用：先清空重装，避免残缺/损坏的依赖树（如缺失 js-yaml）
    let _ = std::fs::remove_dir_all(&install_prefix);
    let _ = std::fs::create_dir_all(&install_prefix);

    let registry = shared.registry();

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

    let _ = app.emit("install-stage", "install");
    state::push_log(
        &shared.logs,
        &format!("[系统] 清空旧安装后开始安装 {spec} 到 {install_prefix}"),
    );
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    state::run_streaming(app, &node, &arg_refs)?;

    let _ = app.emit("install-stage", "verify");
    state::push_log(&shared.logs, "[系统] 安装完成，正在校验");
    Ok(state::detect_env(node_dir.as_deref()))
}

#[tauri::command]
pub async fn install_dsh(app: AppHandle, st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    let app_for_sync = app.clone();
    let env = tauri::async_runtime::spawn_blocking(move || install_dsh_inner(&app, &shared, "@deepseek-ai/dsh"))
        .await
        .map_err(|e| e.to_string())??;
    // 安装/更新会清空应用目录，重同步鲸仔设置分区插件。
    let _ = crate::settings_plugin::ensure_settings_plugin(&app_for_sync);
    Ok(env)
}

#[tauri::command]
pub async fn update_dsh(app: AppHandle, st: State<'_, AppState>) -> Result<EnvInfo, String> {
    let shared = Shared::from_state(&st);
    let app_for_sync = app.clone();
    let env = tauri::async_runtime::spawn_blocking(move || {
        install_dsh_inner(&app, &shared, "@deepseek-ai/dsh@latest")
    })
    .await
    .map_err(|e| e.to_string())??;
    let _ = crate::settings_plugin::ensure_settings_plugin(&app_for_sync);
    Ok(env)
}

fn verify_dsh_inner(node_dir: Option<String>) -> Result<String, String> {
    let env = state::detect_env(node_dir.as_deref());
    let node = env.node_path.clone().ok_or("未检测到 Node.js。".to_string())?;
    let bin = state::resolve_dsh_bin(&env).ok_or("未安装 DeepSeek Harness。".to_string())?;
    let version = state::run_output(&node, &[bin.to_str().unwrap_or(""), "--version"])?;
    state::run_output(&node, &[bin.to_str().unwrap_or(""), "web", "--dump-default-config"])?;
    Ok(format!("DeepSeek Harness {version} 安装正常，可正常启动"))
}

#[tauri::command]
pub async fn verify_dsh(st: State<'_, AppState>) -> Result<String, String> {
    let node_dir = st.settings.lock().unwrap().node_dir.clone();
    tauri::async_runtime::spawn_blocking(move || verify_dsh_inner(node_dir))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn check_latest_version(st: State<'_, AppState>) -> Result<Option<String>, String> {
    let shared = Shared::from_state(&st);
    let registry = shared.registry();
    tauri::async_runtime::spawn_blocking(move || {
        let env = state::detect_env(shared.node_dir().as_deref());
        let Some(node) = env.node_path else {
            return Ok(None);
        };
        let Some(cli) = state::npm_cli(&node) else {
            return Ok(None);
        };
        let mut args: Vec<String> = vec![
            cli.to_string_lossy().to_string(),
            "view".to_string(),
            "@deepseek-ai/dsh".to_string(),
            "version".to_string(),
        ];
        if !registry.is_empty() {
            args.push("--registry".to_string());
            args.push(registry);
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        match state::run_output(&node, &arg_refs) {
            Ok(v) => Ok(Some(v.trim().to_string())),
            Err(_) => Ok(None),
        }
    })
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

    let env = state::detect_env(shared.node_dir().as_deref());
    if !env.dsh_installed {
        return Err("尚未安装 DeepSeek Harness，请先点击“安装/更新”。".to_string());
    }
    let node = env
        .node_path
        .clone()
        .ok_or("未检测到 Node.js，请先安装。".to_string())?;
    let bin = state::resolve_dsh_bin(&env).ok_or("找不到 dsh 入口文件，请重新安装 Harness。".to_string())?;

    // 启动前强制同步鲸仔设置分区插件（幂等），保证 Loader 能解析插件条目。
    crate::settings_plugin::ensure_settings_plugin(app)?;

    shared.stop.store(false, Ordering::SeqCst);
    *shared.url.lock().unwrap() = None;

    let mut cmd = std::process::Command::new(&node);
    // DSH 家目录与插件同步保持一致（测试构建使用隔离的 ~/.dsh-test）。
    cmd.env("DSH_HOME", crate::settings_plugin::dsh_home());
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
        let mut p = pid_slot.lock().unwrap();
        let is_current = *p == Some(pid);
        if is_current {
            *p = None;
        }
        drop(p);
        if is_current {
            *url_slot.lock().unwrap() = None;
            if !was_stopped {
                let code = result.as_ref().ok().and_then(|s| s.code()).unwrap_or(-1);
                state::refresh_tray(&app2, false);
                state::push_log(&logs, &format!("[系统] 服务器进程已退出（退出码 {code}）"));
                let _ = app2.emit("server-exited", code);
            }
        }
    });

    // 就绪等待：轮询健康检查（不依赖 stdout 里 URL 的抽取），带超时。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut ready = false;
    while std::time::Instant::now() < deadline {
        if state::health(&probe) {
            ready = true;
            break;
        }
        if shared.pid.lock().unwrap().is_none() {
            return Err("服务器进程启动后立即退出，请查看日志".to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    if !ready {
        return Err(format!(
            "服务器未能在 30 秒内就绪（{probe}），请查看日志"
        ));
    }

    {
        let mut slot = shared.url.lock().unwrap();
        if slot.is_none() {
            *slot = Some(probe.clone());
        }
    }

    Ok(ServerStatus {
        phase: "running".to_string(),
        url: Some(probe),
        pid: Some(pid),
    })
}

#[tauri::command]
pub async fn start_server(app: AppHandle, st: State<'_, AppState>) -> Result<ServerStatus, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || start_server_impl(&app, &shared))
        .await
        .map_err(|e| e.to_string())?
}

pub fn stop_server_inner(app: &AppHandle, shared: &Shared) -> Result<ServerStatus, String> {
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
        #[cfg(windows)]
        let _ = state::run_output("taskkill", &["/PID", &pid.to_string(), "/T", "/F"]);
        #[cfg(not(windows))]
        {
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .spawn();
        }
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
pub async fn restart_server(app: AppHandle, st: State<'_, AppState>) -> Result<ServerStatus, String> {
    let shared = Shared::from_state(&st);
    tauri::async_runtime::spawn_blocking(move || {
        let _ = stop_server_inner(&app, &shared);
        start_server_impl(&app, &shared)
    })
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
pub fn update_tray_state(app: AppHandle, running: bool) {
    state::refresh_tray(&app, running);
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
