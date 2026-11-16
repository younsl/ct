//! Shared metadata-store test harness.

use crate::meta::Store;

/// A fresh temp-backed store. The `TempDir` must outlive the store.
pub async fn test_store() -> (Store, tempfile::TempDir) {
    Store::open_temp().await.expect("open temp store")
}

/// True when the suite runs against PostgreSQL. Tests of SQLite-only
/// machinery (snapshots, change watches, the single write connection) return
/// early on it.
pub fn on_postgres() -> bool {
    std::env::var(crate::meta::TEST_POSTGRES_URL_ENV).is_ok_and(|v| !v.is_empty())
}
