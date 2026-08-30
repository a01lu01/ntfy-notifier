use serde::{Deserialize, Serialize};
#[cfg(mobile)]
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MobileConfigDto {
    server: String,
    username: String,
    password: String,
    topic: String,
    theme_mode: String,
    auto_start: bool,
    auto_copy_otp: bool,
    allow_insecure_http: bool,
}

impl From<&crate::config::Config> for MobileConfigDto {
    fn from(config: &crate::config::Config) -> Self {
        Self {
            server: config.server.clone(),
            username: config.username.clone(),
            password: config.password.clone(),
            topic: config.topic.clone(),
            theme_mode: config.theme_mode.clone(),
            auto_start: config.auto_start,
            auto_copy_otp: config.auto_copy_otp,
            allow_insecure_http: config.allow_insecure_http,
        }
    }
}

impl From<MobileConfigDto> for crate::config::Config {
    fn from(config: MobileConfigDto) -> Self {
        Self {
            server: config.server,
            username: config.username,
            password: config.password,
            topic: config.topic,
            theme_mode: config.theme_mode,
            auto_start: config.auto_start,
            auto_copy_otp: config.auto_copy_otp,
            allow_insecure_http: config.allow_insecure_http,
        }
    }
}

#[cfg(mobile)]
pub struct NotifyMobile<R: Runtime>(PluginHandle<R>);

#[cfg(mobile)]
impl<R: Runtime> NotifyMobile<R> {
    pub fn get_config(&self) -> Result<crate::config::Config, String> {
        let response: MobileConfigDto = self
            .0
            .run_mobile_plugin("getConfig", ())
            .map_err(|error| format!("读取 Android 安全配置失败：{error}"))?;
        Ok(response.into())
    }

    pub fn save_config(
        &self,
        config: &crate::config::Config,
    ) -> Result<crate::config::Config, String> {
        #[derive(Serialize)]
        struct Payload {
            config: MobileConfigDto,
        }

        let response: MobileConfigDto = self
            .0
            .run_mobile_plugin(
                "saveConfig",
                Payload {
                    config: MobileConfigDto::from(config),
                },
            )
            .map_err(|error| format!("保存 Android 安全配置失败：{error}"))?;
        Ok(response.into())
    }

    pub fn start_service(&self) {
        let _: Result<(), _> = self.0.run_mobile_plugin("startService", ());
    }

    pub fn set_auto_start(&self, enabled: bool) {
        #[derive(Serialize)]
        struct Payload {
            enabled: bool,
        }
        let _: Result<(), _> = self
            .0
            .run_mobile_plugin("setAutoStart", Payload { enabled });
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
            Payload {
                title,
                message,
                otp,
            },
        );
    }

    pub fn copy_to_clipboard(&self, text: &str) {
        #[derive(Serialize)]
        struct Payload<'a> {
            text: &'a str,
        }
        let _: Result<(), _> = self
            .0
            .run_mobile_plugin("copyToClipboard", Payload { text });
    }
}

#[cfg(mobile)]
pub fn init() -> TauriPlugin<tauri::Wry> {
    Builder::new("ntfy-notify")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.ntfy.notifier", "NtfyNotifierPlugin")?;
            app.manage(NotifyMobile(handle));
            Ok(())
        })
        .build()
}

#[cfg(mobile)]
fn state<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::State<'_, NotifyMobile<R>>> {
    app.try_state::<NotifyMobile<R>>()
}

#[cfg(mobile)]
pub fn start_service(app: &AppHandle) {
    if let Some(s) = state(app) {
        s.start_service();
    }
}

#[cfg(mobile)]
pub fn get_config(app: &AppHandle) -> Result<crate::config::Config, String> {
    state(app)
        .ok_or_else(|| "Android 通知插件尚未初始化".to_string())?
        .get_config()
}

#[cfg(mobile)]
pub fn save_config(
    app: &AppHandle,
    config: &crate::config::Config,
) -> Result<crate::config::Config, String> {
    state(app)
        .ok_or_else(|| "Android 通知插件尚未初始化".to_string())?
        .save_config(config)
}

#[cfg(mobile)]
pub fn set_auto_start(app: &AppHandle, enabled: bool) {
    if let Some(s) = state(app) {
        s.set_auto_start(enabled);
    }
}

#[cfg(mobile)]
pub fn update_notifications(app: &AppHandle, title: &str, message: &str, otp: Option<&str>) {
    if let Some(s) = state(app) {
        s.update_notifications(title, message, otp);
    }
}

#[cfg(mobile)]
pub fn copy_to_clipboard(app: &AppHandle, text: &str) {
    if let Some(s) = state(app) {
        s.copy_to_clipboard(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn response_json() -> Value {
        json!({
            "server": "https://ntfy.example.com",
            "username": "alice",
            "password": "secret",
            "topic": "alerts",
            "theme_mode": "dark",
            "auto_start": true,
            "auto_copy_otp": false,
            "allow_insecure_http": false
        })
    }

    #[test]
    fn mobile_config_response_requires_every_field() {
        let mut value = response_json();
        value.as_object_mut().unwrap().remove("password");

        let error = serde_json::from_value::<MobileConfigDto>(value).unwrap_err();

        assert!(error.to_string().contains("missing field `password`"));
    }

    #[test]
    fn mobile_config_response_rejects_unknown_fields() {
        let mut value = response_json();
        value["unexpected"] = json!(true);

        let error = serde_json::from_value::<MobileConfigDto>(value).unwrap_err();

        assert!(error.to_string().contains("unknown field `unexpected`"));
    }

    #[test]
    fn mobile_config_request_and_response_use_exact_snake_case_fields() {
        let dto = serde_json::from_value::<MobileConfigDto>(response_json()).unwrap();
        let config: crate::config::Config = dto.into();
        let request = serde_json::to_value(MobileConfigDto::from(&config)).unwrap();
        let mut fields = request
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
        assert_eq!(request["password"], "secret");
        assert_eq!(request.as_object().unwrap().len(), 8);
    }
}
