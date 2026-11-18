//! Wire contract between `RemoteStore` and the scraper's internal API.
//!
//! These serve `api::router` over a real socket and drive it through a real
//! `RemoteStore`. Anything else only proves the client and the server each
//! compile: the point of the store is that the two agree on paths, query
//! names, and response envelopes, and that is a property of the pair rather
//! than of either half.

use std::sync::Arc;

use trivy_collector_core::trivy::ReportPayload;
use trivy_collector_core::*;
use trivy_collector_database::Database;
use trivy_collector_scraper::api::{self, InternalApi};
use trivy_collector_scraper::status::{ReportKind, WatcherStatus};
use trivy_collector_server::remote::RemoteStore;

const TOKEN: &str = "shared-internal-token";

/// A live internal API plus a store pointed at it. Dropping the guard stops
/// the server.
struct Harness {
    store: RemoteStore,
    db: Arc<Database>,
    status: Arc<WatcherStatus>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl Harness {
    async fn start() -> Self {
        Self::start_with_token(TOKEN).await
    }

    async fn start_with_token(client_token: &str) -> Self {
        let db = Arc::new(Database::new(":memory:").await.unwrap());
        let status = Arc::new(WatcherStatus::new());
        let internal = InternalApi::new(db.clone(), status.clone(), TOKEN.to_string());
        let app = api::router(internal);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown, mut rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.changed().await;
                })
                .await;
        });

        let store = RemoteStore::new(&format!("http://{addr}"), client_token).unwrap();
        Self {
            store,
            db,
            status,
            shutdown,
        }
    }

    /// Seed through the local database rather than the API, so a broken
    /// ingest route cannot hide a broken read route.
    async fn seed(&self) {
        for payload in [
            vuln_payload("prod", "default", "nginx-vuln"),
            vuln_payload("stage", "default", "redis-vuln"),
            sbom_payload("prod", "default", "nginx-sbom"),
        ] {
            self.db.upsert_report(&payload).await.unwrap();
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

fn vuln_payload(cluster: &str, namespace: &str, name: &str) -> ReportPayload {
    ReportPayload {
        cluster: cluster.into(),
        namespace: namespace.into(),
        name: name.into(),
        report_type: "vulnerabilityreport".into(),
        data_json: serde_json::json!({
            "metadata": {"labels": {"trivy-operator.resource.name": "web"}},
            "report": {
                "artifact": {"repository": "library/nginx", "tag": "1.25"},
                "registry": {"server": "docker.io"},
                "summary": {
                    "criticalCount": 1, "highCount": 2, "mediumCount": 0,
                    "lowCount": 0, "unknownCount": 0
                },
                "vulnerabilities": [
                    {"vulnerabilityID": "CVE-2024-0001", "severity": "CRITICAL",
                     "score": 9.8, "resource": "openssl",
                     "installedVersion": "1.1.1", "fixedVersion": "1.1.2"},
                    {"vulnerabilityID": "CVE-2024-0002", "severity": "HIGH",
                     "score": 7.5, "resource": "zlib",
                     "installedVersion": "1.2", "fixedVersion": ""}
                ]
            }
        })
        .to_string(),
        received_at: chrono::Utc::now(),
    }
}

fn sbom_payload(cluster: &str, namespace: &str, name: &str) -> ReportPayload {
    ReportPayload {
        cluster: cluster.into(),
        namespace: namespace.into(),
        name: name.into(),
        report_type: "sbomreport".into(),
        data_json: serde_json::json!({
            "metadata": {"labels": {"trivy-operator.resource.name": "web"}},
            "report": {
                "artifact": {"repository": "library/nginx", "tag": "1.25"},
                "registry": {"server": "docker.io"},
                "summary": {"componentsCount": 2},
                "components": {
                    "bomFormat": "CycloneDX",
                    "components": [
                        {"name": "log4j-core", "version": "2.17.1", "type": "library"},
                        {"name": "axios", "version": "1.6.0", "type": "library"}
                    ]
                }
            }
        })
        .to_string(),
        received_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn query_reports_round_trips_through_the_wire() {
    let h = Harness::start().await;
    h.seed().await;

    let (reports, total) = h
        .store
        .query_reports("vulnerabilityreport", &QueryParams::default())
        .await
        .unwrap();
    assert_eq!(total, 2);
    assert_eq!(reports.len(), 2);
    // Severity counts survive the JSON envelope.
    let summary = reports[0].summary.as_ref().unwrap();
    assert_eq!(summary.critical, 1);
    assert_eq!(summary.high, 2);
}

#[tokio::test]
async fn query_filters_reach_the_sql_layer() {
    let h = Harness::start().await;
    h.seed().await;

    // A filter that is dropped on the wire would return both clusters.
    let (reports, total) = h
        .store
        .query_reports(
            "vulnerabilityreport",
            &QueryParams {
                cluster: Some("prod".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(reports[0].cluster, "prod");

    // Severity is sent as a comma-joined list and split server-side.
    let (_, high) = h
        .store
        .query_reports(
            "vulnerabilityreport",
            &QueryParams {
                severity: Some(vec!["critical".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(high, 2);

    // A severity no report carries must narrow the result, proving the
    // filter is applied rather than ignored.
    let (_, none) = h
        .store
        .query_reports(
            "vulnerabilityreport",
            &QueryParams {
                severity: Some(vec!["medium".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(none, 0);
}

#[tokio::test]
async fn pagination_is_carried_across_the_wire() {
    let h = Harness::start().await;
    h.seed().await;

    let (page, total) = h
        .store
        .query_reports(
            "vulnerabilityreport",
            &QueryParams {
                limit: Some(1),
                offset: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.len(), 1, "limit must be honoured");
    assert_eq!(total, 2, "total must count the whole match, not the page");
}

#[tokio::test]
async fn get_report_returns_the_full_data_blob() {
    let h = Harness::start().await;
    h.seed().await;

    let report = h
        .store
        .get_report("prod", "default", "nginx-vuln", "vulnerabilityreport")
        .await
        .unwrap()
        .expect("seeded report");

    assert_eq!(report.meta.cluster, "prod");
    // FullReport serializes `data` as JSON and parses it back into a raw
    // string, so the round trip has to preserve the payload.
    let data: serde_json::Value = serde_json::from_str(&report.data_json).unwrap();
    assert_eq!(
        data["report"]["vulnerabilities"][0]["vulnerabilityID"],
        "CVE-2024-0001"
    );
}

#[tokio::test]
async fn a_missing_report_is_none_rather_than_an_error() {
    let h = Harness::start().await;
    let missing = h
        .store
        .get_report("prod", "default", "nope", "sbomreport")
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn notes_are_never_served_by_the_scraper() {
    let h = Harness::start().await;
    h.seed().await;

    // The scraper's schema has no notes columns; the server joins them in
    // from the ConfigMap. Anything else would mean the split leaked.
    let report = h
        .store
        .get_report("prod", "default", "nginx-vuln", "vulnerabilityreport")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.meta.notes, "");
    assert!(report.meta.notes_created_at.is_none());

    let (reports, _) = h
        .store
        .query_reports("vulnerabilityreport", &QueryParams::default())
        .await
        .unwrap();
    assert!(reports.iter().all(|r| r.notes.is_empty()));
}

#[tokio::test]
async fn aggregates_round_trip() {
    let h = Harness::start().await;
    h.seed().await;

    let stats = h.store.get_stats().await.unwrap();
    assert_eq!(stats.total_clusters, 2);
    assert_eq!(stats.total_vuln_reports, 2);
    assert_eq!(stats.total_sbom_reports, 1);
    assert_eq!(stats.total_critical, 2);

    let clusters = h.store.list_clusters().await.unwrap();
    assert_eq!(clusters.len(), 2);

    let namespaces = h.store.list_namespaces(None).await.unwrap();
    assert_eq!(namespaces, vec!["default".to_string()]);

    // The cluster filter is a query parameter, so a dropped one would
    // still return every namespace.
    let scoped = h.store.list_namespaces(Some("prod")).await.unwrap();
    assert_eq!(scoped, vec!["default".to_string()]);
    assert_eq!(
        h.store.list_namespaces(Some("nonexistent")).await.unwrap(),
        [] as [std::string::String; 0]
    );
}

#[tokio::test]
async fn search_and_suggest_round_trip() {
    let h = Harness::start().await;
    h.seed().await;

    let (vulns, total) = h
        .store
        .search_vulnerabilities("CVE-2024", 10, 0)
        .await
        .unwrap();
    assert_eq!(total, 4, "two CVEs across two vulnerability reports");
    assert!(vulns.iter().any(|v| v.vulnerability_id == "CVE-2024-0001"));

    let (components, total) = h
        .store
        .search_sbom_components("log4j", 10, 0)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(components[0].component_name, "log4j-core");
    assert_eq!(components[0].component_version, "2.17.1");

    let ids = h.store.suggest_vulnerability_ids("CVE", 10).await.unwrap();
    assert!(ids.contains(&"CVE-2024-0001".to_string()));

    let names = h.store.suggest_component_names("axi", 10).await.unwrap();
    assert_eq!(names, vec!["axios".to_string()]);
}

#[tokio::test]
async fn component_matches_carry_the_alert_matcher_filters() {
    let h = Harness::start().await;
    h.seed().await;

    let all = h
        .store
        .list_sbom_component_matches(&[], None, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // package_name is pushed into SQL, so a dropped filter would return
    // both components instead of one.
    let scoped = h
        .store
        .list_sbom_component_matches(&["prod".to_string()], Some("default"), Some("log4j-core"))
        .await
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].version, "2.17.1");
    assert_eq!(scoped[0].workload_name, "nginx-sbom");

    // A cluster with no SBOM reports must come back empty.
    assert!(
        h.store
            .list_sbom_component_matches(&["stage".to_string()], None, None)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Every filter is pushed into SQL on the far side, so a field dropped on
/// the wire would widen the result rather than fail.
#[tokio::test]
async fn vulnerability_matches_carry_every_filter() {
    let h = Harness::start().await;
    h.seed().await;

    let all = h
        .store
        .list_vulnerability_matches(&VulnerabilityMatchFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 4, "two vulnerabilities in each of two reports");

    let critical = h
        .store
        .list_vulnerability_matches(&VulnerabilityMatchFilter {
            clusters: vec!["prod".to_string()],
            severities: vec!["critical".to_string()],
            min_score: Some(9.0),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(critical.len(), 1);
    assert_eq!(critical[0].vulnerability_id, "CVE-2024-0001");
    assert_eq!(critical[0].score, Some(9.8));
    assert_eq!(critical[0].package_name, "openssl");
    assert_eq!(critical[0].fixed_version, "1.1.2");
    assert_eq!(critical[0].workload_name, "nginx-vuln");

    let above_threshold = h
        .store
        .list_vulnerability_matches(&VulnerabilityMatchFilter {
            min_score: Some(9.9),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(above_threshold.is_empty());

    let unfixed = h
        .store
        .list_vulnerability_matches(&VulnerabilityMatchFilter {
            fix_available: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(unfixed.len(), 2);
    assert!(unfixed.iter().all(|m| m.package_name == "zlib"));

    let by_id = h
        .store
        .list_vulnerability_matches(&VulnerabilityMatchFilter {
            ids: vec!["cve-2024-0002".to_string()],
            package_name: Some("ZLIB".to_string()),
            namespace: Some("default".to_string()),
            fix_available: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(by_id.len(), 2);
}

#[tokio::test]
async fn dashboard_endpoints_round_trip() {
    let h = Harness::start().await;
    h.seed().await;

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let trends = h
        .store
        .get_live_trends(&today, &today, None, "day")
        .await
        .unwrap();
    assert_eq!(trends.meta.granularity, "day");

    let (from, to) = h.store.get_reports_data_range().await.unwrap();
    assert!(
        from.is_some(),
        "seeded reports give the range a lower bound"
    );
    assert!(to.is_some());
}

#[tokio::test]
async fn hydration_reflects_the_scrapers_own_state() {
    let h = Harness::start().await;

    let empty = h.store.hydration().await.unwrap();
    assert!(!empty.hydrated, "an empty fleet has confirmed nothing");

    h.status.register_cluster("prod");
    h.status
        .set_sync_done("prod", ReportKind::Vulnerability, true);
    let partial = h.store.hydration().await.unwrap();
    assert!(!partial.hydrated);
    assert!(partial.clusters.contains_key("prod"));

    h.status.set_sync_done("prod", ReportKind::Sbom, true);
    assert!(h.store.hydration().await.unwrap().hydrated);
}

#[tokio::test]
async fn writes_round_trip_through_the_ingest_route() {
    let h = Harness::start().await;

    h.store
        .upsert_report(&sbom_payload("prod", "default", "pushed"))
        .await
        .unwrap();
    assert!(
        h.store
            .get_report("prod", "default", "pushed", "sbomreport")
            .await
            .unwrap()
            .is_some()
    );

    assert!(
        h.store
            .delete_report("prod", "default", "pushed", "sbomreport")
            .await
            .unwrap()
    );
    assert!(
        h.store
            .get_report("prod", "default", "pushed", "sbomreport")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn deleting_an_absent_report_is_false_not_an_error() {
    let h = Harness::start().await;
    assert!(
        !h.store
            .delete_report("prod", "default", "nope", "sbomreport")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn deleting_a_cluster_reports_the_row_count() {
    let h = Harness::start().await;
    h.seed().await;

    // prod holds one vulnerability report and one SBOM report.
    assert_eq!(h.store.delete_reports_for_cluster("prod").await.unwrap(), 2);
    assert_eq!(h.store.list_clusters().await.unwrap().len(), 1);
    assert_eq!(h.store.delete_reports_for_cluster("gone").await.unwrap(), 0);
}

#[tokio::test]
async fn identities_that_need_encoding_survive_the_round_trip() {
    let h = Harness::start().await;
    // Cluster names come from a registration Secret and are not guaranteed
    // path-safe, so the encoding has to hold end to end.
    let payload = sbom_payload("prod/eu", "kube-system", "a b");
    h.db.upsert_report(&payload).await.unwrap();

    let report = h
        .store
        .get_report("prod/eu", "kube-system", "a b", "sbomreport")
        .await
        .unwrap()
        .expect("encoded identity must resolve");
    assert_eq!(report.meta.cluster, "prod/eu");
    assert_eq!(report.meta.name, "a b");

    assert_eq!(
        h.store.delete_reports_for_cluster("prod/eu").await.unwrap(),
        1
    );
}

#[tokio::test]
async fn a_wrong_token_fails_every_read() {
    let h = Harness::start_with_token("not-the-token").await;

    let err = h.store.get_stats().await.expect_err("must be refused");
    assert!(err.to_string().contains("401"), "got: {err}");

    // A refused read must not look like an empty answer.
    assert!(h.store.list_clusters().await.is_err());
    assert!(h.store.hydration().await.is_err());
    assert!(
        h.store
            .get_report("prod", "default", "nginx-vuln", "vulnerabilityreport")
            .await
            .is_err(),
        "a 401 must not be reported as a missing report"
    );
}

#[tokio::test]
async fn an_unreachable_scraper_is_an_error_not_an_empty_answer() {
    // Port 1 is reserved and refuses connections, so this exercises the
    // transport-failure path rather than a 500 from a live scraper.
    let store = RemoteStore::new("http://127.0.0.1:1", "shh").unwrap();
    let err = store.get_stats().await.expect_err("must not succeed");
    assert!(err.to_string().contains("Internal API request"));
}
