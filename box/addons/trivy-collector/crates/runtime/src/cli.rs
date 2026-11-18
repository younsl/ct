//! Command line settings every binary takes, flattened into each binary's own
//! `Config` so neither parses the other's flags.

use clap::Args;

pub mod env {
    pub const LOG_FORMAT: &str = "LOG_FORMAT";
    pub const LOG_LEVEL: &str = "LOG_LEVEL";
    pub const HEALTH_PORT: &str = "HEALTH_PORT";
}

#[derive(Args, Debug, Clone)]
pub struct CommonArgs {
    /// Log format: json or pretty
    #[arg(long, env = env::LOG_FORMAT, default_value = "json")]
    pub log_format: String,

    /// Log level: trace, debug, info, warn, error
    #[arg(long, env = env::LOG_LEVEL, default_value = "info")]
    pub log_level: String,

    /// Port serving /healthz, /readyz, and /metrics
    #[arg(long, env = env::HEALTH_PORT, default_value = "8080")]
    pub health_port: u16,
}

impl Default for CommonArgs {
    fn default() -> Self {
        Self {
            log_format: "json".to_string(),
            log_level: "info".to_string(),
            health_port: 8080,
        }
    }
}
