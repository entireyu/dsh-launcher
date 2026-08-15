//! 桌宠样式契约：`$DSH_HOME/pet-style.json`（生产 ~/.dsh、测试 ~/.dsh-test）。
//! 用户可通过 DSH（agent 直接编辑该 JSON）或未来 UI 调整桌宠外观；
//! Pet.vue 只作为"默认渲染器"，样式全部来自本契约，文件变更 2 秒内热更新。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::settings_plugin::dsh_home;

/// 样式变更广播事件（pet 窗口监听）。
pub const STYLE_EVENT: &str = "pet-style";
const STYLE_FILENAME: &str = "pet-style.json";
/// 头像 data URI 上限（超出视为非法，回退内置 logo）。
const MAX_AVATAR_BYTES: usize = 256 * 1024;

/// 桌宠样式（全字段可选，缺省走 Default；非法值 sanitize 回退）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct PetStyle {
    pub schema_version: u32,
    /// 鲸仔直径（px），clamp 48..160。
    pub size: f32,
    /// 窗口位置（物理坐标）；null = 默认右下角。
    pub position: Option<PetPosition>,
    /// 头像：本地路径或 data URI；null = 内置 logo。
    pub avatar: Option<String>,
    /// 强调色（徽标/菜单悬停等）。
    pub accent: String,
    pub bubble: PetBubbleStyle,
    pub animations: PetAnimations,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PetPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct PetBubbleStyle {
    pub bg: String,
    pub fg: String,
    pub sub: String,
    pub font_size: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct PetAnimations {
    pub bob: bool,
    pub float: bool,
    pub attention: bool,
}

impl Default for PetStyle {
    fn default() -> Self {
        Self {
            schema_version: 1,
            size: 96.0,
            position: None,
            avatar: None,
            accent: "#f87171".into(),
            bubble: PetBubbleStyle::default(),
            animations: PetAnimations::default(),
        }
    }
}

impl Default for PetBubbleStyle {
    fn default() -> Self {
        Self {
            bg: "#171a21".into(),
            fg: "#e8eaf0".into(),
            sub: "#9aa3b2".into(),
            font_size: 12.0,
        }
    }
}

impl Default for PetAnimations {
    fn default() -> Self {
        Self {
            bob: true,
            float: true,
            attention: true,
        }
    }
}

impl PetStyle {
    /// 范围与合法性收敛：越界数值与非法颜色/超大头像一律回退默认值。
    pub fn sanitize(mut self) -> Self {
        if !(48.0..=160.0).contains(&self.size) {
            self.size = 96.0;
        }
        if !(10.0..=18.0).contains(&self.bubble.font_size) {
            self.bubble.font_size = 12.0;
        }
        if self.accent.trim().is_empty() {
            self.accent = "#f87171".into();
        }
        if self.bubble.bg.trim().is_empty() {
            self.bubble.bg = "#171a21".into();
        }
        if self.bubble.fg.trim().is_empty() {
            self.bubble.fg = "#e8eaf0".into();
        }
        if self.bubble.sub.trim().is_empty() {
            self.bubble.sub = "#9aa3b2".into();
        }
        if let Some(a) = &self.avatar {
            if a.trim().is_empty() || a.len() > MAX_AVATAR_BYTES {
                self.avatar = None;
            }
        }
        self
    }
}

/// 契约文件路径（DSH 家目录内；测试构建自动指向 ~/.dsh-test）。
pub fn style_path() -> PathBuf {
    dsh_home().join(STYLE_FILENAME)
}

/// 解析样式文本：非法 JSON / 类型错误 → 默认值（字段级容错由 serde default 保证）。
pub fn parse_style(text: &str) -> PetStyle {
    serde_json::from_str::<PetStyle>(text)
        .map(|s| s.sanitize())
        .unwrap_or_else(|_| PetStyle::default())
}

/// 读取当前生效样式（文件缺失/损坏 → 默认值）。
pub fn load() -> PetStyle {
    match std::fs::read_to_string(style_path()) {
        Ok(text) => parse_style(&text),
        Err(_) => PetStyle::default(),
    }
}

/// 写回契约文件。
pub fn save(style: &PetStyle) -> Result<(), String> {
    let path = style_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建样式目录失败：{e}"))?;
    }
    let text = serde_json::to_string_pretty(style).map_err(|e| format!("序列化样式失败：{e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("写入样式文件失败：{e}"))
}

/// 广播样式给 pet 窗口。
pub fn broadcast_style(app: &AppHandle, style: &PetStyle) {
    if let Ok(v) = serde_json::to_value(style) {
        let _ = app.emit(STYLE_EVENT, v);
    }
}

/// 按样式契约应用窗口位置（position 为 null 时不动窗口）。
pub fn apply_position(app: &AppHandle, pos: Option<&PetPosition>) {
    if let (Some(p), Some(w)) = (pos, app.get_webview_window("pet")) {
        let _ = w.set_position(tauri::PhysicalPosition::new(p.x, p.y));
    }
}

#[tauri::command]
pub fn pet_get_style() -> PetStyle {
    load()
}

/// 整体替换样式（UI/agent 均可调用；会 sanitize 并写回文件、广播、应用位置）。
#[tauri::command]
pub fn pet_set_style(app: AppHandle, style: PetStyle) -> Result<PetStyle, String> {
    let style = style.sanitize();
    save(&style)?;
    broadcast_style(&app, &style);
    apply_position(&app, style.position.as_ref());
    Ok(style)
}

/// 拖拽后的位置落盘（物理坐标）。
#[tauri::command]
pub fn pet_set_position(app: AppHandle, x: f64, y: f64) -> Result<(), String> {
    let mut style = load();
    style.position = Some(PetPosition { x, y });
    save(&style)?;
    broadcast_style(&app, &style);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = PetStyle::default();
        assert_eq!(s.size, 96.0);
        assert!(s.animations.bob);
        assert_eq!(s.bubble.font_size, 12.0);
        assert!(s.position.is_none());
        assert!(s.avatar.is_none());
    }

    #[test]
    fn parse_merges_partial_json() {
        let s = parse_style(r##"{"size":120,"bubble":{"bg":"#000000"}}"##);
        assert_eq!(s.size, 120.0);
        assert_eq!(s.bubble.bg, "#000000");
        assert_eq!(s.bubble.fg, "#e8eaf0"); // 未提供字段走默认
        assert!(s.animations.bob);
    }

    #[test]
    fn parse_bad_json_falls_back_to_default() {
        let s = parse_style("{not json");
        assert_eq!(s.size, 96.0);
    }

    #[test]
    fn sanitize_clamps_and_falls_back() {
        let s = PetStyle {
            schema_version: 1,
            size: 999.0,
            position: Some(PetPosition { x: 1.0, y: 2.0 }),
            avatar: Some("x".repeat(MAX_AVATAR_BYTES + 1)),
            accent: "".into(),
            bubble: PetBubbleStyle { bg: "#123456".into(), fg: "".into(), sub: "#abcdef".into(), font_size: 99.0 },
            animations: PetAnimations { bob: false, float: false, attention: false },
        }
        .sanitize();
        assert_eq!(s.size, 96.0);
        assert!(s.avatar.is_none());
        assert_eq!(s.accent, "#f87171");
        assert_eq!(s.bubble.fg, "#e8eaf0");
        assert_eq!(s.bubble.font_size, 12.0);
        assert_eq!(s.bubble.bg, "#123456");
        assert_eq!(s.bubble.sub, "#abcdef");
        assert!(!s.animations.bob);
    }

    #[test]
    fn style_roundtrip_through_json() {
        let s = PetStyle::default().sanitize();
        let text = serde_json::to_string(&s).unwrap();
        let back = parse_style(&text);
        assert_eq!(back.size, s.size);
        assert_eq!(back.bubble.bg, s.bubble.bg);
    }
}
