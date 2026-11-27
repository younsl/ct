use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::credential;
use crate::credential::refresher;
use crate::credential::source::{ContainerSource, SourceError};
use crate::imds::{self, ImdsState, TokenError};
use crate::observability::metrics::Metrics;
use crate::observability::server::{self, AdminState};

pub struct Settings {
    pub credentials_uri: String,
    pub authorization_token_file: PathBuf,
    pub role_name: String,
    pub refresh_interval: Duration,
    pub expiry_margin: time::Duration,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Token(#[from] TokenError),
    #[error("server failed: {0}")]
    Serve(#[from] std::io::Error),
}

pub async fn run(
    settings: Settings,
    imds_listener: TcpListener,
    admin_listener: TcpListener,
    shutdown: CancellationToken,
) -> Result<(), AppError> {
    let source =
        ContainerSource::new(&settings.credentials_uri, settings.authorization_token_file)?;
    let session_token = imds::generate_session_token()?;
    let metrics = Arc::new(Metrics::new());
    let (writer, cache) = credential::cache();

    let refresher = tokio::spawn(refresher::run(
        source,
        writer,
        settings.refresh_interval,
        Arc::clone(&metrics),
        shutdown.clone(),
    ));

    let imds_app = imds::router(ImdsState {
        cache: cache.clone(),
        role_name: Arc::from(settings.role_name),
        session_token: Arc::from(session_token),
        expiry_margin: settings.expiry_margin,
        metrics: Arc::clone(&metrics),
    });
    let admin_app = server::router(AdminState { cache, metrics });

    let imds_shutdown = shutdown.clone();
    let admin_shutdown = shutdown.clone();
    let result = tokio::try_join!(
        async {
            axum::serve(imds_listener, imds_app)
                .with_graceful_shutdown(async move { imds_shutdown.cancelled().await })
                .await
        },
        async {
            axum::serve(admin_listener, admin_app)
                .with_graceful_shutdown(async move { admin_shutdown.cancelled().await })
                .await
        },
    );
    shutdown.cancel();
    let _ = refresher.await;
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use reqwest::Client;
    use serde_json::Value;

    use super::*;
    use crate::credential::source::testing;

    #[tokio::test]
    async fn serves_upstream_credentials_end_to_end() {
        let upstream =
            testing::upstream(testing::body("ASIAEXAMPLE", "2099-01-01T00:00:00Z")).await;
        let token_file = testing::token_file();
        let imds_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let admin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let imds = format!("http://{}", imds_listener.local_addr().unwrap());
        let admin = format!("http://{}", admin_listener.local_addr().unwrap());
        let shutdown = CancellationToken::new();
        let settings = Settings {
            credentials_uri: upstream.uri.clone(),
            authorization_token_file: token_file.path().to_path_buf(),
            role_name: "harbor-registry".to_string(),
            refresh_interval: Duration::from_millis(50),
            expiry_margin: time::Duration::seconds(300),
        };
        let app = tokio::spawn(run(
            settings,
            imds_listener,
            admin_listener,
            shutdown.clone(),
        ));

        let client = Client::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let ready = client.get(format!("{admin}/readyz")).send().await;
                if ready.is_ok_and(|r| r.status() == 200) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        let session = client
            .put(format!("{imds}/latest/api/token"))
            .header("x-aws-ec2-metadata-token-ttl-seconds", "21600")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let base = format!("{imds}/latest/meta-data/iam/security-credentials/");
        let role = client
            .get(&base)
            .header("x-aws-ec2-metadata-token", &session)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(role, "harbor-registry");
        let body = client
            .get(format!("{base}{role}"))
            .header("x-aws-ec2-metadata-token", &session)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["Code"], "Success");
        assert_eq!(body["AccessKeyId"], "ASIAEXAMPLE");
        assert_eq!(body["Expiration"], "2098-12-31T23:55:00Z");

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), app)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn rejects_a_forbidden_credentials_uri() {
        let settings = Settings {
            credentials_uri: "http://example.com/creds".to_string(),
            authorization_token_file: PathBuf::from("/nonexistent"),
            role_name: "role".to_string(),
            refresh_interval: Duration::from_secs(60),
            expiry_margin: time::Duration::seconds(300),
        };
        let imds_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let admin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let result = run(
            settings,
            imds_listener,
            admin_listener,
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::Source(SourceError::ForbiddenHost(_)))
        ));
    }
}
