use std::fmt::Write;
use std::sync::Arc;

use axum::extract::{MatchedPath, Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Serialize;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::credential::{self, Cache};
use crate::observability::metrics::Metrics;

const TOKEN_HEADER: &str = "x-aws-ec2-metadata-token";
const TOKEN_TTL_HEADER: &str = "x-aws-ec2-metadata-token-ttl-seconds";
const MAX_TOKEN_TTL_SECONDS: u32 = 21_600;
const CREDENTIALS_PATH: &str = "/latest/meta-data/iam/security-credentials";

#[derive(Debug, Error)]
#[error("failed to read system randomness: {0}")]
pub struct TokenError(getrandom::Error);

pub fn generate_session_token() -> Result<String, TokenError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(TokenError)?;
    Ok(bytes.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}

#[derive(Clone)]
pub struct ImdsState {
    pub cache: Cache,
    pub role_name: Arc<str>,
    pub session_token: Arc<str>,
    pub expiry_margin: Duration,
    pub metrics: Arc<Metrics>,
}

pub fn router(state: ImdsState) -> Router {
    Router::new()
        .route("/latest/api/token", put(issue_token))
        .route(CREDENTIALS_PATH, get(list_roles))
        .route(&format!("{CREDENTIALS_PATH}/"), get(list_roles))
        .route(
            &format!("{CREDENTIALS_PATH}/{{role}}"),
            get(role_credentials),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            count_requests,
        ))
        .with_state(state)
}

async fn count_requests(State(state): State<ImdsState>, request: Request, next: Next) -> Response {
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_string(), |m| m.as_str().to_string());
    let response = next.run(request).await;
    state
        .metrics
        .record_request(&path, response.status().as_u16());
    response
}

async fn issue_token(State(state): State<ImdsState>, headers: HeaderMap) -> Response {
    if headers.contains_key("x-forwarded-for") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let ttl = headers
        .get(TOKEN_TTL_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|ttl| (1..=MAX_TOKEN_TTL_SECONDS).contains(ttl));
    let Some(ttl) = ttl else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    (
        [(TOKEN_TTL_HEADER, ttl.to_string())],
        state.session_token.to_string(),
    )
        .into_response()
}

fn authorized(state: &ImdsState, headers: &HeaderMap) -> bool {
    headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok()) == Some(&*state.session_token)
}

async fn list_roles(State(state): State<ImdsState>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    state.role_name.to_string().into_response()
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct RoleCredentials<'a> {
    code: &'static str,
    last_updated: String,
    r#type: &'static str,
    access_key_id: &'a str,
    secret_access_key: &'a str,
    token: &'a str,
    expiration: String,
}

fn rfc3339(instant: OffsetDateTime) -> Option<String> {
    instant.replace_nanosecond(0).ok()?.format(&Rfc3339).ok()
}

async fn role_credentials(
    State(state): State<ImdsState>,
    Path(role): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if role != *state.role_name {
        return StatusCode::NOT_FOUND.into_response();
    }
    let now = OffsetDateTime::now_utc();
    let Some(snapshot) = credential::current(&state.cache, now) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let creds = &snapshot.credentials;
    let expiration = creds.advertised_expiration(now, state.expiry_margin);
    let (Some(last_updated), Some(expiration)) =
        (rfc3339(snapshot.fetched_at), rfc3339(expiration))
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    Json(RoleCredentials {
        code: "Success",
        last_updated,
        r#type: "AWS-HMAC",
        access_key_id: &creds.access_key_id,
        secret_access_key: &creds.secret_access_key,
        token: &creds.token,
        expiration,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use reqwest::Client;
    use serde_json::Value;

    use super::*;
    use crate::credential::{CacheWriter, testing};

    struct Fixture {
        base: String,
        writer: CacheWriter,
        metrics: Arc<Metrics>,
        client: Client,
    }

    async fn fixture() -> Fixture {
        let (writer, cache) = credential::cache();
        let metrics = Arc::new(Metrics::new());
        let state = ImdsState {
            cache,
            role_name: Arc::from("harbor-registry"),
            session_token: Arc::from("session"),
            expiry_margin: Duration::seconds(300),
            metrics: Arc::clone(&metrics),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
        Fixture {
            base,
            writer,
            metrics,
            client: Client::new(),
        }
    }

    impl Fixture {
        async fn get(&self, path: &str, token: Option<&str>) -> reqwest::Response {
            let mut request = self.client.get(format!("{}{path}", self.base));
            if let Some(token) = token {
                request = request.header(TOKEN_HEADER, token);
            }
            request.send().await.unwrap()
        }

        async fn put_token(&self, headers: &[(&str, &str)]) -> reqwest::Response {
            let mut request = self.client.put(format!("{}/latest/api/token", self.base));
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            request.send().await.unwrap()
        }
    }

    #[test]
    fn session_tokens_are_random_hex() {
        let first = generate_session_token().unwrap();
        let second = generate_session_token().unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn issues_token_with_ttl_echo() {
        let f = fixture().await;
        let response = f.put_token(&[(TOKEN_TTL_HEADER, "21600")]).await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()[TOKEN_TTL_HEADER], "21600");
        assert_eq!(response.text().await.unwrap(), "session");
    }

    #[tokio::test]
    async fn token_request_validation() {
        let f = fixture().await;
        assert_eq!(f.put_token(&[]).await.status(), 400);
        assert_eq!(f.put_token(&[(TOKEN_TTL_HEADER, "0")]).await.status(), 400);
        assert_eq!(
            f.put_token(&[(TOKEN_TTL_HEADER, "21601")]).await.status(),
            400
        );
        let forwarded = [(TOKEN_TTL_HEADER, "60"), ("x-forwarded-for", "10.0.0.1")];
        assert_eq!(f.put_token(&forwarded).await.status(), 403);
    }

    #[tokio::test]
    async fn metadata_requires_the_session_token() {
        let f = fixture().await;
        let list = format!("{CREDENTIALS_PATH}/");
        assert_eq!(f.get(&list, None).await.status(), 401);
        assert_eq!(f.get(&list, Some("wrong")).await.status(), 401);
        let role = format!("{CREDENTIALS_PATH}/harbor-registry");
        assert_eq!(f.get(&role, None).await.status(), 401);
    }

    #[tokio::test]
    async fn lists_the_role_with_and_without_trailing_slash() {
        let f = fixture().await;
        for path in [CREDENTIALS_PATH.to_string(), format!("{CREDENTIALS_PATH}/")] {
            let response = f.get(&path, Some("session")).await;
            assert_eq!(response.status(), 200);
            assert_eq!(response.text().await.unwrap(), "harbor-registry");
        }
    }

    #[tokio::test]
    async fn serves_credentials_in_instance_metadata_format() {
        let f = fixture().await;
        let role = format!("{CREDENTIALS_PATH}/harbor-registry");
        assert_eq!(f.get(&role, Some("session")).await.status(), 503);

        let expiration = (OffsetDateTime::now_utc() + Duration::hours(1))
            .replace_nanosecond(0)
            .unwrap();
        f.writer
            .send_replace(Some(Arc::new(testing::snapshot(expiration))));
        let response = f.get(&role, Some("session")).await;
        assert_eq!(response.status(), 200);
        let body: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(body["Code"], "Success");
        assert_eq!(body["Type"], "AWS-HMAC");
        assert_eq!(body["AccessKeyId"], "ASIAEXAMPLE");
        assert_eq!(body["SecretAccessKey"], "secret-example");
        assert_eq!(body["Token"], "token-example");
        assert_eq!(body["LastUpdated"], "1970-01-01T00:00:00Z");
        let advertised = rfc3339(expiration - Duration::seconds(300)).unwrap();
        assert_eq!(body["Expiration"], advertised.as_str());
    }

    #[tokio::test]
    async fn unknown_role_and_expired_credentials() {
        let f = fixture().await;
        let other = format!("{CREDENTIALS_PATH}/other");
        assert_eq!(f.get(&other, Some("session")).await.status(), 404);

        let expired = OffsetDateTime::now_utc() - Duration::seconds(1);
        f.writer
            .send_replace(Some(Arc::new(testing::snapshot(expired))));
        let role = format!("{CREDENTIALS_PATH}/harbor-registry");
        assert_eq!(f.get(&role, Some("session")).await.status(), 503);
    }

    #[tokio::test]
    async fn counts_requests_by_route() {
        let f = fixture().await;
        f.put_token(&[(TOKEN_TTL_HEADER, "60")]).await;
        f.get("/latest/meta-data/instance-id", Some("session"))
            .await;
        let text = f.metrics.encode();
        assert!(text.contains(r#"path="/latest/api/token",code="200""#));
        assert!(text.contains(r#"path="unmatched",code="404""#));
    }
}
