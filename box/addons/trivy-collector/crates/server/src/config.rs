//! The server's command line and environment. Only what the server reads:
//! the scraper's settings are not parsed here.

use clap::{Parser, Subcommand};

use trivy_collector_runtime::cli::CommonArgs;

pub mod env {
    pub use trivy_collector_runtime::cli::env::*;

    pub const SERVER_PORT: &str = "SERVER_PORT";
    pub const SCRAPER_URL: &str = "SCRAPER_URL";
    pub const INTERNAL_TOKEN: &str = "INTERNAL_TOKEN";
    pub const NOTES_CONFIGMAP: &str = "NOTES_CONFIGMAP";
    pub const API_TOKENS_SECRET: &str = "API_TOKENS_SECRET";
    pub const HUB_SECRET_NAMESPACE: &str = "HUB_SECRET_NAMESPACE";
    pub const EXTERNAL_URL: &str = "EXTERNAL_URL";
    pub const MCP_ENABLED: &str = "MCP_ENABLED";
    pub const MCP_ALLOWED_HOSTS: &str = "MCP_ALLOWED_HOSTS";
    pub const MCP_STATELESS: &str = "MCP_STATELESS";
    pub const MCP_MAX_CONCURRENCY: &str = "MCP_MAX_CONCURRENCY";

    pub use crate::auth::config::env::*;
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Show version information
    Version,
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "trivy-collector-server",
    version,
    about = "Serves the trivy-collector web UI, REST API, and MCP server"
)]
pub struct Config {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub common: CommonArgs,

    /// API/UI server port (server mode only)
    #[arg(long, env = env::SERVER_PORT, default_value = "3000")]
    pub server_port: u16,

    /// Base URL of the scraper's internal API (server mode only).
    #[arg(long, env = env::SCRAPER_URL, default_value = "http://localhost:8081")]
    pub scraper_url: String,

    /// Shared token guarding the internal API, mounted into both pods from the
    /// same Secret. An empty value makes the scraper reject every request.
    #[arg(long, env = env::INTERNAL_TOKEN, default_value = "")]
    pub internal_token: String,

    /// ConfigMap holding report notes (server mode only).
    #[arg(long, env = env::NOTES_CONFIGMAP, default_value = "trivy-collector-notes")]
    pub notes_configmap: String,

    /// Secret holding API tokens (server mode only).
    #[arg(long, env = env::API_TOKENS_SECRET, default_value = "trivy-collector-api-tokens")]
    pub api_tokens_secret: String,

    /// Namespace where cluster-registration Secrets live, injected through the Downward
    /// API. Empty disables the Edge cluster watcher.
    #[arg(long, env = env::HUB_SECRET_NAMESPACE, default_value = "")]
    pub hub_secret_namespace: String,

    /// External base URL (e.g. `https://trivy.example.com`) used to build
    /// deep links in outbound notifications. Empty = no link emitted.
    #[arg(long, env = env::EXTERNAL_URL, default_value = "")]
    pub external_url: String,

    /// Mount the embedded MCP server (Streamable HTTP) at `/mcp`
    #[arg(long, env = env::MCP_ENABLED, default_value = "false")]
    pub mcp_enabled: bool,

    /// Allowed `Host` header values for `/mcp`, comma-separated.
    /// Empty = disable Host validation (required for in-cluster Service DNS).
    #[arg(long, env = env::MCP_ALLOWED_HOSTS, value_delimiter = ',', default_value = "")]
    pub mcp_allowed_hosts: Vec<String>,

    /// Serve `/mcp` without sessions. Required when more than one server replica
    /// sits behind the same Service.
    #[arg(long, env = env::MCP_STATELESS, default_value = "false")]
    pub mcp_stateless: bool,

    /// Maximum MCP tool calls executing at once across all sessions. Protects
    /// the shared SQLite pool from agent fan-out. 0 = unlimited.
    #[arg(long, env = env::MCP_MAX_CONCURRENCY, default_value = "8")]
    pub mcp_max_concurrency: usize,

    /// Authentication mode: "none" or "keycloak"
    #[arg(long, env = env::AUTH_MODE, default_value = "none")]
    pub auth_mode: String,

    /// OIDC issuer URL (required for keycloak mode)
    #[arg(long, env = env::OIDC_ISSUER_URL)]
    pub oidc_issuer_url: Option<String>,

    /// OIDC client ID (required for keycloak mode)
    #[arg(long, env = env::OIDC_CLIENT_ID)]
    pub oidc_client_id: Option<String>,

    /// OIDC client secret (required for keycloak mode)
    #[arg(long, env = env::OIDC_CLIENT_SECRET)]
    pub oidc_client_secret: Option<String>,

    /// OIDC redirect URL (required for keycloak mode)
    #[arg(long, env = env::OIDC_REDIRECT_URL)]
    pub oidc_redirect_url: Option<String>,

    /// OIDC scopes (space-separated)
    #[arg(long, env = env::OIDC_SCOPES, default_value = "openid profile email groups")]
    pub oidc_scopes: String,

    /// RBAC policy CSV (inline or file path)
    #[arg(long, env = env::RBAC_POLICY_CSV, default_value = "")]
    pub rbac_policy_csv: String,

    /// Default RBAC policy (applied when no matching rules found)
    #[arg(long, env = env::RBAC_DEFAULT_POLICY, default_value = "role:readonly")]
    pub rbac_default_policy: String,
}

impl Config {
    #[must_use]
    pub fn from_args() -> Self {
        <Self as clap::Parser>::parse()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.scraper_url.trim().is_empty() {
            return Err(format!(
                "{} is required: the server holds no database",
                env::SCRAPER_URL
            ));
        }
        if self.auth_mode == "keycloak" {
            crate::auth::config::validate_keycloak_config(
                &self.oidc_issuer_url,
                &self.oidc_client_id,
                &self.oidc_client_secret,
                &self.oidc_redirect_url,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
impl Config {
    #[must_use]
    pub fn for_test() -> Self {
        Self {
            command: None,
            common: CommonArgs::default(),
            server_port: 3000,
            scraper_url: "http://localhost:8081".to_string(),
            internal_token: "test-token".to_string(),
            notes_configmap: "trivy-collector-notes".to_string(),
            api_tokens_secret: "trivy-collector-api-tokens".to_string(),
            hub_secret_namespace: String::new(),
            external_url: String::new(),
            mcp_enabled: false,
            mcp_allowed_hosts: vec![],
            mcp_stateless: false,
            mcp_max_concurrency: 8,
            auth_mode: "none".to_string(),
            oidc_issuer_url: None,
            oidc_client_id: None,
            oidc_client_secret: None,
            oidc_redirect_url: None,
            oidc_scopes: "openid profile email groups".to_string(),
            rbac_policy_csv: String::new(),
            rbac_default_policy: "role:readonly".to_string(),
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
    fn the_defaults_validate() {
        assert!(Config::for_test().validate().is_ok());
    }

    #[test]
    fn keycloak_without_oidc_settings_is_rejected() {
        let mut config = Config::for_test();
        config.auth_mode = "keycloak".to_string();
        let err = config.validate().unwrap_err();
        assert!(err.contains("OIDC_ISSUER_URL"));
    }

    #[test]
    fn keycloak_with_every_oidc_setting_validates() {
        let mut config = Config::for_test();
        config.auth_mode = "keycloak".to_string();
        config.oidc_issuer_url = Some("https://keycloak.example.com/realms/test".to_string());
        config.oidc_client_id = Some("trivy-collector".to_string());
        config.oidc_client_secret = Some("secret".to_string());
        config.oidc_redirect_url = Some("http://localhost:3000/auth/callback".to_string());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn a_blank_scraper_url_is_rejected() {
        let mut config = Config::for_test();
        config.scraper_url = "  ".to_string();
        assert!(config.validate().unwrap_err().contains("SCRAPER_URL"));
    }

    #[test]
    fn scraper_flags_are_not_accepted() {
        assert!(
            Config::try_parse_from(["trivy-collector-server", "--storage-path", "/x"]).is_err()
        );
        assert!(Config::try_parse_from(["trivy-collector-server", "--mode", "server"]).is_err());
    }
}
