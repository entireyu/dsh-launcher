//! 鲸仔设置分区插件同步：把内嵌的 DSH 客户端插件包幂等写入 web profile
//! （node_modules + cordis.patch.yml 标记块）。所有维护动作只发生在标记块内，
//! 标记块外的用户内容绝不改动。不动 deepseek-harness 源码。

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::{push_log, AppState};

/// 插件包名（同时是 cordis 条目名与 node_modules 下的目录名）。
pub const PKG_NAME: &str = "@entireyu/whalito-dsh-settings";
/// 内嵌插件文件（编译期携带；运行时按内容比对同步，无需外部依赖）。
const PKG_FILES: &[(&str, &str)] = &[
    ("package.json", include_str!("../whalito-dsh-settings/package.json")),
    ("index.js", include_str!("../whalito-dsh-settings/index.js")),
    ("client.js", include_str!("../whalito-dsh-settings/client.js")),
];

/// cordis.patch.yml 中的托管标记：同步只替换/追加这两个标记之间的块。
const MARK_BEGIN: &str = "# ⟪ whalito-managed begin ⟫";
const MARK_END: &str = "# ⟪ whalito-managed end ⟫";

/// 同步结果（installed=已就绪无变化 / updated=有写入 / skipped=跳过）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PluginSyncReport {
    pub status: String,
    pub detail: String,
    pub profile_dir: String,
}

/// DSH 家目录。测试构建固定使用隔离的 ~/.dsh-test（忽略外部 DSH_HOME，
/// 避免误指生产数据）；生产构建 DSH_HOME 环境变量优先，否则 ~/.dsh。
pub fn dsh_home() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    if crate::state::TEST_BUILD {
        return PathBuf::from(base).join(".dsh-test");
    }
    if let Ok(h) = std::env::var("DSH_HOME") {
        if !h.trim().is_empty() {
            return PathBuf::from(h);
        }
    }
    PathBuf::from(base).join(".dsh")
}

/// web profile 目录（鲸仔只启动 dsh web 这一个 profile）。
pub fn profile_dir() -> PathBuf {
    dsh_home().join("profiles").join("web")
}

/// 生成托管的 cordis.patch.yml 标记块。
fn managed_block() -> String {
    format!(
        "{}\n- insert:\n    - id: whalito-settings\n      name: '{}'\n{}\n",
        MARK_BEGIN, PKG_NAME, MARK_END
    )
}

/// 判断现有补丁层是否为"仅空列表"（新 profile 默认内容：注释 + `[]`）。
/// 这种文件不能直接在 `[]` 后追加 block 条目（YAML 会把 flow 序列
/// 之后的内容判为语法错误），必须把 `[]` 就地替换为块。
fn bare_empty_list(existing: &str) -> bool {
    let mut items: Vec<&str> = Vec::new();
    for raw in existing.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        items.push(line);
    }
    items.len() == 1 && items[0] == "[]"
}

/// 在标记块内 upsert 插件 insert 行：两个标记都存在则整块替换；
/// 否则追加——若文件是"仅空列表"则把 `[]` 就地替换为块（避免
/// flow 序列后接 block 条目的 YAML 语法错误）。标记块外内容原样保留。
pub fn upsert_marker_block(existing: &str) -> String {
    match (existing.find(MARK_BEGIN), existing.find(MARK_END)) {
        (Some(begin), Some(end)) if end > begin => {
            let tail_start = match existing[end..].find('\n') {
                Some(i) => end + i + 1,
                None => existing.len(),
            };
            let mut out = String::with_capacity(existing.len() + managed_block().len());
            out.push_str(&existing[..begin]);
            out.push_str(&managed_block());
            out.push_str(&existing[tail_start..]);
            out
        }
        _ => {
            let mut out = existing.to_string();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            if bare_empty_list(existing) {
                if let Some(pos) = out.find("[]") {
                    out.replace_range(pos..pos + 2, &managed_block());
                    return out;
                }
            }
            out.push_str(&managed_block());
            out
        }
    }
}

/// 把内嵌插件文件写入 profile 的 node_modules；按内容比对，返回是否有写入。
pub fn sync_package_files(profile: &Path) -> Result<bool, String> {
    let pkg_dir = profile.join("node_modules").join(PKG_NAME);
    let mut changed = false;
    for (name, content) in PKG_FILES {
        let path = pkg_dir.join(name);
        let same = fs::read_to_string(&path)
            .map(|c| c == *content)
            .unwrap_or(false);
        if same {
            continue;
        }
        fs::create_dir_all(&pkg_dir)
            .map_err(|e| format!("创建插件目录 {} 失败：{e}", pkg_dir.display()))?;
        fs::write(&path, *content)
            .map_err(|e| format!("写入插件文件 {} 失败：{e}", path.display()))?;
        changed = true;
    }
    Ok(changed)
}

/// 维护 cordis.patch.yml 标记块；返回是否有写入。
pub fn sync_patch_layer(profile: &Path) -> Result<bool, String> {
    let path = profile.join("cordis.patch.yml");
    let current = if path.exists() {
        fs::read_to_string(&path).map_err(|e| format!("读取 cordis.patch.yml 失败：{e}"))?
    } else {
        // 文件不存在：直接写标记块（不要先写 `[]` 再追加，避免中间态语法错误）。
        fs::write(&path, managed_block()).map_err(|e| format!("写入 cordis.patch.yml 失败：{e}"))?;
        return Ok(true);
    };
    let updated = upsert_marker_block(&current);
    if updated == current {
        return Ok(false);
    }
    fs::write(&path, updated).map_err(|e| format!("写入 cordis.patch.yml 失败：{e}"))?;
    Ok(true)
}

/// 幂等同步：写入插件文件 + 维护标记块，并写入应用日志。
/// 错误会先记日志再返回（调用方可决定是否继续阻断）。
pub fn ensure_settings_plugin(app: &AppHandle) -> Result<PluginSyncReport, String> {
    let profile = profile_dir();
    let profile_str = profile.to_string_lossy().into_owned();
    if !profile.exists() {
        // dsh 首次启动会补全 package.json / cordis.yml；这里先创建目录并预置
        // 插件包与补丁层，让首次启动即装载"鲸仔"分区（无需二次重启）。
        fs::create_dir_all(&profile)
            .map_err(|e| format!("创建 profile 目录 {} 失败：{e}", profile.display()))?;
    }
    match (|| -> Result<PluginSyncReport, String> {
        let files_changed = sync_package_files(&profile)?;
        let patch_changed = sync_patch_layer(&profile)?;
        let status = if files_changed || patch_changed { "updated" } else { "installed" };
        Ok(PluginSyncReport {
            status: status.into(),
            detail: format!("插件包已就绪（{}）", PKG_NAME),
            profile_dir: profile_str.clone(),
        })
    })() {
        Ok(report) => {
            push_log(
                &app.state::<AppState>().logs,
                &format!("[系统] 鲸仔设置分区插件：{}（{profile_str}）", report.detail),
            );
            Ok(report)
        }
        Err(e) => {
            push_log(
                &app.state::<AppState>().logs,
                &format!("[系统] 鲸仔设置分区插件同步失败：{e}"),
            );
            Err(e)
        }
    }
}

/// 供面板手动触发重同步/排障的命令。
#[tauri::command]
pub fn sync_settings_plugin(app: AppHandle) -> Result<PluginSyncReport, String> {
    ensure_settings_plugin(&app)
}

/// 诊断辅助：把鲸仔桥事件追加写入 %TEMP%\whalito-bridge.log，
/// 供排障时直接查看（内嵌页 postMessage 链路两侧的行为都落到这里）。
#[tauri::command]
pub fn bridge_diag(line: String) {
    use std::io::Write;
    let path = std::env::temp_dir().join("whalito-bridge.log");
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{secs}] {line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_block_when_markers_absent() {
        let input = "# 用户自己的注释\n- id: some-other\n  name: 'x'\n";
        let out = upsert_marker_block(input);
        assert!(out.starts_with(input));
        assert!(out.contains(MARK_BEGIN));
        assert!(out.contains(MARK_END));
        assert!(out.contains("id: whalito-settings"));
        assert!(out.contains("name: '@entireyu/whalito-dsh-settings'"));
    }

    #[test]
    fn replaces_existing_managed_block_only() {
        let stale = format!(
            "{}\n- insert:\n    - id: whalito-settings\n      name: 'stale'\n{}\n",
            MARK_BEGIN, MARK_END
        );
        let input = format!("# 前文\n{stale}# 后文\n");
        let out = upsert_marker_block(&input);
        assert!(out.starts_with("# 前文\n"));
        assert!(out.ends_with("# 后文\n"));
        assert!(!out.contains("name: 'stale'"));
        assert!(out.contains("name: '@entireyu/whalito-dsh-settings'"));
        assert_eq!(out.matches(MARK_BEGIN).count(), 1);
        assert_eq!(out.matches(MARK_END).count(), 1);
    }

    #[test]
    fn converts_bare_empty_list_to_block() {
        let input = "# Your patch layer for this dsh profile.\n[]\n";
        let out = upsert_marker_block(input);
        assert!(!out.contains("[]"));
        assert!(out.contains(MARK_BEGIN));
        assert!(out.contains("- insert:"));
        assert!(out.contains("id: whalito-settings"));
        assert!(out.starts_with("# Your patch layer"));
    }

    #[test]
    fn appends_after_user_block_items() {
        let input = "- id: user-row\n  name: 'user-pkg'\n";
        let out = upsert_marker_block(input);
        assert!(out.starts_with(input));
        assert!(out.contains(MARK_BEGIN));
        assert!(out.contains("id: whalito-settings"));
    }

    #[test]
    fn is_idempotent() {
        let input = "# 前\n- id: a\n  name: 'b'\n";
        let once = upsert_marker_block(input);
        let twice = upsert_marker_block(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn sync_package_files_roundtrip() {
        let base = std::env::temp_dir().join(format!("whalito-sync-test-{}", std::process::id()));
        let profile = base.join("profiles").join("web");
        let _ = fs::remove_dir_all(&base);
        assert!(sync_package_files(&profile).unwrap());
        assert!(!sync_package_files(&profile).unwrap());
        for (name, content) in PKG_FILES {
            let path = profile.join("node_modules").join(PKG_NAME).join(name);
            assert_eq!(fs::read_to_string(&path).unwrap(), *content);
        }
        let _ = fs::remove_dir_all(&base);
    }
}
