//! Process start and stop shared by both binaries: the health and metrics
//! listener, and the shutdown signal.

use std::sync::Arc;

use prometheus_client::registry::Registry;
use tokio::sync::watch;
use tracing::{error, info};

use crate::health::HealthServer;

/// What a binary needs once the process is up.
pub struct Started {
    pub health_server: HealthServer,
    /// Flips to `true` on SIGTERM or Ctrl-C.
    pub shutdown: watch::Receiver<bool>,
}

/// Serve /healthz, /readyz, and /metrics on `health_port`, wait until the
/// listener is bound, and arm the shutdown signal.
pub async fn start(health_port: u16, registry: Registry) -> Started {
    let health_server = HealthServer::new(Arc::new(registry));
    let serving = health_server.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        if let Err(e) = serving.serve(health_port, ready_tx).await {
            error!(error = %e, "Health check server failed");
        }
    });
    ready_rx.await.ok();
    info!(
        port = health_port,
        endpoint = "/metrics",
        "Prometheus metrics endpoint is ready"
    );

    let (shutdown_tx, shutdown) = watch::channel(false);
    tokio::spawn(async move {
        wait_for_signal().await;
        info!("Received shutdown signal");
        let _ = shutdown_tx.send(true);
    });

    Started {
        health_server,
        shutdown,
    }
}

async fn wait_for_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                error!(error = %e, "Failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
