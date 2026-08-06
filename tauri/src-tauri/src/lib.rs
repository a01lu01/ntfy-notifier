mod clipboard;
mod config;
mod history;
mod notify;
mod ntfy;
mod otp;
mod startup;
mod ui_state;

use std::collections::HashMap;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

pub struct AppState {
    pub ntfy: ntfy::NtfyManager,
}

#[tauri::command]
fn get_config() -> config::Config {
    config::load_config().0
}

#[tauri::command]
fn save_config(
    config: config::Config,
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<config::Config, String> {
    config::save_config(&config)?;
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let _ = startup::set_auto_start(config.auto_start, &exe);
    state.ntfy.restart(config.clone(), app);
    Ok(config)
}

#[tauri::command]
fn get_messages() -> Vec<history::HistoryItem> {
    history::get_messages(1000)
}

#[tauri::command]
fn clear_history() -> Result<(), String> {
    history::clear_history()
}

#[tauri::command]
fn get_ui_state() -> ui_state::UiState {
    ui_state::load()
}

#[tauri::command]
fn save_ui_state(
    order: Vec<String>,
    widths: HashMap<String, i64>,
) -> Result<ui_state::UiState, String> {
    ui_state::save(order, widths)?;
    Ok(ui_state::load())
}

fn show_main(app: &AppHandle, page: &str) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
    let _ = app.emit("navigate", page);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app, "push");
        }))
        .manage(AppState {
            ntfy: ntfy::NtfyManager::new(),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_messages,
            clear_history,
            get_ui_state,
            save_ui_state
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg = config::load_config().0;
            let exe = std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let _ = startup::register_aumid(&exe);
            if cfg.auto_start {
                let _ = startup::set_auto_start(true, &exe);
            }

            let push = MenuItem::with_id(app, "push", "推送", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&push, &settings, &quit])?;

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "push" => show_main(app, "push"),
                    "settings" => show_main(app, "settings"),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle(), "push");
                    }
                })
                .build(app)?;

            let state = app.state::<AppState>();
            state.ntfy.restart(cfg, handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
