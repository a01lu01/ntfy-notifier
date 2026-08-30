use base64::Engine;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const CONFIG_VERSION: u64 = 2;
const CREDENTIAL_VERSION: u64 = 1;
const DEFAULT_THEME: &str = "system";
const TEMP_CREATE_ATTEMPTS: usize = 128;
const MAX_CONFIG_JSON_BYTES: usize = 1024 * 1024;
const MAX_PREFERENCES_JSON_BYTES: usize = 16 * 1024;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
const CREDENTIAL_PROVIDER: &str = "windows-dpapi";
#[cfg(not(windows))]
const CREDENTIAL_PROVIDER: &str = "legacy-utf16le-base64";

#[derive(Serialize, Deserialize, Clone)]
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

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Config(<redacted>)")
    }
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDiskConfig {
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

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct CredentialEnvelope {
    provider: String,
    version: u64,
    ciphertext: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct DiskConfigV2 {
    version: u64,
    server: String,
    username: String,
    topic: String,
    #[serde(default)]
    allow_insecure_http: bool,
    auto_start: bool,
    auto_copy_otp: bool,
    credential: CredentialEnvelope,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct Preferences {
    theme_mode: String,
}

pub fn config_path() -> PathBuf {
    crate::appdata::resolve().join("config.json")
}

pub fn preferences_path() -> PathBuf {
    crate::appdata::resolve().join("preferences.json")
}

fn read_optional_bounded(
    path: &Path,
    max_bytes: usize,
    label: &str,
) -> Result<Option<Vec<u8>>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取{label}失败：{error}")),
    };
    let limit = u64::try_from(max_bytes)
        .map_err(|_| format!("{label}大小限制无效"))?
        .saturating_add(1);
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取{label}失败：{error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!("{label}超过大小限制（最多 {max_bytes} 字节）"));
    }
    Ok(Some(bytes))
}

pub fn load_config() -> Result<Config, String> {
    with_config_lock(load_config_unlocked)
}

fn load_config_unlocked() -> Result<Config, String> {
    let path = config_path();
    let raw = match read_optional_bounded(&path, MAX_CONFIG_JSON_BYTES, "配置")? {
        Some(raw) => raw,
        None => {
            return Ok(Config {
                theme_mode: load_preferences_or_default().theme_mode,
                ..Config::default()
            });
        }
    };
    let root: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|error| format!("配置文件格式无效：{error}"))?;

    match root.get("version") {
        None => migrate_legacy_config(&raw),
        Some(version) if version.as_u64() == Some(CONFIG_VERSION) => load_v2_config(&raw),
        Some(version) => Err(format!("不支持的配置版本：{version}")),
    }
}

pub fn save_config(cfg: &Config) -> Result<(), String> {
    with_config_lock(|| save_config_unlocked(cfg))
}

fn save_config_unlocked(cfg: &Config) -> Result<(), String> {
    validate_theme(&cfg.theme_mode)?;
    let preferences = Preferences {
        theme_mode: cfg.theme_mode.clone(),
    };
    let disk = DiskConfigV2 {
        version: CONFIG_VERSION,
        server: cfg.server.clone(),
        username: cfg.username.clone(),
        topic: cfg.topic.clone(),
        allow_insecure_http: cfg.allow_insecure_http,
        auto_start: cfg.auto_start,
        auto_copy_otp: cfg.auto_copy_otp,
        credential: credential_from_plaintext(&cfg.password)?,
    };
    let preference_bytes = serialize_json(&preferences, "界面偏好", MAX_PREFERENCES_JSON_BYTES)?;
    let config_bytes = serialize_json(&disk, "配置", MAX_CONFIG_JSON_BYTES)?;

    // 两个文件无法作为单个文件系统事务提交。安全偏好先写，敏感配置最后写；
    // 因此配置写失败时，旧 config.json 仍是下一次迁移的权威来源。
    write_config_and_preferences(&config_bytes, &preference_bytes)
}

#[cfg(not(target_os = "android"))]
fn with_config_lock<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let data_dir = crate::appdata::resolve();
    fs::create_dir_all(&data_dir).map_err(|error| format!("创建配置数据目录失败：{error}"))?;
    let lock_path = data_dir.join(".config.lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("打开配置锁失败：{error}"))?;
    lock_file
        .lock()
        .map_err(|error| format!("获取配置锁失败：{error}"))?;

    let result = operation();
    // File 的 Drop 在正常返回、错误和 panic 展开时都会释放操作系统锁；不在写盘
    // 成功后再把显式 unlock 的次生错误报告成“保存失败”，避免调用方与磁盘状态分裂。
    drop(lock_file);
    result
}

#[cfg(target_os = "android")]
fn with_config_lock<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    operation()
}

fn load_v2_config(raw: &[u8]) -> Result<Config, String> {
    let disk: DiskConfigV2 =
        serde_json::from_slice(raw).map_err(|error| format!("配置文件格式无效：{error}"))?;
    if disk.version != CONFIG_VERSION {
        return Err(format!("不支持的配置版本：{}", disk.version));
    }
    let preferences = load_preferences_or_default();
    let password = plaintext_from_credential(&disk.credential)?;
    Ok(Config {
        server: disk.server,
        username: disk.username,
        password,
        topic: disk.topic,
        theme_mode: preferences.theme_mode,
        auto_start: disk.auto_start,
        auto_copy_otp: disk.auto_copy_otp,
        allow_insecure_http: disk.allow_insecure_http,
    })
}

fn migrate_legacy_config(raw: &[u8]) -> Result<Config, String> {
    let legacy: LegacyDiskConfig = serde_json::from_slice(raw)
        .map_err(|error| format!("旧配置文件格式无效，无法迁移：{error}"))?;
    validate_theme(&legacy.theme_mode)?;

    let (password, credential) = if !legacy.password_encrypted.is_empty() {
        let password = decrypt_password(&legacy.password_encrypted).map_err(|error| {
            format!("旧凭据无法安全解密，原配置文件已保留；请重新输入密码并确认覆盖：{error}")
        })?;
        let credential = CredentialEnvelope {
            provider: CREDENTIAL_PROVIDER.to_string(),
            version: CREDENTIAL_VERSION,
            ciphertext: legacy.password_encrypted,
        };
        (password, credential)
    } else {
        let password = legacy.password;
        let credential = credential_from_plaintext(&password)?;
        (password, credential)
    };

    let allow_insecure_http = legacy
        .allow_insecure_http
        .unwrap_or_else(|| crate::endpoint::requires_insecure_http_opt_in(&legacy.server));
    let preferences = Preferences {
        theme_mode: legacy.theme_mode.clone(),
    };
    let disk = DiskConfigV2 {
        version: CONFIG_VERSION,
        server: legacy.server.clone(),
        username: legacy.username.clone(),
        topic: legacy.topic.clone(),
        allow_insecure_http,
        auto_start: legacy.auto_start,
        auto_copy_otp: legacy.auto_copy_otp,
        credential,
    };

    // 在修改任何文件前完成解密、校验和序列化。这样所有准备阶段的失败都不会
    // 触碰原始 config.json。
    let preference_bytes = serialize_json(&preferences, "界面偏好", MAX_PREFERENCES_JSON_BYTES)?;
    let config_bytes = serialize_json(&disk, "配置", MAX_CONFIG_JSON_BYTES)?;
    write_bytes(&preferences_path(), &preference_bytes)?;
    write_bytes(&config_path(), &config_bytes)?;

    Ok(Config {
        server: legacy.server,
        username: legacy.username,
        password,
        topic: legacy.topic,
        theme_mode: legacy.theme_mode,
        auto_start: legacy.auto_start,
        auto_copy_otp: legacy.auto_copy_otp,
        allow_insecure_http,
    })
}

fn load_preferences() -> Result<Preferences, String> {
    let path = preferences_path();
    let raw = match read_optional_bounded(&path, MAX_PREFERENCES_JSON_BYTES, "界面偏好")? {
        Some(raw) => raw,
        None => return Ok(default_preferences()),
    };
    let preferences: Preferences =
        serde_json::from_slice(&raw).map_err(|error| format!("界面偏好文件格式无效：{error}"))?;
    validate_theme(&preferences.theme_mode)?;
    Ok(preferences)
}

fn load_preferences_or_default() -> Preferences {
    load_preferences().unwrap_or_else(|error| {
        eprintln!("preferences load failed; replacing it with a safe default: {error}");
        let safe_preferences = default_preferences();
        let sanitize_result = serialize_json(
            &safe_preferences,
            "界面偏好",
            MAX_PREFERENCES_JSON_BYTES,
        )
        .and_then(|bytes| write_bytes(&preferences_path(), &bytes));
        if let Err(write_error) = sanitize_result {
            eprintln!(
                "failed to replace unsafe preferences; removing the exact backup-eligible file: {write_error}"
            );
            match fs::remove_file(preferences_path()) {
                Ok(()) => {}
                Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => {}
                Err(remove_error) => {
                    eprintln!("failed to remove unsafe preferences file: {remove_error}")
                }
            }
        }
        safe_preferences
    })
}

fn default_preferences() -> Preferences {
    Preferences {
        theme_mode: DEFAULT_THEME.to_string(),
    }
}

fn write_config_and_preferences(
    config_bytes: &[u8],
    preference_bytes: &[u8],
) -> Result<(), String> {
    let preference_path = preferences_path();
    // A malformed file must never remain at the only backup-eligible preference path.
    // Normalize it before capturing rollback state so a later config write failure cannot
    // restore attacker-controlled or obsolete fields into Android system backup.
    let _ = load_preferences_or_default();
    let previous_preferences =
        read_optional_bounded(&preference_path, MAX_PREFERENCES_JSON_BYTES, "原界面偏好")?;

    write_bytes(&preference_path, preference_bytes)?;
    if let Err(config_error) = write_bytes(&config_path(), config_bytes) {
        let rollback = match previous_preferences {
            Some(bytes) => write_bytes(&preference_path, &bytes),
            None => match fs::remove_file(&preference_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("移除新界面偏好失败：{error}")),
            },
        };
        return match rollback {
            Ok(()) => Err(config_error),
            Err(rollback_error) => Err(format!(
                "{config_error}；同时回滚界面偏好失败：{rollback_error}"
            )),
        };
    }
    Ok(())
}

fn validate_theme(theme: &str) -> Result<(), String> {
    if matches!(theme, "system" | "light" | "dark") {
        Ok(())
    } else {
        Err(format!("不支持的界面主题：{theme}"))
    }
}

fn credential_from_plaintext(plain: &str) -> Result<CredentialEnvelope, String> {
    Ok(CredentialEnvelope {
        provider: CREDENTIAL_PROVIDER.to_string(),
        version: CREDENTIAL_VERSION,
        ciphertext: encrypt_password(plain)?,
    })
}

fn plaintext_from_credential(credential: &CredentialEnvelope) -> Result<String, String> {
    if credential.provider != CREDENTIAL_PROVIDER {
        return Err(format!("不支持的凭据提供方：{}", credential.provider));
    }
    if credential.version != CREDENTIAL_VERSION {
        return Err(format!("不支持的凭据版本：{}", credential.version));
    }
    decrypt_password(&credential.ciphertext)
}

fn serialize_json<T: Serialize>(
    value: &T,
    label: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("序列化{label}失败：{error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!("{label}超过大小限制（最多 {max_bytes} 字节）"));
    }
    Ok(bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "数据文件缺少父目录".to_string())?;
    fs::create_dir_all(dir).map_err(|error| format!("创建数据目录失败：{error}"))?;
    let (tmp_path, tmp_file) = create_unique_temp_file(path)?;
    let result = write_and_replace(tmp_file, &tmp_path, path, bytes);
    if result.is_err() {
        // 只清理由本次调用以 create_new 创建的精确临时文件，不扫描或删除其他
        // 进程留下的文件。
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

fn create_unique_temp_file(target: &Path) -> Result<(PathBuf, File), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "数据文件缺少父目录".to_string())?;
    let target_name = target
        .file_name()
        .ok_or_else(|| "数据文件缺少文件名".to_string())?;

    for _ in 0..TEMP_CREATE_ATTEMPTS {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_path = temporary_path(parent, target_name, sequence);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("创建唯一临时文件失败：{error}")),
        }
    }
    Err("无法创建唯一临时文件".to_string())
}

fn temporary_path(parent: &Path, target_name: &std::ffi::OsStr, sequence: u64) -> PathBuf {
    let mut temp_name = OsString::from(target_name);
    temp_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
    parent.join(temp_name)
}

fn write_and_replace(
    mut temp_file: File,
    temp_path: &Path,
    target: &Path,
    bytes: &[u8],
) -> Result<(), String> {
    temp_file
        .write_all(bytes)
        .map_err(|error| format!("写入临时文件失败：{error}"))?;
    temp_file
        .flush()
        .map_err(|error| format!("刷新临时文件缓冲区失败：{error}"))?;
    temp_file
        .sync_all()
        .map_err(|error| format!("同步临时文件失败：{error}"))?;
    drop(temp_file);

    fs::rename(temp_path, target).map_err(|error| format!("原子替换数据文件失败：{error}"))?;
    // rename 成功后目标已提交，不能再把父目录 fsync 的次生失败报告成“保存失败”，
    // 否则调用方会回滚另一份文件并制造跨文件状态分裂。
    if let Err(error) = sync_parent_directory(target) {
        eprintln!("configuration directory sync failed after atomic replace: {error}");
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "数据文件缺少父目录".to_string())?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("同步数据目录失败：{error}"))
}

#[cfg(not(unix))]
fn sync_parent_directory(_target: &Path) -> Result<(), String> {
    Ok(())
}

pub fn encrypt_password(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    let bytes = dpapi::protect(plain)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

pub fn decrypt_password(encoded: &str) -> Result<String, String> {
    if encoded.is_empty() {
        return Ok(String::new());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "凭据密文不是有效的 Base64".to_string())?;
    let raw = dpapi::unprotect(&bytes)?;
    if raw.len() % 2 != 0 {
        return Err("解密后的凭据长度无效".to_string());
    }
    let wide: Vec<u16> = raw
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&wide).map_err(|_| "解密后的凭据不是有效文本".to_string())
}

#[cfg(windows)]
mod dpapi {
    use std::io;
    use std::ptr;
    use winapi::um::dpapi::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN};
    use winapi::um::winbase::LocalFree;
    use winapi::um::wincrypt::DATA_BLOB;

    pub(super) fn checked_blob_len(byte_len: usize, label: &str) -> Result<u32, String> {
        u32::try_from(byte_len).map_err(|_| format!("{label}长度超出 Windows DPAPI 限制"))
    }

    pub fn protect(plain: &str) -> Result<Vec<u8>, String> {
        let wide: Vec<u16> = plain.encode_utf16().collect();
        let byte_len = wide
            .len()
            .checked_mul(std::mem::size_of::<u16>())
            .ok_or_else(|| "凭据长度超出 Windows DPAPI 限制".to_string())?;
        let byte_len = checked_blob_len(byte_len, "凭据")?;
        let mut input = DATA_BLOB {
            cbData: byte_len,
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
            // 必须紧接失败的 Win32 调用捕获线程本地错误，避免被其他调用覆盖。
            let error = io::Error::last_os_error();
            return Err(format!("Windows DPAPI 保护凭据失败：{error}"));
        }
        if output.pbData.is_null() {
            return Err("Windows DPAPI 保护凭据返回空输出".to_string());
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(output.pbData as *mut _);
        }
        if bytes.is_empty() {
            return Err("Windows DPAPI 保护凭据返回空输出".to_string());
        }
        Ok(bytes)
    }

    pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
        let blob_len = checked_blob_len(blob.len(), "凭据密文")?;
        let mut input = DATA_BLOB {
            cbData: blob_len,
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
            // 必须紧接失败的 Win32 调用捕获线程本地错误，避免被其他调用覆盖。
            let error = io::Error::last_os_error();
            return Err(format!("Windows DPAPI 解密凭据失败：{error}"));
        }
        if output.pbData.is_null() {
            return Err("Windows DPAPI 解密凭据返回空输出".to_string());
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(output.pbData as *mut _);
        }
        if bytes.is_empty() {
            return Err("Windows DPAPI 解密凭据返回空输出".to_string());
        }
        Ok(bytes)
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(plain: &str) -> Result<Vec<u8>, String> {
        // 与 Windows 端保持一致：按 UTF-16 LE 编码后存储，
        // 解密逻辑（decrypt_password）才能正确还原。
        let mut bytes = Vec::with_capacity(plain.len() * 2);
        for u in plain.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
        Ok(blob.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Barrier, MutexGuard};
    use std::time::Duration;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_env() -> MutexGuard<'static, ()> {
        let guard = crate::appdata::test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let number = COUNTER.fetch_add(1, Ordering::SeqCst);
        let directory =
            std::env::temp_dir().join(format!("ntfy-test-config-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        crate::appdata::set(directory);
        guard
    }

    fn write_json_value(path: &Path, value: &Value) -> Vec<u8> {
        let bytes = serde_json::to_vec_pretty(value).unwrap();
        std::fs::write(path, &bytes).unwrap();
        bytes
    }

    fn assert_safe_default_preferences() {
        let bytes = std::fs::read(preferences_path()).unwrap();
        let preferences: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(preferences, json!({"theme_mode": DEFAULT_THEME}));
        assert_eq!(preferences.as_object().unwrap().len(), 1);
    }

    fn write_legacy_config(
        server: &str,
        allow_insecure_http: Option<bool>,
        theme: &str,
        password: &str,
        encrypted: &str,
    ) -> Vec<u8> {
        let mut value = json!({
            "server": server,
            "username": "alice",
            "topic": "alerts",
            "theme_mode": theme,
            "auto_start": true,
            "auto_copy_otp": false,
            "password": password,
            "password_encrypted": encrypted
        });
        if let Some(allow) = allow_insecure_http {
            value["allow_insecure_http"] = Value::Bool(allow);
        }
        write_json_value(&config_path(), &value)
    }

    fn sample_config(theme: &str) -> Config {
        Config {
            server: "https://ntfy.example.com".to_string(),
            username: "alice".to_string(),
            password: "secret123".to_string(),
            topic: "alerts".to_string(),
            theme_mode: theme.to_string(),
            auto_start: true,
            auto_copy_otp: true,
            allow_insecure_http: false,
        }
    }

    fn create_temp_collisions(target: &Path, first_sequence: u64, count: usize) -> Vec<PathBuf> {
        let parent = target.parent().unwrap();
        let target_name = target.file_name().unwrap();
        (0..count)
            .map(|offset| {
                let path = temporary_path(parent, target_name, first_sequence + offset as u64);
                std::fs::write(&path, b"belongs-to-another-writer").unwrap();
                path
            })
            .collect()
    }

    #[test]
    fn default_config_shape() {
        let cfg = Config::default();
        assert_eq!(cfg.theme_mode, "system");
        assert_eq!(cfg.topic, "");
        assert!(!cfg.allow_insecure_http);
    }

    #[test]
    fn config_debug_output_never_exposes_connection_fields() {
        let config = sample_config("dark");
        let debug = format!("{config:?}");

        assert_eq!(debug, "Config(<redacted>)");
        assert!(!debug.contains("secret123"));
        assert!(!debug.contains("ntfy.example.com"));
    }

    #[test]
    fn missing_config_reads_preferences_without_creating_sensitive_file() {
        let _guard = unique_env();
        write_json_value(&preferences_path(), &json!({"theme_mode": "dark"}));

        let config = load_config().unwrap();

        assert_eq!(config.theme_mode, "dark");
        assert!(config.server.is_empty());
        assert!(!config_path().exists());
    }

    #[test]
    fn missing_files_return_defaults_without_creating_files() {
        let _guard = unique_env();

        let config = load_config().unwrap();

        assert_eq!(config.theme_mode, DEFAULT_THEME);
        assert!(!config_path().exists());
        assert!(!preferences_path().exists());
    }

    #[test]
    fn oversized_config_is_rejected_without_rewriting_it() {
        let _guard = unique_env();
        let original = vec![b'x'; MAX_CONFIG_JSON_BYTES + 1];
        std::fs::write(config_path(), &original).unwrap();

        let error = load_config().unwrap_err();

        assert!(error.contains("配置超过大小限制"));
        assert_eq!(std::fs::read(config_path()).unwrap(), original);
        assert!(!preferences_path().exists());
    }

    #[test]
    fn oversized_preferences_are_sanitized_before_they_can_be_backed_up() {
        let _guard = unique_env();
        std::fs::write(
            preferences_path(),
            vec![b'x'; MAX_PREFERENCES_JSON_BYTES + 1],
        )
        .unwrap();

        let loaded = load_config().unwrap();
        assert_eq!(loaded.theme_mode, DEFAULT_THEME);
        assert_safe_default_preferences();

        save_config(&sample_config("dark")).unwrap();
        assert!(config_path().exists());
        let preferences: Value =
            serde_json::from_slice(&std::fs::read(preferences_path()).unwrap()).unwrap();
        assert_eq!(preferences, json!({"theme_mode": "dark"}));
    }

    #[test]
    fn serialized_output_cannot_exceed_the_corresponding_read_limit() {
        let oversized_config = json!({"value": "x".repeat(MAX_CONFIG_JSON_BYTES)});
        let oversized_preferences = json!({"value": "x".repeat(MAX_PREFERENCES_JSON_BYTES)});

        assert!(serialize_json(&oversized_config, "配置", MAX_CONFIG_JSON_BYTES).is_err());
        assert!(serialize_json(
            &oversized_preferences,
            "界面偏好",
            MAX_PREFERENCES_JSON_BYTES
        )
        .is_err());
    }

    #[test]
    fn save_uses_v2_envelope_and_theme_only_preferences() {
        let _guard = unique_env();
        let config = sample_config("dark");

        save_config(&config).unwrap();

        let disk: Value = serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        assert_eq!(disk["version"], 2);
        assert_eq!(disk["server"], config.server);
        assert_eq!(disk["credential"]["provider"], CREDENTIAL_PROVIDER);
        assert_eq!(disk["credential"]["version"], CREDENTIAL_VERSION);
        assert!(disk["credential"]["ciphertext"].as_str().unwrap().len() > 1);
        assert!(disk.get("theme_mode").is_none());
        assert!(disk.get("password").is_none());
        assert!(disk.get("password_encrypted").is_none());

        let preferences: Value =
            serde_json::from_slice(&std::fs::read(preferences_path()).unwrap()).unwrap();
        assert_eq!(preferences.as_object().unwrap().len(), 1);
        assert_eq!(preferences["theme_mode"], "dark");

        let loaded = load_config().unwrap();
        assert_eq!(loaded.server, config.server);
        assert_eq!(loaded.password, config.password);
        assert_eq!(loaded.theme_mode, "dark");
        assert!(loaded.auto_copy_otp);
    }

    #[test]
    fn repeated_save_atomically_replaces_existing_config_and_preferences() {
        let _guard = unique_env();
        save_config(&sample_config("dark")).unwrap();

        let mut replacement = sample_config("light");
        replacement.server = "https://second.example.com".to_string();
        replacement.password = "replacement-secret".to_string();
        replacement.auto_copy_otp = false;
        save_config(&replacement).unwrap();

        let loaded = load_config().unwrap();
        assert_eq!(loaded.server, replacement.server);
        assert_eq!(loaded.password, replacement.password);
        assert_eq!(loaded.theme_mode, "light");
        assert!(!loaded.auto_copy_otp);
        let preferences: Value =
            serde_json::from_slice(&std::fs::read(preferences_path()).unwrap()).unwrap();
        assert_eq!(preferences, json!({"theme_mode": "light"}));
    }

    #[test]
    fn legacy_remote_http_is_migrated_without_reencrypting_ciphertext() {
        let _guard = unique_env();
        let ciphertext = encrypt_password("legacy-secret").unwrap();
        write_legacy_config("HTTP://EXAMPLE.COM", None, "system", "", &ciphertext);

        let config = load_config().unwrap();
        assert!(config.allow_insecure_http);
        assert_eq!(config.password, "legacy-secret");

        let migrated: Value =
            serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        assert_eq!(migrated["version"], 2);
        assert_eq!(migrated["allow_insecure_http"], Value::Bool(true));
        assert_eq!(
            migrated["credential"]["ciphertext"],
            Value::String(ciphertext)
        );
        assert!(migrated.get("theme_mode").is_none());
    }

    #[test]
    fn legacy_loopback_http_keeps_insecure_opt_in_disabled() {
        let _guard = unique_env();
        write_legacy_config("http://127.0.0.1:8080", None, "system", "", "");

        let config = load_config().unwrap();
        assert!(!config.allow_insecure_http);
        let migrated: Value =
            serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        assert_eq!(migrated["allow_insecure_http"], Value::Bool(false));
    }

    #[test]
    fn explicit_insecure_http_choice_is_not_overwritten() {
        let _guard = unique_env();
        write_legacy_config("http://example.com", Some(false), "system", "", "");

        let config = load_config().unwrap();
        assert!(!config.allow_insecure_http);
        let stored: Value = serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        assert_eq!(stored["allow_insecure_http"], Value::Bool(false));
    }

    #[test]
    fn legacy_plaintext_password_is_protected_during_migration() {
        let _guard = unique_env();
        write_legacy_config(
            "https://ntfy.example.com",
            Some(false),
            "light",
            "plain-secret",
            "",
        );

        let config = load_config().unwrap();

        assert_eq!(config.password, "plain-secret");
        assert_eq!(config.theme_mode, "light");
        let raw = std::fs::read(config_path()).unwrap();
        let stored: Value = serde_json::from_slice(&raw).unwrap();
        assert_ne!(stored["credential"]["ciphertext"], "plain-secret");
        assert!(!String::from_utf8(raw).unwrap().contains("plain-secret"));
        let preferences: Value =
            serde_json::from_slice(&std::fs::read(preferences_path()).unwrap()).unwrap();
        assert_eq!(preferences["theme_mode"], "light");
    }

    #[test]
    fn invalid_legacy_json_is_preserved_without_creating_preferences() {
        let _guard = unique_env();
        let original = b"{this is not json".to_vec();
        std::fs::write(config_path(), &original).unwrap();

        assert!(load_config().is_err());

        assert_eq!(std::fs::read(config_path()).unwrap(), original);
        assert!(!preferences_path().exists());
    }

    #[test]
    fn legacy_decryption_failure_preserves_original_bytes() {
        let _guard = unique_env();
        let original = write_legacy_config(
            "https://ntfy.example.com",
            Some(false),
            "system",
            "",
            "not-valid-base64",
        );

        let error = load_config().unwrap_err();

        assert!(error.contains("Base64"));
        assert_eq!(std::fs::read(config_path()).unwrap(), original);
        assert!(!preferences_path().exists());
    }

    #[test]
    fn unsupported_config_version_is_strict_and_preserves_file() {
        let _guard = unique_env();
        let original = write_json_value(&config_path(), &json!({"version": 3}));

        let error = load_config().unwrap_err();

        assert!(error.contains("不支持的配置版本"));
        assert_eq!(std::fs::read(config_path()).unwrap(), original);
        assert!(!preferences_path().exists());
    }

    #[test]
    fn unknown_credential_provider_is_rejected_without_rewrite() {
        let _guard = unique_env();
        save_config(&sample_config("system")).unwrap();
        let mut value: Value =
            serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        value["credential"]["provider"] = Value::String("unknown-provider".to_string());
        let original = write_json_value(&config_path(), &value);

        let error = load_config().unwrap_err();

        assert!(error.contains("不支持的凭据提供方"));
        assert_eq!(std::fs::read(config_path()).unwrap(), original);
    }

    #[test]
    fn unknown_credential_version_is_rejected_without_rewrite() {
        let _guard = unique_env();
        save_config(&sample_config("system")).unwrap();
        let mut value: Value =
            serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        value["credential"]["version"] = Value::from(99);
        let original = write_json_value(&config_path(), &value);

        let error = load_config().unwrap_err();

        assert!(error.contains("不支持的凭据版本"));
        assert_eq!(std::fs::read(config_path()).unwrap(), original);
    }

    #[test]
    fn malformed_v2_does_not_fall_back_to_legacy_defaults() {
        let _guard = unique_env();
        let original = write_json_value(
            &config_path(),
            &json!({
                "version": 2,
                "server": "https://ntfy.example.com"
            }),
        );

        assert!(load_config().is_err());

        assert_eq!(std::fs::read(config_path()).unwrap(), original);
        assert!(!preferences_path().exists());
    }

    #[test]
    fn v2_missing_new_insecure_http_field_defaults_without_rewrite() {
        let _guard = unique_env();
        save_config(&sample_config("dark")).unwrap();
        let mut value: Value =
            serde_json::from_slice(&std::fs::read(config_path()).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("allow_insecure_http");
        let original = write_json_value(&config_path(), &value);

        let config = load_config().unwrap();

        assert!(!config.allow_insecure_http);
        assert_eq!(config.theme_mode, "dark");
        assert_eq!(std::fs::read(config_path()).unwrap(), original);
    }

    #[test]
    fn invalid_preferences_are_sanitized_without_creating_sensitive_config() {
        let _guard = unique_env();
        write_json_value(
            &preferences_path(),
            &json!({
                "version": 7,
                "theme_mode": "dark",
                "password": "must-not-be-backed-up"
            }),
        );

        let config = load_config().unwrap();

        assert_eq!(config.theme_mode, DEFAULT_THEME);
        assert_safe_default_preferences();
        assert!(
            !String::from_utf8_lossy(&std::fs::read(preferences_path()).unwrap())
                .contains("must-not-be-backed-up")
        );
        assert!(!config_path().exists());
    }

    #[test]
    fn invalid_preferences_do_not_block_v2_connection_or_password() {
        let _guard = unique_env();
        let sample = sample_config("dark");
        save_config(&sample).unwrap();
        write_json_value(
            &preferences_path(),
            &json!({"theme_mode": "dark", "server": "must-not-be-backed-up"}),
        );

        let config = load_config().unwrap();

        assert_eq!(config.server, sample.server);
        assert_eq!(config.password, sample.password);
        assert_eq!(config.theme_mode, DEFAULT_THEME);
        assert_safe_default_preferences();
        assert!(
            !String::from_utf8_lossy(&std::fs::read(preferences_path()).unwrap())
                .contains("must-not-be-backed-up")
        );
    }

    #[test]
    fn save_rejects_unknown_theme_without_writing_files() {
        let _guard = unique_env();
        let config = sample_config("sepia");

        let error = save_config(&config).unwrap_err();

        assert!(error.contains("不支持的界面主题"));
        assert!(!config_path().exists());
        assert!(!preferences_path().exists());
    }

    #[test]
    fn preference_write_failure_leaves_legacy_config_untouched() {
        let _guard = unique_env();
        let original = write_legacy_config("https://ntfy.example.com", Some(false), "dark", "", "");
        std::fs::create_dir(preferences_path()).unwrap();
        std::fs::write(preferences_path().join("keep"), b"do-not-delete").unwrap();

        assert!(load_config().is_err());

        assert_eq!(std::fs::read(config_path()).unwrap(), original);
        assert!(preferences_path().is_dir());
        assert_eq!(
            std::fs::read(preferences_path().join("keep")).unwrap(),
            b"do-not-delete"
        );
    }

    #[test]
    fn final_config_write_failure_keeps_legacy_as_migration_authority() {
        let _guard = unique_env();
        let original =
            write_legacy_config("https://ntfy.example.com", Some(false), "light", "", "");
        let first_sequence = TEMP_COUNTER.load(Ordering::Relaxed);
        // 迁移先为 preferences 消耗一个序号；随后让 config 的全部 create_new
        // 候选都与其他写入者的临时文件冲突。
        let collisions =
            create_temp_collisions(&config_path(), first_sequence + 1, TEMP_CREATE_ATTEMPTS);

        assert!(load_config().is_err());

        assert_eq!(std::fs::read(config_path()).unwrap(), original);
        assert!(collisions.iter().all(|path| path.exists()));
        let preferences: Value =
            serde_json::from_slice(&std::fs::read(preferences_path()).unwrap()).unwrap();
        assert_eq!(preferences["theme_mode"], "light");
    }

    #[test]
    fn save_failure_restores_existing_preferences() {
        let _guard = unique_env();
        let original_preferences =
            write_json_value(&preferences_path(), &json!({"theme_mode": "light"}));
        std::fs::create_dir(config_path()).unwrap();
        std::fs::write(config_path().join("keep"), b"do-not-delete").unwrap();

        assert!(save_config(&sample_config("dark")).is_err());

        assert_eq!(
            std::fs::read(preferences_path()).unwrap(),
            original_preferences
        );
        assert!(config_path().is_dir());
        assert_eq!(
            std::fs::read(config_path().join("keep")).unwrap(),
            b"do-not-delete"
        );
    }

    #[test]
    fn save_failure_removes_new_preferences() {
        let _guard = unique_env();
        std::fs::create_dir(config_path()).unwrap();
        std::fs::write(config_path().join("keep"), b"do-not-delete").unwrap();

        assert!(save_config(&sample_config("dark")).is_err());

        assert!(!preferences_path().exists());
        assert!(config_path().is_dir());
        assert_eq!(
            std::fs::read(config_path().join("keep")).unwrap(),
            b"do-not-delete"
        );
    }

    #[test]
    fn stale_fixed_temp_files_do_not_block_atomic_save() {
        let _guard = unique_env();
        let stale_config = config_path().with_extension("tmp");
        let stale_preferences = preferences_path().with_extension("tmp");
        std::fs::write(&stale_config, b"old-config-temp").unwrap();
        std::fs::write(&stale_preferences, b"old-preferences-temp").unwrap();

        save_config(&sample_config("dark")).unwrap();

        assert_eq!(std::fs::read(stale_config).unwrap(), b"old-config-temp");
        assert_eq!(
            std::fs::read(stale_preferences).unwrap(),
            b"old-preferences-temp"
        );
        assert!(config_path().is_file());
        assert!(preferences_path().is_file());
    }

    #[test]
    fn create_new_collision_is_skipped_without_deleting_foreign_temp() {
        let _guard = unique_env();
        let sequence = TEMP_COUNTER.load(Ordering::Relaxed);
        let collision = create_temp_collisions(&config_path(), sequence, 1).remove(0);

        write_bytes(&config_path(), b"new-config").unwrap();

        assert_eq!(std::fs::read(config_path()).unwrap(), b"new-config");
        assert_eq!(
            std::fs::read(collision).unwrap(),
            b"belongs-to-another-writer"
        );
    }

    #[test]
    fn failed_replace_cleans_only_its_own_unique_temp() {
        let _guard = unique_env();
        let target = crate::appdata::resolve().join("blocked.json");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep"), b"target-directory").unwrap();
        let foreign_sequence =
            TEMP_COUNTER.load(Ordering::Relaxed) + TEMP_CREATE_ATTEMPTS as u64 + 100;
        let foreign = create_temp_collisions(&target, foreign_sequence, 1).remove(0);
        let prefix = format!("blocked.json.tmp-{}-", std::process::id());

        assert!(write_bytes(&target, b"must-not-replace-directory").is_err());

        assert_eq!(
            std::fs::read(target.join("keep")).unwrap(),
            b"target-directory"
        );
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"belongs-to-another-writer"
        );
        let matching: Vec<_> = std::fs::read_dir(crate::appdata::resolve())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .map(|entry| entry.path())
            .collect();
        assert_eq!(matching, vec![foreign]);
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn config_file_lock_serializes_parallel_operations() {
        let _guard = unique_env();
        let (first_acquired_tx, first_acquired_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let first = std::thread::spawn(move || {
            with_config_lock(|| {
                first_acquired_tx.send(()).unwrap();
                release_first_rx.recv().unwrap();
                Ok(())
            })
        });
        first_acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let start_second = Arc::new(Barrier::new(2));
        let second_barrier = Arc::clone(&start_second);
        let (second_acquired_tx, second_acquired_rx) = mpsc::channel();
        let second = std::thread::spawn(move || {
            second_barrier.wait();
            with_config_lock(|| {
                second_acquired_tx.send(()).unwrap();
                Ok(())
            })
        });
        start_second.wait();
        assert!(second_acquired_rx
            .recv_timeout(Duration::from_millis(150))
            .is_err());

        release_first_tx.send(()).unwrap();
        second_acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    }

    #[test]
    fn successful_legacy_migration_overwrites_interrupted_preferences() {
        let _guard = unique_env();
        write_legacy_config("https://ntfy.example.com", Some(false), "light", "", "");
        write_json_value(&preferences_path(), &json!({"theme_mode": "dark"}));

        let config = load_config().unwrap();

        assert_eq!(config.theme_mode, "light");
        let preferences: Value =
            serde_json::from_slice(&std::fs::read(preferences_path()).unwrap()).unwrap();
        assert_eq!(preferences["theme_mode"], "light");
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip() {
        let enc = encrypt_password("secret123").unwrap();
        assert_ne!(enc, "secret123");
        assert_eq!(decrypt_password(&enc).unwrap(), "secret123");
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn dpapi_rejects_blob_lengths_that_do_not_fit_u32() {
        let error = dpapi::checked_blob_len(usize::MAX, "测试数据").unwrap_err();
        assert!(error.contains("Windows DPAPI 限制"));
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_decryption_error_does_not_echo_ciphertext() {
        let ciphertext = base64::engine::general_purpose::STANDARD.encode([1_u8, 2, 3, 4]);

        let error = decrypt_password(&ciphertext).unwrap_err();

        assert!(error.contains("Windows DPAPI 解密凭据失败"));
        assert!(!error.contains(&ciphertext));
    }
}
