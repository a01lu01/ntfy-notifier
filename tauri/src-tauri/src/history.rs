use crate::subscription::{
    is_valid_message_id, validate_persistable_message, validate_persistable_text, GenerationGuard,
    StoreCommit, SubscriptionKey, SubscriptionMessage, SubscriptionStore, MAX_MESSAGE_BYTES,
    MAX_TITLE_BYTES,
};
use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

static LOCK: Mutex<()> = Mutex::new(());
const MAX_HISTORY: usize = 1000;

pub(crate) struct SqliteSubscriptionStore;

#[derive(Serialize, Clone)]
pub struct HistoryItem {
    pub time: String,
    pub title: String,
    pub message: String,
}

fn db_path() -> PathBuf {
    crate::appdata::resolve().join("history.db")
}

fn open() -> Result<Connection, String> {
    let path = db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(&path).map_err(|e| e.to_string())?;
    // The Activity and the Android :subscriber service use separate processes. The Rust mutex
    // below only serializes callers within one process, so SQLite itself must wait for the other
    // process instead of immediately failing a cursor/message transaction with SQLITE_BUSY.
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            received_at TEXT NOT NULL,
            topic TEXT,
            title TEXT,
            message TEXT
         );
         CREATE TABLE IF NOT EXISTS subscription_cursors (
            server TEXT NOT NULL,
            topic TEXT NOT NULL,
            last_id TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (server, topic)
         );
         CREATE TABLE IF NOT EXISTS history_metadata (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            revision INTEGER NOT NULL CHECK(revision >= 0)
         );
         INSERT OR IGNORE INTO history_metadata (singleton, revision) VALUES (1, 0);",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn increment_history_revision(transaction: &rusqlite::Transaction<'_>) -> Result<(), String> {
    let changed = transaction
        .execute(
            "UPDATE history_metadata
             SET revision = CASE WHEN revision = ?1 THEN 0 ELSE revision + 1 END
             WHERE singleton = 1",
            params![i64::MAX],
        )
        .map_err(|error| error.to_string())?;
    if changed == 1 {
        Ok(())
    } else {
        Err("历史版本元数据缺失".to_string())
    }
}

impl SqliteSubscriptionStore {
    fn commit_current(
        key: &SubscriptionKey,
        message: &SubscriptionMessage,
    ) -> Result<StoreCommit, String> {
        validate_persistable_message(message).map_err(str::to_string)?;
        if !is_valid_message_id(&message.id) {
            // 官方 ntfy message ID 为 12 位字母数字。忽略其他形状，
            // 防止 all/latest/时长等保留词在下次连接中改变 since 语义。
            return Ok(StoreCommit::Duplicate);
        }

        let mut conn = open()?;
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        let now = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO messages (id, received_at, topic, title, message)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    message.id,
                    now,
                    message.topic,
                    message.title,
                    message.message
                ],
            )
            .map_err(|error| error.to_string())?;
        if inserted > 0 {
            transaction
                .execute(
                    "DELETE FROM messages WHERE id NOT IN (
                        SELECT id FROM messages ORDER BY received_at DESC, rowid DESC LIMIT ?1
                     )",
                    params![MAX_HISTORY as i64],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction
            .execute(
                "INSERT INTO subscription_cursors (server, topic, last_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(server, topic) DO UPDATE SET
                    last_id = excluded.last_id,
                    updated_at = excluded.updated_at",
                params![key.server, key.topic, message.id, now],
            )
            .map_err(|error| error.to_string())?;
        if inserted > 0 {
            increment_history_revision(&transaction)?;
        }
        transaction.commit().map_err(|error| error.to_string())?;

        Ok(if inserted > 0 {
            StoreCommit::Inserted
        } else {
            StoreCommit::Duplicate
        })
    }
}

impl SubscriptionStore for SqliteSubscriptionStore {
    fn load_cursor(&self, key: &SubscriptionKey) -> Result<Option<String>, String> {
        let _guard = LOCK.lock().unwrap();
        let conn = open()?;
        let cursor = conn
            .query_row(
                "SELECT last_id FROM subscription_cursors WHERE server = ?1 AND topic = ?2",
                params![key.server, key.topic],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        match cursor {
            Some(last_id) if is_valid_message_id(&last_id) => Ok(Some(last_id)),
            Some(_) => Err("存储的订阅游标格式无效".to_string()),
            None => Ok(None),
        }
    }

    fn commit_message(
        &self,
        key: &SubscriptionKey,
        message: &SubscriptionMessage,
        generation: &GenerationGuard,
    ) -> Result<StoreCommit, String> {
        let _guard = LOCK.lock().unwrap();
        if !generation.is_current() {
            return Ok(StoreCommit::StaleGeneration);
        }
        Self::commit_current(key, message)
    }
}

pub fn get_messages(limit: usize) -> Vec<HistoryItem> {
    let _guard = LOCK.lock().unwrap();
    let conn = match open() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT received_at, title, message FROM messages
         WHERE length(CAST(COALESCE(title, '') AS BLOB)) <= ?2
           AND length(CAST(COALESCE(message, '') AS BLOB)) <= ?3
         ORDER BY received_at DESC, rowid DESC LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(
        params![
            limit as i64,
            MAX_TITLE_BYTES as i64,
            MAX_MESSAGE_BYTES as i64
        ],
        |row| {
            Ok(HistoryItem {
                time: row.get(0)?,
                title: row.get(1).unwrap_or_default(),
                message: row.get(2).unwrap_or_default(),
            })
        },
    );
    match rows {
        Ok(rows) => rows
            .filter_map(|row| row.ok())
            .filter(|item| validate_persistable_text(&item.title, &item.message).is_ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn get_history_revision() -> Result<i64, String> {
    let _guard = LOCK.lock().unwrap();
    let conn = open()?;
    let revision = conn
        .query_row(
            "SELECT revision FROM history_metadata WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    if revision < 0 {
        Err("历史版本元数据无效".to_string())
    } else {
        Ok(revision)
    }
}

pub fn clear_history() -> Result<(), String> {
    let _guard = LOCK.lock().unwrap();
    let mut conn = open()?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    // 清空可见历史不得删除订阅游标，否则重连会重复提醒旧消息。
    transaction
        .execute("DELETE FROM messages", [])
        .map_err(|error| error.to_string())?;
    increment_history_revision(&transaction)?;
    transaction.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::MutexGuard;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    const ID_0: &str = "000000000000";
    const ID_1: &str = "000000000001";
    const ID_2: &str = "000000000002";
    const ID_3: &str = "000000000003";

    fn unique_env() -> MutexGuard<'static, ()> {
        let guard = crate::appdata::test_lock().lock().unwrap();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ntfy-test-history-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::appdata::set(dir);
        guard
    }

    fn key(server: &str, topic: &str) -> SubscriptionKey {
        SubscriptionKey {
            server: server.to_string(),
            topic: topic.to_string(),
        }
    }

    fn message(id: &str) -> SubscriptionMessage {
        SubscriptionMessage {
            id: id.to_string(),
            topic: "test-topic".to_string(),
            title: format!("title-{id}"),
            message: format!("message-{id}"),
        }
    }

    fn commit(
        _store: &SqliteSubscriptionStore,
        key: &SubscriptionKey,
        message: &SubscriptionMessage,
    ) -> Result<StoreCommit, String> {
        let _guard = LOCK.lock().unwrap();
        SqliteSubscriptionStore::commit_current(key, message)
    }

    fn message_count() -> i64 {
        let _guard = LOCK.lock().unwrap();
        open()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap()
    }

    fn revision() -> i64 {
        get_history_revision().unwrap()
    }

    #[test]
    fn database_enables_cross_process_wal_and_busy_timeout() {
        let _guard = unique_env();
        let conn = open().unwrap();

        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        let busy_timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(synchronous, 1);
        assert_eq!(busy_timeout, 5_000);
        assert_eq!(revision(), 0);
    }

    #[test]
    fn legacy_messages_database_adds_empty_cursor_table_without_replay_seed() {
        let _guard = unique_env();
        let conn = rusqlite::Connection::open(db_path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                received_at TEXT NOT NULL,
                topic TEXT,
                title TEXT,
                message TEXT
             );
             INSERT INTO messages (id, received_at, topic, title, message)
             VALUES ('legacy', '2026-01-01T00:00:00', 'test-topic', 'old', 'cached');",
        )
        .unwrap();
        drop(conn);

        let store = SqliteSubscriptionStore;
        assert_eq!(
            store
                .load_cursor(&key("https://example.test", "test-topic"))
                .unwrap(),
            None
        );
        let items = get_messages(10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "old");
        assert_eq!(revision(), 0);
    }

    #[test]
    fn legacy_oversized_or_nul_history_is_not_loaded_into_the_webview() {
        let _guard = unique_env();
        {
            let _db_guard = LOCK.lock().unwrap();
            let conn = open().unwrap();
            conn.execute(
                "INSERT INTO messages (id, received_at, topic, title, message)
                 VALUES ('legacy-large', '2026-01-01T00:00:00', 'test-topic', 'old', ?1)",
                params!["x".repeat(MAX_MESSAGE_BYTES + 1)],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (id, received_at, topic, title, message)
                 VALUES ('legacy-nul', '2026-01-02T00:00:00', 'test-topic', 'bad', ?1)",
                params!["unsafe\0body"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (id, received_at, topic, title, message)
                 VALUES ('legacy-safe', '2026-01-03T00:00:00', 'test-topic', 'safe', 'kept')",
                [],
            )
            .unwrap();
        }

        let items = get_messages(10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "safe");
        assert_eq!(items[0].message, "kept");
    }

    #[test]
    fn message_and_cursor_commit_atomically() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");

        assert_eq!(
            commit(&store, &key, &message(ID_1)).unwrap(),
            StoreCommit::Inserted
        );
        assert_eq!(store.load_cursor(&key).unwrap().as_deref(), Some(ID_1));
        assert_eq!(message_count(), 1);
        assert_eq!(revision(), 1);
    }

    #[test]
    fn cursor_failure_rolls_back_message_and_preserves_previous_cursor() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");
        assert_eq!(
            commit(&store, &key, &message(ID_0)).unwrap(),
            StoreCommit::Inserted
        );

        {
            let _db_guard = LOCK.lock().unwrap();
            open()
                .unwrap()
                .execute_batch(
                    "CREATE TRIGGER reject_cursor_update
                     BEFORE UPDATE ON subscription_cursors
                     BEGIN
                        SELECT RAISE(ABORT, 'cursor update rejected');
                     END;",
                )
                .unwrap();
        }

        assert!(commit(&store, &key, &message(ID_1)).is_err());
        assert_eq!(message_count(), 1);
        assert_eq!(store.load_cursor(&key).unwrap().as_deref(), Some(ID_0));
        assert_eq!(get_messages(10)[0].title, format!("title-{ID_0}"));
        assert_eq!(revision(), 1);
    }

    #[test]
    fn revision_failure_rolls_back_message_and_cursor_together() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");
        assert_eq!(
            commit(&store, &key, &message(ID_0)).unwrap(),
            StoreCommit::Inserted
        );

        {
            let _db_guard = LOCK.lock().unwrap();
            open()
                .unwrap()
                .execute_batch(
                    "CREATE TRIGGER reject_history_revision
                     BEFORE UPDATE ON history_metadata
                     BEGIN
                        SELECT RAISE(ABORT, 'history revision rejected');
                     END;",
                )
                .unwrap();
        }

        assert!(commit(&store, &key, &message(ID_1)).is_err());
        assert_eq!(message_count(), 1);
        assert_eq!(store.load_cursor(&key).unwrap().as_deref(), Some(ID_0));
        assert_eq!(revision(), 1);
    }

    #[test]
    fn duplicate_message_advances_cursor_without_duplicate_history() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let first = key("https://first.example", "test-topic");
        let second = key("https://second.example", "test-topic");
        let message = message(ID_1);

        assert_eq!(
            commit(&store, &first, &message).unwrap(),
            StoreCommit::Inserted
        );
        assert_eq!(
            commit(&store, &second, &message).unwrap(),
            StoreCommit::Duplicate
        );
        assert_eq!(message_count(), 1);
        assert_eq!(store.load_cursor(&second).unwrap().as_deref(), Some(ID_1));
        assert_eq!(revision(), 1);
    }

    #[test]
    fn clearing_history_preserves_subscription_cursor() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");
        assert_eq!(
            commit(&store, &key, &message(ID_1)).unwrap(),
            StoreCommit::Inserted
        );

        clear_history().unwrap();

        assert_eq!(get_messages(10).len(), 0);
        assert_eq!(store.load_cursor(&key).unwrap().as_deref(), Some(ID_1));
        assert_eq!(revision(), 2);
    }

    #[test]
    fn clear_revision_failure_rolls_back_history_and_preserves_cursor() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");
        assert_eq!(
            commit(&store, &key, &message(ID_1)).unwrap(),
            StoreCommit::Inserted
        );

        {
            let _db_guard = LOCK.lock().unwrap();
            open()
                .unwrap()
                .execute_batch(
                    "CREATE TRIGGER reject_clear_revision
                     BEFORE UPDATE ON history_metadata
                     BEGIN
                        SELECT RAISE(ABORT, 'clear revision rejected');
                     END;",
                )
                .unwrap();
        }

        assert!(clear_history().is_err());
        assert_eq!(message_count(), 1);
        assert_eq!(store.load_cursor(&key).unwrap().as_deref(), Some(ID_1));
        assert_eq!(revision(), 1);
    }

    #[test]
    fn cursors_are_isolated_by_server_and_topic() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let server_a = key("https://a.example", "alerts");
        let server_b = key("https://b.example", "alerts");
        let topic_b = key("https://a.example", "backups");

        assert_eq!(
            commit(&store, &server_a, &message(ID_1)).unwrap(),
            StoreCommit::Inserted
        );
        assert_eq!(
            commit(&store, &server_b, &message(ID_2)).unwrap(),
            StoreCommit::Inserted
        );
        assert_eq!(
            commit(&store, &topic_b, &message(ID_3)).unwrap(),
            StoreCommit::Inserted
        );

        assert_eq!(store.load_cursor(&server_a).unwrap().as_deref(), Some(ID_1));
        assert_eq!(store.load_cursor(&server_b).unwrap().as_deref(), Some(ID_2));
        assert_eq!(store.load_cursor(&topic_b).unwrap().as_deref(), Some(ID_3));
    }

    #[test]
    fn empty_message_id_is_ignored_without_cursor_write() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");

        assert_eq!(
            commit(&store, &key, &message("")).unwrap(),
            StoreCommit::Duplicate
        );
        assert_eq!(message_count(), 0);
        assert_eq!(store.load_cursor(&key).unwrap(), None);
    }

    #[test]
    fn oversized_or_nul_persistent_fields_are_rejected_without_cursor_write() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");

        let mut oversized_title = message(ID_1);
        oversized_title.title = "x".repeat(crate::subscription::MAX_TITLE_BYTES + 1);
        assert!(commit(&store, &key, &oversized_title).is_err());

        let mut oversized_body = message(ID_2);
        oversized_body.message = "x".repeat(crate::subscription::MAX_MESSAGE_BYTES + 1);
        assert!(commit(&store, &key, &oversized_body).is_err());

        let mut nul = message(ID_3);
        nul.message = "unsafe\0body".to_string();
        assert!(commit(&store, &key, &nul).is_err());

        assert_eq!(message_count(), 0);
        assert_eq!(store.load_cursor(&key).unwrap(), None);
    }

    #[test]
    fn malformed_stored_cursor_is_reported_as_storage_error() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");
        {
            let _db_guard = LOCK.lock().unwrap();
            open()
                .unwrap()
                .execute(
                    "INSERT INTO subscription_cursors (server, topic, last_id, updated_at)
                     VALUES (?1, ?2, 'all', '2026-01-01T00:00:00')",
                    params![key.server, key.topic],
                )
                .unwrap();
        }

        assert_eq!(
            store.load_cursor(&key).unwrap_err(),
            "存储的订阅游标格式无效"
        );
    }

    #[test]
    fn prune_to_1000() {
        let _guard = unique_env();
        let store = SqliteSubscriptionStore;
        let key = key("https://example.test", "test-topic");
        for i in 0..1005 {
            let id = format!("{i:012}");
            assert_eq!(
                commit(&store, &key, &message(&id)).unwrap(),
                StoreCommit::Inserted
            );
        }
        let items = get_messages(2000);
        assert_eq!(items.len(), 1000);
        assert_eq!(
            store.load_cursor(&key).unwrap().as_deref(),
            Some("000000001004")
        );
        assert_eq!(revision(), 1005);
    }
}
