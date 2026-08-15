use std::{
    collections::VecDeque,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::menu::MenuItem;
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Emitter, Manager};

pub const LOG_CAP: usize = 2000;

/// Node.js 最低可用版本（含）：< 22.19.0 视为不可用。
pub const MIN_NODE_VERSION: (u64, u64, u64) = (22, 19, 0);

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
    /// 桌宠读取器：true 表示请求停止（应用退出时置位）。
    pub pet_stop: Arc<AtomicBool>,
    /// 最近一次桌宠状态快照（JSON），供 pet_status 命令即时返回。
    pub pet_snapshot: Arc<Mutex<Option<serde_json::Value>>>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub port: u16,
    pub registry: String,
    pub autostart: bool,
    pub auto_restart: bool,
    pub workspace_dir: Option<String>,
    /// 用户自定义的 Node.js 安装目录（便携版），检测时优先使用该目录下的 node.exe。
    pub node_dir: Option<String>,
    /// 是否显示桌宠（托盘可切换）。
    #[serde(default = "default_pet_enabled")]
    pub pet_enabled: bool,
}

fn default_pet_enabled() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: 3080,
            registry: "https://registry.npmjs.org".to_string(),
            autostart: false,
            auto_restart: true,
            workspace_dir: None,
            node_dir: None,
            pet_enabled: true,
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
    pub dsh_installed: bool,
    pub dsh_version: Option<String>,
    pub dsh_bin: Option<String>,
    /// 是否已安装且版本 >= MIN_NODE_VERSION。
    pub node_version_ok: bool,
    /// 是否已安装但版本不可用（缺失 / 解析失败 / 低于最低版本）。
    pub node_too_old: bool,
    pub nvm_found: bool,
    pub nvm_path: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub phase: String,
    pub url: Option<String>,
    pub pid: Option<u32>,
}

impl Default for EnvInfo {
    fn default() -> Self {
        Self {
            found: false,
            version: None,
            node_path: None,
            npm_prefix: None,
            install_prefix: None,
            dsh_installed: false,
            dsh_version: None,
            dsh_bin: None,
            node_version_ok: false,
            node_too_old: false,
            nvm_found: false,
            nvm_path: None,
        }
    }
}

impl Default for ServerStatus {
    fn default() -> Self {
        Self {
            phase: "stopped".to_string(),
            url: None,
            pid: None,
        }
    }
}

/// 解析 `v22.19.0` / `22.19.0` 这类版本号；非法输入返回 None。
pub fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    let s = v.trim().trim_start_matches('v');
    let mut parts = s.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts
        .next()
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .filter(|p| !p.is_empty())
        .and_then(|p| p.parse::<u64>().ok())
        .unwrap_or(0);
    Some((major, minor, patch))
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

pub fn status_from(
    pid: &Mutex<Option<u32>>,
    url: &Mutex<Option<String>>,
    port: u16,
) -> ServerStatus {
    let pid_val = *pid.lock().unwrap();
    let url_val = url.lock().unwrap().clone();
    let (phase, url_val) = match (pid_val, url_val) {
        (Some(_), None) => ("starting".to_string(), None),
        (Some(_), Some(u)) => {
            if health(&u) {
                ("running".to_string(), Some(u))
            } else {
                ("error".to_string(), Some(u))
            }
        }
        (None, _) => {
            // 应用未启动服务器：探测配置端口上是否已有外部运行的实例
            let probe = format!("http://127.0.0.1:{port}");
            if health(&probe) {
                ("external".to_string(), Some(probe))
            } else {
                ("stopped".to_string(), None)
            }
        }
    };
    ServerStatus {
        phase,
        url: url_val,
        pid: pid_val,
    }
}

#[cfg(windows)]
pub fn find_pid_on_port(port: u16) -> Option<u32> {
    let out = run_output("netstat", &["-ano", "-p", "tcp"]).ok()?;
    let suffix = format!(":{port}");
    for line in out.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5
            && parts[0].eq_ignore_ascii_case("tcp")
            && parts[3].eq_ignore_ascii_case("listening")
            && parts[1].ends_with(&suffix)
        {
            if let Ok(pid) = parts[4].parse::<u32>() {
                return Some(pid);
            }
        }
    }
    None
}

#[cfg(not(windows))]
pub fn find_pid_on_port(_port: u16) -> Option<u32> {
    None
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

pub fn app_prefix_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&base).join("dsh-launcher").join("npm")
}

pub fn resolve_dsh_bin(env: &EnvInfo) -> Option<PathBuf> {
    env.dsh_bin.as_ref().map(PathBuf::from)
}

/// 探测 nvm-windows：PATH → NVM_HOME → %APPDATA%\nvm。返回 nvm 可执行文件路径。
pub fn detect_nvm() -> Option<String> {
    if let Ok(out) = run_output("where.exe", &["nvm"]) {
        if let Some(first) = out.lines().next() {
            let s = first.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Ok(home) = std::env::var("NVM_HOME") {
        let p = Path::new(&home).join("nvm.exe");
        if p.exists() {
            return Some(p.to_string_lossy().to_string());
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let p = Path::new(&appdata).join("nvm").join("nvm.exe");
        if p.exists() {
            return Some(p.to_string_lossy().to_string());
        }
    }
    None
}

/// 从 nodejs.org 的 index.json 解析最新的 22.x 版本号（如 "22.19.0"）。失败返回 None。
pub fn latest_node_lts_major22() -> Option<String> {
    let resp = ureq::get("https://nodejs.org/dist/index.json")
        .timeout(Duration::from_secs(20))
        .call()
        .ok()?;
    let mut reader = resp.into_reader();
    let mut body = String::new();
    reader.read_to_string(&mut body).ok()?;
    let arr: serde_json::Value = serde_json::from_str(&body).ok()?;
    for item in arr.as_array()? {
        let v = item.get("version")?.as_str()?;
        if v.starts_with("v22.") {
            return Some(v.trim_start_matches('v').to_string());
        }
    }
    None
}

/// 依据 npm 镜像源推断 Node 分发下载基地址（国内 npmmirror 自动切换）。
pub fn node_dist_base(registry: &str) -> String {
    if registry.contains("npmmirror") {
        "https://npmmirror.com/mirrors/node".to_string()
    } else {
        "https://nodejs.org/dist".to_string()
    }
}

pub fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(600))
        .call()
        .map_err(|e| format!("下载失败：{e}"))?;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("创建文件失败：{e}"))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("写入失败：{e}"))?;
    Ok(())
}

pub fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开压缩包失败：{e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析压缩包失败：{e}"))?;
    std::fs::create_dir_all(dest).map_err(|e| format!("创建目录失败：{e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("读取压缩包条目失败：{e}"))?;
        // enclosed_name 防止 zip-slip 路径穿越
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).ok();
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut o = std::fs::File::create(&out).map_err(|e| format!("创建文件失败：{e}"))?;
            std::io::copy(&mut entry, &mut o).map_err(|e| format!("解压失败：{e}"))?;
        }
    }
    Ok(())
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

/// 检测环境。`node_dir` 为用户自定义的 Node 目录（优先于 PATH / Program Files）。
pub fn detect_env(node_dir: Option<&str>) -> EnvInfo {
    let version_on_path = run_output("node", &["--version"]).ok();

    let node_path = node_dir
        .map(|dir| {
            let p = Path::new(dir).join("node.exe");
            p.exists().then(|| p.to_string_lossy().to_string())
        })
        .flatten()
        .or_else(|| {
            run_output("where.exe", &["node"])
                .ok()
                .and_then(|o| o.lines().next().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
        })
        .or_else(fallback_node_path);

    let node = node_path.clone().unwrap_or_else(|| "node".to_string());

    let version = if node_path.is_some() {
        run_output(&node, &["--version"]).ok()
    } else {
        None
    }
    .or(version_on_path);

    let npm_prefix = npm_cli(&node)
        .and_then(|cli| run_output(&node, &[cli.to_str().unwrap_or(""), "prefix", "-g"]).ok())
        .filter(|s| !s.is_empty());

    // 应用专用安装目录（始终使用，隔离且免管理员权限）
    let install_prefix = {
        let dir = app_prefix_dir();
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.to_string_lossy().to_string())
    };

    // 依次尝试：应用目录 → 全局前缀；以 `dsh --version` 能否成功为准（排除残缺安装）
    let mut dsh_bin_path: Option<PathBuf> = None;
    let mut dsh_version: Option<String> = None;
    for prefix in [install_prefix.as_deref(), npm_prefix.as_deref()]
        .into_iter()
        .flatten()
    {
        if let Some(bin) = dsh_bin(prefix) {
            if let Ok(v) = run_output(&node, &[bin.to_str().unwrap_or(""), "--version"]) {
                dsh_bin_path = Some(bin);
                dsh_version = Some(v);
                break;
            }
        }
    }

    let found = node_path.is_some() && version.is_some();
    let version_tuple = version.as_deref().and_then(parse_semver);
    let node_version_ok = found && version_tuple.is_some_and(|v| v >= MIN_NODE_VERSION);
    let node_too_old = found && !node_version_ok;

    let nvm_path = detect_nvm();

    EnvInfo {
        found,
        version,
        node_path,
        npm_prefix,
        install_prefix,
        dsh_installed: dsh_bin_path.is_some(),
        dsh_version,
        dsh_bin: dsh_bin_path.map(|p| p.to_string_lossy().to_string()),
        node_version_ok,
        node_too_old,
        nvm_found: nvm_path.is_some(),
        nvm_path,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_semver() {
        assert_eq!(parse_semver("v22.19.0"), Some((22, 19, 0)));
        assert_eq!(parse_semver("22.19.0"), Some((22, 19, 0)));
        assert_eq!(parse_semver("20.0.0"), Some((20, 0, 0)));
        assert_eq!(parse_semver("22.19"), Some((22, 19, 0)));
        assert_eq!(parse_semver("abc"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn compares_against_min() {
        assert!(parse_semver("v22.19.0").unwrap() >= MIN_NODE_VERSION);
        assert!(parse_semver("v22.20.1").unwrap() >= MIN_NODE_VERSION);
        assert!(parse_semver("v20.0.0").unwrap() < MIN_NODE_VERSION);
    }
}
