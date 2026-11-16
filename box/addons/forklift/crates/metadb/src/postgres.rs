//! PostgreSQL connections for the facade.
//!
//! The store runs every statement on the blocking thread pool, so a connection
//! is driven synchronously: each call blocks its thread on the async driver
//! through the runtime handle captured at connect time.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rustls::pki_types::CertificateDer;
use tokio::runtime::Handle;
use tokio_postgres::config::SslMode;
use tokio_postgres::types::ToSql;
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::error::{Error, Result};
use crate::value::Value;

mod translate;
mod types;

pub use types::PgValue;

/// Statements kept prepared per connection. Statements built with a variable
/// number of placeholders would otherwise grow the cache without bound.
const STATEMENT_CACHE_MAX: usize = 512;

/// How to reach a PostgreSQL server. Built once and shared by every
/// connection of a pool.
pub struct PgConfig {
    config: tokio_postgres::Config,
    tls: Option<MakeRustlsConnect>,
    search_path: Option<String>,
}

impl PgConfig {
    /// Parses a `postgres://` URL or a key-value connection string. TLS is
    /// used unless the string sets `sslmode=disable`, and server certificates
    /// are verified against the system roots plus `extra_roots`.
    pub fn parse(conn_str: &str, extra_roots: &[CertificateDer<'static>]) -> Result<Self> {
        let config: tokio_postgres::Config = conn_str
            .parse()
            .map_err(|e| Error::Connect(format!("parse connection string: {e}")))?;
        let tls = if config.get_ssl_mode() == SslMode::Disable {
            None
        } else {
            Some(MakeRustlsConnect::new(tls_config(extra_roots)?))
        };
        Ok(Self {
            config,
            tls,
            search_path: None,
        })
    }

    /// Makes every connection resolve unqualified names in `schema` instead of
    /// `public`. The name is restricted to identifier characters because it is
    /// spliced into a `SET` statement.
    pub fn with_search_path(mut self, schema: &str) -> Result<Self> {
        if schema.is_empty()
            || !schema
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(Error::Connect(format!("invalid schema name {schema:?}")));
        }
        self.search_path = Some(schema.to_owned());
        Ok(self)
    }
}

fn tls_config(extra_roots: &[CertificateDer<'static>]) -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    // A scratch image may ship no system bundle, so unreadable or invalid
    // system roots are skipped and verification fails later with the real
    // cause if nothing usable was loaded.
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    for cert in extra_roots {
        roots
            .add(cert.clone())
            .map_err(|e| Error::Connect(format!("add root certificate: {e}")))?;
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Connect(format!("tls protocol versions: {e}")))?
        .with_root_certificates(roots)
        .with_no_client_auth())
}

/// One PostgreSQL session.
pub struct PgConnection {
    client: tokio_postgres::Client,
    handle: Handle,
    statements: RefCell<HashMap<String, tokio_postgres::Statement>>,
}

impl PgConnection {
    /// Connects and spawns the task that drives the socket. Must be called
    /// from a thread that may block: a blocking-pool thread, not an async
    /// task.
    pub fn connect(cfg: &PgConfig) -> Result<Self> {
        let handle =
            Handle::try_current().map_err(|e| Error::Connect(format!("no tokio runtime: {e}")))?;
        let client = handle.block_on(async {
            match &cfg.tls {
                None => {
                    let (client, conn) = cfg.config.connect(tokio_postgres::NoTls).await?;
                    tokio::spawn(conn);
                    Ok::<_, tokio_postgres::Error>(client)
                }
                Some(tls) => {
                    let (client, conn) = cfg.config.connect(tls.clone()).await?;
                    tokio::spawn(conn);
                    Ok(client)
                }
            }
        })?;
        let conn = Self {
            client,
            handle,
            statements: RefCell::new(HashMap::new()),
        };
        if let Some(schema) = &cfg.search_path {
            conn.batch_execute(&format!("SET search_path TO \"{schema}\""))?;
        }
        Ok(conn)
    }

    /// True once the server or the network dropped the session.
    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    fn statement(&self, sql: &str) -> Result<tokio_postgres::Statement> {
        if let Some(st) = self.statements.borrow().get(sql) {
            return Ok(st.clone());
        }
        let translated = translate::translate(sql);
        let st = self.handle.block_on(self.client.prepare(&translated))?;
        let mut cache = self.statements.borrow_mut();
        if cache.len() >= STATEMENT_CACHE_MAX {
            cache.clear();
        }
        cache.insert(sql.to_owned(), st.clone());
        Ok(st)
    }

    fn bind(values: &[Value]) -> Vec<&(dyn ToSql + Sync)> {
        values.iter().map(|v| v as &(dyn ToSql + Sync)).collect()
    }

    /// Runs a statement and returns the affected row count.
    pub fn execute(&self, sql: &str, values: &[Value]) -> Result<u64> {
        let st = self.statement(sql)?;
        Ok(self
            .handle
            .block_on(self.client.execute(&st, &Self::bind(values)))?)
    }

    /// Runs a query and returns every row.
    pub fn query(&self, sql: &str, values: &[Value]) -> Result<Vec<tokio_postgres::Row>> {
        let st = self.statement(sql)?;
        Ok(self
            .handle
            .block_on(self.client.query(&st, &Self::bind(values)))?)
    }

    /// Runs one or more statements without parameters or translation.
    pub fn batch_execute(&self, sql: &str) -> Result<()> {
        Ok(self.handle.block_on(self.client.batch_execute(sql))?)
    }
}
