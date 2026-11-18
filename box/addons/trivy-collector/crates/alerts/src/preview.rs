//! Run a rule's matchers against already-stored reports without firing
//! receivers. Used by the rule editor to show what current data would match.
//!
//! Rows come from `findings::fetch`, so every stored report contributes and
//! preview applies exactly the predicate the evaluator does.

use serde::Serialize;
use utoipa::ToSchema;

use super::findings;
use super::matchers::CompiledRule;
use trivy_collector_alertrule_crd::types::Matchers;
use trivy_collector_core::ReportStore;

const MAX_MATCHES: usize = 50;

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PreviewMatch {
    pub cluster: String,
    pub namespace: String,
    pub name: String,
    pub package: String,
    pub version: String,
    /// Set for a vulnerability rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vulnerability_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Empty when no fix is published. Absent for a component rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_version: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PreviewResult {
    pub items: Vec<PreviewMatch>,
    pub total: usize,
    pub truncated: bool,
    /// Number of distinct workloads (cluster+namespace+name) contributing
    /// at least one match. Field name kept as `scanned_reports` for frontend
    /// compatibility — the prior implementation populated this with the size
    /// of a 200-row sample window, which no longer exists.
    #[serde(rename = "scanned_reports")]
    pub matched_workloads: usize,
}

pub async fn run(
    db: &dyn ReportStore,
    matchers: Option<&Matchers>,
    terms: &[Matchers],
) -> Result<PreviewResult, String> {
    let compiled = CompiledRule::compile(matchers, terms).map_err(|e| e.to_string())?;
    let rows = findings::fetch(db, &compiled)
        .await
        .map_err(|e| e.to_string())?;

    let total = rows.len();
    let matched_workloads = rows
        .iter()
        .map(findings::WorkloadFinding::workload_key)
        .collect::<std::collections::HashSet<_>>()
        .len();
    let items: Vec<PreviewMatch> = rows
        .into_iter()
        .take(MAX_MATCHES)
        .map(|row| {
            let v = row.finding.vulnerability;
            PreviewMatch {
                cluster: row.cluster,
                namespace: row.namespace,
                name: row.workload,
                package: row.finding.package,
                version: row.finding.version,
                vulnerability_id: v.as_ref().map(|v| v.id.clone()),
                severity: v.as_ref().map(|v| v.severity.clone()),
                score: v.as_ref().and_then(|v| v.score),
                fixed_version: v.map(|v| v.fixed_version),
            }
        })
        .collect();

    Ok(PreviewResult {
        truncated: total > items.len(),
        matched_workloads,
        items,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use trivy_collector_core::trivy::ReportPayload;
    use trivy_collector_database::Database;

    fn sbom_payload(
        cluster: &str,
        namespace: &str,
        name: &str,
        components: &serde_json::Value,
    ) -> ReportPayload {
        ReportPayload {
            cluster: cluster.to_string(),
            namespace: namespace.to_string(),
            name: name.to_string(),
            report_type: "sbomreport".to_string(),
            data_json: json!({
                "metadata": {"labels": {}},
                "report": {
                    "artifact": {"repository": "app", "tag": "v1"},
                    "registry": {"server": "ghcr.io"},
                    "summary": {"componentsCount": 1},
                    "components": {
                        "bomFormat": "CycloneDX",
                        "specVersion": "1.5",
                        "components": components,
                    }
                }
            })
            .to_string(),
            received_at: chrono::Utc::now(),
        }
    }

    async fn seed_db() -> Database {
        let db = Database::new(":memory:")
            .await
            .expect("Failed to create database");
        db.upsert_report(&sbom_payload(
            "prod",
            "default",
            "node-app",
            &json!([
                {"type": "library", "name": "axios", "version": "1.6.0"},
                {"type": "library", "name": "axios", "version": "0.27.2"},
            ]),
        ))
        .await
        .unwrap();
        db.upsert_report(&sbom_payload(
            "prod",
            "team-a",
            "other-app",
            &json!([{"type": "library", "name": "axios", "version": "1.6.0"}]),
        ))
        .await
        .unwrap();
        db.upsert_report(&sbom_payload(
            "prod",
            "default",
            "no-axios",
            &json!([{"type": "library", "name": "lodash", "version": "4.17.21"}]),
        ))
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn preview_matches_axios_across_workloads() {
        let db = seed_db().await;
        let matchers = Matchers {
            package_name: Some("axios".to_string()),
            version_expr: None,
            clusters: vec![],
            namespace: None,
            ..Default::default()
        };
        let result = run(&db, Some(&matchers), &[]).await.unwrap();
        // 2 versions in node-app + 1 in other-app
        assert_eq!(result.total, 3);
        assert_eq!(result.items.len(), 3);
        assert!(!result.truncated);
        // Two distinct workloads contribute matches.
        assert_eq!(result.matched_workloads, 2);
    }

    #[tokio::test]
    async fn preview_applies_version_expr_in_app_layer() {
        let db = seed_db().await;
        let matchers = Matchers {
            package_name: Some("axios".to_string()),
            version_expr: Some(">=1.0.0".to_string()),
            clusters: vec![],
            namespace: None,
            ..Default::default()
        };
        let result = run(&db, Some(&matchers), &[]).await.unwrap();
        // Only the 1.6.0 entries survive; 0.27.2 is filtered.
        assert_eq!(result.total, 2);
        assert!(result.items.iter().all(|m| m.version == "1.6.0"));
    }

    #[tokio::test]
    async fn preview_namespace_filter_narrows_rows() {
        let db = seed_db().await;
        let matchers = Matchers {
            package_name: Some("axios".to_string()),
            version_expr: None,
            clusters: vec![],
            namespace: Some("team-a".to_string()),
            ..Default::default()
        };
        let result = run(&db, Some(&matchers), &[]).await.unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].namespace, "team-a");
    }

    #[tokio::test]
    async fn preview_invalid_version_expr_returns_error() {
        let db = seed_db().await;
        let matchers = Matchers {
            package_name: Some("axios".to_string()),
            version_expr: Some(">=".to_string()),
            clusters: vec![],
            namespace: None,
            ..Default::default()
        };
        let err = run(&db, Some(&matchers), &[]).await.unwrap_err();
        assert!(err.starts_with("matchers.version_expr:"));
    }

    #[tokio::test]
    async fn preview_truncates_when_total_exceeds_max() {
        let db = Database::new(":memory:").await.unwrap();
        // Build a single SBOM with MAX_MATCHES + 5 distinct axios versions
        // so we exercise the truncated=true branch without seeding 50
        // separate workloads.
        let components: Vec<serde_json::Value> = (0..(MAX_MATCHES + 5))
            .map(|i| json!({"type": "library", "name": "axios", "version": format!("1.{}.0", i)}))
            .collect();
        db.upsert_report(&sbom_payload(
            "prod",
            "default",
            "fat-app",
            &json!(components),
        ))
        .await
        .unwrap();
        let matchers = Matchers {
            package_name: Some("axios".to_string()),
            version_expr: None,
            clusters: vec![],
            namespace: None,
            ..Default::default()
        };
        let result = run(&db, Some(&matchers), &[]).await.unwrap();
        assert_eq!(result.total, MAX_MATCHES + 5);
        assert_eq!(result.items.len(), MAX_MATCHES);
        assert!(result.truncated);
        assert_eq!(result.matched_workloads, 1);
    }

    #[tokio::test]
    async fn preview_lists_vulnerabilities_above_a_score() {
        let db = Database::new(":memory:").await.unwrap();
        db.upsert_report(&ReportPayload {
            cluster: "prod".to_string(),
            namespace: "payments".to_string(),
            name: "api".to_string(),
            report_type: "vulnerabilityreport".to_string(),
            data_json: json!({"report": {"vulnerabilities": [
                {"vulnerabilityID": "CVE-2021-44228", "resource": "log4j-core",
                 "installedVersion": "2.14.1", "fixedVersion": "2.15.0",
                 "severity": "CRITICAL", "score": 10.0},
                {"vulnerabilityID": "CVE-2022-0001", "resource": "zlib",
                 "installedVersion": "1.2.11", "severity": "MEDIUM", "score": 5.3},
            ]}})
            .to_string(),
            received_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
        let matchers = Matchers {
            vulnerability: Some(crate::VulnerabilityMatchers {
                min_score: Some(9.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = run(&db, Some(&matchers), &[]).await.unwrap();
        assert_eq!(result.total, 1);
        let m = &result.items[0];
        assert_eq!(m.vulnerability_id.as_deref(), Some("CVE-2021-44228"));
        assert_eq!(m.severity.as_deref(), Some("CRITICAL"));
        assert_eq!(m.score, Some(10.0));
        assert_eq!(m.fixed_version.as_deref(), Some("2.15.0"));
        assert_eq!(m.package, "log4j-core");
    }
}
