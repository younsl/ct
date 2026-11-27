use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::source::ContainerSource;
use super::{CacheWriter, Snapshot};
use crate::observability::metrics::Metrics;

const INITIAL_RETRY: Duration = Duration::from_secs(5);

/// Keeps the cache filled from a background task, so request handlers never
/// touch the token file or the upstream endpoint.
pub async fn run(
    source: ContainerSource,
    cache: CacheWriter,
    interval: Duration,
    metrics: Arc<Metrics>,
    shutdown: CancellationToken,
) {
    loop {
        let delay = match source.fetch().await {
            Ok(credentials) => {
                metrics.record_refresh(true);
                metrics.set_expiration(credentials.expiration.unix_timestamp());
                let rotated = cache.borrow().as_ref().is_none_or(|current| {
                    current.credentials.access_key_id != credentials.access_key_id
                });
                if rotated {
                    info!(expiration = %credentials.expiration, "Loaded new credentials");
                } else {
                    debug!(expiration = %credentials.expiration, "Credentials unchanged");
                }
                cache.send_replace(Some(Arc::new(Snapshot {
                    credentials,
                    fetched_at: OffsetDateTime::now_utc(),
                })));
                interval
            }
            Err(error) => {
                metrics.record_refresh(false);
                warn!(%error, "Failed to fetch credentials");
                if cache.borrow().is_none() {
                    interval.min(INITIAL_RETRY)
                } else {
                    interval
                }
            }
        };
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::credential::source::testing;
    use crate::credential::{self, Cache};

    async fn wait_for(cache: &mut Cache) -> Arc<Snapshot> {
        tokio::time::timeout(Duration::from_secs(5), cache.wait_for(Option::is_some))
            .await
            .unwrap()
            .unwrap()
            .clone()
            .unwrap()
    }

    #[tokio::test]
    async fn fills_cache_and_stops_on_shutdown() {
        let upstream =
            testing::upstream(testing::body("ASIAEXAMPLE", "2099-01-01T00:00:00Z")).await;
        let token = testing::token_file();
        let source = ContainerSource::new(&upstream.uri, token.path().to_path_buf()).unwrap();
        let (writer, mut cache) = credential::cache();
        let metrics = Arc::new(Metrics::new());
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(run(
            source,
            writer,
            Duration::from_millis(20),
            Arc::clone(&metrics),
            shutdown.clone(),
        ));

        let snapshot = wait_for(&mut cache).await;
        assert_eq!(snapshot.credentials.access_key_id, "ASIAEXAMPLE");
        assert!(metrics.encode().contains(r#"result="success""#));

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn keeps_cached_credentials_when_upstream_fails() {
        let upstream =
            testing::upstream(testing::body("ASIAEXAMPLE", "2099-01-01T00:00:00Z")).await;
        let token = testing::token_file();
        let source = ContainerSource::new(&upstream.uri, token.path().to_path_buf()).unwrap();
        let (writer, mut cache) = credential::cache();
        let metrics = Arc::new(Metrics::new());
        let shutdown = CancellationToken::new();
        tokio::spawn(run(
            source,
            writer,
            Duration::from_millis(20),
            Arc::clone(&metrics),
            shutdown.clone(),
        ));
        wait_for(&mut cache).await;

        upstream.status.store(500, Ordering::Relaxed);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !metrics.encode().contains(r#"result="failure""#) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(cache.borrow().is_some());
        shutdown.cancel();
    }
}
