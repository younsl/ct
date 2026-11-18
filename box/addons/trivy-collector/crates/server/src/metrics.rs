//! Prometheus metrics the server owns: request handling, MCP tool calls, and
//! the authored-state caches.

use std::sync::Arc;

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::Histogram;
use prometheus_client::registry::Registry;

use trivy_collector_runtime::build_info;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct HttpLabels {
    pub method: String,
    pub status: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct HttpDurationLabels {
    pub method: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct McpToolLabels {
    pub tool: String,
    pub result: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct McpToolDurationLabels {
    pub tool: String,
}

const DURATION_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];

pub struct Metrics {
    pub http_requests_total: Family<HttpLabels, Counter>,
    pub http_request_duration_seconds: Family<HttpDurationLabels, Histogram>,
    /// Serialized size of the notes ConfigMap. A ConfigMap caps at roughly
    /// 1MiB, so its headroom is worth watching rather than discovering.
    pub notes_configmap_bytes: Gauge,
    /// API tokens currently held in the Secret-backed store.
    pub api_tokens_total: Gauge,
    pub mcp_tool_calls_total: Family<McpToolLabels, Counter>,
    pub mcp_tool_duration_seconds: Family<McpToolDurationLabels, Histogram>,
    pub mcp_tool_calls_in_flight: Gauge,
}

fn duration_histogram() -> Histogram {
    Histogram::new(DURATION_BUCKETS.iter().copied())
}

impl Metrics {
    pub fn new(registry: &mut Registry) -> Arc<Self> {
        build_info::register_metric(registry, "server");

        let metrics = Self {
            http_requests_total: Family::default(),
            http_request_duration_seconds: Family::new_with_constructor(duration_histogram),
            notes_configmap_bytes: Gauge::default(),
            api_tokens_total: Gauge::default(),
            mcp_tool_calls_total: Family::default(),
            mcp_tool_duration_seconds: Family::new_with_constructor(duration_histogram),
            mcp_tool_calls_in_flight: Gauge::default(),
        };
        registry.register(
            "trivy_collector_http_requests",
            "Total HTTP requests",
            metrics.http_requests_total.clone(),
        );
        registry.register(
            "trivy_collector_http_request_duration_seconds",
            "HTTP request duration in seconds",
            metrics.http_request_duration_seconds.clone(),
        );
        registry.register(
            "trivy_collector_notes_configmap_bytes",
            "Serialized size of the notes ConfigMap in bytes",
            metrics.notes_configmap_bytes.clone(),
        );
        registry.register(
            "trivy_collector_api_tokens",
            "API tokens held in the tokens Secret",
            metrics.api_tokens_total.clone(),
        );
        registry.register(
            "trivy_collector_mcp_tool_calls",
            "Total MCP tool invocations by tool and result",
            metrics.mcp_tool_calls_total.clone(),
        );
        registry.register(
            "trivy_collector_mcp_tool_duration_seconds",
            "MCP tool execution time in seconds, including queueing for a concurrency slot",
            metrics.mcp_tool_duration_seconds.clone(),
        );
        registry.register(
            "trivy_collector_mcp_tool_calls_in_flight",
            "MCP tool invocations currently executing",
            metrics.mcp_tool_calls_in_flight.clone(),
        );
        for method in ["GET", "POST", "PUT", "DELETE"] {
            for status in ["200", "400", "404", "500"] {
                let _ = metrics.http_requests_total.get_or_create(&HttpLabels {
                    method: method.to_string(),
                    status: status.to_string(),
                });
            }
        }
        Arc::new(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_client::encoding::text::encode;

    fn scrape(registry: &Registry) -> String {
        let mut buf = String::new();
        encode(&mut buf, registry).unwrap();
        buf
    }

    #[test]
    fn registers_only_what_the_server_owns() {
        let mut registry = Registry::default();
        let _ = Metrics::new(&mut registry);
        let out = scrape(&registry);
        assert!(out.contains(r#"mode="server""#));
        // The server owns no database, so it publishes no database gauges.
        assert!(!out.contains("trivy_collector_db_"));
    }

    #[test]
    fn counters_start_at_zero_rather_than_absent() {
        let mut registry = Registry::default();
        let _ = Metrics::new(&mut registry);
        let out = scrape(&registry);
        assert!(
            out.contains(r#"trivy_collector_http_requests_total{method="GET",status="200"} 0"#)
        );
        assert!(out.contains("trivy_collector_notes_configmap_bytes 0"));
    }
}
