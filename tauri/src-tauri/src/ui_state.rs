use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const DEFAULT_ORDER: [&str; 3] = ["time", "title", "message"];
const DEFAULT_WIDTHS: [(&str, i64); 3] = [("time", 180), ("title", 220), ("message", 640)];
const MIN_WIDTHS: [(&str, i64); 3] = [("time", 120), ("title", 80), ("message", 160)];
static APP_DATA_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
fn set_test_dir(dir: PathBuf) {
    *APP_DATA_OVERRIDE.lock().unwrap() = Some(dir);
}

fn appdata_dir() -> PathBuf {
    if let Some(dir) = APP_DATA_OVERRIDE.lock().unwrap().clone() {
        return dir;
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata)
    } else if let Some(home) = dirs::home_dir() {
        home.join("AppData").join("Roaming")
    } else {
        PathBuf::from(".")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UiState {
    pub column_order: Vec<String>,
    pub column_widths: HashMap<String, i64>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            column_order: DEFAULT_ORDER.iter().map(|s| s.to_string()).collect(),
            column_widths: DEFAULT_WIDTHS
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
        }
    }
}

fn state_path() -> PathBuf {
    appdata_dir().join("ntfy-notifier").join("ui_state.json")
}

pub fn load() -> UiState {
    let default = UiState::default();
    let raw = match fs::read_to_string(state_path()) {
        Ok(r) => r,
        Err(_) => return default,
    };
    let parsed: UiState = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(_) => return default,
    };
    // 补全缺失列并约束最小宽度
    let mut order = parsed.column_order;
    for col in DEFAULT_ORDER {
        if !order.iter().any(|c| c == col) {
            order.push(col.to_string());
        }
    }
    let mut widths = parsed.column_widths;
    for (col, min) in MIN_WIDTHS {
        let w = widths.get(col).copied().unwrap_or(0);
        widths.insert(col.to_string(), w.max(min));
    }
    UiState { column_order: order, column_widths: widths }
}

pub fn save(order: Vec<String>, widths: HashMap<String, i64>) -> Result<(), String> {
    let mut clamped = widths;
    for (col, min) in MIN_WIDTHS {
        let w = clamped.get(col).copied().unwrap_or(0);
        clamped.insert(col.to_string(), w.max(min));
    }
    let state = UiState { column_order: order, column_widths: clamped };
    let json = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
    let path = state_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    drop(f);
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_env() {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ntfy-test-uistate-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_test_dir(dir);
    }

    #[test]
    fn default_when_missing() {
        unique_env();
        let s = load();
        assert_eq!(s.column_order.len(), 3);
    }

    #[test]
    fn roundtrip_and_clamp() {
        unique_env();
        let mut widths = HashMap::new();
        widths.insert("time".to_string(), 10);
        widths.insert("message".to_string(), 99999);
        save(vec!["message".into(), "time".into(), "title".into()], widths).unwrap();
        let s = load();
        assert_eq!(s.column_widths["time"], 120);
        assert_eq!(s.column_widths["message"], 99999);
        assert_eq!(s.column_order[0], "message");
    }
}
