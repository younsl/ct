//! Prometheus metrics the scraper owns: the database it mirrors reports into
//! and the hydration of the fleet it watches.

use std::sync::Arc;

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;

use trivy_collector_database::Database;
use trivy_collector_runtime::build_info;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ReportTypeLabels {
    pub report_type: String,
}

/// The two report types, pre-initialized so a fresh scrape shows zeros rather
/// than "no data".
pub const REPORT_TYPES: [&str; 2] = ["vulnerabilityreport", "sbomreport"];

pub struct Metrics {
    pub db_size_bytes: Gauge,
    pub db_reports_total: Family<ReportTypeLabels, Gauge>,
    /// Clusters registered with the scraper.
    pub clusters_total: Gauge,
    /// 1 once every registered cluster has replayed its initial list. Reads on
    /// an unhydrated fleet are legitimately partial.
    pub fleet_hydrated: Gauge,
}

impl Metrics {
    pub fn new(registry: &mut Registry) -> Arc<Self> {
        build_info::register_metric(registry, "scraper");

        let metrics = Self {
            db_size_bytes: Gauge::default(),
            db_reports_total: Family::default(),
            clusters_total: Gauge::default(),
            fleet_hydrated: Gauge::default(),
        };
        registry.register(
            "trivy_collector_db_size_bytes",
            "SQLite database file size in bytes",
            metrics.db_size_bytes.clone(),
        );
        registry.register(
            "trivy_collector_db_reports",
            "Reports currently mirrored into the database",
            metrics.db_reports_total.clone(),
        );
        registry.register(
            "trivy_collector_clusters",
            "Clusters registered with the scraper",
            metrics.clusters_total.clone(),
        );
        registry.register(
            "trivy_collector_fleet_hydrated",
            "1 when every registered cluster has finished its initial sync",
            metrics.fleet_hydrated.clone(),
        );
        for rt in REPORT_TYPES {
            let _ = metrics.db_reports_total.get_or_create(&ReportTypeLabels {
                report_type: rt.to_string(),
            });
        }
        Arc::new(metrics)
    }

    /// Refresh the gauges that describe the database.
    pub async fn refresh_db_gauges(&self, db: &Database) {
        let (size_bytes, _) = db.db_size().await;
        self.db_size_bytes.set(size_bytes as i64);
        for rt in REPORT_TYPES {
            if let Ok(count) = db.count_reports(rt).await {
                self.db_reports_total
                    .get_or_create(&ReportTypeLabels {
                        report_type: rt.to_string(),
                    })
                    .set(count);
            }
        }
    }

    /// Publish the fleet's hydration state.
    pub fn record_hydration(&self, clusters: usize, hydrated: bool) {
        self.clusters_total.set(clusters as i64);
        self.fleet_hydrated.set(i64::from(hydrated));
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
    fn registers_only_what_the_scraper_owns() {
        let mut registry = Registry::default();
        let _ = Metrics::new(&mut registry);
        let out = scrape(&registry);
        assert!(out.contains(r#"mode="scraper""#));
        assert!(!out.contains("trivy_collector_http_requests"));
    }

    #[test]
    fn gauges_start_at_zero_rather_than_absent() {
        let mut registry = Registry::default();
        let _ = Metrics::new(&mut registry);
        let out = scrape(&registry);
        assert!(out.contains(r#"trivy_collector_db_reports{report_type="sbomreport"} 0"#));
        assert!(out.contains("trivy_collector_fleet_hydrated 0"));
    }

    #[test]
    fn record_hydration_publishes_cluster_count_and_state() {
        let mut registry = Registry::default();
        let metrics = Metrics::new(&mut registry);
        metrics.record_hydration(3, true);
        let out = scrape(&registry);
        assert!(out.contains("trivy_collector_clusters 3"));
        assert!(out.contains("trivy_collector_fleet_hydrated 1"));
    }

    #[tokio::test]
    async fn refresh_db_gauges_reads_report_counts() {
        let mut registry = Registry::default();
        let metrics = Metrics::new(&mut registry);
        let db = Database::new(":memory:").await.unwrap();
        metrics.refresh_db_gauges(&db).await;
        let out = scrape(&registry);
        assert!(out.contains(r#"trivy_collector_db_reports{report_type="vulnerabilityreport"} 0"#));
        // The pragma-based size works on `:memory:` too, so the gauge is set.
        assert!(!out.contains("trivy_collector_db_size_bytes 0\n"));
    }
}
