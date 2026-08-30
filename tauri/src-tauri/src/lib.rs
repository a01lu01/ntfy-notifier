#[cfg(any(target_os = "android", test))]
mod android_subscriber;
mod appdata;
mod config;
mod endpoint;
mod history;
#[cfg(desktop)]
mod ntfy;
mod otp;
mod rules;
mod sse;
mod subscription;
mod ui_state;

#[cfg(any(mobile, test))]
mod notify_mobile;

#[cfg(target_os = "windows")]
mod clipboard;
#[cfg(target_os = "windows")]
mod notify;
#[cfg(target_os = "windows")]
mod startup;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(desktop)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(desktop)]
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(desktop)]
use std::time::{Duration, Instant};
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem};
#[cfg(desktop)]
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
#[cfg(desktop)]
use tauri::Emitter;
use tauri::{AppHandle, Manager};

const MAX_SAVE_CONFIG_JSON_BYTES: usize = 16 * 1024;
const MAX_SERVER_BYTES: usize = 4 * 1024;
const MAX_USERNAME_BYTES: usize = 1024;
const MAX_PASSWORD_BYTES: usize = 8 * 1024;
const MAX_TOPIC_BYTES: usize = 64;
const MAX_THEME_MODE_BYTES: usize = 16;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SaveConfigInput {
    server: String,
    username: String,
    password: String,
    topic: String,
    theme_mode: String,
    auto_start: bool,
    auto_copy_otp: bool,
    allow_insecure_http: bool,
}

impl std::fmt::Debug for SaveConfigInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SaveConfigInput(<redacted>)")
    }
}

impl SaveConfigInput {
    fn into_config(self) -> Result<config::Config, String> {
        self.validate_size_limits()?;
        Ok(config::Config {
            server: self.server,
            username: self.username,
            password: self.password,
            topic: self.topic,
            theme_mode: self.theme_mode,
            auto_start: self.auto_start,
            auto_copy_otp: self.auto_copy_otp,
            allow_insecure_http: self.allow_insecure_http,
        })
    }

    fn validate_size_limits(&self) -> Result<(), String> {
        validate_field_size("server", &self.server, MAX_SERVER_BYTES)?;
        validate_field_size("username", &self.username, MAX_USERNAME_BYTES)?;
        validate_field_size("password", &self.password, MAX_PASSWORD_BYTES)?;
        validate_field_size("topic", &self.topic, MAX_TOPIC_BYTES)?;
        validate_field_size("theme_mode", &self.theme_mode, MAX_THEME_MODE_BYTES)?;

        let serialized_size = serde_json::to_vec(self)
            .map_err(|_| "配置请求无法安全校验".to_string())?
            .len();
        if serialized_size > MAX_SAVE_CONFIG_JSON_BYTES {
            return Err(format!(
                "配置请求超过大小限制（最多 {MAX_SAVE_CONFIG_JSON_BYTES} 字节）"
            ));
        }
        Ok(())
    }
}

fn validate_field_size(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.len() > max_bytes {
        Err(format!(
            "配置字段 {label} 超过大小限制（最多 {max_bytes} 字节）"
        ))
    } else {
        Ok(())
    }
}

pub struct AppState {
    #[cfg(desktop)]
    pub ntfy: ntfy::NtfyManager,
    config: Mutex<Option<Result<config::Config, String>>>,
}

#[tauri::command]
fn get_config(state: tauri::State<'_, AppState>) -> Result<config::Config, String> {
    state
        .config
        .lock()
        .map_err(|_| "配置状态锁已损坏".to_string())?
        .clone()
        .unwrap_or_else(|| Err("配置尚未初始化".to_string()))
}

#[tauri::command]
fn save_config(
    config: SaveConfigInput,
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<config::Config, String> {
    // IPC 输入不复用带 `serde(default)` 的读盘模型：所有字段必须显式传入，
    // 并在触达 Android Keystore、DPAPI 或任何配置文件之前完成大小校验。
    let config = config.into_config()?;
    endpoint::validate_subscription_endpoint(
        &config.server,
        &config.topic,
        &config.username,
        &config.password,
        config.allow_insecure_http,
    )?;
    let state = state.inner();
    // Hold the same operation lock through persistence, platform side effects, and restart so
    // concurrent saves cannot leave the subscriber running an older configuration than disk.
    let mut cached = state
        .config
        .lock()
        .map_err(|_| "配置状态锁已损坏".to_string())?;
    #[cfg(mobile)]
    let stored_config = notify_mobile::save_config(&app, &config)?;
    #[cfg(not(mobile))]
    let stored_config = {
        config::save_config(&config)?;
        config.clone()
    };
    *cached = Some(Ok(stored_config.clone()));
    #[cfg(target_os = "windows")]
    {
        match std::env::current_exe() {
            Ok(exe) => {
                if let Err(error) = startup::set_auto_start(stored_config.auto_start, &exe) {
                    eprintln!("Windows auto-start update failed after saving config: {error}");
                }
            }
            Err(error) => {
                eprintln!(
                    "cannot resolve the application executable for the auto-start update: {error}"
                );
            }
        }
    }
    #[cfg(mobile)]
    notify_mobile::set_auto_start(&app, stored_config.auto_start)?;
    #[cfg(target_os = "android")]
    notify_mobile::reconfigure_service(&app)?;
    #[cfg(desktop)]
    state.ntfy.restart(stored_config.clone(), app);
    drop(cached);
    Ok(stored_config)
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
fn get_history_revision() -> Result<i64, String> {
    history::get_history_revision()
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

#[tauri::command]
fn get_rules() -> Vec<rules::Rule> {
    rules::load()
}

#[tauri::command]
fn save_rules(rules: Vec<crate::rules::Rule>) -> Result<Vec<crate::rules::Rule>, String> {
    crate::rules::save(&rules)?;
    Ok(crate::rules::load())
}

#[tauri::command]
fn is_mobile() -> bool {
    cfg!(mobile)
}

#[tauri::command]
fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[cfg(desktop)]
fn show_main(app: &AppHandle, page: &str) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
    let _ = app.emit("navigate", page);
}

#[cfg(desktop)]
#[derive(Default)]
struct TrayClickState {
    generation: AtomicU64,
    last_double_click: Mutex<Option<Instant>>,
}

#[cfg(desktop)]
impl TrayClickState {
    fn is_recent_double_click(&self) -> bool {
        self.last_double_click
            .lock()
            .map(|last| last.is_some_and(|t| t.elapsed() < Duration::from_millis(500)))
            .unwrap_or(false)
    }

    fn next_generation(&self) -> u64 {
        self.generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app, "push");
        }));
    }

    #[cfg(mobile)]
    {
        builder = builder.plugin(notify_mobile::init());
    }

    builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(desktop)]
    {
        builder = builder.on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        });
    }

    builder
        .manage(AppState {
            #[cfg(desktop)]
            ntfy: ntfy::NtfyManager::new(),
            config: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_messages,
            clear_history,
            get_history_revision,
            get_ui_state,
            save_ui_state,
            get_rules,
            save_rules,
            is_mobile,
            app_version
        ])
        .setup(|app| {
            #[cfg(mobile)]
            {
                if let Ok(dir) = app.path().app_data_dir() {
                    appdata::set(dir);
                }
            }

            let handle = app.handle().clone();
            #[cfg(mobile)]
            let loaded_config = notify_mobile::get_config(&handle);
            #[cfg(not(mobile))]
            let loaded_config = config::load_config();
            {
                let state = app.state::<AppState>();
                let mut cached = state
                    .config
                    .lock()
                    .map_err(|_| std::io::Error::other("configuration state lock poisoned"))?;
                *cached = Some(loaded_config.clone());
            }
            let cfg = match loaded_config {
                Ok(cfg) => Some(cfg),
                Err(error) => {
                    eprintln!("configuration load failed; subscriber remains stopped: {error}");
                    None
                }
            };

            #[cfg(target_os = "windows")]
            {
                match std::env::current_exe() {
                    Ok(exe) => {
                        if let Err(error) = startup::register_aumid(&exe) {
                            eprintln!("Windows AppUserModelID registration failed: {error}");
                        }
                        if cfg.as_ref().is_some_and(|cfg| cfg.auto_start) {
                            if let Err(error) = startup::set_auto_start(true, &exe) {
                                eprintln!("Windows auto-start repair failed: {error}");
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("cannot resolve the application executable: {error}");
                    }
                }
            }

            #[cfg(desktop)]
            {
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
                    .on_tray_icon_event(move |tray, event| match event {
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
                    })
                    .build(app)?;
            }

            #[cfg(mobile)]
            {
                if let Some(cfg) = cfg.as_ref() {
                    if let Err(error) = notify_mobile::set_auto_start(&handle, cfg.auto_start) {
                        eprintln!("Android auto-start compatibility update failed: {error}");
                    }
                    if !cfg.server.trim().is_empty() && !cfg.topic.trim().is_empty() {
                        if let Err(error) = notify_mobile::start_service(&handle) {
                            eprintln!("Android subscriber service start failed: {error}");
                        }
                    }
                }
            }

            #[cfg(desktop)]
            if let Some(cfg) = cfg {
                let state = app.state::<AppState>();
                state.ntfy.restart(cfg, handle);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(all(desktop, test))]
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

#[cfg(test)]
mod save_config_input_tests {
    use super::*;
    use serde_json::{json, Value};

    fn sample_input() -> SaveConfigInput {
        SaveConfigInput {
            server: "https://ntfy.example.com".to_string(),
            username: "alice".to_string(),
            password: "secret".to_string(),
            topic: "alerts".to_string(),
            theme_mode: "system".to_string(),
            auto_start: true,
            auto_copy_otp: false,
            allow_insecure_http: false,
        }
    }

    fn sample_json() -> Value {
        serde_json::to_value(sample_input()).unwrap()
    }

    #[test]
    fn save_input_requires_every_field() {
        let mut value = sample_json();
        value.as_object_mut().unwrap().remove("password");

        let error = serde_json::from_value::<SaveConfigInput>(value).unwrap_err();

        assert!(error.to_string().contains("missing field `password`"));
    }

    #[test]
    fn save_input_debug_output_is_redacted() {
        let debug = format!("{:?}", sample_input());

        assert_eq!(debug, "SaveConfigInput(<redacted>)");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("ntfy.example.com"));
    }

    #[test]
    fn save_input_rejects_unknown_fields() {
        let mut value = sample_json();
        value["unexpected"] = json!(true);

        let error = serde_json::from_value::<SaveConfigInput>(value).unwrap_err();

        assert!(error.to_string().contains("unknown field `unexpected`"));
    }

    #[test]
    fn save_input_keeps_the_frontend_snake_case_contract() {
        let value = sample_json();
        let mut fields = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        fields.sort();

        assert_eq!(
            fields,
            [
                "allow_insecure_http",
                "auto_copy_otp",
                "auto_start",
                "password",
                "server",
                "theme_mode",
                "topic",
                "username",
            ]
        );
        let config = serde_json::from_value::<SaveConfigInput>(value)
            .unwrap()
            .into_config()
            .unwrap();
        assert_eq!(config.server, "https://ntfy.example.com");
        assert_eq!(config.theme_mode, "system");
    }

    #[test]
    fn save_input_accepts_exact_per_field_size_boundaries() {
        let mut input = sample_input();
        input.server = "s".repeat(MAX_SERVER_BYTES);
        input.username = "u".repeat(MAX_USERNAME_BYTES);
        input.password = "p".repeat(MAX_PASSWORD_BYTES);
        input.topic = "t".repeat(MAX_TOPIC_BYTES);
        input.theme_mode = "m".repeat(MAX_THEME_MODE_BYTES);

        assert!(input.into_config().is_ok());
    }

    #[test]
    fn save_input_rejects_each_oversized_field_without_echoing_values() {
        let cases = [
            ("server", MAX_SERVER_BYTES),
            ("username", MAX_USERNAME_BYTES),
            ("password", MAX_PASSWORD_BYTES),
            ("topic", MAX_TOPIC_BYTES),
            ("theme_mode", MAX_THEME_MODE_BYTES),
        ];

        for (field, limit) in cases {
            let secret_marker = format!("do-not-echo-{field}");
            let oversized = format!("{}{}", "x".repeat(limit + 1), secret_marker);
            let mut value = sample_json();
            value[field] = Value::String(oversized);
            let error = serde_json::from_value::<SaveConfigInput>(value)
                .unwrap()
                .into_config()
                .unwrap_err();

            assert!(error.contains(field), "{field}: {error}");
            assert!(!error.contains(&secret_marker), "{field}: {error}");
        }
    }

    #[test]
    fn save_input_rejects_oversized_serialized_payload_before_storage() {
        let mut input = sample_input();
        // JSON escapes each control character as six ASCII bytes. This remains below the
        // password field limit while exercising the bound on the actual serialized request.
        input.password = "\u{1}".repeat(3_000);
        assert!(input.password.len() < MAX_PASSWORD_BYTES);
        assert!(serde_json::to_vec(&input).unwrap().len() > MAX_SAVE_CONFIG_JSON_BYTES);

        let error = input.into_config().unwrap_err();

        assert!(error.contains("配置请求超过大小限制"));
        assert!(!error.contains(&"\u{1}".repeat(16)));
    }
}
