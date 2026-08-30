use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub server: String,
    pub username: String,
    pub password: String,
    pub topic: String,
    pub theme_mode: String,
    pub auto_start: bool,
    pub auto_copy_otp: bool,
    pub allow_insecure_http: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: String::new(),
            username: String::new(),
            password: String::new(),
            topic: String::new(),
            theme_mode: "system".to_string(),
            auto_start: false,
            auto_copy_otp: false,
            allow_insecure_http: false,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct DiskConfig {
    server: String,
    username: String,
    topic: String,
    theme_mode: String,
    auto_start: bool,
    auto_copy_otp: bool,
    #[serde(default)]
    allow_insecure_http: Option<bool>,
    #[serde(default)]
    password: String,
    #[serde(default)]
    password_encrypted: String,
}

pub fn config_path() -> PathBuf {
    crate::appdata::resolve().join("config.json")
}

pub fn load_config() -> (Config, bool) {
    let path = config_path();
    if !path.exists() {
        let cfg = Config::default();
        let _ = save_config(&cfg);
        return (cfg, true);
    }
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return (Config::default(), true),
    };
    let mut disk: DiskConfig = match serde_json::from_str(&raw) {
        Ok(d) => d,
        Err(_) => {
            // 损坏：备份并重置
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let backup = path.with_file_name(format!("config.json.corrupt-{stamp}"));
            let _ = fs::rename(&path, &backup);
            let cfg = Config::default();
            let _ = save_config(&cfg);
            return (cfg, true);
        }
    };

    let allow_insecure_http = disk
        .allow_insecure_http
        .unwrap_or_else(|| crate::endpoint::requires_insecure_http_opt_in(&disk.server));
    if disk.allow_insecure_http.is_none() {
        disk.allow_insecure_http = Some(allow_insecure_http);
        let _ = write_disk_config(&disk);
    }

    let mut cfg = Config {
        server: disk.server,
        username: disk.username,
        password: String::new(),
        topic: disk.topic,
        theme_mode: disk.theme_mode,
        auto_start: disk.auto_start,
        auto_copy_otp: disk.auto_copy_otp,
        allow_insecure_http,
    };

    if !disk.password_encrypted.is_empty() {
        cfg.password = decrypt_password(&disk.password_encrypted).unwrap_or_default();
    } else if !disk.password.is_empty() {
        // 旧版明文：迁移为加密存储
        cfg.password = disk.password;
        let _ = save_config(&cfg);
    }
    (cfg, false)
}

pub fn save_config(cfg: &Config) -> Result<(), String> {
    let disk = DiskConfig {
        server: cfg.server.clone(),
        username: cfg.username.clone(),
        topic: cfg.topic.clone(),
        theme_mode: cfg.theme_mode.clone(),
        auto_start: cfg.auto_start,
        auto_copy_otp: cfg.auto_copy_otp,
        allow_insecure_http: Some(cfg.allow_insecure_http),
        password: String::new(),
        password_encrypted: encrypt_password(&cfg.password),
    };
    write_disk_config(&disk)
}

fn write_disk_config(disk: &DiskConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&disk).map_err(|e| e.to_string())?;
    let path = config_path();
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn encrypt_password(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    match dpapi::protect(plain) {
        Some(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
        None => plain.to_string(),
    }
}

pub fn decrypt_password(encoded: &str) -> Option<String> {
    if encoded.is_empty() {
        return Some(String::new());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let raw = dpapi::unprotect(&bytes)?;
    let wide: Vec<u16> = raw
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&wide).ok()
}

#[cfg(windows)]
mod dpapi {
    use std::ptr;
    use winapi::um::dpapi::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN};
    use winapi::um::winbase::LocalFree;
    use winapi::um::wincrypt::DATA_BLOB;

    pub fn protect(plain: &str) -> Option<Vec<u8>> {
        let wide: Vec<u16> = plain.encode_utf16().collect();
        let mut input = DATA_BLOB {
            cbData: (wide.len() * 2) as u32,
            pbData: wide.as_ptr() as *mut u8,
        };
        let mut output = DATA_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &mut input,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return None;
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(output.pbData as *mut _);
        }
        Some(bytes)
    }

    pub fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
        let mut input = DATA_BLOB {
            cbData: blob.len() as u32,
            pbData: blob.as_ptr() as *mut u8,
        };
        let mut output = DATA_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &mut input,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return None;
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(output.pbData as *mut _);
        }
        Some(bytes)
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(plain: &str) -> Option<Vec<u8>> {
        // 与 Windows 端保持一致：按 UTF-16 LE 编码后存储，
        // 解密逻辑（decrypt_password）才能正确还原。
        let mut bytes = Vec::with_capacity(plain.len() * 2);
        for u in plain.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        Some(bytes)
    }

    pub fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
        Some(blob.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::MutexGuard;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_env() -> MutexGuard<'static, ()> {
        let guard = crate::appdata::test_lock().lock().unwrap();
        let number = COUNTER.fetch_add(1, Ordering::SeqCst);
        let directory =
            std::env::temp_dir().join(format!("ntfy-test-config-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        crate::appdata::set(directory);
        guard
    }

    fn write_legacy_config(server: &str, allow_insecure_http: Option<bool>) {
        let mut value = json!({
            "server": server,
            "username": "alice",
            "topic": "alerts",
            "theme_mode": "system",
            "auto_start": true,
            "auto_copy_otp": false,
            "password": "",
            "password_encrypted": "not-valid-base64"
        });
        if let Some(allow) = allow_insecure_http {
            value["allow_insecure_http"] = Value::Bool(allow);
        }
        std::fs::write(config_path(), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    #[test]
    fn default_config_shape() {
        let cfg = Config::default();
        assert_eq!(cfg.theme_mode, "system");
        assert_eq!(cfg.topic, "");
        assert!(!cfg.allow_insecure_http);
    }

    #[test]
    fn legacy_remote_http_is_migrated_without_rewriting_ciphertext() {
        let _guard = unique_env();
        write_legacy_config("HTTP://EXAMPLE.COM", None);

        let (config, _) = load_config();
        assert!(config.allow_insecure_http);

        let migrated: Value =
            serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        assert_eq!(migrated["allow_insecure_http"], Value::Bool(true));
        assert_eq!(
            migrated["password_encrypted"],
            Value::String("not-valid-base64".to_string())
        );
    }

    #[test]
    fn legacy_loopback_http_keeps_insecure_opt_in_disabled() {
        let _guard = unique_env();
        write_legacy_config("http://127.0.0.1:8080", None);

        let (config, _) = load_config();
        assert!(!config.allow_insecure_http);
        let migrated: Value =
            serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        assert_eq!(migrated["allow_insecure_http"], Value::Bool(false));
    }

    #[test]
    fn explicit_insecure_http_choice_is_not_overwritten() {
        let _guard = unique_env();
        write_legacy_config("http://example.com", Some(false));

        let (config, _) = load_config();
        assert!(!config.allow_insecure_http);
        let stored: Value = serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        assert_eq!(stored["allow_insecure_http"], Value::Bool(false));
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip() {
        let enc = encrypt_password("secret123");
        assert_ne!(enc, "secret123");
        assert_eq!(decrypt_password(&enc).as_deref(), Some("secret123"));
    }
}
