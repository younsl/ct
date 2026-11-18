use anyhow::Result;
use prometheus_client::registry::Registry;
use tracing::{error, info};

use trivy_collector_runtime::{build_info, lifecycle, logging};
use trivy_collector_server::config::{Command, Config};
use trivy_collector_server::metrics::Metrics;
use trivy_collector_server::web;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = Config::from_args();

    if matches!(config.command, Some(Command::Version)) {
        println!(
            "trivy-collector-server {}, commit: {}, build_date: {}",
            build_info::VERSION,
            build_info::GIT_SHA,
            build_info::BUILD_TIMESTAMP,
        );
        return Ok(());
    }

    logging::init(&config.common.log_format, &config.common.log_level);
    info!(
        version = build_info::VERSION,
        commit = build_info::GIT_SHA,
        build_date = build_info::BUILD_TIMESTAMP,
        rustc = build_info::RUSTC_SEMVER,
        port = config.server_port,
        scraper_url = %config.scraper_url,
        "trivy-collector-server starting"
    );

    if let Err(e) = config.validate() {
        error!(error = %e, "Configuration validation failed");
        std::process::exit(1);
    }

    let mut registry = Registry::default();
    let metrics = Metrics::new(&mut registry);
    let started = lifecycle::start(config.common.health_port, registry).await;

    if let Err(e) = web::run(config, started.health_server, started.shutdown, metrics).await {
        error!(error = %e, "Application error");
        std::process::exit(1);
    }
    info!("Shutdown complete");
    Ok(())
}
