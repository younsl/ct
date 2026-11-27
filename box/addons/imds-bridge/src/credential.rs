pub mod refresher;
pub mod source;

use std::fmt;
use std::sync::Arc;

use serde::Deserialize;
use time::{Duration, OffsetDateTime};
use tokio::sync::watch;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub token: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expiration: OffsetDateTime,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("expiration", &self.expiration)
            .finish_non_exhaustive()
    }
}

impl Credentials {
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        self.expiration <= now
    }

    /// Expiration reported to clients. Old SDK chains refresh only at the
    /// reported time, so it is pulled forward while the margin still fits.
    pub fn advertised_expiration(&self, now: OffsetDateTime, margin: Duration) -> OffsetDateTime {
        let early = self.expiration - margin;
        if early > now { early } else { self.expiration }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub credentials: Credentials,
    pub fetched_at: OffsetDateTime,
}

pub type CacheWriter = watch::Sender<Option<Arc<Snapshot>>>;
pub type Cache = watch::Receiver<Option<Arc<Snapshot>>>;

pub fn cache() -> (CacheWriter, Cache) {
    watch::channel(None)
}

/// Cached credentials that are still valid at `now`.
pub fn current(cache: &Cache, now: OffsetDateTime) -> Option<Arc<Snapshot>> {
    cache
        .borrow()
        .clone()
        .filter(|snapshot| !snapshot.credentials.is_expired(now))
}

#[cfg(test)]
pub mod testing {
    use super::{Credentials, OffsetDateTime, Snapshot};

    pub fn credentials(expiration: OffsetDateTime) -> Credentials {
        Credentials {
            access_key_id: "ASIAEXAMPLE".to_string(),
            secret_access_key: "secret-example".to_string(),
            token: "token-example".to_string(),
            expiration,
        }
    }

    pub fn snapshot(expiration: OffsetDateTime) -> Snapshot {
        Snapshot {
            credentials: credentials(expiration),
            fetched_at: OffsetDateTime::UNIX_EPOCH,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(seconds).unwrap()
    }

    #[test]
    fn debug_hides_secrets() {
        let rendered = format!("{:?}", testing::credentials(at(100)));
        assert!(!rendered.contains("secret-example"));
        assert!(!rendered.contains("token-example"));
        assert!(!rendered.contains("ASIAEXAMPLE"));
    }

    #[test]
    fn advertised_expiration_applies_margin_when_it_fits() {
        let creds = testing::credentials(at(1000));
        assert_eq!(
            creds.advertised_expiration(at(0), Duration::seconds(300)),
            at(700)
        );
    }

    #[test]
    fn advertised_expiration_falls_back_to_real_expiration() {
        let creds = testing::credentials(at(1000));
        assert_eq!(
            creds.advertised_expiration(at(700), Duration::seconds(300)),
            at(1000)
        );
        assert_eq!(
            creds.advertised_expiration(at(900), Duration::seconds(300)),
            at(1000)
        );
    }

    #[test]
    fn current_drops_expired_credentials() {
        let (writer, cache) = cache();
        assert!(current(&cache, at(0)).is_none());
        writer.send_replace(Some(Arc::new(testing::snapshot(at(1000)))));
        assert!(current(&cache, at(999)).is_some());
        assert!(current(&cache, at(1000)).is_none());
    }

    #[test]
    fn deserializes_pod_identity_response() {
        let body = r#"{"AccessKeyId":"ASIAEXAMPLE","SecretAccessKey":"s","Token":"t","AccountId":"123456789012","Expiration":"2026-11-27T12:00:00Z"}"#;
        let creds: Credentials = serde_json::from_str(body).unwrap();
        assert_eq!(creds.access_key_id, "ASIAEXAMPLE");
        assert_eq!(creds.expiration, at(1_795_780_800));
    }
}
