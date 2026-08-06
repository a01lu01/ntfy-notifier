use chrono::Local;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
const MAX_HISTORY: usize = 1000;
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

#[derive(Serialize, Clone)]
pub struct HistoryItem {
    pub time: String,
    pub title: String,
    pub message: String,
}

fn db_path() -> PathBuf {
    appdata_dir().join("ntfy-notifier").join("history.db")
}

fn open() -> Result<Connection, String> {
    let path = db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(&path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            received_at TEXT NOT NULL,
            topic TEXT,
            title TEXT,
            message TEXT
         );",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// 记录一条消息；返回 Ok(true) 为新记录，Ok(false) 为重复。
pub fn record_message(
    id: &str,
    topic: &str,
    title: &str,
    message: &str,
) -> Result<bool, String> {
    if id.is_empty() {
        return Ok(false);
    }
    let _guard = LOCK.lock().unwrap();
    let conn = open()?;
    let now = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO messages (id, received_at, topic, title, message)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, now, topic, title, message],
        )
        .map_err(|e| e.to_string())?;
    if inserted > 0 {
        conn.execute(
            "DELETE FROM messages WHERE id NOT IN (
                SELECT id FROM messages ORDER BY received_at DESC, rowid DESC LIMIT ?1
             )",
            params![MAX_HISTORY as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(inserted > 0)
}

pub fn get_messages(limit: usize) -> Vec<HistoryItem> {
    let _guard = LOCK.lock().unwrap();
    let conn = match open() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT received_at, title, message FROM messages
         ORDER BY received_at DESC, rowid DESC LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(HistoryItem {
            time: row.get(0)?,
            title: row.get(1).unwrap_or_default(),
            message: row.get(2).unwrap_or_default(),
        })
    });
    match rows {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn clear_history() -> Result<(), String> {
    let _guard = LOCK.lock().unwrap();
    let conn = open()?;
    conn.execute("DELETE FROM messages", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::MutexGuard;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn unique_env() -> MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ntfy-test-history-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_test_dir(dir);
        guard
    }

    #[test]
    fn record_and_dedup() {
        let _guard = unique_env();
        assert!(record_message("1", "test-topic", "t", "m").unwrap());
        assert!(!record_message("1", "test-topic", "t", "m").unwrap());
        let items = get_messages(10);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn prune_to_1000() {
        let _guard = unique_env();
        clear_history().unwrap();
        for i in 0..1005 {
            record_message(&i.to_string(), "test-topic", &format!("t{i}"), "m").unwrap();
        }
        let items = get_messages(2000);
        assert!(items.len() <= 1000);
    }
}
