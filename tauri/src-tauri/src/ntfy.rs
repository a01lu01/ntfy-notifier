use crate::config::Config;
use crate::history;
#[cfg(mobile)]
use crate::notify_mobile;
#[cfg(any(target_os = "windows", mobile))]
use crate::rules;
use crate::subscription::{
    SubscriptionConfig, SubscriptionController, SubscriptionCore, SubscriptionMessage,
    SubscriptionSink, SubscriptionState,
};
#[cfg(target_os = "windows")]
use crate::{clipboard, notify};
use tauri::{AppHandle, Emitter};

pub struct NtfyManager {
    controller: SubscriptionController,
    core: SubscriptionCore,
}

impl NtfyManager {
    pub fn new() -> Self {
        Self {
            controller: SubscriptionController::default(),
            core: SubscriptionCore::default(),
        }
    }

    pub fn restart(&self, config: Config, app: AppHandle) {
        let sink = TauriSink {
            app,
            #[cfg(any(target_os = "windows", mobile))]
            config: config.clone(),
        };
        self.controller.reconfigure(
            self.core.clone(),
            SubscriptionConfig::from(&config),
            sink,
            |task| {
                tauri::async_runtime::spawn(task);
            },
        );
    }

    pub fn stop(&self) {
        self.controller.stop();
    }
}

impl Drop for NtfyManager {
    fn drop(&mut self) {
        self.stop();
    }
}

struct TauriSink {
    app: AppHandle,
    #[cfg(any(target_os = "windows", mobile))]
    config: Config,
}

impl SubscriptionSink for TauriSink {
    fn state_changed(&self, state: SubscriptionState) {
        let connected = state == SubscriptionState::Connected;
        #[cfg(desktop)]
        update_tray(&self.app, connected);
        let _ = self.app.emit("connection", connected);
        let _ = self.app.emit("connection-state", state);
    }

    fn message_received(&self, message: SubscriptionMessage) -> Result<(), String> {
        let inserted = match history::record_message(
            &message.id,
            &message.topic,
            &message.title,
            &message.message,
        ) {
            Ok(inserted) => inserted,
            Err(error) => {
                eprintln!("[ntfy] 消息入库失败：{error}");
                return Ok(());
            }
        };
        if !inserted {
            return Ok(());
        }

        #[cfg(target_os = "windows")]
        {
            if self.config.auto_copy_otp {
                let rule_list = rules::load();
                if let Some(otp) = rules::find_otp(&message.message, &rule_list) {
                    let app = self.app.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(error) = clipboard::copy_text(&otp) {
                            eprintln!("[ntfy] 验证码复制失败：{error}");
                        }
                        let _ = app.emit("history-updated", ());
                    });
                }
            }
            notify::show(&message.title, &message.message, "ntfy-Notifier");
        }

        #[cfg(mobile)]
        {
            let otp = rules::find_otp(&message.message, &rules::load());
            notify_mobile::update_notifications(
                &self.app,
                &message.title,
                &message.message,
                otp.as_deref(),
            );
            if self.config.auto_copy_otp {
                if let Some(otp) = otp {
                    let app = self.app.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        notify_mobile::copy_to_clipboard(&app, &otp);
                    });
                }
            }
        }

        let _ = self.app.emit("history-updated", ());
        Ok(())
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
        if let Ok(image) = tauri::image::Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(image));
        }
    }
}
