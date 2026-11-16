//! Moves the metadata of a SQLite install into PostgreSQL.
//!
//! `forklift migrate-database` reads a SQLite metadata database, either a
//! local file or the copy the s3 backend keeps in the bucket, and writes
//! every row into the PostgreSQL database named by `FORKLIFT_DATABASE_*`. The
//! blob store is not touched: blobs are addressed by digest and both layouts
//! read the same ones.
//!
//! The copy runs in one PostgreSQL transaction, so the target ends up either
//! complete or untouched. The source is opened read-only and stays valid,
//! which is what makes going back possible.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use forklift_metadb::{Connection, Value};
use serde::Serialize;

use super::lease::{LeaseHold, in_cluster_client, wait_for_no_writers};
use crate::{config, meta, objstore, outbound, storage};

pub const USAGE: &str = "Usage of forklift migrate-database:
  -from-sqlite path
        SQLite metadata file to read. Without it the metadata is rebuilt from
        the bucket of the s3 backend (FORKLIFT_STORAGE_BACKEND=s3)
  -staging path
        directory for the rebuilt SQLite file, needs about twice its size
  -dry-run
        check the source and the target and report, copy nothing
  -overwrite
        empty the target tables first instead of refusing a non-empty target
  -skip-orphans
        leave out rows that reference a missing parent row instead of failing
  -keep-source
        keep the SQLite file rebuilt from the bucket instead of deleting it
  -lease-name name
        hold this HA Lease during the copy so no forklift replica can lead
  -writer-selector selector
        wait until no pod with these labels is left before copying
  -wait duration
        how long to wait for the writers to stop and for the Lease (default 3m)

The target comes from FORKLIFT_DATABASE_HOST, _PORT, _NAME, _USER, _PASSWORD,
_SSLMODE, _CA_FILE and _SCHEMA, or FORKLIFT_DATABASE_URL. The bucket comes from
FORKLIFT_STORAGE_S3_*, the same settings the server reads.";

/// Tables whose rows only mean something next to files on the pod that wrote
/// them, so they are not carried over.
const SKIPPED_TABLES: [&str; 1] = ["oci_upload_sessions"];

/// Rows per `INSERT`. PostgreSQL takes at most 65,535 parameters a statement.
const MAX_BATCH_ROWS: usize = 500;
const MAX_BATCH_PARAMS: usize = 30_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub from_sqlite: Option<PathBuf>,
    pub staging: PathBuf,
    pub dry_run: bool,
    pub overwrite: bool,
    pub skip_orphans: bool,
    pub keep_source: bool,
    pub lease_name: String,
    pub writer_selector: String,
    pub wait: Duration,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            from_sqlite: None,
            staging: std::env::temp_dir(),
            dry_run: false,
            overwrite: false,
            skip_orphans: false,
            keep_source: false,
            lease_name: String::new(),
            writer_selector: String::new(),
            wait: Duration::from_secs(180),
        }
    }
}

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let flag = arg.trim_start_matches('-');
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (flag, None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| it.next().cloned())
                .ok_or_else(|| format!("flag needs an argument: -{name}"))
        };
        match name {
            "from-sqlite" => out.from_sqlite = Some(PathBuf::from(value()?)),
            "staging" => out.staging = PathBuf::from(value()?),
            "dry-run" => out.dry_run = true,
            "overwrite" => out.overwrite = true,
            "skip-orphans" => out.skip_orphans = true,
            "keep-source" => out.keep_source = true,
            "lease-name" => out.lease_name = value()?,
            "writer-selector" => out.writer_selector = value()?,
            "wait" => {
                let v = value()?;
                out.wait = humantime::parse_duration(&v)
                    .map_err(|e| format!("invalid value {v:?} for flag -wait: {e}"))?;
            }
            "help" | "h" => return Err(String::new()),
            _ => return Err(format!("flag provided but not defined: {arg}")),
        }
    }
    if out.lease_name.is_empty() != out.writer_selector.is_empty() {
        return Err("-lease-name and -writer-selector are used together".into());
    }
    Ok(out)
}

/// What one table contributed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TableReport {
    pub name: String,
    pub source_rows: i64,
    pub copied_rows: i64,
    /// Rows left out because they reference a missing parent row.
    pub orphan_rows: i64,
    /// The table is not carried over at all.
    pub skipped: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub source: String,
    pub source_bytes: u64,
    pub dry_run: bool,
    pub tables: Vec<TableReport>,
    pub rows_copied: i64,
    /// Text values that held a NUL byte or invalid UTF-8, which PostgreSQL
    /// does not store, and were cleaned on the way.
    pub sanitized_values: i64,
    pub duration_ms: u64,
}

/// One foreign key of the target schema.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ForeignKey {
    child: String,
    child_cols: Vec<String>,
    parent: String,
    parent_cols: Vec<String>,
}

/// The shape of one target table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Table {
    name: String,
    columns: Vec<String>,
    identity: Option<String>,
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn db(op: &'static str) -> impl Fn(forklift_metadb::Error) -> String {
    move |e| format!("{op}: {e}")
}

/// Reads the target's tables, columns and foreign keys from its catalog, so
/// the copy follows whatever schema the migrations created.
fn target_schema(conn: &Connection) -> Result<(Vec<Table>, Vec<ForeignKey>), String> {
    let names: Vec<String> = conn
        .prepare(
            "SELECT table_name FROM information_schema.tables
              WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'
                AND table_name <> 'schema_migrations'
              ORDER BY table_name",
        )
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(db("list target tables"))?;
    let mut tables = Vec::with_capacity(names.len());
    for name in names {
        let cols: Vec<(String, String)> = conn
            .prepare(
                "SELECT column_name, is_identity FROM information_schema.columns
                  WHERE table_schema = current_schema() AND table_name = ?
                  ORDER BY ordinal_position",
            )
            .and_then(|mut s| {
                s.query_map([name.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .map_err(db("list target columns"))?;
        let identity = cols
            .iter()
            .find(|(_, is_identity)| is_identity == "YES")
            .map(|(c, _)| c.clone());
        tables.push(Table {
            name,
            columns: cols.into_iter().map(|(c, _)| c).collect(),
            identity,
        });
    }
    let split = |s: String| s.split(',').map(str::to_owned).collect::<Vec<_>>();
    let fks: Vec<ForeignKey> = conn
        .prepare(
            "SELECT child.relname, parent.relname,
                    (SELECT string_agg(a.attname, ',' ORDER BY k.ord)
                       FROM unnest(c.conkey) WITH ORDINALITY AS k(attnum, ord)
                       JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum),
                    (SELECT string_agg(a.attname, ',' ORDER BY k.ord)
                       FROM unnest(c.confkey) WITH ORDINALITY AS k(attnum, ord)
                       JOIN pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.attnum)
               FROM pg_constraint c
               JOIN pg_class child ON child.oid = c.conrelid
               JOIN pg_class parent ON parent.oid = c.confrelid
               JOIN pg_namespace n ON n.oid = c.connamespace
              WHERE c.contype = 'f' AND n.nspname = current_schema()
              ORDER BY 1, 2, 3",
        )
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<forklift_metadb::Result<Vec<_>>>()
        })
        .map_err(db("list target foreign keys"))?
        .into_iter()
        .map(|(child, parent, child_cols, parent_cols)| ForeignKey {
            child,
            child_cols: split(child_cols),
            parent,
            parent_cols: split(parent_cols),
        })
        .collect();
    Ok((tables, fks))
}

/// Orders tables so every parent comes before the tables that reference it.
fn parents_first(tables: Vec<Table>, fks: &[ForeignKey]) -> Result<Vec<Table>, String> {
    let mut left: BTreeMap<String, Table> =
        tables.into_iter().map(|t| (t.name.clone(), t)).collect();
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::with_capacity(left.len());
    while !left.is_empty() {
        let ready: Vec<String> = left
            .keys()
            .filter(|name| {
                fks.iter()
                    .filter(|fk| &fk.child == *name && fk.parent != fk.child)
                    .all(|fk| done.contains(&fk.parent) || !left.contains_key(&fk.parent))
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            return Err(format!(
                "foreign keys form a cycle among: {}",
                left.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        for name in ready {
            if let Some(t) = left.remove(&name) {
                done.insert(name);
                out.push(t);
            }
        }
    }
    Ok(out)
}

/// The `WHERE` condition that is true for rows of `fk.child` whose parent row
/// exists (or that reference none).
fn has_parent(fk: &ForeignKey) -> String {
    let nulls = fk
        .child_cols
        .iter()
        .map(|c| format!("c.{} IS NULL", quote(c)))
        .collect::<Vec<_>>()
        .join(" OR ");
    let joins = fk
        .child_cols
        .iter()
        .zip(&fk.parent_cols)
        .map(|(c, p)| format!("p.{} = c.{}", quote(p), quote(c)))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!(
        "({nulls} OR EXISTS (SELECT 1 FROM {} p WHERE {joins}))",
        quote(&fk.parent)
    )
}

fn sqlite(op: &'static str) -> impl Fn(rusqlite::Error) -> String {
    move |e| format!("{op}: {e}")
}

fn source_columns(src: &rusqlite::Connection, table: &str) -> Result<BTreeSet<String>, String> {
    let mut stmt = src
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .map_err(sqlite("read source columns"))?;
    stmt.query_map([table], |r| r.get::<_, String>(0))
        .and_then(Iterator::collect)
        .map_err(sqlite("read source columns"))
}

fn count(src: &rusqlite::Connection, sql: &str) -> Result<i64, String> {
    src.query_row(sql, [], |r| r.get(0))
        .map_err(|e| format!("count source rows: {e}"))
}

/// Turns a SQLite value into one PostgreSQL stores. Text may hold NUL bytes
/// or invalid UTF-8 in SQLite, and a text column may hold a blob. PostgreSQL
/// accepts none of those, so they are cleaned and counted.
fn to_value(v: rusqlite::types::ValueRef<'_>, sanitized: &mut i64) -> Value {
    use rusqlite::types::ValueRef;
    let text = |bytes: &[u8], sanitized: &mut i64| {
        let s = String::from_utf8_lossy(bytes);
        if matches!(s, std::borrow::Cow::Owned(_)) || s.contains('\0') {
            *sanitized += 1;
        }
        Value::Text(s.replace('\0', ""))
    };
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Integer(i),
        ValueRef::Real(f) => Value::Real(f),
        ValueRef::Text(t) => text(t, sanitized),
        ValueRef::Blob(b) => text(b, sanitized),
    }
}

/// What [`copy`] needs to know beyond the two databases.
#[derive(Debug, Clone, Copy, Default)]
pub struct CopyOptions {
    pub dry_run: bool,
    pub overwrite: bool,
    pub skip_orphans: bool,
}

/// Copies every table of the SQLite file at `source` into `target` and
/// reports per table. Nothing is committed when any step fails.
pub async fn copy(
    source: &Path,
    target: &meta::Store,
    opts: CopyOptions,
) -> Result<Report, String> {
    if !target.is_postgres() {
        return Err("the target store is not PostgreSQL".into());
    }
    let started = Instant::now();
    let path = source.to_path_buf();
    let mut report = target
        .write(move |conn| copy_blocking(&path, conn, opts).map_err(meta::Error::Other))
        .await
        .map_err(|e| e.to_string())?;
    if !opts.dry_run {
        // Statistics for the planner, which a bulk load leaves empty. Only
        // the tables just loaded: a bare ANALYZE walks the whole database.
        let analyze = format!(
            "ANALYZE {}",
            report
                .tables
                .iter()
                .map(|t| quote(&t.name))
                .collect::<Vec<_>>()
                .join(", ")
        );
        target
            .write(move |conn| {
                conn.execute_batch(&analyze)
                    .map_err(|e| meta::Error::sqlite("analyze", e))
            })
            .await
            .map_err(|e| e.to_string())?;
    }
    report.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(report)
}

fn copy_blocking(
    source: &Path,
    conn: &mut Connection,
    opts: CopyOptions,
) -> Result<Report, String> {
    let src = rusqlite::Connection::open_with_flags(
        source,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(sqlite("open source"))?;
    let have = count(&src, "SELECT COUNT(*) FROM schema_migrations")?;
    let want = meta::sqlite_schema_version();
    if have != want {
        return Err(format!(
            "the source is at SQLite schema version {have} and this build expects {want}: \
             start this forklift version on the SQLite database once, then migrate"
        ));
    }

    let (tables, fks) = target_schema(conn)?;
    let tables = parents_first(tables, &fks)?;
    let mut report = Report {
        source: source.display().to_string(),
        source_bytes: std::fs::metadata(source).map_or(0, |m| m.len()),
        dry_run: opts.dry_run,
        ..Report::default()
    };

    // Preflight on the source: every column the target has, and no row
    // pointing at a parent that is not there.
    let mut filters: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut orphan_notes = Vec::new();
    for t in &tables {
        let skipped = SKIPPED_TABLES.contains(&t.name.as_str());
        let have = source_columns(&src, &t.name)?;
        if have.is_empty() {
            return Err(format!("the source has no table {}", t.name));
        }
        if let Some(missing) = t.columns.iter().find(|c| !have.contains(*c)) {
            return Err(format!(
                "the source table {} has no column {missing}",
                t.name
            ));
        }
        let source_rows = count(&src, &format!("SELECT COUNT(*) FROM {}", quote(&t.name)))?;
        let mut orphan_rows = 0;
        if !skipped {
            for fk in fks.iter().filter(|fk| fk.child == t.name) {
                if SKIPPED_TABLES.contains(&fk.parent.as_str()) {
                    continue;
                }
                let cond = has_parent(fk);
                let n = count(
                    &src,
                    &format!("SELECT COUNT(*) FROM {} c WHERE NOT {cond}", quote(&t.name)),
                )?;
                if n > 0 {
                    orphan_rows += n;
                    orphan_notes.push(format!(
                        "{n} row(s) of {} reference a missing {} row",
                        t.name, fk.parent
                    ));
                    filters.entry(t.name.clone()).or_default().push(cond);
                }
            }
        }
        report.tables.push(TableReport {
            name: t.name.clone(),
            source_rows,
            orphan_rows,
            skipped,
            ..TableReport::default()
        });
    }
    if !orphan_notes.is_empty() && !opts.skip_orphans {
        return Err(format!(
            "the source has rows without their parent row, which PostgreSQL rejects: {}. \
             Pass -skip-orphans to leave them out",
            orphan_notes.join("; ")
        ));
    }

    // The target must be empty, apart from the row the schema seeds.
    let mut occupied = Vec::new();
    for t in &tables {
        let n: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {}", quote(&t.name)),
                [],
                |r| r.get(0),
            )
            .map_err(db("count target rows"))?;
        if n > 0 && t.name != "artifact_download_backfill" {
            occupied.push(format!("{} ({n})", t.name));
        }
    }
    if !occupied.is_empty() && !opts.overwrite {
        return Err(format!(
            "the target already holds rows in: {}. Pass -overwrite to replace them",
            occupied.join(", ")
        ));
    }
    if opts.dry_run {
        return Ok(report);
    }

    let tx = conn.transaction_unlocked().map_err(db("begin"))?;
    let all = tables
        .iter()
        .map(|t| quote(&t.name))
        .collect::<Vec<_>>()
        .join(", ");
    tx.execute_batch(&format!("TRUNCATE TABLE {all} RESTART IDENTITY CASCADE"))
        .map_err(db("empty target tables"))?;

    for (t, entry) in tables.iter().zip(report.tables.iter_mut()) {
        if entry.skipped {
            continue;
        }
        let cols = t
            .columns
            .iter()
            .map(|c| quote(c))
            .collect::<Vec<_>>()
            .join(", ");
        let mut select = format!("SELECT {cols} FROM {} c", quote(&t.name));
        if let Some(conds) = filters.get(&t.name) {
            select.push_str(" WHERE ");
            select.push_str(&conds.join(" AND "));
        }
        let width = t.columns.len();
        let batch_rows = (MAX_BATCH_PARAMS / width).clamp(1, MAX_BATCH_ROWS);
        let row_marks = format!("({})", vec!["?"; width].join(", "));
        let insert = |rows: usize| {
            format!(
                "INSERT INTO {}({cols}) VALUES {}",
                quote(&t.name),
                vec![row_marks.as_str(); rows].join(", ")
            )
        };
        let full = insert(batch_rows);

        let mut stmt = src.prepare(&select).map_err(sqlite("read source table"))?;
        let mut rows = stmt.query([]).map_err(sqlite("read source table"))?;
        let mut batch: Vec<Value> = Vec::with_capacity(batch_rows * width);
        let mut copied = 0_i64;
        loop {
            let row = rows.next().map_err(sqlite("read source row"))?;
            if let Some(row) = row {
                for i in 0..width {
                    let v = row.get_ref(i).map_err(sqlite("read source value"))?;
                    batch.push(to_value(v, &mut report.sanitized_values));
                }
            }
            let buffered = batch.len() / width;
            if buffered == batch_rows || (row.is_none() && buffered > 0) {
                let sql = if buffered == batch_rows {
                    full.clone()
                } else {
                    insert(buffered)
                };
                tx.execute(&sql, std::mem::take(&mut batch))
                    .map_err(|e| format!("insert into {}: {e}", t.name))?;
                copied += i64::try_from(buffered).unwrap_or(i64::MAX);
            }
            if row.is_none() {
                break;
            }
        }
        if let Some(id) = &t.identity {
            // Explicit ids do not advance the identity sequence.
            tx.query_row(
                &format!(
                    "SELECT setval(pg_get_serial_sequence('{name}', '{id}'),
                            GREATEST((SELECT COALESCE(MAX({qid}), 0) FROM {qname}), 1),
                            (SELECT COUNT(*) > 0 FROM {qname}))",
                    name = t.name,
                    qname = quote(&t.name),
                    qid = quote(id),
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| format!("reset the id sequence of {}: {e}", t.name))?;
        }
        let stored: i64 = tx
            .query_row(
                &format!("SELECT COUNT(*) FROM {}", quote(&t.name)),
                [],
                |r| r.get(0),
            )
            .map_err(db("count copied rows"))?;
        if stored != copied || copied != entry.source_rows - entry.orphan_rows {
            return Err(format!(
                "{}: read {} row(s), wrote {copied}, the target holds {stored}",
                t.name,
                entry.source_rows - entry.orphan_rows
            ));
        }
        entry.copied_rows = copied;
        report.rows_copied += copied;
        tracing::info!(table = %t.name, rows = copied, "migrate-database: table copied");
    }
    tx.commit().map_err(db("commit"))?;
    Ok(report)
}

/// Rebuilds the SQLite metadata the s3 backend keeps in the bucket into
/// `staging` and returns the file.
async fn fetch_from_bucket(s3: &config::S3Config, staging: &Path) -> Result<PathBuf, String> {
    let cfg = storage::S3Config {
        bucket: s3.bucket.clone(),
        prefix: s3.prefix.clone(),
        region: s3.region.clone(),
        endpoint: s3.endpoint.clone(),
        force_path_style: s3.force_path_style,
        access_key_id: s3.access_key_id.clone(),
        secret_access_key: s3.secret_access_key.clone(),
        read_timeout: s3.read_timeout,
    };
    let client = storage::s3::new_s3_client(&cfg)
        .await
        .map_err(|e| format!("open bucket {}: {e}", s3.bucket))?;
    let api = objstore::S3Api::new(client);
    tokio::fs::create_dir_all(staging)
        .await
        .map_err(|e| format!("create staging dir: {e}"))?;
    let dst = staging.join("forklift-migrate-source.db");
    let fetched = objstore::fetch_metadata(&api, &s3.bucket, &objstore::meta_key(&s3.prefix), &dst)
        .await
        .map_err(|e| format!("rebuild metadata from bucket {}: {e}", s3.bucket))?
        .ok_or_else(|| format!("bucket {} holds no metadata to migrate", s3.bucket))?;
    tracing::info!(
        bucket = %s3.bucket,
        bytes = fetched.size,
        version = %fetched.version,
        "migrate-database: rebuilt the metadata from the bucket"
    );
    Ok(dst)
}

/// Waits for every forklift pod to be gone and then holds the HA Lease, so
/// the metadata in the bucket is final and stays that way during the copy.
async fn stop_writers(args: &Args) -> Result<Option<LeaseHold>, String> {
    if args.lease_name.is_empty() {
        return Ok(None);
    }
    let namespace = std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "default".into());
    let client = in_cluster_client()?;
    let left = wait_for_no_writers(&client, &namespace, &args.writer_selector, args.wait).await?;
    if !left.is_empty() {
        return Err(format!(
            "forklift pods still running after {}s: {}",
            args.wait.as_secs(),
            left.join(", ")
        ));
    }
    let identity = format!(
        "forklift-migrate-{}",
        std::env::var("POD_NAME").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string())
    );
    let ha = config::HAConfig {
        enabled: true,
        lease_name: args.lease_name.clone(),
        lease_namespace: namespace,
        identity,
        lease_duration: Duration::from_secs(15),
        renew_deadline: Duration::from_secs(10),
        retry_period: Duration::from_secs(2),
    };
    LeaseHold::acquire(ha, args.wait).await.map(Some)
}

/// Runs the whole command: stop writers, find the source, open the target,
/// copy.
pub async fn run(cfg: &config::Config, args: &Args) -> Result<Report, String> {
    let conn_str = cfg.database.connection_string().ok_or(
        "migrate-database needs a PostgreSQL target: set FORKLIFT_DATABASE_HOST or FORKLIFT_DATABASE_URL",
    )?;
    let hold = stop_writers(args).await?;
    let result = async {
        if args.from_sqlite.is_none() && cfg.storage.backend != "s3" {
            return Err(
                "no source: pass -from-sqlite, or set FORKLIFT_STORAGE_BACKEND=s3 to read the bucket"
                    .to_string(),
            );
        }
        let extra_roots = outbound::database_roots(&cfg.database.ca_file)
            .map_err(|e| format!("load the database ca file: {e}"))?;
        // Opened before the bucket download so a wrong target fails in seconds.
        let target = meta::Store::open_postgres(
            &conn_str,
            meta::PostgresOptions {
                max_connections: 2,
                schema: cfg.database.schema.clone(),
                extra_roots,
            },
        )
        .await
        .map_err(|e| format!("open the PostgreSQL target: {e}"))?;
        let source = match &args.from_sqlite {
            Some(path) => path.clone(),
            None => match fetch_from_bucket(&cfg.storage.s3, &args.staging).await {
                Ok(path) => path,
                Err(e) => {
                    target.close();
                    return Err(e);
                }
            },
        };
        let report = copy(
            &source,
            &target,
            CopyOptions {
                dry_run: args.dry_run,
                overwrite: args.overwrite,
                skip_orphans: args.skip_orphans,
            },
        )
        .await;
        target.close();
        if args.from_sqlite.is_none() && !args.keep_source {
            for suffix in ["", "-wal", "-shm"] {
                let mut name = source.clone().into_os_string();
                name.push(suffix);
                let _ = tokio::fs::remove_file(PathBuf::from(name)).await;
            }
        }
        report
    }
    .await;
    if let Some(hold) = hold {
        hold.release().await;
    }
    result
}

/// A short table of the report for a terminal.
pub fn render(r: &Report) -> String {
    let mut out = format!(
        "migrate-database{}: {} ({} bytes)\n",
        if r.dry_run { " (dry run)" } else { "" },
        r.source,
        r.source_bytes
    );
    for t in &r.tables {
        let note = if t.skipped {
            "  not carried over".to_string()
        } else if t.orphan_rows > 0 {
            format!("  {} orphan row(s) left out", t.orphan_rows)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "  {:<34} {:>10} -> {:>10}{note}\n",
            t.name, t.source_rows, t.copied_rows
        ));
    }
    out.push_str(&format!(
        "  rows copied: {}, text values cleaned: {}, took {} ms\n",
        r.rows_copied, r.sanitized_values, r.duration_ms
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{Artifact, AuditLog, Repository, Store};

    fn fk(child: &str, parent: &str) -> ForeignKey {
        ForeignKey {
            child: child.into(),
            child_cols: vec!["parent_id".into()],
            parent: parent.into(),
            parent_cols: vec!["id".into()],
        }
    }

    fn table(name: &str) -> Table {
        Table {
            name: name.into(),
            columns: vec!["id".into()],
            identity: None,
        }
    }

    #[test]
    fn parse_args_reads_every_flag() {
        let args: Vec<String> = [
            "-from-sqlite=/tmp/a.db",
            "--staging",
            "/work",
            "-dry-run",
            "-overwrite",
            "-skip-orphans",
            "-keep-source",
            "-lease-name=l",
            "-writer-selector=app=x",
            "-wait=30s",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let got = parse_args(&args).unwrap();
        assert_eq!(got.from_sqlite, Some(PathBuf::from("/tmp/a.db")));
        assert_eq!(got.staging, PathBuf::from("/work"));
        assert!(got.dry_run && got.overwrite && got.skip_orphans && got.keep_source);
        assert_eq!(
            (got.lease_name.as_str(), got.writer_selector.as_str()),
            ("l", "app=x")
        );
        assert_eq!(got.wait, Duration::from_secs(30));

        assert_eq!(parse_args(&[]).unwrap(), Args::default());
        assert!(
            parse_args(&["-lease-name=l".into()]).is_err(),
            "lease without selector"
        );
        assert!(parse_args(&["-nope".into()]).is_err());
        assert!(parse_args(&["-wait=soon".into()]).is_err());
        assert_eq!(parse_args(&["-h".into()]).unwrap_err(), "");
    }

    #[test]
    fn parents_come_before_children() {
        let tables = vec![table("c"), table("b"), table("a")];
        let fks = vec![fk("c", "b"), fk("b", "a"), fk("a", "a")];
        let order: Vec<String> = parents_first(tables, &fks)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(order, ["a", "b", "c"]);

        let cycle = vec![fk("a", "b"), fk("b", "a")];
        assert!(parents_first(vec![table("a"), table("b")], &cycle).is_err());
    }

    #[test]
    fn has_parent_matches_every_key_column() {
        let key = ForeignKey {
            child: "labels".into(),
            child_cols: vec!["repo_id".into(), "path".into()],
            parent: "artifacts".into(),
            parent_cols: vec!["repo_id".into(), "path".into()],
        };
        assert_eq!(
            has_parent(&key),
            "(c.\"repo_id\" IS NULL OR c.\"path\" IS NULL OR EXISTS (SELECT 1 FROM \"artifacts\" p \
             WHERE p.\"repo_id\" = c.\"repo_id\" AND p.\"path\" = c.\"path\"))"
        );
    }

    #[test]
    fn values_postgres_cannot_store_are_cleaned_and_counted() {
        use rusqlite::types::ValueRef;
        let mut n = 0;
        assert_eq!(
            to_value(ValueRef::Text(b"plain"), &mut n),
            Value::Text("plain".into())
        );
        assert_eq!(to_value(ValueRef::Integer(7), &mut n), Value::Integer(7));
        assert_eq!(to_value(ValueRef::Null, &mut n), Value::Null);
        assert_eq!(n, 0);
        assert_eq!(
            to_value(ValueRef::Text(b"a\0b"), &mut n),
            Value::Text("ab".into())
        );
        assert_eq!(
            to_value(ValueRef::Text(b"a\xffb"), &mut n),
            Value::Text("a\u{fffd}b".into())
        );
        assert_eq!(
            to_value(ValueRef::Blob(b"blob"), &mut n),
            Value::Text("blob".into())
        );
        assert_eq!(n, 2);
    }

    #[test]
    fn render_names_what_was_left_out() {
        let r = Report {
            source: "s.db".into(),
            dry_run: true,
            tables: vec![
                TableReport {
                    name: "a".into(),
                    source_rows: 2,
                    copied_rows: 2,
                    ..Default::default()
                },
                TableReport {
                    name: "b".into(),
                    source_rows: 3,
                    orphan_rows: 1,
                    ..Default::default()
                },
                TableReport {
                    name: "c".into(),
                    skipped: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let text = render(&r);
        assert!(text.contains("(dry run)"), "{text}");
        assert!(text.contains("1 orphan row(s) left out"), "{text}");
        assert!(text.contains("not carried over"), "{text}");
    }

    /// The target needs a real PostgreSQL, named by the same variable the
    /// rest of the suite uses.
    async fn postgres_target() -> Option<(Store, tempfile::TempDir)> {
        if !crate::testing::meta::on_postgres() {
            return None;
        }
        Some(Store::open_temp().await.expect("open postgres target"))
    }

    /// A SQLite store with one of everything the copy has to get right.
    async fn sqlite_source(dir: &Path) -> (PathBuf, i64) {
        let path = dir.join("source.db");
        let s = Store::open(&path).await.expect("open sqlite source");
        let repo = s
            .create_repository(Repository {
                name: "npm-hosted".into(),
                format: meta::FORMAT_NPM.into(),
                r#type: meta::TYPE_HOSTED.into(),
                ..Default::default()
            })
            .await
            .expect("repository");
        for (path, blob) in [
            ("left-pad/1.0.0", "sha-a"),
            ("left-pad/1.0.1", "sha-b"),
            ("dup/1.0.0", "sha-a"),
        ] {
            s.put_artifact(Artifact {
                repo_id: repo.id,
                path: path.into(),
                version: "1.0.0".into(),
                blob_sha256: blob.into(),
                size: 3,
                ..Default::default()
            })
            .await
            .expect("artifact");
        }
        s.insert_audit_logs(vec![AuditLog {
            repo_name: "npm-hosted".into(),
            event: meta::EVENT_DOWNLOAD.into(),
            method: "GET".into(),
            path: "left-pad/1.0.0".into(),
            status: 200,
            created_at: chrono::Utc::now(),
            ..Default::default()
        }])
        .await
        .expect("audit log");
        s.create_oci_upload_session("0123456789abcdef0123456789abcdef", repo.id, "app")
            .await
            .expect("oci session");
        s.close();
        (path, repo.id)
    }

    fn raw(path: &Path) -> rusqlite::Connection {
        rusqlite::Connection::open(path).expect("open raw sqlite")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn copies_a_sqlite_store_into_postgres() {
        let Some((target, _guard)) = postgres_target().await else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (source, repo_id) = sqlite_source(dir.path()).await;

        let dry = copy(
            &source,
            &target,
            CopyOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .expect("dry run");
        assert!(dry.dry_run);
        assert_eq!(dry.rows_copied, 0);
        assert!(
            target.list_repositories().await.unwrap().is_empty(),
            "dry run wrote rows"
        );

        let report = copy(&source, &target, CopyOptions::default())
            .await
            .expect("copy");
        let rows = |name: &str| {
            report
                .tables
                .iter()
                .find(|t| t.name == name)
                .map(|t| (t.source_rows, t.copied_rows, t.skipped))
        };
        assert_eq!(rows("repositories"), Some((1, 1, false)));
        assert_eq!(rows("artifacts"), Some((3, 3, false)));
        assert_eq!(rows("blobs"), Some((2, 2, false)));
        assert_eq!(
            rows("oci_upload_sessions"),
            Some((1, 0, true)),
            "pod-local sessions stay behind"
        );
        assert_eq!(report.sanitized_values, 0);

        // Read back through the store, not through SQL.
        let art = target
            .get_artifact(repo_id, "left-pad/1.0.0")
            .await
            .expect("artifact");
        assert_eq!(art.blob_sha256, "sha-a");
        assert_eq!(target.get_blob("sha-a").await.unwrap().ref_count, 2);
        assert_eq!(target.count_audit_logs("npm-hosted", "").await.unwrap(), 1);
        assert_eq!(target.list_blob_ref_drift(10).await.unwrap(), Vec::new());

        // Explicit ids must not leave the identity sequence behind them.
        let next = target
            .create_repository(Repository {
                name: "after-migration".into(),
                format: meta::FORMAT_NPM.into(),
                r#type: meta::TYPE_HOSTED.into(),
                ..Default::default()
            })
            .await
            .expect("insert after migration");
        assert!(next.id > repo_id, "id {} reused", next.id);

        // A second run refuses the now populated target, then replaces it.
        let err = copy(&source, &target, CopyOptions::default())
            .await
            .unwrap_err();
        assert!(err.contains("already holds rows"), "{err}");
        let again = copy(
            &source,
            &target,
            CopyOptions {
                overwrite: true,
                ..Default::default()
            },
        )
        .await
        .expect("overwrite");
        assert_eq!(again.rows_copied, report.rows_copied);
        assert_eq!(target.list_repositories().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn orphans_and_bad_text_are_handled() {
        let Some((target, _guard)) = postgres_target().await else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (source, repo_id) = sqlite_source(dir.path()).await;
        {
            let c = raw(&source);
            c.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
            c.execute(
                "INSERT INTO artifacts(repo_id, path, blob_sha256, size, cached_at, last_accessed_at, updated_at)
                 VALUES(9999, 'ghost', 'sha-a', 1, 't', 't', 't')",
                [],
            )
            .unwrap();
            c.execute(
                "UPDATE repositories SET description = CAST(x'6100ff62' AS TEXT) WHERE id = ?1",
                [repo_id],
            )
            .unwrap();
        }

        let err = copy(&source, &target, CopyOptions::default())
            .await
            .unwrap_err();
        assert!(
            err.contains("reference a missing repositories row"),
            "{err}"
        );
        assert!(
            target.list_repositories().await.unwrap().is_empty(),
            "failed copy left rows"
        );

        let report = copy(
            &source,
            &target,
            CopyOptions {
                skip_orphans: true,
                ..Default::default()
            },
        )
        .await
        .expect("copy without orphans");
        let artifacts = report
            .tables
            .iter()
            .find(|t| t.name == "artifacts")
            .unwrap();
        assert_eq!(
            (
                artifacts.source_rows,
                artifacts.copied_rows,
                artifacts.orphan_rows
            ),
            (4, 3, 1)
        );
        assert_eq!(report.sanitized_values, 1);
        let repo = target.get_repository(repo_id).await.expect("repository");
        assert_eq!(repo.description, "a\u{fffd}b");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refuses_a_source_at_another_schema_version() {
        let Some((target, _guard)) = postgres_target().await else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (source, _) = sqlite_source(dir.path()).await;
        raw(&source)
            .execute("DELETE FROM schema_migrations WHERE version = (SELECT MAX(version) FROM schema_migrations)", [])
            .unwrap();
        let err = copy(&source, &target, CopyOptions::default())
            .await
            .unwrap_err();
        assert!(err.contains("schema version"), "{err}");
    }

    #[tokio::test]
    async fn refuses_a_sqlite_target() {
        if crate::testing::meta::on_postgres() {
            return;
        }
        let (target, dir) = crate::testing::meta::test_store().await;
        let (source, _) = sqlite_source(dir.path()).await;
        let err = copy(&source, &target, CopyOptions::default())
            .await
            .unwrap_err();
        assert!(err.contains("not PostgreSQL"), "{err}");
    }
}
