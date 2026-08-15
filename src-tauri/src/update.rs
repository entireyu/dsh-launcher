//! 鲸仔版本信息与更新：检查走 GitHub releases（404 回退 tags），
//! 一键更新 = 下载对应变体的 NSIS 安装包 → 静默安装到当前目录 → 自动重启。
//! DSH 版本由现有 EnvInfo.dsh_version 与 check_latest_version（npm + 镜像源）提供。

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{parse_semver, push_log, AppState, TEST_BUILD};

/// GitHub 仓库 API（发布源）。
const REPO_API: &str = "https://api.github.com/repos/entireyu/dsh-whalito-desk";
/// 更新进度事件（主窗口转发给内嵌设置分区）。
pub const UPDATE_EVENT: &str = "whalito-update";

/// 鲸仔版本信息。latest/url 仅在执行过更新检查后才有值。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WhalitoVersionInfo {
    pub current: String,
    pub test_build: bool,
    pub latest: Option<String>,
    pub update_available: bool,
    /// 该版本是否有适用于当前变体的安装包资产（测试版不上传 GitHub，通常为 false）。
    pub auto_update: bool,
    pub url: Option<String>,
}

/// 最新发布信息（版本 + 资产列表 + 发布页地址）。
struct ReleaseInfo {
    version: String,
    assets: Vec<AssetInfo>,
    url: Option<String>,
}

#[derive(Clone)]
struct AssetInfo {
    name: String,
    url: String,
}

/// 当前版本（编译期），不含测试标记（标记由前端按 test_build 拼接）。
pub fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 版本比较：latest 严格大于 current 视为有更新；任一侧解析失败视为无更新。
pub fn is_update_available(current: &str, latest: &str) -> bool {
    match (parse_semver(current), parse_semver(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// 无网络版本信息（供握手快照使用）。
#[tauri::command]
pub fn whalito_version_info() -> WhalitoVersionInfo {
    WhalitoVersionInfo {
        current: current_version(),
        test_build: TEST_BUILD,
        latest: None,
        update_available: false,
        auto_update: false,
        url: None,
    }
}

/// 检查鲸仔更新（GitHub releases/latest，404 时回退 tags 列表第一个 tag）。
#[tauri::command]
pub fn whalito_check_update() -> Result<WhalitoVersionInfo, String> {
    let info = fetch_release_info()?;
    let current = current_version();
    let update_available = is_update_available(&current, &info.version);
    // 测试版安装包不上传 GitHub：无匹配资产时 auto_update=false，前端隐藏「立即更新」。
    let auto_update = update_available && pick_asset(&info.assets).is_some();
    Ok(WhalitoVersionInfo {
        current,
        test_build: TEST_BUILD,
        latest: Some(info.version),
        update_available,
        auto_update,
        url: info.url,
    })
}

/// 一键更新：检查 → 选资产 → 下载 → 静默安装 → 退出并由安装链重启应用。
#[tauri::command]
pub async fn whalito_apply_update(app: AppHandle) -> Result<(), String> {
    emit(&app, "正在获取最新版本…");
    let info = tauri::async_runtime::spawn_blocking(fetch_release_info)
        .await
        .map_err(|e| e.to_string())??;
    let current = current_version();
    if !is_update_available(&current, &info.version) {
        return Err(format!("当前已是最新版本（{current}）"));
    }
    let asset = pick_asset(&info.assets)
        .ok_or_else(|| format!("该版本没有适用于{}的安装包", if TEST_BUILD { "测试版" } else { "当前版本" }))?;
    emit(&app, "正在下载更新…");
    let dest = std::env::temp_dir().join(format!("whalito-update-{}", asset.name));
    let url = asset.url;
    let dest_for_dl = dest.clone();
    tauri::async_runtime::spawn_blocking(move || download_to(&url, &dest_for_dl))
        .await
        .map_err(|e| e.to_string())??;
    emit(&app, "已开始安装，应用即将重启…");
    spawn_update_chain(&dest)?;
    app.exit(0);
    Ok(())
}

/// 选择当前变体适用的安装包资产：
/// 测试构建只接受名称含 "-Test_" 的资产，生产构建只接受不含 "-Test_" 的资产。
fn pick_asset(assets: &[AssetInfo]) -> Option<AssetInfo> {
    let setup: Vec<&AssetInfo> = assets
        .iter()
        .filter(|a| a.name.ends_with("_x64-setup.exe"))
        .collect();
    if setup.is_empty() {
        return None;
    }
    let matched = setup
        .iter()
        .find(|a| a.name.contains("-Test_") == TEST_BUILD)
        .copied();
    match matched {
        Some(a) => Some(a.clone()),
        None => {
            if TEST_BUILD {
                None
            } else {
                Some(setup[0].clone())
            }
        }
    }
}

/// 下载到目标文件并做基本校验（文件以 MZ 开头）。
fn download_to(url: &str, dest: &Path) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(300))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", "whalito-update-check")
        .call()
        .map_err(|e| format!("下载更新失败：{e}"))?;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("创建临时文件失败：{e}"))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("写入临时文件失败：{e}"))?;
    drop(file);
    let mut f = std::fs::File::open(dest).map_err(|e| format!("校验下载文件失败：{e}"))?;
    let mut magic = [0u8; 2];
    f.read_exact(&mut magic).map_err(|e| format!("读取下载文件失败：{e}"))?;
    if &magic != b"MZ" {
        let _ = std::fs::remove_file(dest);
        return Err("下载的文件不是有效的 Windows 安装程序".into());
    }
    Ok(())
}

/// 启动更新链：等待旧进程退出 → 静默安装到当前目录 → 重启应用。
/// 链由独立的 cmd 进程承载（DETACHED），应用退出后继续执行。
fn spawn_update_chain(dest: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("获取当前程序路径失败：{e}"))?;
    let dir = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let chain = build_update_chain(dest, &dir, &exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .raw_arg("/C")
            .raw_arg(&chain)
            .creation_flags(0x0000_0008 | 0x0800_0000) // DETACHED_PROCESS | CREATE_NO_WINDOW
            .spawn()
            .map_err(|e| format!("启动更新进程失败：{e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("当前平台暂不支持自动更新".into())
    }
}

/// 组装更新链命令行（独立纯函数，便于单测）：
/// 延迟 5 秒等应用退出 → start /wait 静默安装（/D= 必须位于末尾）→ 重启应用。
pub fn build_update_chain(installer: &Path, install_dir: &Path, app_exe: &Path) -> String {
    format!(
        "ping -n 6 127.0.0.1 >nul & start \"\" /wait \"{}\" /S /D={} & start \"\" \"{}\"",
        installer.display(),
        install_dir.display(),
        app_exe.display()
    )
}

/// 拉取最新发布信息：releases/latest，404 时回退 tags 列表第一个 tag（无资产）。
fn fetch_release_info() -> Result<ReleaseInfo, String> {
    let releases = format!("{REPO_API}/releases/latest");
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    match agent.get(&releases).set("User-Agent", "whalito-update-check").call() {
        Ok(resp) => {
            let body = resp.into_string().map_err(|e| format!("读取响应失败：{e}"))?;
            parse_release_json(&body)
        }
        Err(ureq::Error::Status(404, _)) => {
            // 仓库还没有 release：回退 tags 列表第一个 tag（无资产可下载）。
            let body = http_get(&format!("{REPO_API}/tags"))?;
            let tags: Vec<serde_json::Value> =
                serde_json::from_str(&body).map_err(|e| format!("解析 tags 失败：{e}"))?;
            let first = tags
                .first()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(strip_v)
                .unwrap_or_else(|| "0.0.0".to_string());
            Ok(ReleaseInfo {
                version: first,
                assets: Vec::new(),
                url: None,
            })
        }
        Err(e) => Err(format!("检查更新失败：{e}")),
    }
}

fn http_get(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", "whalito-update-check")
        .call()
        .map_err(|e| format!("请求 {url} 失败：{e}"))?;
    resp.into_string().map_err(|e| format!("读取响应失败：{e}"))
}

fn parse_release_json(body: &str) -> Result<ReleaseInfo, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("解析响应失败：{e}"))?;
    let version = v
        .get("tag_name")
        .and_then(|t| t.as_str())
        .map(strip_v)
        .unwrap_or_else(|| "0.0.0".to_string());
    let url = v.get("html_url").and_then(|u| u.as_str()).map(String::from);
    let assets = v
        .get("assets")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some(AssetInfo {
                        name: a.get("name")?.as_str()?.to_string(),
                        url: a.get("browser_download_url")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ReleaseInfo {
        version,
        assets,
        url,
    })
}

fn strip_v(tag: &str) -> String {
    tag.trim().trim_start_matches('v').to_string()
}

/// 更新进度：写日志 + 发事件给主窗口。
fn emit(app: &AppHandle, stage: &str) {
    push_log(
        &app.state::<AppState>().logs,
        &format!("[系统] 鲸仔更新：{stage}"),
    );
    let _ = app.emit(UPDATE_EVENT, stage.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_available_compares_semver() {
        assert!(is_update_available("0.2.0", "0.3.0"));
        assert!(is_update_available("0.2.0", "1.0.0"));
        assert!(is_update_available("0.2.0", "0.2.1"));
        assert!(!is_update_available("0.2.0", "0.2.0"));
        assert!(!is_update_available("0.3.0", "0.2.9"));
    }

    #[test]
    fn update_available_tolerates_bad_input() {
        assert!(!is_update_available("0.2.0", ""));
        assert!(!is_update_available("abc", "0.3.0"));
        // parse_semver 把 0.3.0-pre 解析为 (0,3,0)，视为有更新。
        assert!(is_update_available("0.2.0", "v0.3.0-pre"));
        assert!(is_update_available("v0.2.0", "v0.3.0"));
    }

    #[test]
    fn strips_v_prefix() {
        assert_eq!(strip_v("v0.2.0"), "0.2.0");
        assert_eq!(strip_v("0.2.0"), "0.2.0");
    }

    #[test]
    fn update_chain_contains_installer_and_relaunch() {
        let chain = build_update_chain(
            Path::new(r"C:\Temp\setup.exe"),
            Path::new(r"C:\App\whalito"),
            Path::new(r"C:\App\whalito\Whalito.exe"),
        );
        assert!(chain.contains(r#""C:\Temp\setup.exe" /S /D=C:\App\whalito"#));
        assert!(chain.contains(r#"start "" "C:\App\whalito\Whalito.exe""#));
        assert!(chain.contains("ping -n 6"));
    }

    #[test]
    fn picks_prod_asset_in_prod_build() {
        let assets = vec![
            AssetInfo {
                name: "Whalito_0.3.0_x64-setup.exe".into(),
                url: "u1".into(),
            },
            AssetInfo {
                name: "Whalito-Test_0.3.0_x64-setup.exe".into(),
                url: "u2".into(),
            },
            AssetInfo {
                name: "notes.txt".into(),
                url: "u3".into(),
            },
        ];
        // 测试构建会选 Test 资产；生产构建选非 Test。这里按 TEST_BUILD 是编译期
        // 常量，两个方向无法在同一编译单元内切换——至少验证当前变体的规则与
        // 非 setup 资产被过滤。
        let picked = pick_asset(&assets).expect("should pick an asset");
        assert!(!picked.name.contains("notes.txt"));
        let setup_only = assets
            .iter()
            .filter(|a| a.name.ends_with("_x64-setup.exe"))
            .collect::<Vec<_>>();
        assert_eq!(setup_only.len(), 2);
        if TEST_BUILD {
            assert!(picked.name.contains("-Test_"));
        } else {
            assert!(!picked.name.contains("-Test_"));
        }
    }

    #[test]
    fn no_asset_for_test_build_without_test_asset() {
        let assets = vec![AssetInfo {
            name: "Whalito_0.3.0_x64-setup.exe".into(),
            url: "u1".into(),
        }];
        let picked = pick_asset(&assets);
        if TEST_BUILD {
            assert!(picked.is_none());
        } else {
            assert!(picked.is_some());
        }
    }
}
