use crate::config::Config;
#[cfg(target_os = "windows")]
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
            #[cfg(target_os = "windows")]
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
    #[cfg(target_os = "windows")]
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
        #[cfg(not(target_os = "windows"))]
        let _ = &message;

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
            let title = windows_notification_preview(&message.title, 128);
            let body = windows_notification_preview(&message.message, 512);
            notify::show(&title, &body, "ntfy-Notifier");
        }

        let _ = self.app.emit("history-updated", ());
        Ok(())
    }
}

#[cfg(any(target_os = "windows", test))]
fn windows_notification_preview(input: &str, max_scalars: usize) -> String {
    let mut preview = String::new();
    let mut chars = input.chars();
    for _ in 0..max_scalars {
        let Some(character) = chars.next() else {
            return preview;
        };
        preview.push(if is_xml_10_character(character) {
            character
        } else {
            '\u{fffd}'
        });
    }
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

#[cfg(any(target_os = "windows", test))]
fn is_xml_10_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{a}' | '\u{d}')
        || ('\u{20}'..='\u{d7ff}').contains(&character)
        || ('\u{e000}'..='\u{fffd}').contains(&character)
        || ('\u{10000}'..='\u{10ffff}').contains(&character)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_notification_preview_is_bounded_and_xml_safe() {
        let title = windows_notification_preview(&"<&\"'".repeat(100), 128);
        let body = windows_notification_preview(
            &format!("{}\u{1}{}", "&".repeat(10), "&".repeat(600)),
            512,
        );

        assert_eq!(title.chars().count(), 129);
        assert_eq!(body.chars().count(), 513);
        assert!(title.ends_with('…'));
        assert!(body.ends_with('…'));
        assert!(title.chars().all(is_xml_10_character));
        assert!(body.chars().all(is_xml_10_character));
        assert!(!body.contains('\u{1}'));
        assert!(body.contains('\u{fffd}'));

        let escaped_bytes = |text: &str| {
            text.chars()
                .map(|character| match character {
                    '&' => 5,
                    '<' | '>' => 4,
                    '\'' | '"' => 6,
                    character => character.len_utf8(),
                })
                .sum::<usize>()
        };
        assert!(escaped_bytes(&title) + escaped_bytes(&body) < 4 * 1024);
    }

    #[test]
    fn windows_notification_preview_keeps_short_text_unchanged() {
        assert_eq!(windows_notification_preview("正常通知", 128), "正常通知");
    }
}
