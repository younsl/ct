//! A connection facade over SQLite and PostgreSQL.
//!
//! The metadata store is written once, against the small rusqlite-shaped API
//! here (`execute`, `query_row`, `prepare`, `transaction`), and runs on either
//! backend. The SQL stays in the subset both databases accept, and
//! [`Connection::is_postgres`] gates the few statements that cannot.

mod error;
mod params;
pub mod postgres;
mod value;

pub use error::{Error, OptionalExtension, Result};
pub use params::{Params, ParamsFromIter, params_from_iter};
pub use postgres::{PgConfig, ServerInfo};
pub use value::{FromSql, ToSql, Value};

/// Mirrors rusqlite's module path for the value type.
pub mod types {
    pub use crate::value::Value;
}

use postgres::{PgConnection, PgValue};

/// Serializes every default transaction on PostgreSQL, see
/// [`Connection::transaction`].
const GLOBAL_LOCK_KEY: i64 = 0x666f_726b_6c69_6674;

/// How a SQLite transaction takes its lock. PostgreSQL ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionBehavior {
    Deferred,
    Immediate,
}

/// What a PostgreSQL transaction locks before its first statement. SQLite has
/// one writer and ignores it.
#[derive(Debug, Clone, Copy)]
enum TxLock<'a> {
    Global,
    Keyed(&'a str),
    None,
}

/// A database session on either backend.
pub enum Connection {
    Sqlite(rusqlite::Connection),
    Postgres(Box<PgConnection>),
}

impl rusqlite::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, ValueRef};
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Null => ValueRef::Null,
            Self::Integer(i) => ValueRef::Integer(*i),
            Self::Real(f) => ValueRef::Real(*f),
            Self::Text(s) => ValueRef::Text(s.as_bytes()),
            Self::Blob(b) => ValueRef::Blob(b),
        }))
    }
}

fn sqlite_value(v: rusqlite::types::ValueRef<'_>) -> Result<Value> {
    use rusqlite::types::ValueRef;
    Ok(match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Integer(i),
        ValueRef::Real(f) => Value::Real(f),
        ValueRef::Text(t) => Value::Text(
            std::str::from_utf8(t)
                .map_err(|e| Error::Conversion(e.to_string()))?
                .to_owned(),
        ),
        ValueRef::Blob(b) => Value::Blob(b.to_vec()),
    })
}

/// FNV-1a, so one key maps to the same advisory lock on every pod.
fn lock_key(key: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    i64::from_ne_bytes(h.to_ne_bytes())
}

impl Connection {
    /// Opens a PostgreSQL session. Must run on a thread that may block.
    pub fn connect_postgres(cfg: &PgConfig) -> Result<Self> {
        Ok(Self::Postgres(Box::new(PgConnection::connect(cfg)?)))
    }

    pub const fn is_postgres(&self) -> bool {
        matches!(self, Self::Postgres(_))
    }

    /// True when the session can no longer run statements and must be
    /// replaced. A SQLite connection never closes by itself.
    pub fn is_closed(&self) -> bool {
        match self {
            Self::Sqlite(_) => false,
            Self::Postgres(pg) => pg.is_closed(),
        }
    }

    /// The SQLite connection, for the operations only SQLite has (snapshots,
    /// WAL shipping, pragmas).
    pub const fn as_sqlite(&self) -> Result<&rusqlite::Connection> {
        match self {
            Self::Sqlite(c) => Ok(c),
            Self::Postgres(_) => Err(Error::Unsupported("a SQLite-only operation")),
        }
    }

    /// Mutable form of [`Connection::as_sqlite`].
    pub const fn as_sqlite_mut(&mut self) -> Result<&mut rusqlite::Connection> {
        match self {
            Self::Sqlite(c) => Ok(c),
            Self::Postgres(_) => Err(Error::Unsupported("a SQLite-only operation")),
        }
    }

    /// Runs a statement and returns the number of rows it changed.
    pub fn execute<P: Params>(&self, sql: &str, params: P) -> Result<usize> {
        let values = params.into_values();
        match self {
            Self::Sqlite(c) => Ok(c.execute(sql, rusqlite::params_from_iter(values.iter()))?),
            Self::Postgres(pg) => {
                Ok(usize::try_from(pg.execute(sql, &values)?).unwrap_or(usize::MAX))
            }
        }
    }

    /// Runs one or more statements that take no parameters. The text is sent
    /// as written, so it must already suit the backend.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        match self {
            Self::Sqlite(c) => Ok(c.execute_batch(sql)?),
            Self::Postgres(pg) => pg.batch_execute(sql),
        }
    }

    /// Feeds each result row to `f` until it returns `false`.
    fn query_each<F>(&self, sql: &str, values: &[Value], mut f: F) -> Result<()>
    where
        F: FnMut(&Row<'_>) -> bool,
    {
        match self {
            Self::Sqlite(c) => {
                let mut stmt = c.prepare(sql)?;
                let mut rows = stmt.query(rusqlite::params_from_iter(values.iter()))?;
                while let Some(row) = rows.next()? {
                    if !f(&Row(RowInner::Sqlite(row))) {
                        break;
                    }
                }
            }
            Self::Postgres(pg) => {
                for row in &pg.query(sql, values)? {
                    if !f(&Row(RowInner::Postgres(row))) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Runs a query expected to return one row and maps it with `f`. Fails
    /// with [`Error::QueryReturnedNoRows`] when nothing matches.
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> Result<T>
    where
        P: Params,
        F: FnOnce(&Row<'_>) -> Result<T>,
    {
        let mut f = Some(f);
        let mut out = None;
        self.query_each(sql, &params.into_values(), |row| {
            if let Some(f) = f.take() {
                out = Some(f(row));
            }
            false
        })?;
        out.unwrap_or(Err(Error::QueryReturnedNoRows))
    }

    /// Holds a statement for repeated or multi-row use.
    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>> {
        if let Self::Sqlite(c) = self {
            // Surface syntax errors here, where rusqlite reports them.
            c.prepare(sql)?;
        }
        Ok(Statement {
            conn: self,
            sql: sql.to_owned(),
        })
    }

    fn begin(&self, behavior: TransactionBehavior, lock: TxLock<'_>) -> Result<Transaction<'_>> {
        match self {
            Self::Sqlite(c) => c.execute_batch(match behavior {
                TransactionBehavior::Deferred => "BEGIN DEFERRED",
                TransactionBehavior::Immediate => "BEGIN IMMEDIATE",
            })?,
            Self::Postgres(pg) => pg.batch_execute("BEGIN")?,
        }
        let tx = Transaction {
            conn: self,
            open: true,
        };
        if tx.is_postgres() {
            match lock {
                TxLock::Global => {
                    tx.execute("SELECT pg_advisory_xact_lock(?)", params![GLOBAL_LOCK_KEY])?;
                }
                TxLock::Keyed(key) => {
                    tx.execute(
                        "SELECT pg_advisory_xact_lock_shared(?)",
                        params![GLOBAL_LOCK_KEY],
                    )?;
                    tx.execute("SELECT pg_advisory_xact_lock(?)", params![lock_key(key)])?;
                }
                TxLock::None => {}
            }
        }
        Ok(tx)
    }

    /// Starts a transaction that rolls back unless committed.
    ///
    /// SQLite has one writer, so the store's multi-statement writes never
    /// interleave there. On PostgreSQL this takes a database-wide advisory
    /// lock for the transaction, which keeps that guarantee across pods.
    pub fn transaction(&mut self) -> Result<Transaction<'_>> {
        self.begin(TransactionBehavior::Deferred, TxLock::Global)
    }

    /// [`Connection::transaction`] with an explicit SQLite locking behavior.
    pub fn transaction_with_behavior(
        &mut self,
        behavior: TransactionBehavior,
    ) -> Result<Transaction<'_>> {
        self.begin(behavior, TxLock::Global)
    }

    /// Starts a transaction that on PostgreSQL excludes only transactions
    /// holding the same `key` and every [`Connection::transaction`]. Use it on
    /// hot paths whose writes are confined to rows identified by the key.
    pub fn transaction_keyed(&mut self, key: &str) -> Result<Transaction<'_>> {
        self.begin(TransactionBehavior::Deferred, TxLock::Keyed(key))
    }

    /// Starts a transaction that takes no advisory lock on PostgreSQL and
    /// relies on row locks alone.
    pub fn transaction_unlocked(&mut self) -> Result<Transaction<'_>> {
        self.begin(TransactionBehavior::Deferred, TxLock::None)
    }
}

/// A statement bound to a connection.
pub struct Statement<'c> {
    conn: &'c Connection,
    sql: String,
}

/// The rows of a finished query, each already mapped.
pub type MappedRows<T> = std::vec::IntoIter<Result<T>>;

impl Statement<'_> {
    /// Runs the query and maps every row with `f`.
    pub fn query_map<T, P, F>(&mut self, params: P, mut f: F) -> Result<MappedRows<T>>
    where
        P: Params,
        F: FnMut(&Row<'_>) -> Result<T>,
    {
        let mut out = Vec::new();
        self.conn
            .query_each(&self.sql, &params.into_values(), |row| {
                out.push(f(row));
                true
            })?;
        Ok(out.into_iter())
    }

    /// Runs the query and maps its first row with `f`.
    pub fn query_row<T, P, F>(&mut self, params: P, f: F) -> Result<T>
    where
        P: Params,
        F: FnOnce(&Row<'_>) -> Result<T>,
    {
        self.conn.query_row(&self.sql, params, f)
    }

    /// True when the query returns at least one row.
    pub fn exists<P: Params>(&mut self, params: P) -> Result<bool> {
        let mut found = false;
        self.conn
            .query_each(&self.sql, &params.into_values(), |_| {
                found = true;
                false
            })?;
        Ok(found)
    }

    /// Runs the statement and returns the number of rows it changed.
    pub fn execute<P: Params>(&mut self, params: P) -> Result<usize> {
        self.conn.execute(&self.sql, params)
    }
}

/// An open transaction. Dereferences to its [`Connection`], and rolls back on
/// drop unless [`Transaction::commit`] ran.
pub struct Transaction<'c> {
    conn: &'c Connection,
    open: bool,
}

impl Transaction<'_> {
    pub fn commit(mut self) -> Result<()> {
        self.open = false;
        self.conn.execute_batch("COMMIT")
    }

    pub fn rollback(mut self) -> Result<()> {
        self.open = false;
        self.conn.execute_batch("ROLLBACK")
    }
}

impl std::ops::Deref for Transaction<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.open {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

enum RowInner<'a> {
    Sqlite(&'a rusqlite::Row<'a>),
    Postgres(&'a tokio_postgres::Row),
}

/// One result row.
pub struct Row<'a>(RowInner<'a>);

/// A column position. Only zero-based indexes are supported.
pub trait RowIndex {
    fn index(&self) -> usize;
}

impl RowIndex for usize {
    fn index(&self) -> usize {
        *self
    }
}

impl Row<'_> {
    /// Reads column `idx` as `T`.
    pub fn get<I: RowIndex, T: FromSql>(&self, idx: I) -> Result<T> {
        let idx = idx.index();
        let value = match &self.0 {
            RowInner::Sqlite(row) => sqlite_value(row.get_ref(idx)?)?,
            RowInner::Postgres(row) => row.try_get::<_, PgValue>(idx)?.0,
        };
        T::from_value(value).map_err(|e| match e {
            Error::Conversion(msg) => Error::Conversion(format!("column {idx}: {msg}")),
            other => other,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite() -> Connection {
        let conn = Connection::Sqlite(rusqlite::Connection::open_in_memory().expect("open"));
        conn.execute_batch(
            "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, n INTEGER)",
        )
        .expect("schema");
        conn
    }

    #[test]
    fn execute_and_query_row_round_trip() {
        let conn = sqlite();
        let n = conn
            .execute("INSERT INTO t(name, n) VALUES(?, ?)", params!["a", 3_i64])
            .expect("insert");
        assert_eq!(n, 1);
        let (name, n): (String, Option<i64>) = conn
            .query_row("SELECT name, n FROM t WHERE name = ?", ["a"], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("select");
        assert_eq!((name.as_str(), n), ("a", Some(3)));
    }

    #[test]
    fn missing_row_is_optional() {
        let conn = sqlite();
        let got = conn
            .query_row("SELECT n FROM t WHERE name = ?", ["zz"], |r| {
                r.get::<_, i64>(0)
            })
            .optional()
            .expect("optional");
        assert_eq!(got, None);
    }

    #[test]
    fn unique_violation_is_classified() {
        let conn = sqlite();
        conn.execute("INSERT INTO t(name) VALUES(?)", ["a"])
            .expect("first");
        let err = conn
            .execute("INSERT INTO t(name) VALUES(?)", ["a"])
            .expect_err("duplicate");
        assert!(err.is_unique_violation(), "{err}");
    }

    #[test]
    fn query_map_and_exists() {
        let conn = sqlite();
        for name in ["a", "b"] {
            conn.execute("INSERT INTO t(name) VALUES(?)", [name])
                .expect("insert");
        }
        let mut stmt = conn
            .prepare("SELECT name FROM t ORDER BY name")
            .expect("prepare");
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_>>()
            .expect("rows");
        assert_eq!(names, ["a", "b"]);
        let mut stmt = conn
            .prepare("SELECT 1 FROM t WHERE name = ?")
            .expect("prepare");
        assert!(stmt.exists(["a"]).expect("exists"));
        assert!(!stmt.exists(["zz"]).expect("exists"));
        assert!(conn.prepare("SELEC nope").is_err());
    }

    #[test]
    fn transaction_rolls_back_unless_committed() {
        let mut conn = sqlite();
        {
            let tx = conn.transaction().expect("begin");
            tx.execute("INSERT INTO t(name) VALUES(?)", ["a"])
                .expect("insert");
        }
        {
            let tx = conn.transaction_keyed("k").expect("begin");
            tx.execute("INSERT INTO t(name) VALUES(?)", ["b"])
                .expect("insert");
            tx.commit().expect("commit");
        }
        let names: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .expect("count");
        assert_eq!(names, 1);
    }

    #[test]
    fn sqlite_only_access_and_lock_keys() {
        let mut conn = sqlite();
        assert!(!conn.is_postgres());
        assert!(!conn.is_closed());
        assert!(conn.as_sqlite().is_ok());
        assert!(conn.as_sqlite_mut().is_ok());
        assert_eq!(lock_key("a/b"), lock_key("a/b"));
        assert_ne!(lock_key("a/b"), lock_key("a/c"));
    }

    /// The PostgreSQL arm, against the server named by
    /// `FORKLIFT_TEST_POSTGRES_URL`. Skipped when it is unset, so the suite
    /// needs no database by default.
    mod postgres {
        use super::super::*;

        fn connect() -> Option<Connection> {
            let url = std::env::var("FORKLIFT_TEST_POSTGRES_URL").ok()?;
            if url.is_empty() {
                return None;
            }
            let cfg = PgConfig::parse(&url, &[]).expect("config");
            let conn = Connection::connect_postgres(&cfg).expect("connect");
            // A session-local table keeps parallel tests apart.
            conn.execute_batch(
                "CREATE TEMP TABLE t(
                    id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    n BIGINT,
                    flag BIGINT NOT NULL DEFAULT 0)",
            )
            .expect("schema");
            Some(conn)
        }

        /// Runs `f` on a blocking thread inside a runtime, the way the store
        /// drives a connection.
        fn on_blocking_thread(f: impl FnOnce() + Send + 'static) {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(async { tokio::task::spawn_blocking(f).await.expect("join") });
        }

        #[test]
        fn statements_written_for_sqlite_run_unchanged() {
            on_blocking_thread(|| {
                let Some(conn) = connect() else { return };
                let id: i64 = conn
                    .query_row(
                        "INSERT INTO t(name, n, flag) VALUES(?, ?, ?) RETURNING id",
                        params!["Alpha", 3_i64, true],
                        |r| r.get(0),
                    )
                    .expect("insert");
                // Numbered placeholders, a repeated one, and a boolean read back
                // from an integer column.
                let (name, n, flag): (String, Option<i64>, bool) = conn
                    .query_row(
                        "SELECT name, n, flag FROM t WHERE id = ?1 AND (n = ?2 OR ?2 IS NULL)",
                        params![id, 3_i64],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .expect("select");
                assert_eq!((name.as_str(), n, flag), ("Alpha", Some(3), true));

                // LIKE is case-insensitive, as it is in SQLite.
                let mut stmt = conn
                    .prepare("SELECT 1 FROM t WHERE name LIKE ?")
                    .expect("prepare");
                assert!(stmt.exists(["alp%"]).expect("exists"));

                // SUM over BIGINT is NUMERIC on the wire, COUNT is BIGINT, and a
                // comparison is BOOL.
                let (sum, count, positive): (i64, i64, bool) = conn
                    .query_row("SELECT SUM(n), COUNT(*), SUM(n) > 0 FROM t", [], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })
                    .expect("aggregate");
                assert_eq!((sum, count, positive), (3, 1, true));
            });
        }

        #[test]
        fn errors_are_classified_like_sqlite() {
            on_blocking_thread(|| {
                let Some(conn) = connect() else { return };
                conn.execute("INSERT INTO t(name) VALUES(?)", ["a"])
                    .expect("first");
                let err = conn
                    .execute("INSERT INTO t(name) VALUES(?)", ["a"])
                    .expect_err("duplicate");
                assert!(err.is_unique_violation(), "{err}");
                let missing = conn
                    .query_row("SELECT n FROM t WHERE name = ?", ["zz"], |r| {
                        r.get::<_, i64>(0)
                    })
                    .optional()
                    .expect("optional");
                assert_eq!(missing, None);
                assert!(conn.as_sqlite().is_err());
                assert!(conn.is_postgres());
                assert!(!conn.is_closed());
            });
        }

        #[test]
        fn transactions_commit_roll_back_and_take_their_locks() {
            on_blocking_thread(|| {
                let Some(mut conn) = connect() else { return };
                {
                    let tx = conn.transaction().expect("begin");
                    tx.execute("INSERT INTO t(name) VALUES(?)", ["dropped"])
                        .expect("insert");
                }
                {
                    let tx = conn.transaction_keyed("repo/path").expect("begin keyed");
                    tx.execute("INSERT INTO t(name) VALUES(?)", ["kept"])
                        .expect("insert");
                    tx.commit().expect("commit");
                }
                {
                    let tx = conn.transaction_unlocked().expect("begin unlocked");
                    tx.execute("INSERT INTO t(name) VALUES(?)", ["undone"])
                        .expect("insert");
                    tx.rollback().expect("rollback");
                }
                let mut stmt = conn
                    .prepare("SELECT name FROM t ORDER BY name")
                    .expect("prepare");
                let names: Vec<String> = stmt
                    .query_map([], |r| r.get(0))
                    .expect("query")
                    .collect::<Result<_>>()
                    .expect("rows");
                assert_eq!(names, ["kept"]);
            });
        }

        #[test]
        fn search_path_selects_the_schema() {
            on_blocking_thread(|| {
                let Ok(url) = std::env::var("FORKLIFT_TEST_POSTGRES_URL") else {
                    return;
                };
                if url.is_empty() {
                    return;
                }
                assert!(
                    PgConfig::parse(&url, &[])
                        .expect("config")
                        .with_search_path("no;pe")
                        .is_err()
                );
                let admin =
                    Connection::connect_postgres(&PgConfig::parse(&url, &[]).expect("config"))
                        .expect("connect");
                admin
                    .execute_batch("CREATE SCHEMA IF NOT EXISTS metadb_search_path_test")
                    .expect("schema");
                let cfg = PgConfig::parse(&url, &[])
                    .expect("config")
                    .with_search_path("metadb_search_path_test")
                    .expect("search path");
                let conn = Connection::connect_postgres(&cfg).expect("connect");
                let schema: String = conn
                    .query_row("SELECT current_schema()", [], |r| r.get(0))
                    .expect("current schema");
                assert_eq!(schema, "metadb_search_path_test");
            });
        }
    }
}
