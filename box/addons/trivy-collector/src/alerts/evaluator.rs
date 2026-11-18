//! Match an incoming report against alert rules and dispatch to receivers.
//! Rules are re-listed from the API server on each evaluation pass, and a
//! per-rule, per-target cooldown prevents Slack flooding when reports arrive
//! frequently. Matches from a single report are collected per rule and
//! dispatched as one grouped message to avoid per-finding fan-out.
//!
//! A rule watches SBOM reports or vulnerability reports, never both, as its
//! matchers decide. See `alerts::matchers`.
//!
//! What happened is written back to each rule's status subresource, but only
//! when it changed: a firing, or a `Ready` transition. A write per rule per
//! report would put the API server on the ingest path.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::{debug, error};

use super::findings::{self, Finding, ReportKind, WorkloadFinding};
use super::matchers::{CompiledRule, MatcherError};
use super::notifier::{AlertContext, SlackNotifier, TestDeliveryResult};
use super::store::{AlertStore, FiringOutcome};
use super::types::AlertRule;
use crate::collector::types::ReportPayload;
use crate::storage::ReportStore;

const DEFAULT_COOLDOWN_SECS: u64 = 3600;

#[derive(Clone)]
pub struct AlertEvaluator {
    store: AlertStore,
    notifier: SlackNotifier,
    cooldown: Arc<Mutex<HashMap<String, Instant>>>,
}

impl AlertEvaluator {
    #[must_use]
    pub fn new(store: AlertStore) -> Self {
        Self::with_external_url(store, None)
    }

    #[must_use]
    pub fn with_external_url(store: AlertStore, external_url: Option<String>) -> Self {
        Self {
            store,
            notifier: SlackNotifier::with_external_url(external_url),
            cooldown: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[must_use]
    pub const fn store(&self) -> &AlertStore {
        &self.store
    }

    /// Send a test alert built from real reports already stored in the DB.
    /// The message is indistinguishable from a production firing so the
    /// operator sees what an actual alert will look like rather than mock
    /// placeholders. A test-mode footer is appended noting how many other
    /// workloads would also fire if real ingestion happened, so the
    /// operator can gauge fleet-wide impact without flooding the channel.
    /// Returns `Err(TestRunError::NoMatches)` when no current report
    /// matches the rule's matchers — surfaced as 422 so the operator can
    /// adjust matchers or wait for a matching report.
    pub async fn test_with_rule(
        &self,
        rule: AlertRule,
        db: &dyn ReportStore,
    ) -> Result<Vec<TestDeliveryResult>, TestRunError> {
        let (contexts, other_workloads) = build_test_contexts(db, &rule).await?;
        Ok(self
            .notifier
            .send_test(&rule, &contexts, other_workloads)
            .await)
    }

    /// Evaluate a freshly received report against every enabled rule that
    /// watches its report kind. `db` is used to compute a fleet-wide "also
    /// matches N workloads" scope hint that gets surfaced on the outbound
    /// message.
    pub async fn evaluate(
        &self,
        payload: &ReportPayload,
        prev_data_json: Option<&str>,
        db: &dyn ReportStore,
    ) {
        let Some(kind) = ReportKind::from_report_type(&payload.report_type) else {
            return;
        };
        let rules = match self.store.list().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "Failed to load alert rules");
                return;
            }
        };
        // Readiness is not reconciled here. It is a property of the rule, not
        // of the report in hand, and this path only runs when a report arrives
        // — after hydration that is whenever Trivy Operator next rescans. See
        // `alerts::readiness`, which watches the rules instead. A rule that
        // does not compile is reported there, so this only declines to match.
        let candidates: Vec<(&AlertRule, CompiledRule)> = rules
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|r| CompiledRule::for_rule(r).ok().map(|m| (r, m)))
            .filter(|(_, m)| {
                !m.applicable(kind, &payload.cluster, &payload.namespace)
                    .is_empty()
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        let Some(findings) = findings::from_report(kind, &payload.data_json) else {
            debug!(report_type = %payload.report_type, "Failed to parse report payload");
            return;
        };
        // Keys from the previous revision, computed once per pass. Empty when
        // it is missing or unparseable, which treats every match as new.
        let prev_keys = prev_data_json.map_or_default(|p| findings::keys(kind, p));
        for (rule, matchers) in candidates {
            self.evaluate_rule(rule, &matchers, payload, &findings, &prev_keys, db)
                .await;
        }
    }

    async fn evaluate_rule(
        &self,
        rule: &AlertRule,
        matchers: &CompiledRule,
        payload: &ReportPayload,
        findings: &[Finding],
        prev_keys: &HashSet<String>,
        db: &dyn ReportStore,
    ) {
        let mut contexts: Vec<AlertContext> = Vec::new();
        // Dedup within this single report. Trivy can list the same package
        // twice with different `bom-ref` values when an image has multiple
        // binaries that each link the same library, and the same CVE twice
        // for different targets — surfacing both as separate rows is noise.
        let mut seen: HashSet<String> = HashSet::new();
        for finding in findings
            .iter()
            .filter(|f| matchers.matches(&payload.cluster, &payload.namespace, f))
        {
            let finding_key = finding.key();
            if !seen.insert(finding_key.clone()) {
                continue;
            }
            // Diff-aware: skip findings already present in the prior revision.
            if prev_keys.contains(&finding_key) {
                continue;
            }
            let key = format!(
                "{}|{}|{}|{}|{}",
                rule.name, payload.cluster, payload.namespace, payload.name, finding_key
            );
            if !self.try_acquire(&key, rule.cooldown_secs).await {
                continue;
            }
            contexts.push(AlertContext::new(
                &payload.cluster,
                &payload.namespace,
                &payload.name,
                finding.clone(),
            ));
        }
        if contexts.is_empty() {
            return;
        }
        // Count other workloads in the DB that would also match this rule.
        // Surfaced on the message so a recipient can immediately tell
        // whether this is an isolated finding or fleet-wide.
        let other_workloads = count_other_matching_workloads(
            db,
            matchers,
            &payload.cluster,
            &payload.namespace,
            &payload.name,
        )
        .await
        .unwrap_or(0);
        let outcome = self.notifier.fire(rule, &contexts, other_workloads).await;
        self.record_firing(rule, payload, contexts.len(), other_workloads, &outcome)
            .await;
    }

    /// Put the firing on the rule's status subresource. Best-effort: the
    /// notification has already gone out, and losing the record of it must not
    /// look like a failed alert.
    async fn record_firing(
        &self,
        rule: &AlertRule,
        payload: &ReportPayload,
        finding_count: usize,
        other_workloads: usize,
        outcome: &super::notifier::FireOutcome,
    ) {
        let workload = format!("{}/{}/{}", payload.cluster, payload.namespace, payload.name);
        let result = self
            .store
            .record_firing(
                &rule.name,
                rule.status.as_ref(),
                FiringOutcome {
                    workload: &workload,
                    finding_count: finding_count as u32,
                    // The scope hint counts the others; the blast radius
                    // includes the workload that just fired.
                    matching_workloads: other_workloads.saturating_add(1) as u32,
                    generation: rule.generation,
                    failure: outcome
                        .all_failed()
                        .then_some(())
                        .and(outcome.last_error.as_deref()),
                },
            )
            .await;
        if let Err(e) = result {
            debug!(rule = %rule.name, error = %e, "Failed to record alert firing on the rule status");
        }
    }

    async fn try_acquire(&self, key: &str, cooldown_secs: Option<u64>) -> bool {
        let cd = Duration::from_secs(cooldown_secs.unwrap_or(DEFAULT_COOLDOWN_SECS));
        let now = Instant::now();
        let mut map = self.cooldown.lock().await;
        if let Some(prev) = map.get(key)
            && now.duration_since(*prev) < cd
        {
            return false;
        }
        map.insert(key.to_string(), now);
        if map.len() > 4096 {
            map.retain(|_, t| now.duration_since(*t) < cd);
        }
        true
    }
}

const TEST_MAX_FINDINGS: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum TestRunError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("no current reports match the rule's matchers")]
    NoMatches,
    #[error("invalid matcher: {0}")]
    InvalidMatchers(#[from] MatcherError),
}

/// Pick the most recent workload that has at least one finding satisfying
/// the rule and return up to `TEST_MAX_FINDINGS` deduped real
/// `AlertContext`s from it. Mirrors the production evaluation shape (one
/// report → grouped findings) so a test message is indistinguishable from
/// a real firing. Returns (contexts of the first matching workload, number
/// of OTHER workloads that would also fire on real ingestion). Workload
/// recency comes from `findings::fetch`, which returns the most recently
/// updated workload first.
async fn build_test_contexts(
    db: &dyn ReportStore,
    rule: &AlertRule,
) -> Result<(Vec<AlertContext>, usize), TestRunError> {
    let compiled = CompiledRule::for_rule(rule)?;
    let rows = findings::fetch(db, &compiled)
        .await
        .map_err(|e| TestRunError::Storage(e.to_string()))?;

    let Some(first) = rows.first().map(WorkloadFinding::workload_key) else {
        return Err(TestRunError::NoMatches);
    };
    let mut contexts: Vec<AlertContext> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut other_workloads: HashSet<(String, String, String)> = HashSet::new();
    for row in rows {
        let key = row.workload_key();
        if key != first {
            other_workloads.insert(key);
            continue;
        }
        if seen.insert(row.finding.key()) && contexts.len() < TEST_MAX_FINDINGS {
            contexts.push(AlertContext::new(
                &row.cluster,
                &row.namespace,
                &row.workload,
                row.finding,
            ));
        }
    }
    Ok((contexts, other_workloads.len()))
}

/// Count distinct workloads (other than the firing one) with at least one
/// finding matching the rule. Used to render the "also matches N other
/// workload(s)" scope hint on production firings.
///
/// Errors are swallowed (caller defaults to 0) — the scope hint is best-
/// effort context, not a hard requirement of dispatch.
async fn count_other_matching_workloads(
    db: &dyn ReportStore,
    matchers: &CompiledRule,
    exclude_cluster: &str,
    exclude_namespace: &str,
    exclude_name: &str,
) -> Option<usize> {
    let rows = findings::fetch(db, matchers).await.ok()?;
    let workloads: HashSet<(String, String, String)> = rows
        .iter()
        .filter(|r| {
            !(r.cluster == exclude_cluster
                && r.namespace == exclude_namespace
                && r.workload == exclude_name)
        })
        .map(WorkloadFinding::workload_key)
        .collect();
    Some(workloads.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;
    use std::collections::BTreeMap;

    use crate::alerts::types::{Matchers, Receiver, Severity, VulnerabilityMatchers};
    use crate::collector::types::ReportPayload;
    use serde_json::json;

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

    fn rule_for_axios(version_expr: Option<&str>) -> AlertRule {
        AlertRule {
            name: "axios-rule".to_string(),
            description: String::new(),
            enabled: true,
            matchers: Some(Matchers {
                package_name: Some("axios".to_string()),
                version_expr: version_expr.map(std::string::ToString::to_string),
                clusters: vec![],
                namespace: None,
                ..Default::default()
            }),
            matcher_terms: vec![],
            labels: BTreeMap::default(),
            annotations: BTreeMap::default(),
            receivers: vec![Receiver {
                name: "noop".to_string(),
                slack: None,
            }],
            cooldown_secs: None,
            created_at: "2026-04-27T00:00:00Z".to_string(),
            created_by: "test".to_string(),
            updated_at: None,
            updated_by: None,
            generation: None,
            status: None,
        }
    }

    async fn seed_db_with_axios_fleet() -> Database {
        let db = Database::new(":memory:")
            .await
            .expect("Failed to create database");
        // Workload A: two axios versions (dedup target — only one row per
        // (name, version) should reach the test contexts).
        db.upsert_report(&sbom_payload(
            "prod",
            "default",
            "node-app-a",
            &json!([
                {"type": "library", "name": "axios", "version": "1.6.0"},
                {"type": "library", "name": "axios", "version": "1.6.0"},
                {"type": "library", "name": "axios", "version": "0.27.2"},
            ]),
        ))
        .await
        .unwrap();
        // Workload B: also has axios — should count toward "other workloads".
        db.upsert_report(&sbom_payload(
            "prod",
            "team-b",
            "node-app-b",
            &json!([{"type": "library", "name": "axios", "version": "1.6.0"}]),
        ))
        .await
        .unwrap();
        // Workload C: no axios — must not contribute to either count.
        db.upsert_report(&sbom_payload(
            "prod",
            "team-c",
            "no-axios",
            &json!([{"type": "library", "name": "lodash", "version": "4.17.21"}]),
        ))
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn build_test_contexts_groups_first_workload_and_counts_others() {
        let db = seed_db_with_axios_fleet().await;
        let rule = rule_for_axios(None);
        let (contexts, other_workloads) = build_test_contexts(&db, &rule).await.unwrap();

        // The first workload (most-recent updated_at — workload C is newest
        // but doesn't match; rows iterate in updated_at DESC) contributes
        // deduped contexts; whichever workload it is, it must have exactly
        // its distinct (name, version) pairs.
        assert!(!contexts.is_empty());
        let workload_keys: std::collections::HashSet<_> = contexts
            .iter()
            .map(|c| (c.cluster.clone(), c.namespace.clone(), c.name.clone()))
            .collect();
        assert_eq!(
            workload_keys.len(),
            1,
            "all contexts must come from a single workload"
        );

        // All contexts should be axios.
        assert!(contexts.iter().all(|c| c.finding.package == "axios"));
        // Other workloads count = matching workloads minus the chosen one.
        // 2 workloads match axios → other = 1.
        assert_eq!(other_workloads, 1);
    }

    #[tokio::test]
    async fn build_test_contexts_dedups_by_name_version() {
        let db = Database::new(":memory:").await.unwrap();
        db.upsert_report(&sbom_payload(
            "prod",
            "default",
            "dup-app",
            &json!([
                {"type": "library", "name": "axios", "version": "1.6.0"},
                {"type": "library", "name": "axios", "version": "1.6.0"},
                {"type": "library", "name": "axios", "version": "1.6.0"},
            ]),
        ))
        .await
        .unwrap();
        let rule = rule_for_axios(None);
        let (contexts, _) = build_test_contexts(&db, &rule).await.unwrap();
        assert_eq!(contexts.len(), 1);
    }

    #[tokio::test]
    async fn build_test_contexts_returns_no_matches_when_empty() {
        let db = Database::new(":memory:").await.unwrap();
        db.upsert_report(&sbom_payload(
            "prod",
            "default",
            "no-axios",
            &json!([{"type": "library", "name": "lodash", "version": "4.17.21"}]),
        ))
        .await
        .unwrap();
        let rule = rule_for_axios(None);
        let err = build_test_contexts(&db, &rule).await.unwrap_err();
        assert!(matches!(err, TestRunError::NoMatches));
    }

    #[tokio::test]
    async fn build_test_contexts_invalid_expr_surfaces() {
        let db = Database::new(":memory:").await.unwrap();
        let rule = rule_for_axios(Some(">="));
        let err = build_test_contexts(&db, &rule).await.unwrap_err();
        assert!(matches!(err, TestRunError::InvalidMatchers(_)));
    }

    #[tokio::test]
    async fn count_other_matching_workloads_excludes_firing_one() {
        let db = seed_db_with_axios_fleet().await;
        let matchers = CompiledRule::for_rule(&rule_for_axios(None)).unwrap();
        // Pretend node-app-a fired — count the other matching workloads.
        let n = count_other_matching_workloads(&db, &matchers, "prod", "default", "node-app-a")
            .await
            .unwrap();
        assert_eq!(n, 1);

        // If we exclude a non-existent workload the firing-one filter is a
        // no-op and we get the full matching count (2).
        let n_all = count_other_matching_workloads(&db, &matchers, "prod", "ghost", "ghost")
            .await
            .unwrap();
        assert_eq!(n_all, 2);
    }

    fn vuln_payload(namespace: &str, name: &str, vulns: &serde_json::Value) -> ReportPayload {
        ReportPayload {
            cluster: "prod".to_string(),
            namespace: namespace.to_string(),
            name: name.to_string(),
            report_type: "vulnerabilityreport".to_string(),
            data_json: json!({
                "metadata": {"labels": {}},
                "report": {
                    "artifact": {"repository": "app", "tag": "v1"},
                    "registry": {"server": "ghcr.io"},
                    "summary": {"criticalCount": 1},
                    "vulnerabilities": vulns,
                }
            })
            .to_string(),
            received_at: chrono::Utc::now(),
        }
    }

    fn critical_rule() -> AlertRule {
        let mut rule = rule_for_axios(None);
        rule.name = "cvss-9".to_string();
        rule.matchers.as_mut().unwrap().package_name = None;
        rule.matchers.as_mut().unwrap().vulnerability = Some(VulnerabilityMatchers {
            severities: vec![Severity::Critical],
            min_score: Some(9.0),
            ..Default::default()
        });
        rule
    }

    async fn seed_db_with_vulnerabilities() -> Database {
        let db = Database::new(":memory:").await.unwrap();
        db.upsert_report(&vuln_payload(
            "payments",
            "api",
            &json!([
                {"vulnerabilityID": "CVE-2021-44228", "resource": "log4j-core",
                 "installedVersion": "2.14.1", "fixedVersion": "2.15.0",
                 "severity": "CRITICAL", "score": 10.0},
                {"vulnerabilityID": "CVE-2021-44228", "resource": "log4j-core",
                 "installedVersion": "2.14.1", "fixedVersion": "2.15.0",
                 "severity": "CRITICAL", "score": 10.0},
                {"vulnerabilityID": "CVE-2024-0001", "resource": "openssl",
                 "installedVersion": "3.0.7", "severity": "CRITICAL", "score": 8.1},
            ]),
        ))
        .await
        .unwrap();
        db.upsert_report(&vuln_payload(
            "default",
            "web",
            &json!([
                {"vulnerabilityID": "CVE-2023-9999", "resource": "glibc",
                 "installedVersion": "2.36", "severity": "CRITICAL", "score": 9.8},
            ]),
        ))
        .await
        .unwrap();
        db.upsert_report(&vuln_payload(
            "default",
            "quiet",
            &json!([
                {"vulnerabilityID": "CVE-2023-1111", "resource": "zlib",
                 "installedVersion": "1.2.13", "severity": "HIGH", "score": 9.1},
            ]),
        ))
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn a_vulnerability_rule_tests_against_stored_vulnerability_reports() {
        let db = seed_db_with_vulnerabilities().await;
        let (contexts, other_workloads) = build_test_contexts(&db, &critical_rule()).await.unwrap();
        assert!(!contexts.is_empty());
        for c in &contexts {
            let v = c
                .finding
                .vulnerability
                .as_ref()
                .expect("a vulnerability context");
            assert_eq!(v.severity, "CRITICAL");
            assert!(v.score.unwrap() >= 9.0);
        }
        // api and web match. quiet is HIGH, and openssl in api scores 8.1.
        assert_eq!(other_workloads, 1);
    }

    #[tokio::test]
    async fn a_vulnerability_rule_counts_only_matching_workloads() {
        let db = seed_db_with_vulnerabilities().await;
        let matchers = CompiledRule::for_rule(&critical_rule()).unwrap();
        let n = count_other_matching_workloads(&db, &matchers, "prod", "payments", "api")
            .await
            .unwrap();
        assert_eq!(n, 1, "only web, since quiet is HIGH");

        let by_id = CompiledRule::compile(
            Some(&Matchers {
                vulnerability: Some(VulnerabilityMatchers {
                    ids: vec!["cve-2021-44228".to_string()],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            &[],
        )
        .unwrap();
        let n = count_other_matching_workloads(&db, &by_id, "prod", "ghost", "ghost")
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn a_component_rule_ignores_vulnerability_reports() {
        let db = seed_db_with_vulnerabilities().await;
        let mut rule = rule_for_axios(None);
        rule.matchers.as_mut().unwrap().package_name = Some("log4j-core".to_string());
        let err = build_test_contexts(&db, &rule).await.unwrap_err();
        assert!(matches!(err, TestRunError::NoMatches));
    }
}
