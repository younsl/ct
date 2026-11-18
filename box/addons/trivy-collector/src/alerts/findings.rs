//! What a rule is matched against: one finding per SBOM component or per
//! vulnerability, read from an ingested report or from the report store.

use std::collections::HashSet;

use serde::Deserialize;

use super::matchers::{CompiledMatchers, CompiledRule};
use crate::collector::types::{SbomReportData, VulnerabilityReportData};
use crate::storage::ReportStore;

/// The report kind a rule evaluates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Sbom,
    Vulnerability,
}

impl ReportKind {
    /// The `report_type` value reports of this kind are stored under.
    #[must_use]
    pub const fn report_type(self) -> &'static str {
        match self {
            Self::Sbom => "sbomreport",
            Self::Vulnerability => "vulnerabilityreport",
        }
    }

    #[must_use]
    pub fn from_report_type(report_type: &str) -> Option<Self> {
        [Self::Sbom, Self::Vulnerability]
            .into_iter()
            .find(|k| k.report_type().eq_ignore_ascii_case(report_type))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VulnerabilityFinding {
    pub id: String,
    pub severity: String,
    pub score: Option<f64>,
    /// Empty when no fix is published.
    pub fixed_version: String,
    pub title: String,
}

/// An SBOM component, or a vulnerability in an installed package.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Finding {
    pub package: String,
    pub version: String,
    /// Package ecosystem (`debian`, `npm`, ...) when the SBOM carries it.
    pub pkg_type: Option<String>,
    pub vulnerability: Option<VulnerabilityFinding>,
}

impl Finding {
    #[must_use]
    pub const fn kind(&self) -> ReportKind {
        if self.vulnerability.is_some() {
            ReportKind::Vulnerability
        } else {
            ReportKind::Sbom
        }
    }

    /// Identity within one workload's report, for dedup and net-new diffing.
    #[must_use]
    pub fn key(&self) -> String {
        match &self.vulnerability {
            Some(v) => format!("{}|{}|{}", v.id, self.package, self.version),
            None => format!("{}|{}", self.package, self.version),
        }
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(default)]
    report: T,
}

/// Every finding in a stored or ingested report's `data_json`, or `None` when
/// it does not parse.
#[must_use]
pub fn from_report(kind: ReportKind, data_json: &str) -> Option<Vec<Finding>> {
    match kind {
        ReportKind::Sbom => {
            let env: Envelope<SbomReportData> = serde_json::from_str(data_json).ok()?;
            Some(
                env.report
                    .components
                    .components
                    .into_iter()
                    .map(|c| Finding {
                        pkg_type: (!c.component_type.is_empty()).then_some(c.component_type),
                        package: c.name,
                        version: c.version,
                        vulnerability: None,
                    })
                    .collect(),
            )
        }
        ReportKind::Vulnerability => {
            let env: Envelope<VulnerabilityReportData> = serde_json::from_str(data_json).ok()?;
            Some(
                env.report
                    .vulnerabilities
                    .into_iter()
                    .map(|v| Finding {
                        package: v.resource,
                        version: v.installed_version,
                        pkg_type: None,
                        vulnerability: Some(VulnerabilityFinding {
                            id: v.vulnerability_id,
                            severity: v.severity,
                            score: v.score,
                            fixed_version: v.fixed_version,
                            title: v.title,
                        }),
                    })
                    .collect(),
            )
        }
    }
}

/// Finding keys of a stored report. Empty when it does not parse, which
/// treats every finding in the new revision as net-new.
#[must_use]
pub fn keys(kind: ReportKind, data_json: &str) -> HashSet<String> {
    from_report(kind, data_json)
        .unwrap_or_default()
        .iter()
        .map(Finding::key)
        .collect()
}

/// A finding located in a workload.
#[derive(Debug, Clone)]
pub struct WorkloadFinding {
    pub cluster: String,
    pub namespace: String,
    pub workload: String,
    pub finding: Finding,
    /// When the workload's report was last stored, for recency ordering.
    pub updated_at: String,
}

impl WorkloadFinding {
    #[must_use]
    pub fn workload_key(&self) -> (String, String, String) {
        (
            self.cluster.clone(),
            self.namespace.clone(),
            self.workload.clone(),
        )
    }
}

/// Every finding in the report store that any of the rule's terms matches,
/// once each, most recently updated workload first.
///
/// # Errors
///
/// Returns the storage error.
pub async fn fetch(
    db: &dyn ReportStore,
    rule: &CompiledRule,
) -> anyhow::Result<Vec<WorkloadFinding>> {
    let mut seen: HashSet<(String, String, String, String)> = HashSet::new();
    let mut out: Vec<WorkloadFinding> = Vec::new();
    for term in rule.terms() {
        for row in fetch_term(db, term).await? {
            let (cluster, namespace, workload) = row.workload_key();
            if seen.insert((cluster, namespace, workload, row.finding.key())) {
                out.push(row);
            }
        }
    }
    // Stable, so rows of one workload keep their order across terms.
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

/// One term's findings. Row filters go to SQL so only plausible rows cross
/// the wire, and each row is then checked against the compiled term, which
/// stays authoritative.
async fn fetch_term(
    db: &dyn ReportStore,
    matchers: &CompiledMatchers,
) -> anyhow::Result<Vec<WorkloadFinding>> {
    let rows: Vec<WorkloadFinding> = match matchers.kind() {
        ReportKind::Sbom => db
            .list_sbom_component_matches(
                matchers.clusters(),
                matchers.namespace(),
                matchers.package_name(),
            )
            .await?
            .into_iter()
            .map(|r| WorkloadFinding {
                cluster: r.cluster,
                namespace: r.namespace,
                workload: r.workload_name,
                finding: Finding {
                    package: r.package_name,
                    version: r.version,
                    pkg_type: (!r.component_type.is_empty()).then_some(r.component_type),
                    vulnerability: None,
                },
                updated_at: r.updated_at,
            })
            .collect(),
        ReportKind::Vulnerability => db
            .list_vulnerability_matches(&matchers.vulnerability_filter())
            .await?
            .into_iter()
            .map(|r| WorkloadFinding {
                cluster: r.cluster,
                namespace: r.namespace,
                workload: r.workload_name,
                finding: Finding {
                    package: r.package_name,
                    version: r.installed_version,
                    pkg_type: None,
                    vulnerability: Some(VulnerabilityFinding {
                        id: r.vulnerability_id,
                        severity: r.severity,
                        score: r.score,
                        fixed_version: r.fixed_version,
                        title: r.title,
                    }),
                },
                updated_at: r.updated_at,
            })
            .collect(),
    };
    Ok(rows
        .into_iter()
        .filter(|r| matchers.in_scope(&r.cluster, &r.namespace) && matchers.matches(&r.finding))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_kinds_round_trip_through_their_stored_type() {
        for kind in [ReportKind::Sbom, ReportKind::Vulnerability] {
            assert_eq!(ReportKind::from_report_type(kind.report_type()), Some(kind));
        }
        assert_eq!(
            ReportKind::from_report_type("SbomReport"),
            Some(ReportKind::Sbom)
        );
        assert_eq!(ReportKind::from_report_type("configauditreport"), None);
    }

    #[test]
    fn sbom_components_become_findings() {
        let json = r#"{"report":{"components":{"components":[
            {"name":"openssl","version":"3.0.7","type":"library"},
            {"name":"glibc","version":"2.36"}
        ]}}}"#;
        let findings = from_report(ReportKind::Sbom, json).unwrap();
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].pkg_type.as_deref(), Some("library"));
        assert_eq!(findings[1].pkg_type, None);
        assert!(findings.iter().all(|f| f.vulnerability.is_none()));
        let keys = keys(ReportKind::Sbom, json);
        assert!(keys.contains("openssl|3.0.7"));
        assert!(keys.contains("glibc|2.36"));
    }

    #[test]
    fn vulnerabilities_become_findings_keyed_by_id_and_package() {
        let json = r#"{"report":{"vulnerabilities":[
            {"vulnerabilityID":"CVE-2021-44228","resource":"log4j-core","installedVersion":"2.14.1",
             "fixedVersion":"2.15.0","severity":"CRITICAL","score":10.0,"title":"Log4Shell"},
            {"vulnerabilityID":"CVE-2021-44228","resource":"log4j-api","installedVersion":"2.14.1"}
        ]}}"#;
        let findings = from_report(ReportKind::Vulnerability, json).unwrap();
        assert_eq!(findings.len(), 2);
        let v = findings[0].vulnerability.as_ref().unwrap();
        assert_eq!(v.id, "CVE-2021-44228");
        assert_eq!(v.score, Some(10.0));
        assert_eq!(v.fixed_version, "2.15.0");
        assert_eq!(findings[0].package, "log4j-core");
        assert_eq!(findings[0].version, "2.14.1");
        assert_eq!(
            keys(ReportKind::Vulnerability, json).len(),
            2,
            "one CVE in two packages is two findings"
        );
    }

    #[test]
    fn an_unparseable_report_has_no_findings() {
        assert!(from_report(ReportKind::Sbom, "not-json").is_none());
        assert!(keys(ReportKind::Vulnerability, "not-json").is_empty());
    }
}
