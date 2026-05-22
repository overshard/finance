use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// Open (creating if absent) the SQLite database and run migrations.
/// WAL + synchronous=Normal matches the sibling apps: durable enough for a
/// single-operator service, fast enough for the scheduler's frequent upserts.
pub async fn init(data_dir: &Path) -> anyhow::Result<SqlitePool> {
    let db_path = data_dir.join("db.sqlite3");
    let url = format!("sqlite://{}", db_path.display());

    if !db_path.exists() {
        std::fs::File::create(&db_path)?;
    }

    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Current time as UTC epoch-milliseconds. Every `*_at` column uses this.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Upsert a one-off key-value setting into the `meta` table.
pub async fn set_meta(pool: &SqlitePool, key: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO meta (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read a `meta` setting, if present.
pub async fn get_meta(pool: &SqlitePool, key: &str) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("SELECT value FROM meta WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
}
