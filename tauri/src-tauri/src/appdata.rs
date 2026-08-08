use std::path::PathBuf;
use std::sync::Mutex;

static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 注入数据目录（移动端在 setup 时注入 app_data_dir；测试用）。
#[cfg(any(mobile, test))]
pub fn set(dir: PathBuf) {
    *OVERRIDE.lock().unwrap() = Some(dir);
}

/// 应用数据目录：优先注入值，其次 %APPDATA%\ntfy-notifier（桌面），最后 home 兜底。
pub fn resolve() -> PathBuf {
    if let Some(dir) = OVERRIDE.lock().unwrap().clone() {
        return dir;
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("ntfy-notifier")
    } else if let Some(home) = dirs::home_dir() {
        home.join("AppData").join("Roaming").join("ntfy-notifier")
    } else {
        PathBuf::from("ntfy-notifier")
    }
}

/// 串行化所有依赖数据目录的测试（跨模块共享，避免并行测试互相覆盖注入值）。
#[cfg(test)]
pub fn test_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}
