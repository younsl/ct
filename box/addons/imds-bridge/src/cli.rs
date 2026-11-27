use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    Json,
    Text,
}

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Args {
    /// Loopback address the instance metadata endpoint listens on
    #[arg(long, env = "LISTEN_ADDR", default_value = "127.0.0.1:1338", value_parser = parse_loopback)]
    pub listen_addr: SocketAddr,

    /// Address for /healthz, /readyz and /metrics
    #[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9090")]
    pub metrics_addr: SocketAddr,

    /// Role name reported under iam/security-credentials
    #[arg(long, env = "ROLE_NAME", default_value = "imds-bridge", value_parser = parse_role_name)]
    pub role_name: String,

    /// Container credentials endpoint, injected by EKS Pod Identity
    #[arg(long, env = "AWS_CONTAINER_CREDENTIALS_FULL_URI")]
    pub credentials_uri: String,

    /// File holding the authorization token, injected by EKS Pod Identity
    #[arg(long, env = "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE")]
    pub authorization_token_file: PathBuf,

    /// Seconds between credential fetches from the container credentials endpoint
    #[arg(long, env = "REFRESH_INTERVAL_SECONDS", default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
    pub refresh_interval_seconds: u64,

    /// Seconds subtracted from the reported expiration so clients refresh early
    #[arg(long, env = "EXPIRY_MARGIN_SECONDS", default_value_t = 300)]
    pub expiry_margin_seconds: u32,

    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    #[arg(long, env = "LOG_FORMAT", value_enum, default_value_t = LogFormat::Json)]
    pub log_format: LogFormat,
}

fn parse_loopback(raw: &str) -> Result<SocketAddr, String> {
    let addr: SocketAddr = raw.parse().map_err(|e| format!("{e}"))?;
    if addr.ip().is_loopback() {
        Ok(addr)
    } else {
        Err("credentials must not leave the pod, use a loopback address".to_string())
    }
}

fn parse_role_name(raw: &str) -> Result<String, String> {
    let valid = !raw.is_empty()
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+=,.@_-".contains(c));
    if valid {
        Ok(raw.to_string())
    } else {
        Err("allowed characters are alphanumerics and +=,.@_-".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: [&str; 5] = [
        "imds-bridge",
        "--credentials-uri",
        "http://169.254.170.23/v1/credentials",
        "--authorization-token-file",
        "/var/run/token",
    ];

    fn parse(extra: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(REQUIRED.iter().chain(extra))
    }

    #[test]
    fn defaults() {
        let args = parse(&[]).unwrap();
        assert_eq!(args.listen_addr, "127.0.0.1:1338".parse().unwrap());
        assert_eq!(args.metrics_addr, "0.0.0.0:9090".parse().unwrap());
        assert_eq!(args.role_name, "imds-bridge");
        assert_eq!(args.refresh_interval_seconds, 60);
        assert_eq!(args.expiry_margin_seconds, 300);
        assert_eq!(args.log_format, LogFormat::Json);
    }

    #[test]
    fn listen_addr_must_be_loopback() {
        assert!(parse(&["--listen-addr", "0.0.0.0:1338"]).is_err());
        assert!(parse(&["--listen-addr", "10.0.0.5:1338"]).is_err());
        assert!(parse(&["--listen-addr", "[::1]:1338"]).is_ok());
    }

    #[test]
    fn role_name_rejects_path_characters() {
        assert!(parse(&["--role-name", "a/b"]).is_err());
        assert!(parse(&["--role-name", ""]).is_err());
        assert!(parse(&["--role-name", "harbor-registry"]).is_ok());
    }

    #[test]
    fn refresh_interval_must_be_positive() {
        assert!(parse(&["--refresh-interval-seconds", "0"]).is_err());
    }
}
