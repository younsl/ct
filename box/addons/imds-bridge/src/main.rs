//! imds-bridge serves EKS Pod Identity credentials through an EC2 instance
//! metadata endpoint on loopback, for applications whose AWS credential chain
//! only knows static keys and the instance role.

mod app;
mod cli;
mod credential;
mod imds;
mod observability;

use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::app::Settings;
use crate::cli::{Args, LogFormat};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_logging(&args.log_level, args.log_format);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        commit = env!("BUILD_COMMIT"),
        build_date = env!("BUILD_DATE"),
        listen_addr = %args.listen_addr,
        metrics_addr = %args.metrics_addr,
        role_name = %args.role_name,
        credentials_uri = %args.credentials_uri,
        "Starting imds-bridge"
    );

    let imds_listener = TcpListener::bind(args.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.listen_addr))?;
    let admin_listener = TcpListener::bind(args.metrics_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.metrics_addr))?;

    let shutdown = CancellationToken::new();
    tokio::spawn(cancel_on_signal(shutdown.clone()));

    let settings = Settings {
        credentials_uri: args.credentials_uri,
        authorization_token_file: args.authorization_token_file,
        role_name: args.role_name,
        refresh_interval: Duration::from_secs(args.refresh_interval_seconds),
        expiry_margin: time::Duration::seconds(i64::from(args.expiry_margin_seconds)),
    };
    app::run(settings, imds_listener, admin_listener, shutdown).await?;
    info!("Stopped");
    Ok(())
}

fn init_logging(level: &str, format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        LogFormat::Json => builder.json().init(),
        LogFormat::Text => builder.init(),
    }
}

async fn cancel_on_signal(shutdown: CancellationToken) {
    let mut terminate = signal(SignalKind::terminate()).ok();
    let terminated = async {
        match terminate.as_mut() {
            Some(stream) => {
                stream.recv().await;
            }
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = terminated => {}
    }
    info!("Received shutdown signal");
    shutdown.cancel();
}
