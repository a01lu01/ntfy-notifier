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
    password: String,
    #[serde(default)]
    password_encrypted: String,
}

fn data_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("ntfy-notifier")
    } else if let Some(home) = dirs::home_dir() {
        home.join("AppData").join("Roaming").join("ntfy-notifier")
    } else {
        PathBuf::from("ntfy-notifier")
    }
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
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
    let disk: DiskConfig = match serde_json::from_str(&raw) {
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

    let mut cfg = Config {
        server: disk.server,
        username: disk.username,
        password: String::new(),
        topic: disk.topic,
        theme_mode: disk.theme_mode,
        auto_start: disk.auto_start,
        auto_copy_otp: disk.auto_copy_otp,
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
        password: String::new(),
        password_encrypted: encrypt_password(&cfg.password),
    };
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
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
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
    use winapi::um::dpapi::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN,
    };
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
        Some(plain.as_bytes().to_vec())
    }

    pub fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
        Some(blob.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_shape() {
        let cfg = Config::default();
        assert_eq!(cfg.theme_mode, "system");
        assert_eq!(cfg.topic, "");
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip() {
        let enc = encrypt_password("secret123");
        assert_ne!(enc, "secret123");
        assert_eq!(decrypt_password(&enc).as_deref(), Some("secret123"));
    }
}
