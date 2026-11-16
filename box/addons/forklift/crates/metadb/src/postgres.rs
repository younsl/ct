//! PostgreSQL connections for the facade.
//!
//! The store runs every statement on the blocking thread pool, so a connection
//! is driven synchronously: each call blocks its thread on the async driver
//! through the runtime handle captured at connect time.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
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
    /// Parses a `postgres://` URL or a key-value connection string. `sslmode`
    /// follows libpq: `prefer` and `require` encrypt without checking the
    /// server certificate, `verify-ca` checks its chain against the system
    /// roots plus `extra_roots`, and `verify-full` also checks the host name.
    pub fn parse(conn_str: &str, extra_roots: &[CertificateDer<'static>]) -> Result<Self> {
        let (conn_str, verify) = split_verify(conn_str);
        let config: tokio_postgres::Config = conn_str
            .parse()
            .map_err(|e| Error::Connect(format!("parse connection string: {e}")))?;
        let tls = if config.get_ssl_mode() == SslMode::Disable {
            None
        } else {
            Some(MakeRustlsConnect::new(tls_config(verify, extra_roots)?))
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

/// How far the server certificate is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verify {
    None,
    Ca,
    Full,
}

/// Splits libpq's verify modes off the connection string, since the driver
/// only parses disable, prefer and require.
fn split_verify(conn_str: &str) -> (Cow<'_, str>, Verify) {
    for (mode, verify) in [
        ("sslmode=verify-full", Verify::Full),
        ("sslmode=verify-ca", Verify::Ca),
    ] {
        if conn_str.contains(mode) {
            let rewritten = conn_str.replacen(mode, "sslmode=require", 1);
            return (Cow::Owned(rewritten), verify);
        }
    }
    (Cow::Borrowed(conn_str), Verify::None)
}

fn tls_config(
    verify: Verify,
    extra_roots: &[CertificateDer<'static>],
) -> Result<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Connect(format!("tls protocol versions: {e}")))?;
    if verify == Verify::None {
        let verifier = RelaxedVerifier {
            chain: None,
            provider,
        };
        return Ok(builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth());
    }
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
    if verify == Verify::Full {
        return Ok(builder.with_root_certificates(roots).with_no_client_auth());
    }
    let chain = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
        .build()
        .map_err(|e| Error::Connect(format!("server certificate verifier: {e}")))?;
    let verifier = RelaxedVerifier {
        chain: Some(chain),
        provider,
    };
    Ok(builder
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

/// Server certificate check for the modes below `verify-full`. With `chain`
/// the certificate must chain to a trusted root and only a host name mismatch
/// is forgiven (`verify-ca`). Without it any certificate passes.
#[derive(Debug)]
struct RelaxedVerifier {
    chain: Option<Arc<WebPkiServerVerifier>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for RelaxedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let Some(chain) = &self.chain else {
            return Ok(ServerCertVerified::assertion());
        };
        // The host name is checked last, so a name error means the chain held.
        match chain.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now) {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// What a successful login reports about the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub user: String,
    pub database: String,
    pub version: String,
    /// False when `sslmode=prefer` fell back to an unencrypted session.
    pub encrypted: bool,
}

/// Connects and logs in once, then closes the session. Must run on a thread
/// that may block.
pub fn probe(cfg: &PgConfig) -> Result<ServerInfo> {
    let conn = PgConnection::connect(cfg)?;
    let row = conn.handle.block_on(conn.client.query_one(
        "SELECT current_user::text, current_database()::text, \
         current_setting('server_version'), \
         COALESCE((SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()), false)",
        &[],
    ))?;
    Ok(ServerInfo {
        user: row.get(0),
        database: row.get(1),
        version: row.get(2),
        encrypted: row.get(3),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Pki {
        ca: CertificateDer<'static>,
        leaf: CertificateDer<'static>,
    }

    fn pki(host: &str) -> Pki {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("ca params");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca = rcgen::CertifiedIssuer::self_signed(
            params,
            rcgen::KeyPair::generate().expect("ca key"),
        )
        .expect("ca certificate");
        let leaf = rcgen::CertificateParams::new(vec![host.to_owned()])
            .expect("leaf params")
            .signed_by(&rcgen::KeyPair::generate().expect("leaf key"), &ca)
            .expect("leaf certificate");
        Pki {
            ca: ca.der().clone(),
            leaf: leaf.der().clone(),
        }
    }

    fn verifier(trusted: Option<&CertificateDer<'static>>) -> RelaxedVerifier {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let chain = trusted.map(|ca| {
            let mut roots = rustls::RootCertStore::empty();
            roots.add(ca.clone()).expect("root");
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
                .build()
                .expect("verifier")
        });
        RelaxedVerifier { chain, provider }
    }

    fn check(verifier: &RelaxedVerifier, leaf: &CertificateDer<'_>, host: &str) -> bool {
        let name = ServerName::try_from(host.to_owned()).expect("server name");
        verifier
            .verify_server_cert(leaf, &[], &name, &[], UnixTime::now())
            .is_ok()
    }

    #[test]
    fn split_verify_maps_the_verify_modes_to_require() {
        let (conn, verify) = split_verify("host='db' sslmode=verify-full user='u'");
        assert_eq!(conn, "host='db' sslmode=require user='u'");
        assert_eq!(verify, Verify::Full);
        let (conn, verify) = split_verify("postgres://u@db/forklift?sslmode=verify-ca");
        assert_eq!(conn, "postgres://u@db/forklift?sslmode=require");
        assert_eq!(verify, Verify::Ca);
        for mode in ["disable", "prefer", "require"] {
            let conn_str = format!("host='db' sslmode={mode}");
            let (conn, verify) = split_verify(&conn_str);
            assert_eq!(conn, conn_str);
            assert_eq!(verify, Verify::None);
        }
    }

    #[test]
    fn parse_uses_tls_for_every_mode_but_disable() {
        let pki = pki("db.example.com");
        for (mode, tls) in [
            ("disable", false),
            ("prefer", true),
            ("require", true),
            ("verify-ca", true),
            ("verify-full", true),
        ] {
            let cfg = PgConfig::parse(
                &format!("host='db.example.com' sslmode={mode}"),
                std::slice::from_ref(&pki.ca),
            )
            .unwrap_or_else(|e| panic!("{mode}: {e}"));
            assert_eq!(cfg.tls.is_some(), tls, "{mode}");
        }
        assert!(PgConfig::parse("host='db' sslmode=verify", &[]).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn probe_reports_an_unreachable_server_as_retryable() {
        let cfg = PgConfig::parse(
            "host='127.0.0.1' port=1 user='u' connect_timeout=2 sslmode=disable",
            &[],
        )
        .expect("config");
        let err = tokio::task::spawn_blocking(move || probe(&cfg))
            .await
            .expect("join")
            .expect_err("nothing listens on port 1");
        assert_eq!(err.permanent_connect_failure(), None, "err = {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn probe_logs_in_to_a_live_server() {
        let Some(url) = std::env::var("FORKLIFT_TEST_POSTGRES_URL")
            .ok()
            .filter(|url| !url.is_empty())
        else {
            return;
        };
        let cfg = PgConfig::parse(&url, &[]).expect("config");
        let info = tokio::task::spawn_blocking(move || probe(&cfg))
            .await
            .expect("join")
            .expect("probe");
        assert!(!info.user.is_empty() && !info.version.is_empty());
    }

    #[test]
    fn require_accepts_a_certificate_from_an_unknown_ca() {
        let pki = pki("db.example.com");
        assert!(check(&verifier(None), &pki.leaf, "db.example.com"));
        assert!(verifier(None).supported_verify_schemes().len() > 1);
    }

    #[test]
    fn verify_ca_checks_the_chain_but_not_the_host_name() {
        let pki = pki("db.example.com");
        let trusting = verifier(Some(&pki.ca));
        assert!(check(&trusting, &pki.leaf, "db.example.com"));
        assert!(check(&trusting, &pki.leaf, "other.example.com"));
        let stranger = self::pki("db.example.com");
        assert!(!check(
            &verifier(Some(&stranger.ca)),
            &pki.leaf,
            "db.example.com"
        ));
    }
}
