mod commands;
mod pet;
mod pet_style;
mod settings_plugin;
mod state;
mod update;

use std::sync::atomic::Ordering;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

use state::AppState;

pub(crate) fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<tauri::tray::TrayIcon> {
    let show = MenuItem::with_id(app, "show", "打开面板", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "启动服务器", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止服务器", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "在浏览器打开", true, None::<&str>)?;
    let pet = MenuItem::with_id(app, "pet", "显示 / 隐藏桌宠", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &start,
            &stop,
            &open,
            &pet,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    {
        let st = app.state::<AppState>();
        *st.tray_start.lock().unwrap() = Some(start.clone());
        *st.tray_stop.lock().unwrap() = Some(stop.clone());
    }

    // 悬浮提示：应用名称；测试构建末尾追加（测试版）标记。
    let tooltip = if crate::state::TEST_BUILD {
        "鲸仔 Whalito（测试版）"
    } else {
        "鲸仔 Whalito"
    };
    let mut builder = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip(tooltip)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "start" => {
                let _ = app.emit("tray-action", "start");
            }
            "stop" => {
                let _ = app.emit("tray-action", "stop");
            }
            "open" => {
                let _ = app.emit("tray-action", "open");
            }
            "pet" => {
                let st = app.state::<AppState>();
                let enabled = !st.settings.lock().unwrap().pet_enabled;
                let _ = pet::set_enabled(app, enabled);
            }
            "quit" => {
                app.state::<AppState>().quitting.store(true, Ordering::SeqCst);
                app.state::<AppState>().pet_stop.store(true, Ordering::SeqCst);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::detect_env,
            commands::install_node,
            commands::upgrade_node,
            commands::install_node_nvm,
            commands::install_node_portable,
            commands::pick_node_dir,
            commands::install_dsh,
            commands::update_dsh,
            commands::verify_dsh,
            commands::check_latest_version,
            commands::start_server,
            commands::stop_server,
            commands::restart_server,
            commands::server_status,
            commands::update_tray_state,
            commands::get_settings,
            commands::save_settings,
            commands::set_autostart,
            commands::open_url,
            commands::get_logs,
            settings_plugin::sync_settings_plugin,
            settings_plugin::bridge_diag,
            pet::show_main_window,
            pet::quit_app,
            update::whalito_version_info,
            update::whalito_check_update,
            pet::pet_status,
            pet::pet_open_session,
            pet::pet_respond,
            pet::pet_set_enabled,
            pet_style::pet_get_style,
            pet_style::pet_set_style,
            pet_style::pet_set_position,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = state::load_settings(&handle);
            let autostart_flag = std::env::args().any(|a| a == "--autostart");

            {
                let st = app.state::<AppState>();
                *st.settings.lock().unwrap() = settings;
            }

            // 幂等同步鲸仔设置分区插件到 web profile（启动服务器前还会再同步一次）。
            let _ = crate::settings_plugin::ensure_settings_plugin(&handle);

            // 测试构建：窗口标题加标记，便于区分两个共存实例。
            if crate::state::TEST_BUILD {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_title("鲸仔 Whalito（测试版）");
                }
            }

            let tray = setup_tray(&handle)?;
            {
                let st = app.state::<AppState>();
                *st.tray.lock().unwrap() = Some(tray);
            }

            // 桌宠窗口：无边框、透明、置顶、不进任务栏。
            let pet_window = WebviewWindowBuilder::new(&handle, "pet", WebviewUrl::App("pet.html".into()))
                .title("鲸仔")
                .inner_size(200.0, 250.0)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .build()?;

            // 定位：优先样式契约中的自定义位置，否则默认主屏右下角。
            if let Some(pos) = crate::pet_style::load().position {
                let _ = pet_window.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
            } else if let Ok(Some(monitor)) = pet_window.primary_monitor() {
                let size = monitor.size();
                let scale = monitor.scale_factor();
                let margin = (16.0 * scale).round() as i32;
                let w = (200.0 * scale).round() as i32;
                let h = (250.0 * scale).round() as i32;
                let x = (size.width as i32).saturating_sub(w).saturating_sub(margin);
                let y = (size.height as i32).saturating_sub(h).saturating_sub(margin);
                let _ = pet_window.set_position(tauri::PhysicalPosition::new(x, y));
            }

            let pet_enabled = {
                let st = app.state::<AppState>();
                let enabled = st.settings.lock().unwrap().pet_enabled;
                enabled
            };
            if pet_enabled {
                let _ = pet_window.show();
            }

            // 启动桌宠状态读取器（长生命周期后台线程）。
            pet::spawn(handle.clone());

            if !autostart_flag {
                show_main(&handle);
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let quitting = window
                        .app_handle()
                        .state::<AppState>()
                        .quitting
                        .load(Ordering::SeqCst);
                    if !quitting {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
