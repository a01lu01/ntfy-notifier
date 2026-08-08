use serde::Serialize;
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};

pub struct NotifyMobile<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> NotifyMobile<R> {
    pub fn start_service(&self) {
        let _: Result<(), _> = self.0.run_mobile_plugin("startService", ());
    }

    pub fn set_auto_start(&self, enabled: bool) {
        #[derive(Serialize)]
        struct Payload {
            enabled: bool,
        }
        let _: Result<(), _> = self.0.run_mobile_plugin("setAutoStart", Payload { enabled });
    }

    pub fn update_notifications(&self, title: &str, message: &str, otp: Option<&str>) {
        #[derive(Serialize)]
        struct Payload<'a> {
            title: &'a str,
            message: &'a str,
            otp: Option<&'a str>,
        }
        let _: Result<(), _> = self.0.run_mobile_plugin(
            "updateNotifications",
            Payload { title, message, otp },
        );
    }

    pub fn copy_to_clipboard(&self, text: &str) {
        #[derive(Serialize)]
        struct Payload<'a> {
            text: &'a str,
        }
        let _: Result<(), _> = self.0.run_mobile_plugin("copyToClipboard", Payload { text });
    }
}

pub fn init() -> TauriPlugin<tauri::Wry> {
    Builder::new("ntfy-notify")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.ntfy.notifier", "NtfyNotifierPlugin")?;
            app.manage(NotifyMobile(handle));
            Ok(())
        })
        .build()
}

fn state<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::State<'_, NotifyMobile<R>>> {
    app.try_state::<NotifyMobile<R>>()
}

pub fn start_service(app: &AppHandle) {
    if let Some(s) = state(app) {
        s.start_service();
    }
}

pub fn set_auto_start(app: &AppHandle, enabled: bool) {
    if let Some(s) = state(app) {
        s.set_auto_start(enabled);
    }
}

pub fn update_notifications(app: &AppHandle, title: &str, message: &str, otp: Option<&str>) {
    if let Some(s) = state(app) {
        s.update_notifications(title, message, otp);
    }
}

pub fn copy_to_clipboard(app: &AppHandle, text: &str) {
    if let Some(s) = state(app) {
        s.copy_to_clipboard(text);
    }
}
