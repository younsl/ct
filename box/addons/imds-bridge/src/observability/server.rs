use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use time::OffsetDateTime;

use super::metrics::Metrics;
use crate::credential::{self, Cache};

const OPENMETRICS: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

#[derive(Clone)]
pub struct AdminState {
    pub cache: Cache,
    pub metrics: Arc<Metrics>,
}

pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(state)
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}

async fn readyz(State(state): State<AdminState>) -> StatusCode {
    if credential::current(&state.cache, OffsetDateTime::now_utc()).is_some() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn metrics(State(state): State<AdminState>) -> impl IntoResponse {
    ([(CONTENT_TYPE, OPENMETRICS)], state.metrics.encode())
}

#[cfg(test)]
mod tests {
    use time::Duration;

    use super::*;
    use crate::credential::testing;

    async fn serve(state: AdminState) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
        base
    }

    #[tokio::test]
    async fn readiness_follows_the_cache() {
        let (writer, cache) = credential::cache();
        let metrics = Arc::new(Metrics::new());
        let base = serve(AdminState { cache, metrics }).await;

        assert_eq!(
            reqwest::get(format!("{base}/healthz"))
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            reqwest::get(format!("{base}/readyz"))
                .await
                .unwrap()
                .status(),
            503
        );

        let valid = OffsetDateTime::now_utc() + Duration::hours(1);
        writer.send_replace(Some(Arc::new(testing::snapshot(valid))));
        assert_eq!(
            reqwest::get(format!("{base}/readyz"))
                .await
                .unwrap()
                .status(),
            200
        );

        let expired = OffsetDateTime::now_utc() - Duration::seconds(1);
        writer.send_replace(Some(Arc::new(testing::snapshot(expired))));
        assert_eq!(
            reqwest::get(format!("{base}/readyz"))
                .await
                .unwrap()
                .status(),
            503
        );
    }

    #[tokio::test]
    async fn serves_metrics() {
        let (_writer, cache) = credential::cache();
        let metrics = Arc::new(Metrics::new());
        metrics.record_refresh(true);
        let base = serve(AdminState { cache, metrics }).await;
        let body = reqwest::get(format!("{base}/metrics"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(body.contains("imds_bridge_credential_refreshes_total"));
    }
}
