use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const DEFAULT_ORDER: [&str; 3] = ["time", "title", "message"];
const DEFAULT_WIDTHS: [(&str, i64); 3] = [("time", 180), ("title", 220), ("message", 640)];
const MIN_WIDTHS: [(&str, i64); 3] = [("time", 120), ("title", 80), ("message", 160)];
const MAX_COLUMN_WIDTH: i64 = 8_192;
const MAX_UI_STATE_BYTES: usize = 4 * 1024;
const TEMP_CREATE_ATTEMPTS: usize = 32;

static UI_STATE_LOCK: Mutex<()> = Mutex::new(());
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn standard_column_id(legacy: &str) -> &str {
    match legacy {
        "时间" => "time",
        "标题" => "title",
        "内容" => "message",
        other => other,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UiState {
    pub column_order: Vec<String>,
    #[serde(deserialize_with = "deserialize_column_widths")]
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

struct ColumnWidthsVisitor;

impl<'de> Visitor<'de> for ColumnWidthsVisitor {
    type Value = HashMap<String, i64>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a column-width object without duplicate keys")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut widths = HashMap::with_capacity(DEFAULT_ORDER.len());
        while let Some((column, width)) = map.next_entry::<String, i64>()? {
            if widths.insert(column, width).is_some() {
                return Err(de::Error::custom("duplicate column-width key"));
            }
        }
        Ok(widths)
    }
}

fn deserialize_column_widths<'de, D>(deserializer: D) -> Result<HashMap<String, i64>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_map(ColumnWidthsVisitor)
}

fn state_path() -> PathBuf {
    crate::appdata::resolve().join("ui_state.json")
}

pub fn load() -> UiState {
    let _guard = UI_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    load_locked()
}

fn load_locked() -> UiState {
    let path = state_path();
    let raw = match read_bounded(&path) {
        Ok(Some(raw)) => raw,
        Ok(None) => return UiState::default(),
        Err(()) => return replace_invalid_state(&path),
    };

    let parsed = match serde_json::from_slice::<UiState>(&raw) {
        Ok(parsed) => parsed,
        Err(_) => return replace_invalid_state(&path),
    };
    let (state, migrated) = match normalize_and_validate(parsed) {
        Ok(result) => result,
        Err(()) => return replace_invalid_state(&path),
    };

    if migrated {
        if let Err(error) = write_state(&path, &state) {
            eprintln!("failed to rewrite migrated UI layout: {error}");
        }
    }
    state
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, ()> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let mut raw = Vec::with_capacity(MAX_UI_STATE_BYTES.min(512));
    file.take((MAX_UI_STATE_BYTES + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|_| ())?;
    if raw.len() > MAX_UI_STATE_BYTES {
        return Err(());
    }
    Ok(Some(raw))
}

fn normalize_and_validate(mut state: UiState) -> Result<(UiState, bool), ()> {
    let mut migrated = false;
    let mut seen_order = [false; DEFAULT_ORDER.len()];
    for column in &mut state.column_order {
        let normalized = standard_column_id(column);
        if normalized != column {
            *column = normalized.to_string();
            migrated = true;
        }
        let Some(index) = DEFAULT_ORDER
            .iter()
            .position(|candidate| column.as_str() == *candidate)
        else {
            return Err(());
        };
        if seen_order[index] {
            return Err(());
        }
        seen_order[index] = true;
    }
    for (index, column) in DEFAULT_ORDER.iter().enumerate() {
        if !seen_order[index] {
            state.column_order.push((*column).to_string());
            migrated = true;
        }
    }

    let mut normalized_widths = HashMap::with_capacity(DEFAULT_ORDER.len());
    for (column, width) in state.column_widths {
        let normalized = standard_column_id(&column);
        migrated |= normalized != column;
        let Some((_, minimum)) = MIN_WIDTHS
            .iter()
            .find(|(candidate, _)| *candidate == normalized)
        else {
            return Err(());
        };
        let normalized_width = width.clamp(*minimum, MAX_COLUMN_WIDTH);
        migrated |= normalized_width != width;
        if normalized_widths
            .insert(normalized.to_string(), normalized_width)
            .is_some()
        {
            return Err(());
        }
    }
    for (column, default_width) in DEFAULT_WIDTHS {
        if !normalized_widths.contains_key(column) {
            normalized_widths.insert(column.to_string(), default_width);
            migrated = true;
        }
    }
    state.column_widths = normalized_widths;
    validate(&state)?;
    Ok((state, migrated))
}

fn validate(state: &UiState) -> Result<(), ()> {
    if state.column_order.len() != DEFAULT_ORDER.len()
        || state.column_widths.len() != DEFAULT_ORDER.len()
    {
        return Err(());
    }

    let mut seen = [false; DEFAULT_ORDER.len()];
    for column in &state.column_order {
        let Some(index) = DEFAULT_ORDER
            .iter()
            .position(|candidate| column == candidate)
        else {
            return Err(());
        };
        if seen[index] {
            return Err(());
        }
        seen[index] = true;
    }

    for (column, minimum) in MIN_WIDTHS {
        let Some(width) = state.column_widths.get(column) else {
            return Err(());
        };
        if *width < minimum || *width > MAX_COLUMN_WIDTH {
            return Err(());
        }
    }
    Ok(())
}

fn replace_invalid_state(path: &Path) -> UiState {
    let safe = UiState::default();
    if let Err(error) = write_state(path, &safe) {
        eprintln!("failed to sanitize invalid UI layout: {error}");
        // Android backup allowlists this exact file. If atomic replacement is impossible,
        // best-effort removal keeps an untrusted legacy payload out of future backups.
        if let Err(remove_error) = fs::remove_file(path) {
            if remove_error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("failed to remove invalid UI layout: {remove_error}");
            }
        }
    }
    safe
}

pub fn save(order: Vec<String>, widths: HashMap<String, i64>) -> Result<(), String> {
    let _guard = UI_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = UiState {
        column_order: order,
        column_widths: widths,
    };
    validate_accepted_payload(&state)?;
    write_state(&state_path(), &state)
}

fn validate_accepted_payload(state: &UiState) -> Result<(), String> {
    let string_bytes = state
        .column_order
        .iter()
        .map(String::len)
        .chain(state.column_widths.keys().map(String::len))
        .try_fold(0usize, |total, length| total.checked_add(length))
        .ok_or_else(|| "UI layout payload is too large".to_string())?;
    if string_bytes > MAX_UI_STATE_BYTES {
        return Err("UI layout payload is too large".to_string());
    }
    validate(state).map_err(|()| {
        "UI layout must contain one complete time/title/message permutation and valid widths"
            .to_string()
    })
}

fn write_state(path: &Path, state: &UiState) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    if json.len() > MAX_UI_STATE_BYTES {
        return Err("serialized UI layout exceeds the size limit".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "UI layout path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let file_name = path
        .file_name()
        .ok_or_else(|| "UI layout path has no file name".to_string())?;
    let (temp_path, mut temp_file) = create_unique_temp(parent, file_name)?;

    let result = (|| {
        temp_file
            .write_all(&json)
            .map_err(|error| error.to_string())?;
        temp_file.flush().map_err(|error| error.to_string())?;
        temp_file.sync_all().map_err(|error| error.to_string())?;
        drop(temp_file);
        atomic_replace(&temp_path, path)?;
        sync_parent_best_effort(path);
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn create_unique_temp(parent: &Path, file_name: &OsStr) -> Result<(PathBuf, File), String> {
    for _ in 0..TEMP_CREATE_ATTEMPTS {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = OsString::from(file_name);
        temp_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
        let temp_path = parent.join(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("could not create a unique UI layout temporary file".to_string())
}

#[cfg(not(windows))]
fn atomic_replace(temp_path: &Path, target: &Path) -> Result<(), String> {
    fs::rename(temp_path, target).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn atomic_replace(temp_path: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::winbase::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};

    let source: Vec<u16> = temp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_best_effort(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
            eprintln!("failed to sync UI layout directory: {error}");
        }
    }
}

#[cfg(not(unix))]
fn sync_parent_best_effort(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::MutexGuard;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_env() -> MutexGuard<'static, ()> {
        let guard = crate::appdata::test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ntfy-test-uistate-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::appdata::set(dir);
        guard
    }

    fn widths(time: i64, title: i64, message: i64) -> HashMap<String, i64> {
        [
            ("time".to_string(), time),
            ("title".to_string(), title),
            ("message".to_string(), message),
        ]
        .into_iter()
        .collect()
    }

    fn assert_disk_is_safe_default() {
        let raw = fs::read(state_path()).unwrap();
        assert!(raw.len() <= MAX_UI_STATE_BYTES);
        let value: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert_eq!(
            serde_json::from_value::<UiState>(value).unwrap(),
            UiState::default()
        );
    }

    #[test]
    fn default_when_missing() {
        let _guard = unique_env();
        assert_eq!(load(), UiState::default());
        assert!(!state_path().exists());
    }

    #[test]
    fn valid_roundtrip_preserves_layout() {
        let _guard = unique_env();
        let expected = UiState {
            column_order: vec!["message".into(), "time".into(), "title".into()],
            column_widths: widths(150, 300, 720),
        };
        save(
            expected.column_order.clone(),
            expected.column_widths.clone(),
        )
        .unwrap();
        assert_eq!(load(), expected);
    }

    #[test]
    fn unknown_top_level_field_is_removed_from_disk() {
        let _guard = unique_env();
        let path = state_path();
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "column_order": ["time", "title", "message"],
                "column_widths": {"time": 180, "title": 220, "message": 640},
                "password": "must-not-survive-backup"
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(load(), UiState::default());
        let rewritten = fs::read_to_string(&path).unwrap();
        assert!(!rewritten.contains("password"));
        assert!(!rewritten.contains("must-not-survive-backup"));
        assert_disk_is_safe_default();
    }

    #[test]
    fn oversized_file_is_replaced_with_safe_default() {
        let _guard = unique_env();
        fs::write(state_path(), vec![b'x'; MAX_UI_STATE_BYTES + 1]).unwrap();

        assert_eq!(load(), UiState::default());
        assert_disk_is_safe_default();
    }

    #[test]
    fn unknown_and_duplicate_columns_are_replaced_with_safe_default() {
        let _guard = unique_env();
        for invalid in [
            json!({
                "column_order": ["time", "time", "message"],
                "column_widths": {"time": 180, "title": 220, "message": 640}
            }),
            json!({
                "column_order": ["time", "title", "secret"],
                "column_widths": {"time": 180, "title": 220, "message": 640}
            }),
            json!({
                "column_order": ["time", "title", "message"],
                "column_widths": {"time": 180, "title": 220, "secret": 640}
            }),
        ] {
            fs::write(state_path(), serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert_eq!(load(), UiState::default());
            assert_disk_is_safe_default();
        }
    }

    #[test]
    fn partial_legacy_layout_is_completed_and_rewritten() {
        let _guard = unique_env();
        let path = state_path();
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "column_order": ["message"],
                "column_widths": {"message": 500}
            }))
            .unwrap(),
        )
        .unwrap();
        let expected = UiState {
            column_order: vec!["message".into(), "time".into(), "title".into()],
            column_widths: widths(180, 220, 500),
        };

        assert_eq!(load(), expected);
        assert_eq!(
            serde_json::from_slice::<UiState>(&fs::read(path).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn legacy_widths_are_clamped_and_rewritten() {
        let _guard = unique_env();
        let path = state_path();
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "column_order": ["time", "title", "message"],
                "column_widths": {"time": 1, "title": 9000, "message": 160}
            }))
            .unwrap(),
        )
        .unwrap();
        let expected = UiState {
            column_order: vec!["time".into(), "title".into(), "message".into()],
            column_widths: widths(120, MAX_COLUMN_WIDTH, 160),
        };

        assert_eq!(load(), expected);
        assert_eq!(
            serde_json::from_slice::<UiState>(&fs::read(path).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn duplicate_width_key_is_rejected_and_sanitized() {
        let _guard = unique_env();
        fs::write(
            state_path(),
            br#"{"column_order":["time","title","message"],"column_widths":{"time":180,"time":181,"title":220,"message":640}}"#,
        )
        .unwrap();

        assert_eq!(load(), UiState::default());
        assert_disk_is_safe_default();
    }

    #[test]
    fn rejected_save_preserves_original_file() {
        let _guard = unique_env();
        save(
            vec!["time".into(), "title".into(), "message".into()],
            widths(180, 220, 640),
        )
        .unwrap();
        let original = fs::read(state_path()).unwrap();

        let huge = "x".repeat(MAX_UI_STATE_BYTES + 1);
        assert!(save(
            vec!["time".into(), "title".into(), huge],
            widths(180, 220, 640)
        )
        .is_err());
        let mut unexpected_widths = widths(180, 220, 640);
        unexpected_widths.insert("secret".into(), 200);
        assert!(save(
            vec!["time".into(), "title".into(), "message".into()],
            unexpected_widths
        )
        .is_err());
        assert!(save(
            vec!["time".into(), "title".into(), "message".into()],
            widths(180, 220, MAX_COLUMN_WIDTH + 1)
        )
        .is_err());

        assert_eq!(fs::read(state_path()).unwrap(), original);
    }

    #[test]
    fn load_returns_default_when_invalid_target_cannot_be_rewritten() {
        let _guard = unique_env();
        let path = state_path();
        fs::create_dir(&path).unwrap();

        assert_eq!(load(), UiState::default());
        assert!(path.is_dir());
    }

    #[test]
    fn migrates_legacy_chinese_column_order_and_widths() {
        let _guard = unique_env();
        let path = state_path();
        fs::write(
            &path,
            r#"{"column_order":["内容","标题","时间"],"column_widths":{"内容":300,"标题":200,"时间":150}}"#,
        )
        .unwrap();
        let expected = UiState {
            column_order: vec!["message".into(), "title".into(), "time".into()],
            column_widths: widths(150, 200, 300),
        };

        assert_eq!(load(), expected);
        assert_eq!(
            serde_json::from_slice::<UiState>(&fs::read(path).unwrap()).unwrap(),
            expected
        );
    }
}
