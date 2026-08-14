use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder};

use crate::commands;
use crate::state::AppState;

/// 注入到内嵌 dsh 页面里的悬浮按钮脚本（在每次文档创建时执行）。
pub const INJECT_JS: &str = include_str!("../inject.js");

/// 悬浮按钮动作通过导航到一个虚拟主机名触发，由 on_navigation 拦截，
/// 不真正发起网络请求，也无需为远程页面开启 IPC。
const ACTION_HOST: &str = "dshlauncher.local";

fn start_server_action(app: &AppHandle) {
    let st = app.state::<AppState>();
    let shared = commands::Shared::from_state(&st);
    let port = shared.settings.lock().unwrap().port;
    let app = app.clone();
    std::thread::spawn(move || match commands::start_server_impl(&app, &shared) {
        Ok(_) => {
            if let Some(w) = app.get_webview_window("embed") {
                if let Ok(u) = Url::parse(&format!("http://127.0.0.1:{port}")) {
                    let _ = w.navigate(u);
                }
            }
        }
        Err(e) => {
            let _ = app.emit_to("main", "log", format!("[系统] 启动服务器失败：{e}"));
        }
    });
}

fn stop_server_action(app: &AppHandle) {
    let st = app.state::<AppState>();
    let shared = commands::Shared::from_state(&st);
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = commands::stop_server_inner(&app, &shared);
        if let Some(w) = app.get_webview_window("embed") {
            let _ = w.close();
        }
        crate::show_main(&app);
    });
}

fn restart_server_action(app: &AppHandle) {
    let st = app.state::<AppState>();
    let shared = commands::Shared::from_state(&st);
    let port = shared.settings.lock().unwrap().port;
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = commands::stop_server_inner(&app, &shared);
        match commands::start_server_impl(&app, &shared) {
            Ok(_) => {
                if let Some(w) = app.get_webview_window("embed") {
                    if let Ok(u) = Url::parse(&format!("http://127.0.0.1:{port}")) {
                        let _ = w.navigate(u);
                    }
                }
            }
            Err(e) => {
                let _ = app.emit_to("main", "log", format!("[系统] 重启服务器失败：{e}"));
            }
        }
    });
}

/// 返回 true 表示允许导航；对虚拟主机名的动作返回 false（拦截）。
fn handle_navigation(app: &AppHandle, url: &Url) -> bool {
    if url.host_str() != Some(ACTION_HOST) {
        return true;
    }
    let action = url.path().trim_start_matches('/');
    match action {
        "focus-main" => crate::show_main(app),
        "open-settings" => {
            crate::show_main(app);
            let _ = app.emit_to("main", "open-settings", ());
        }
        "start-server" => start_server_action(app),
        "stop-server" => stop_server_action(app),
        "restart-server" => restart_server_action(app),
        _ => {}
    }
    false
}

#[tauri::command]
pub fn open_embedded(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|e| format!("无效地址：{e}"))?;

    if let Some(w) = app.get_webview_window("embed") {
        let _ = w.navigate(parsed);
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }

    let app_for_nav = app.clone();
    WebviewWindowBuilder::new(&app, "embed", WebviewUrl::External(parsed))
        .title("DeepSeek Harness")
        .inner_size(1280.0, 800.0)
        .min_inner_size(720.0, 520.0)
        .initialization_script(INJECT_JS)
        .on_navigation(move |u| handle_navigation(&app_for_nav, u))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}
