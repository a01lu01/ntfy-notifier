use crate::{config::Config, history, rules};
#[cfg(target_os = "windows")]
use crate::{clipboard, notify};
#[cfg(mobile)]
use crate::notify_mobile;
use futures_util::StreamExt;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub struct NtfyManager {
    handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    connected: Arc<AtomicBool>,
}

impl NtfyManager {
    pub fn new() -> Self {
        Self {
            handle: Mutex::new(None),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn restart(&self, cfg: Config, app: AppHandle) {
        self.stop();
        let connected = self.connected.clone();
        let handle = tauri::async_runtime::spawn(async move {
            run_loop(cfg, app, connected).await;
        });
        *self.handle.lock().unwrap() = Some(handle);
    }

    pub fn stop(&self) {
        if let Some(handle) = self.handle.lock().unwrap().take() {
            handle.abort();
        }
        self.connected.store(false, Ordering::SeqCst);
    }
}

#[cfg(desktop)]
fn update_tray(app: &AppHandle, connected: bool) {
    if let Some(tray) = app.tray_by_id("main") {
        let bytes: &[u8] = if connected {
            include_bytes!("../icons/connected.ico")
        } else {
            include_bytes!("../icons/disconnected.ico")
        };
        if let Ok(img) = tauri::image::Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(img));
        }
    }
}

async fn run_loop(cfg: Config, app: AppHandle, connected: Arc<AtomicBool>) {
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ntfy] 客户端创建失败: {e}");
            return;
        }
    };

    let mut delay = Duration::from_secs(5);
    loop {
        if cfg.server.is_empty() || cfg.topic.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let url = format!(
            "{}/{}/sse",
            cfg.server.trim_end_matches('/'),
            cfg.topic
        );
        let mut req = client.get(&url).header("Accept", "text/event-stream");
        if !cfg.username.is_empty() {
            req = req.basic_auth(&cfg.username, Some(&cfg.password));
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                delay = Duration::from_secs(5);
                connected.store(true, Ordering::SeqCst);
                #[cfg(desktop)]
                update_tray(&app, true);
                let _ = app.emit("connection", true);

                let mut stream = resp.bytes_stream();
                let mut buf: Vec<u8> = Vec::new();
                loop {
                    let chunk = match tokio::time::timeout(
                        Duration::from_secs(120),
                        stream.next(),
                    )
                    .await
                    {
                        Ok(Some(Ok(c))) => c,
                        _ => break,
                    };
                    buf.extend_from_slice(&chunk);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=pos).collect();
                        handle_line(&line, &app, &cfg).await;
                    }
                }
            }
            Ok(resp) => {
                eprintln!("[ntfy] SSE 连接失败：HTTP {}", resp.status());
            }
            Err(e) => {
                eprintln!("[ntfy] SSE 连接错误：{e}");
            }
        }

        connected.store(false, Ordering::SeqCst);
        #[cfg(desktop)]
        update_tray(&app, false);
        let _ = app.emit("connection", false);

        let mut waited = Duration::ZERO;
        while waited < delay {
            tokio::time::sleep(Duration::from_secs(1)).await;
            waited += Duration::from_secs(1);
        }
        delay = (delay * 2).min(Duration::from_secs(300));
    }
}

async fn handle_line(line: &[u8], app: &AppHandle, cfg: &Config) {
    let text = String::from_utf8_lossy(line).trim().to_string();
    if !text.starts_with("data: ") {
        return;
    }
    let data = &text[6..];
    let msg: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return,
    };
    let event = msg.get("event").and_then(|v| v.as_str()).unwrap_or("");
    if event != "message" {
        return;
    }

    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let topic = msg.get("topic").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let title = msg
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("ntfy 消息")
        .to_string();
    let message = msg
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or(data)
        .to_string();

    match history::record_message(&id, &topic, &title, &message) {
        Ok(true) => {
            #[cfg(target_os = "windows")]
            {
                if cfg.auto_copy_otp {
                    let rule_list = rules::load();
                    if let Some(otp) = rules::find_otp(&message, &rule_list) {
                        let app2 = app.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(e) = clipboard::copy_text(&otp) {
                                eprintln!("[ntfy] 验证码复制失败：{e}");
                            }
                            let _ = app2.emit("history-updated", ());
                        });
                    }
                }
                notify::show(&title, &message, "ntfy-Notifier");
            }

            #[cfg(mobile)]
            {
                let otp = rules::find_otp(&message, &rules::load());
                notify_mobile::update_notifications(app, &title, &message, otp.as_deref());
                if cfg.auto_copy_otp {
                    if let Some(otp) = otp {
                        let app2 = app.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            notify_mobile::copy_to_clipboard(&app2, &otp);
                        });
                    }
                }
            }

            let _ = app.emit("history-updated", ());
        }
        _ => {}
    }
}
