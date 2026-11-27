use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::time::Duration;

use reqwest::header::AUTHORIZATION;
use reqwest::{Client, Url};
use thiserror::Error;

use super::Credentials;

const ECS_HOST: IpAddr = IpAddr::V4(Ipv4Addr::new(169, 254, 170, 2));
const EKS_HOST_V4: IpAddr = IpAddr::V4(Ipv4Addr::new(169, 254, 170, 23));
const EKS_HOST_V6: IpAddr = IpAddr::V6(Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x23));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("invalid credentials URI: {0}")]
    InvalidUri(String),
    #[error("credentials URI must use plain http, got scheme {0}")]
    UnsupportedScheme(String),
    #[error("credentials URI host {0} is not loopback or a container credentials address")]
    ForbiddenHost(String),
    #[error("failed to build the HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("failed to read the authorization token file: {0}")]
    TokenFile(#[source] std::io::Error),
    #[error("credentials request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("credentials endpoint returned HTTP {0}")]
    Status(u16),
    #[error("credentials endpoint returned a malformed response")]
    Malformed,
}

/// Same host allowlist the AWS SDKs apply to a plain http container
/// credentials URI, so the token is never sent anywhere else.
pub fn validate_uri(raw: &str) -> Result<Url, SourceError> {
    let url = Url::parse(raw).map_err(|e| SourceError::InvalidUri(e.to_string()))?;
    if url.scheme() != "http" {
        return Err(SourceError::UnsupportedScheme(url.scheme().to_string()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| SourceError::InvalidUri("missing host".to_string()))?;
    let allowed = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback() || [ECS_HOST, EKS_HOST_V4, EKS_HOST_V6].contains(&ip));
    if allowed {
        Ok(url)
    } else {
        Err(SourceError::ForbiddenHost(host.to_string()))
    }
}

pub struct ContainerSource {
    client: Client,
    uri: Url,
    token_file: PathBuf,
}

impl ContainerSource {
    pub fn new(uri: &str, token_file: PathBuf) -> Result<Self, SourceError> {
        let uri = validate_uri(uri)?;
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(SourceError::Client)?;
        Ok(Self {
            client,
            uri,
            token_file,
        })
    }

    pub async fn fetch(&self) -> Result<Credentials, SourceError> {
        let token = tokio::fs::read_to_string(&self.token_file)
            .await
            .map_err(SourceError::TokenFile)?;
        let response = self
            .client
            .get(self.uri.clone())
            .header(AUTHORIZATION, token.trim())
            .send()
            .await
            .map_err(SourceError::Request)?;
        let status = response.status();
        if !status.is_success() {
            return Err(SourceError::Status(status.as_u16()));
        }
        let body = response.bytes().await.map_err(SourceError::Request)?;
        let credentials: Credentials =
            serde_json::from_slice(&body).map_err(|_| SourceError::Malformed)?;
        if credentials.access_key_id.is_empty() || credentials.secret_access_key.is_empty() {
            return Err(SourceError::Malformed);
        }
        Ok(credentials)
    }
}

#[cfg(test)]
pub mod testing {
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU16, Ordering};

    use axum::Router;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use tempfile::NamedTempFile;

    pub const TOKEN: &str = "pod-identity-token";

    pub fn body(access_key_id: &str, expiration: &str) -> String {
        format!(
            r#"{{"AccessKeyId":"{access_key_id}","SecretAccessKey":"secret-example","Token":"token-example","AccountId":"123456789012","Expiration":"{expiration}"}}"#
        )
    }

    pub fn token_file() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "{TOKEN}").unwrap();
        file
    }

    pub struct Upstream {
        pub uri: String,
        pub status: Arc<AtomicU16>,
    }

    /// Fake Pod Identity Agent that answers only when the token matches.
    pub async fn upstream(body: String) -> Upstream {
        let status = Arc::new(AtomicU16::new(200));
        let handler_status = Arc::clone(&status);
        let app = Router::new().route(
            "/v1/credentials",
            get(move |headers: HeaderMap| {
                let body = body.clone();
                let status = Arc::clone(&handler_status);
                async move {
                    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(TOKEN) {
                        return (StatusCode::UNAUTHORIZED, String::new());
                    }
                    let code = StatusCode::from_u16(status.load(Ordering::Relaxed)).unwrap();
                    (code, body)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}/v1/credentials", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Upstream { uri, status }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    #[test]
    fn accepts_container_credentials_hosts() {
        for uri in [
            "http://169.254.170.23/v1/credentials",
            "http://[fd00:ec2::23]/v1/credentials",
            "http://169.254.170.2/v2/credentials",
            "http://127.0.0.1:8080/creds",
            "http://localhost:8080/creds",
        ] {
            assert!(validate_uri(uri).is_ok(), "{uri}");
        }
    }

    #[test]
    fn rejects_other_hosts_and_schemes() {
        assert!(matches!(
            validate_uri("http://example.com/creds"),
            Err(SourceError::ForbiddenHost(_))
        ));
        assert!(matches!(
            validate_uri("http://169.254.169.254/latest"),
            Err(SourceError::ForbiddenHost(_))
        ));
        assert!(matches!(
            validate_uri("https://169.254.170.23/v1/credentials"),
            Err(SourceError::UnsupportedScheme(_))
        ));
        assert!(matches!(
            validate_uri("not a uri"),
            Err(SourceError::InvalidUri(_))
        ));
    }

    #[tokio::test]
    async fn fetches_with_token_from_file() {
        let upstream =
            testing::upstream(testing::body("ASIAEXAMPLE", "2099-01-01T00:00:00Z")).await;
        let token = testing::token_file();
        let source = ContainerSource::new(&upstream.uri, token.path().to_path_buf()).unwrap();
        let creds = source.fetch().await.unwrap();
        assert_eq!(creds.access_key_id, "ASIAEXAMPLE");
    }

    #[tokio::test]
    async fn surfaces_upstream_status() {
        let upstream = testing::upstream(String::new()).await;
        upstream.status.store(500, Ordering::Relaxed);
        let token = testing::token_file();
        let source = ContainerSource::new(&upstream.uri, token.path().to_path_buf()).unwrap();
        assert!(matches!(
            source.fetch().await,
            Err(SourceError::Status(500))
        ));
    }

    #[tokio::test]
    async fn rejects_malformed_and_empty_credentials() {
        let token = testing::token_file();
        for body in [
            "not json".to_string(),
            testing::body("", "2099-01-01T00:00:00Z"),
        ] {
            let upstream = testing::upstream(body).await;
            let source = ContainerSource::new(&upstream.uri, token.path().to_path_buf()).unwrap();
            assert!(matches!(source.fetch().await, Err(SourceError::Malformed)));
        }
    }

    #[tokio::test]
    async fn missing_token_file_is_an_error() {
        let source = ContainerSource::new(
            "http://127.0.0.1:1/creds",
            PathBuf::from("/nonexistent/token"),
        )
        .unwrap();
        assert!(matches!(
            source.fetch().await,
            Err(SourceError::TokenFile(_))
        ));
    }
}
