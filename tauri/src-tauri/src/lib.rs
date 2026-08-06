mod clipboard;
mod config;
mod history;
mod notify;
mod ntfy;
mod otp;
mod startup;
mod ui_state;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
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

#[derive(Default)]
struct TrayClickState {
    generation: AtomicU64,
    last_double_click: Mutex<Option<Instant>>,
}

impl TrayClickState {
    fn is_recent_double_click(&self) -> bool {
        self.last_double_click
            .lock()
            .map(|last| {
                last.is_some_and(|t| t.elapsed() < Duration::from_millis(500))
            })
            .unwrap_or(false)
    }

    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst).wrapping_add(1)
    }

    fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn on_double_click(&self, app: &AppHandle) {
        *self.last_double_click.lock().unwrap() = Some(Instant::now());
        self.invalidate();
        show_main(app, "push");
    }

    fn on_left_up(self: &Arc<Self>, tray: TrayIcon<tauri::Wry>) {
        if self.is_recent_double_click() {
            return;
        }
        let generation = self.next_generation();
        let state = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            if state.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            if state.is_recent_double_click() {
                return;
            }
            let _ = tray.with_inner_tray_icon(|inner| inner.show_menu());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    #[test]
    fn recent_double_click_ignores_single_click() {
        let state = TrayClickState::default();
        *state.last_double_click.lock().unwrap() = Some(Instant::now());
        assert!(state.is_recent_double_click());
    }

    #[test]
    fn stale_double_click_allows_single_click() {
        let state = TrayClickState::default();
        *state.last_double_click.lock().unwrap() =
            Some(Instant::now() - Duration::from_millis(600));
        assert!(!state.is_recent_double_click());
    }

    #[test]
    fn generation_change_invalidates_pending_menu() {
        let state = TrayClickState::default();
        let generation = state.next_generation();
        state.invalidate();
        assert_ne!(state.generation.load(Ordering::SeqCst), generation);
    }
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
            let click_state = Arc::new(TrayClickState::default());

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
                .on_tray_icon_event(move |tray, event| {
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => {
                            click_state.on_left_up(tray.clone());
                        }
                        TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        } => {
                            click_state.on_double_click(tray.app_handle());
                        }
                        TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. } => {
                            click_state.invalidate();
                        }
                        _ => {}
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
