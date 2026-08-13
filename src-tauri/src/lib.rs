mod commands;
mod state;

use std::sync::atomic::Ordering;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};

use state::AppState;

fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<tauri::tray::TrayIcon> {
    let show = MenuItem::with_id(app, "show", "打开面板", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "启动服务器", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止服务器", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "在浏览器打开", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &start,
            &stop,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
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
            "quit" => {
                app.state::<AppState>().quitting.store(true, Ordering::SeqCst);
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
        .invoke_handler(tauri::generate_handler![
            commands::detect_env,
            commands::install_node,
            commands::install_dsh,
            commands::update_dsh,
            commands::verify_dsh,
            commands::start_server,
            commands::stop_server,
            commands::server_status,
            commands::get_settings,
            commands::save_settings,
            commands::set_autostart,
            commands::open_url,
            commands::get_logs,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = state::load_settings(&handle);
            let autostart_flag = std::env::args().any(|a| a == "--autostart");
            let auto_start_server = settings.auto_start_server;

            {
                let st = app.state::<AppState>();
                *st.settings.lock().unwrap() = settings;
            }

            let tray = setup_tray(&handle)?;
            {
                let st = app.state::<AppState>();
                *st.tray.lock().unwrap() = Some(tray);
            }

            if !autostart_flag {
                show_main(&handle);
            }

            if auto_start_server {
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    let st = h.state::<AppState>();
                    let _ = commands::start_server_impl(&h, &st);
                });
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
