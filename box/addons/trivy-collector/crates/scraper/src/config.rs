//! The scraper's command line and environment. Only what the scraper reads:
//! the server's settings are not parsed here, so a misplaced server variable
//! is ignored rather than half-applied.

use clap::{Parser, Subcommand};

use trivy_collector_runtime::cli::CommonArgs;

pub mod env {
    pub use trivy_collector_runtime::cli::env::*;

    pub const CLUSTER_NAME: &str = "CLUSTER_NAME";
    pub const NAMESPACES: &str = "NAMESPACES";
    pub const COLLECT_VULN: &str = "COLLECT_VULN";
    pub const COLLECT_SBOM: &str = "COLLECT_SBOM";
    pub const STORAGE_PATH: &str = "STORAGE_PATH";
    pub const WATCH_LOCAL: &str = "WATCH_LOCAL";
    pub const INTERNAL_PORT: &str = "INTERNAL_PORT";
    pub const INTERNAL_TOKEN: &str = "INTERNAL_TOKEN";
    pub const HUB_SECRET_NAMESPACE: &str = "HUB_SECRET_NAMESPACE";
    pub const EXTERNAL_URL: &str = "EXTERNAL_URL";
    pub const NOTES_CONFIGMAP: &str = "NOTES_CONFIGMAP";
    pub const API_TOKENS_SECRET: &str = "API_TOKENS_SECRET";
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Show version information
    Version,
    /// One-shot migration off a PersistentVolume: read an existing SQLite
    /// database and write its authored state (API tokens, report notes) to a
    /// Secret and a ConfigMap through the API server.
    ///
    /// Reports need no export, since the next scraper start relists them from
    /// the clusters that own the CRs.
    ExportState {
        /// Path to the legacy database file, e.g. `/data/trivy.db`.
        #[arg(long)]
        db_path: String,
        /// Namespace to write the objects into. Empty = auto-detect.
        #[arg(long, default_value = "")]
        namespace: String,
        /// Report only what would be written, without writing it.
        #[arg(long, default_value = "false")]
        dry_run: bool,
        /// ConfigMap holding report notes (server mode only).
        #[arg(long, env = env::NOTES_CONFIGMAP, default_value = "trivy-collector-notes")]
        notes_configmap: String,
        /// Secret holding API tokens (server mode only).
        #[arg(long, env = env::API_TOKENS_SECRET, default_value = "trivy-collector-api-tokens")]
        api_tokens_secret: String,
    },
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "trivy-collector-scraper",
    version,
    about = "Watches Trivy Operator reports across clusters and owns the report database"
)]
pub struct Config {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub common: CommonArgs,

    /// Cluster identifier
    #[arg(long, env = env::CLUSTER_NAME, default_value = "local")]
    pub cluster_name: String,

    /// Namespaces to watch, comma-separated (empty = all namespaces)
    #[arg(long, env = env::NAMESPACES, value_delimiter = ',')]
    pub namespaces: Vec<String>,

    /// Collect VulnerabilityReports
    #[arg(long, env = env::COLLECT_VULN, default_value = "true")]
    pub collect_vulnerability_reports: bool,

    /// Collect SbomReports
    #[arg(long, env = env::COLLECT_SBOM, default_value = "true")]
    pub collect_sbom_reports: bool,

    /// Directory holding the SQLite database (scraper mode only). Backed by an
    /// `emptyDir`, so its contents are rebuilt from the watched clusters on
    /// every start.
    #[arg(long, env = env::STORAGE_PATH, default_value = "/data")]
    pub storage_path: String,

    /// Enable the local-cluster watcher (scraper mode only)
    #[arg(long, env = env::WATCH_LOCAL, default_value = "true")]
    pub watch_local: bool,

    /// Port the scraper serves its internal read API on. Never exposed
    /// through an HTTPRoute or ServiceMonitor.
    #[arg(long, env = env::INTERNAL_PORT, default_value = "8081")]
    pub internal_port: u16,

    /// Shared token guarding the internal API, mounted into both pods from the
    /// same Secret. An empty value makes the scraper reject every request.
    #[arg(long, env = env::INTERNAL_TOKEN, default_value = "")]
    pub internal_token: String,

    /// Namespace where cluster-registration Secrets live, injected through the Downward
    /// API. Empty disables the Edge cluster watcher.
    #[arg(long, env = env::HUB_SECRET_NAMESPACE, default_value = "")]
    pub hub_secret_namespace: String,

    /// External base URL (e.g. `https://trivy.example.com`) used to build
    /// deep links in outbound notifications. Empty = no link emitted.
    #[arg(long, env = env::EXTERNAL_URL, default_value = "")]
    pub external_url: String,
}

impl Config {
    #[must_use]
    pub fn from_args() -> Self {
        <Self as clap::Parser>::parse()
    }

    #[must_use]
    pub fn get_db_path(&self) -> String {
        format!("{}/trivy.db", self.storage_path)
    }
}

#[cfg(test)]
impl Config {
    #[must_use]
    pub fn for_test() -> Self {
        Self {
            command: None,
            common: CommonArgs::default(),
            cluster_name: "local".to_string(),
            namespaces: vec![],
            collect_vulnerability_reports: true,
            collect_sbom_reports: true,
            storage_path: "/data".to_string(),
            watch_local: true,
            internal_port: 8081,
            internal_token: "test-token".to_string(),
            hub_secret_namespace: String::new(),
            external_url: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line_is_well_formed() {
        Config::command().debug_assert();
    }

    #[test]
    fn the_db_path_sits_under_the_storage_path() {
        let mut config = Config::for_test();
        assert_eq!(config.get_db_path(), "/data/trivy.db");
        config.storage_path = "/tmp/custom".to_string();
        assert_eq!(config.get_db_path(), "/tmp/custom/trivy.db");
    }

    #[test]
    fn server_flags_are_not_accepted() {
        assert!(Config::try_parse_from(["trivy-collector-scraper", "--scraper-url", "x"]).is_err());
        assert!(Config::try_parse_from(["trivy-collector-scraper", "--mode", "scraper"]).is_err());
    }
}
