use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde::{Deserialize, Serialize};
use tauri::menu::MenuItem;
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Emitter, Manager};

pub const LOG_CAP: usize = 2000;

#[derive(Default)]
pub struct AppState {
    pub pid: Arc<Mutex<Option<u32>>>,
    pub stop_requested: Arc<AtomicBool>,
    pub server_url: Arc<Mutex<Option<String>>>,
    pub logs: Arc<Mutex<VecDeque<String>>>,
    pub settings: Arc<Mutex<Settings>>,
    pub quitting: Arc<AtomicBool>,
    pub tray: Mutex<Option<TrayIcon>>,
    pub tray_start: Mutex<Option<MenuItem<tauri::Wry>>>,
    pub tray_stop: Mutex<Option<MenuItem<tauri::Wry>>>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub port: u16,
    pub registry: String,
    pub autostart: bool,
    pub auto_start_server: bool,
    pub auto_restart: bool,
    pub workspace_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: 3080,
            registry: "https://registry.npmjs.org".to_string(),
            autostart: false,
            auto_start_server: false,
            auto_restart: true,
            workspace_dir: None,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EnvInfo {
    pub found: bool,
    pub version: Option<String>,
    pub node_path: Option<String>,
    pub npm_prefix: Option<String>,
    pub install_prefix: Option<String>,
    pub prefix_fallback: bool,
    pub dsh_installed: bool,
    pub dsh_version: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub phase: String,
    pub url: Option<String>,
    pub pid: Option<u32>,
}

pub fn run_output(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("无法运行 {program}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if stdout.is_empty() {
            stderr
        } else if stderr.is_empty() {
            stdout
        } else {
            format!("{stdout}\n{stderr}")
        };
        Err(if msg.is_empty() {
            format!("{program} 退出码 {}", out.status.code().unwrap_or(-1))
        } else {
            msg
        })
    }
}

pub fn run_streaming(app: &AppHandle, program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 {program}: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let collected = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut handles = Vec::new();
    let mut streams: Vec<Box<dyn std::io::Read + Send>> = Vec::new();
    if let Some(s) = stdout {
        streams.push(Box::new(s));
    }
    if let Some(s) = stderr {
        streams.push(Box::new(s));
    }
    for stream in streams {
        let app = app.clone();
        let collected = Arc::clone(&collected);
        handles.push(std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(stream).lines() {
                match line {
                    Ok(line) => {
                        collected.lock().unwrap().push(line.clone());
                        let _ = app.emit("log", &line);
                    }
                    Err(_) => break,
                }
            }
        }));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    for h in handles {
        let _ = h.join();
    }
    let combined = collected.lock().unwrap().join("\n");
    if status.success() {
        Ok(combined)
    } else {
        Err(format!(
            "{program} 退出码 {}:\n{combined}",
            status.code().unwrap_or(-1)
        ))
    }
}

pub fn push_log(logs: &Mutex<VecDeque<String>>, line: &str) {
    let mut q = logs.lock().unwrap();
    q.push_back(line.to_string());
    while q.len() > LOG_CAP {
        q.pop_front();
    }
}

pub fn extract_url(line: &str) -> Option<String> {
    let idx = line.find("http")?;
    let rest = &line[idx..];
    let url: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'' && *c != ',')
        .collect();
    let url = url.trim_end_matches('.').trim_end_matches(')').to_string();
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url)
    } else {
        None
    }
}

pub fn health(url: &str) -> bool {
    ureq::get(url)
        .timeout(std::time::Duration::from_millis(800))
        .call()
        .is_ok()
}

pub fn status_of(st: &AppState) -> ServerStatus {
    let pid = *st.pid.lock().unwrap();
    let url = st.server_url.lock().unwrap().clone();
    let phase = match (pid, &url) {
        (None, _) => "stopped".to_string(),
        (Some(_), None) => "starting".to_string(),
        (Some(_), Some(u)) => {
            if health(u) {
                "running".to_string()
            } else {
                "error".to_string()
            }
        }
    };
    ServerStatus { phase, url, pid }
}

fn fallback_node_path() -> Option<String> {
    [
        "C:\\Program Files\\nodejs\\node.exe",
        "C:\\Program Files (x86)\\nodejs\\node.exe",
    ]
    .iter()
    .find(|p| Path::new(p).exists())
    .map(|s| s.to_string())
}

pub fn npm_cli(node_path: &str) -> Option<PathBuf> {
    let dir = Path::new(node_path).parent()?;
    let cli = dir.join("node_modules").join("npm").join("bin").join("npm-cli.js");
    cli.exists().then_some(cli)
}

pub fn dsh_bin(npm_prefix: &str) -> Option<PathBuf> {
    let bin = Path::new(npm_prefix)
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh")
        .join("lib")
        .join("bin.js");
    bin.exists().then_some(bin)
}

pub fn fallback_prefix_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&base).join("dsh-launcher").join("npm")
}

pub fn is_dir_writable(dir: &str) -> bool {
    let path = Path::new(dir);
    if !path.is_dir() {
        return path
            .parent()
            .map(|p| is_dir_writable(&p.to_string_lossy()))
            .unwrap_or(false);
    }
    let probe = path.join(format!(".dsh-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub fn resolve_dsh_bin(env: &EnvInfo) -> Option<PathBuf> {
    if let Some(p) = env.npm_prefix.as_deref() {
        if let Some(bin) = dsh_bin(p) {
            return Some(bin);
        }
    }
    if let Some(p) = env.install_prefix.as_deref() {
        if let Some(bin) = dsh_bin(p) {
            return Some(bin);
        }
    }
    None
}

pub fn refresh_tray(app: &AppHandle, running: bool) {
    let st = app.state::<AppState>();
    let start_item = st.tray_start.lock().unwrap().clone();
    let stop_item = st.tray_stop.lock().unwrap().clone();
    if let Some(item) = start_item {
        let _ = item.set_enabled(!running);
        let _ = item.set_text(if running { "服务器运行中" } else { "启动服务器" });
    }
    if let Some(item) = stop_item {
        let _ = item.set_enabled(running);
    }
}

pub fn detect_env() -> EnvInfo {
    let version_on_path = run_output("node", &["--version"]).ok();

    let node_path = run_output("where.exe", &["node"])
        .ok()
        .and_then(|o| o.lines().next().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .or_else(fallback_node_path);

    let node = node_path.clone().unwrap_or_else(|| "node".to_string());

    let version = version_on_path.or_else(|| run_output(&node, &["--version"]).ok());

    let npm_prefix = npm_cli(&node)
        .and_then(|cli| run_output(&node, &[cli.to_str().unwrap_or(""), "prefix", "-g"]).ok())
        .filter(|s| !s.is_empty());

    // 兜底：全局前缀不可写时，改用用户级目录安装（无需管理员权限）
    let (install_prefix, prefix_fallback) = match &npm_prefix {
        Some(p) if is_dir_writable(p) => (Some(p.clone()), false),
        Some(_) => {
            let local = fallback_prefix_dir();
            let _ = std::fs::create_dir_all(&local);
            (Some(local.to_string_lossy().to_string()), true)
        }
        None => (None, false),
    };

    // 优先全局前缀，其次兜底目录
    let dsh_bin_path = npm_prefix
        .as_deref()
        .and_then(dsh_bin)
        .or_else(|| install_prefix.as_deref().and_then(dsh_bin));

    let (dsh_installed, dsh_version) = if let Some(bin) = dsh_bin_path {
        let v = run_output(&node, &[bin.to_str().unwrap_or(""), "--version"]).ok();
        (true, v)
    } else {
        (false, None)
    };

    EnvInfo {
        found: node_path.is_some() && version.is_some(),
        version,
        node_path,
        npm_prefix,
        install_prefix,
        prefix_fallback,
        dsh_installed,
        dsh_version,
    }
}

pub fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

pub fn load_settings(app: &AppHandle) -> Settings {
    if let Ok(path) = config_path(app) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<Settings>(&text) {
                return s;
            }
        }
    }
    Settings::default()
}

pub fn save_settings(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| e.to_string())?;
    if enabled {
        key.set_value(
            "DshLauncher",
            &format!("\"{}\" --autostart", exe.display()),
        )
        .map_err(|e| e.to_string())?;
    } else {
        let _ = key.delete_value("DshLauncher");
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    Err("当前平台不支持开机自启".to_string())
}

#[allow(dead_code)]
fn _unused_order() -> Ordering {
    Ordering::SeqCst
}
