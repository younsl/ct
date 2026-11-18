use anyhow::Result;
use prometheus_client::registry::Registry;
use tracing::{error, info};

use trivy_collector_runtime::{build_info, lifecycle, logging};
use trivy_collector_scraper::config::{Command, Config};
use trivy_collector_scraper::metrics::Metrics;
use trivy_collector_scraper::migrate;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = Config::from_args();
    logging::init(&config.common.log_format, &config.common.log_level);

    if let Some(command) = config.command.clone() {
        return run_command(command).await;
    }

    info!(
        version = build_info::VERSION,
        commit = build_info::GIT_SHA,
        build_date = build_info::BUILD_TIMESTAMP,
        rustc = build_info::RUSTC_SEMVER,
        cluster = %config.cluster_name,
        "trivy-collector-scraper starting"
    );

    let mut registry = Registry::default();
    let metrics = Metrics::new(&mut registry);
    let started = lifecycle::start(config.common.health_port, registry).await;

    if let Err(e) =
        trivy_collector_scraper::run(config, started.health_server, started.shutdown, metrics).await
    {
        error!(error = %e, "Application error");
        std::process::exit(1);
    }
    info!("Shutdown complete");
    Ok(())
}

/// One-shot subcommands, which run instead of the scraper and exit.
async fn run_command(command: Command) -> Result<()> {
    match command {
        Command::Version => {
            println!(
                "trivy-collector-scraper {}, commit: {}, build_date: {}",
                build_info::VERSION,
                build_info::GIT_SHA,
                build_info::BUILD_TIMESTAMP,
            );
            Ok(())
        }
        Command::ExportState {
            db_path,
            namespace,
            dry_run,
            notes_configmap,
            api_tokens_secret,
        } => {
            migrate::export_state(migrate::ExportRequest {
                db_path,
                namespace,
                notes_configmap,
                api_tokens_secret,
                dry_run,
            })
            .await
        }
    }
}
