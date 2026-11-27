use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;

#[derive(Debug, Clone, PartialEq, Eq, Hash, EncodeLabelSet)]
struct RequestLabels {
    path: String,
    code: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, EncodeLabelSet)]
struct RefreshLabels {
    result: &'static str,
}

pub struct Metrics {
    registry: Registry,
    requests: Family<RequestLabels, Counter>,
    refreshes: Family<RefreshLabels, Counter>,
    expiration: Gauge,
}

impl Metrics {
    pub fn new() -> Self {
        let mut registry = Registry::with_prefix("imds_bridge");
        let requests = Family::<RequestLabels, Counter>::default();
        let refreshes = Family::<RefreshLabels, Counter>::default();
        let expiration = Gauge::default();
        registry.register(
            "requests",
            "Instance metadata requests by route and status code",
            requests.clone(),
        );
        registry.register(
            "credential_refreshes",
            "Credential fetches from the container credentials endpoint",
            refreshes.clone(),
        );
        registry.register(
            "credentials_expiration_timestamp_seconds",
            "Expiration of the cached credentials as a Unix timestamp",
            expiration.clone(),
        );
        Self {
            registry,
            requests,
            refreshes,
            expiration,
        }
    }

    pub fn record_request(&self, path: &str, code: u16) {
        self.requests
            .get_or_create(&RequestLabels {
                path: path.to_string(),
                code,
            })
            .inc();
    }

    pub fn record_refresh(&self, success: bool) {
        let result = if success { "success" } else { "failure" };
        self.refreshes
            .get_or_create(&RefreshLabels { result })
            .inc();
    }

    pub fn set_expiration(&self, unix_seconds: i64) {
        self.expiration.set(unix_seconds);
    }

    pub fn encode(&self) -> String {
        let mut out = String::new();
        if encode(&mut out, &self.registry).is_err() {
            out.clear();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_all_series() {
        let metrics = Metrics::new();
        metrics.record_request("/latest/api/token", 200);
        metrics.record_refresh(true);
        metrics.record_refresh(false);
        metrics.set_expiration(1_795_780_800);
        let text = metrics.encode();
        assert!(
            text.contains(r#"imds_bridge_requests_total{path="/latest/api/token",code="200"} 1"#)
        );
        assert!(text.contains(r#"imds_bridge_credential_refreshes_total{result="success"} 1"#));
        assert!(text.contains(r#"imds_bridge_credential_refreshes_total{result="failure"} 1"#));
        assert!(text.contains("imds_bridge_credentials_expiration_timestamp_seconds 1795780800"));
    }
}
