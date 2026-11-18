//! A rule's matchers, validated once and then applied to findings.
//!
//! A rule is a set of terms ORed together, like `nodeSelectorTerms`, and each
//! term ANDs its fields. Inside a list field values are ORed, and `exclude*`
//! fields veto a finding the rest of the term would match.
//!
//! Every surface that asks whether a rule matches goes through
//! `CompiledRule`: the ingest evaluator, preview, test delivery, the fleet
//! count, and readiness. A new matcher is a field on `types::Matchers`, a check
//! in `compile`, and a clause in `matches`, and every surface picks it up.

use super::expr::VersionExpr;
use super::findings::{Finding, ReportKind};
use trivy_collector_alertrule_crd::types::{AlertRule, Matchers, Severity, VulnerabilityMatchers};
use trivy_collector_core::VulnerabilityMatchFilter;

/// Condition reason for an unparseable `versionExpr`.
pub const REASON_INVALID_VERSION_EXPR: &str = "InvalidVersionExpr";

/// Condition reason for any other matcher the evaluator cannot act on.
pub const REASON_INVALID_MATCHERS: &str = "InvalidMatchers";

const MAX_SCORE: f64 = 10.0;

/// Matches the CRD's `maxItems` on `matcherTerms`.
pub const MAX_TERMS: usize = 20;

/// Why a rule's matchers cannot be compiled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {message}")]
pub struct MatcherError {
    /// The HTTP API field at fault, e.g. `matcher_terms[1].version_expr`.
    pub field: String,
    /// Condition reason the readiness watcher reports.
    pub reason: &'static str,
    pub message: String,
}

impl MatcherError {
    fn invalid(field: &str, message: impl Into<String>) -> Self {
        Self {
            field: field.to_string(),
            reason: REASON_INVALID_MATCHERS,
            message: message.into(),
        }
    }

    fn within(mut self, prefix: &str) -> Self {
        self.field = format!("{prefix}.{}", self.field);
        self
    }
}

/// Every term of a rule. A finding matches the rule when any term in scope
/// for its workload matches it.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    terms: Vec<CompiledMatchers>,
}

impl CompiledRule {
    /// Validate a rule's terms.
    ///
    /// # Errors
    ///
    /// Returns the first term the evaluator could not act on, or a rule that
    /// sets both or neither of `matchers` and `matcher_terms`.
    pub fn compile(matchers: Option<&Matchers>, terms: &[Matchers]) -> Result<Self, MatcherError> {
        match (matchers, terms.is_empty()) {
            (Some(_), false) => Err(MatcherError::invalid(
                "matchers",
                "set either matchers or matcher_terms, not both",
            )),
            (None, true) => Err(MatcherError::invalid(
                "matchers",
                "one of matchers or matcher_terms is required",
            )),
            (Some(m), true) => Ok(Self {
                terms: vec![CompiledMatchers::compile(m).map_err(|e| e.within("matchers"))?],
            }),
            (None, false) if terms.len() > MAX_TERMS => Err(MatcherError::invalid(
                "matcher_terms",
                format!("at most {MAX_TERMS} terms"),
            )),
            (None, false) => terms
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    CompiledMatchers::compile(m)
                        .map_err(|e| e.within(&format!("matcher_terms[{i}]")))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|terms| Self { terms }),
        }
    }

    /// # Errors
    ///
    /// See `compile`.
    pub fn for_rule(rule: &AlertRule) -> Result<Self, MatcherError> {
        Self::compile(rule.matchers.as_ref(), &rule.matcher_terms)
    }

    #[must_use]
    pub fn terms(&self) -> &[CompiledMatchers] {
        &self.terms
    }

    /// Terms that watch this report kind and cover this workload. Empty means
    /// the rule has nothing to say about the report.
    #[must_use]
    pub fn applicable(
        &self,
        kind: ReportKind,
        cluster: &str,
        namespace: &str,
    ) -> Vec<&CompiledMatchers> {
        self.terms
            .iter()
            .filter(|t| t.kind() == kind && t.in_scope(cluster, namespace))
            .collect()
    }

    /// Whether any term matches a finding in this workload.
    #[must_use]
    pub fn matches(&self, cluster: &str, namespace: &str, finding: &Finding) -> bool {
        self.applicable(finding.kind(), cluster, namespace)
            .iter()
            .any(|t| t.matches(finding))
    }
}

#[derive(Debug, Clone)]
pub struct CompiledMatchers {
    clusters: Vec<String>,
    namespace: Option<String>,
    exclude_namespaces: Vec<String>,
    package_name: Option<String>,
    exclude_packages: Vec<String>,
    version: Option<VersionExpr>,
    vulnerability: Option<VulnerabilityPredicate>,
}

#[derive(Debug, Clone)]
struct VulnerabilityPredicate {
    ids: Vec<String>,
    exclude_ids: Vec<String>,
    severities: Vec<Severity>,
    min_score: Option<f64>,
    fix_available: Option<bool>,
}

impl CompiledMatchers {
    /// Validate the matchers and build the predicate.
    ///
    /// # Errors
    ///
    /// Returns the first matcher the evaluator could not act on.
    pub fn compile(m: &Matchers) -> Result<Self, MatcherError> {
        let version = m
            .version_expr
            .as_deref()
            .map(VersionExpr::parse)
            .transpose()
            .map_err(|message| MatcherError {
                field: "version_expr".to_string(),
                reason: REASON_INVALID_VERSION_EXPR,
                message,
            })?;
        let vulnerability = m
            .vulnerability
            .as_ref()
            .map(VulnerabilityPredicate::compile)
            .transpose()?;
        Ok(Self {
            clusters: m.clusters.clone(),
            namespace: m.namespace.clone(),
            exclude_namespaces: non_blank("exclude_namespaces", &m.exclude_namespaces)?,
            package_name: m.package_name.clone(),
            exclude_packages: non_blank("exclude_packages", &m.exclude_packages)?,
            version,
            vulnerability,
        })
    }

    /// Which report kind the rule evaluates.
    #[must_use]
    pub const fn kind(&self) -> ReportKind {
        if self.vulnerability.is_some() {
            ReportKind::Vulnerability
        } else {
            ReportKind::Sbom
        }
    }

    /// Whether a workload in this cluster and namespace is in the rule's scope.
    #[must_use]
    pub fn in_scope(&self, cluster: &str, namespace: &str) -> bool {
        (self.clusters.is_empty() || self.clusters.iter().any(|c| c == cluster))
            && self.namespace.as_deref().is_none_or(|ns| ns == namespace)
            && !self.exclude_namespaces.iter().any(|ns| ns == namespace)
    }

    /// Whether one finding satisfies every matcher. Scope is checked
    /// separately by `in_scope`, since it is a property of the report.
    #[must_use]
    pub fn matches(&self, finding: &Finding) -> bool {
        if self
            .package_name
            .as_deref()
            .is_some_and(|name| !name.eq_ignore_ascii_case(&finding.package))
        {
            return false;
        }
        if self
            .exclude_packages
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&finding.package))
        {
            return false;
        }
        if self
            .version
            .as_ref()
            .is_some_and(|e| !e.matches(&finding.version))
        {
            return false;
        }
        match (&self.vulnerability, &finding.vulnerability) {
            (None, None) => true,
            (Some(p), Some(v)) => p.matches(v),
            _ => false,
        }
    }

    #[must_use]
    pub fn clusters(&self) -> &[String] {
        &self.clusters
    }

    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    #[must_use]
    pub fn package_name(&self) -> Option<&str> {
        self.package_name.as_deref()
    }

    /// SQL pushdown for a vulnerability rule. The rows still go through
    /// `matches`, so this only has to be no narrower than the predicate.
    #[must_use]
    pub fn vulnerability_filter(&self) -> VulnerabilityMatchFilter {
        let v = self.vulnerability.as_ref();
        VulnerabilityMatchFilter {
            clusters: self.clusters.clone(),
            namespace: self.namespace.clone(),
            package_name: self.package_name.clone(),
            ids: v.map_or_default(|v| v.ids.clone()),
            severities: v.map_or_default(|v| {
                v.severities
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect()
            }),
            min_score: v.and_then(|v| v.min_score),
            fix_available: v.and_then(|v| v.fix_available),
        }
    }
}

impl VulnerabilityPredicate {
    fn compile(m: &VulnerabilityMatchers) -> Result<Self, MatcherError> {
        let ids = non_blank("vulnerability.ids", &m.ids)?
            .into_iter()
            .map(|id| id.to_ascii_uppercase())
            .collect();
        let exclude_ids = non_blank("vulnerability.exclude_ids", &m.exclude_ids)?;
        if let Some(score) = m.min_score
            && !(0.0..=MAX_SCORE).contains(&score)
        {
            return Err(MatcherError::invalid(
                "vulnerability.min_score",
                format!("{score} is outside 0.0 to {MAX_SCORE}"),
            ));
        }
        Ok(Self {
            ids,
            exclude_ids,
            severities: m.severities.clone(),
            min_score: m.min_score,
            fix_available: m.fix_available,
        })
    }

    fn matches(&self, v: &super::findings::VulnerabilityFinding) -> bool {
        (self.ids.is_empty() || self.ids.iter().any(|id| id.eq_ignore_ascii_case(&v.id)))
            && !self
                .exclude_ids
                .iter()
                .any(|id| id.eq_ignore_ascii_case(&v.id))
            && (self.severities.is_empty()
                || self
                    .severities
                    .iter()
                    .any(|s| s.as_str().eq_ignore_ascii_case(&v.severity)))
            && self
                .min_score
                .is_none_or(|min| v.score.is_some_and(|score| score >= min))
            && self
                .fix_available
                .is_none_or(|want| want != v.fixed_version.is_empty())
    }
}

/// Trimmed list values, rejecting a blank entry rather than letting it
/// silently match nothing.
fn non_blank(field: &str, values: &[String]) -> Result<Vec<String>, MatcherError> {
    values
        .iter()
        .map(|v| {
            let v = v.trim();
            if v.is_empty() {
                Err(MatcherError::invalid(field, "a value must not be empty"))
            } else {
                Ok(v.to_string())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::VulnerabilityFinding;

    fn component(name: &str, version: &str) -> Finding {
        Finding {
            package: name.to_string(),
            version: version.to_string(),
            ..Default::default()
        }
    }

    fn vuln(id: &str, severity: &str, score: Option<f64>, fixed: &str) -> Finding {
        Finding {
            package: "log4j-core".to_string(),
            version: "2.14.1".to_string(),
            pkg_type: None,
            vulnerability: Some(VulnerabilityFinding {
                id: id.to_string(),
                severity: severity.to_string(),
                score,
                fixed_version: fixed.to_string(),
                title: String::new(),
            }),
        }
    }

    fn vuln_rule(v: VulnerabilityMatchers) -> CompiledMatchers {
        CompiledMatchers::compile(&Matchers {
            vulnerability: Some(v),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn a_rule_without_a_vulnerability_block_watches_sbom_components() {
        let m = CompiledMatchers::compile(&Matchers {
            package_name: Some("Axios".to_string()),
            version_expr: Some("<1.0.0".to_string()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(m.kind(), ReportKind::Sbom);
        assert!(m.matches(&component("axios", "0.27.2")));
        assert!(!m.matches(&component("axios", "1.6.0")));
        assert!(!m.matches(&component("lodash", "0.1.0")));
        assert!(
            !m.matches(&vuln("CVE-1", "HIGH", None, "")),
            "a component rule never matches a vulnerability finding"
        );
    }

    #[test]
    fn an_empty_vulnerability_block_matches_every_vulnerability() {
        let m = vuln_rule(VulnerabilityMatchers::default());
        assert_eq!(m.kind(), ReportKind::Vulnerability);
        assert!(m.matches(&vuln("CVE-2021-44228", "CRITICAL", Some(10.0), "2.15.0")));
        assert!(
            !m.matches(&component("log4j-core", "2.14.1")),
            "a vulnerability rule never matches a bare component"
        );
    }

    #[test]
    fn ids_match_case_insensitively() {
        let m = vuln_rule(VulnerabilityMatchers {
            ids: vec![" cve-2021-44228 ".to_string()],
            ..Default::default()
        });
        assert!(m.matches(&vuln("CVE-2021-44228", "CRITICAL", None, "")));
        assert!(!m.matches(&vuln("CVE-2021-45046", "CRITICAL", None, "")));
    }

    #[test]
    fn severities_match_any_listed() {
        let m = vuln_rule(VulnerabilityMatchers {
            severities: vec![Severity::Critical, Severity::High],
            ..Default::default()
        });
        assert!(m.matches(&vuln("CVE-1", "CRITICAL", None, "")));
        assert!(m.matches(&vuln("CVE-1", "high", None, "")));
        assert!(!m.matches(&vuln("CVE-1", "MEDIUM", None, "")));
    }

    #[test]
    fn min_score_is_inclusive_and_needs_a_score() {
        let m = vuln_rule(VulnerabilityMatchers {
            min_score: Some(9.0),
            ..Default::default()
        });
        assert!(m.matches(&vuln("CVE-1", "CRITICAL", Some(9.0), "")));
        assert!(m.matches(&vuln("CVE-1", "CRITICAL", Some(9.8), "")));
        assert!(!m.matches(&vuln("CVE-1", "CRITICAL", Some(8.9), "")));
        assert!(
            !m.matches(&vuln("CVE-1", "CRITICAL", None, "")),
            "an unscored finding cannot meet a score threshold"
        );
    }

    #[test]
    fn fix_available_splits_fixable_from_unfixed() {
        let fixable = vuln_rule(VulnerabilityMatchers {
            fix_available: Some(true),
            ..Default::default()
        });
        let unfixed = vuln_rule(VulnerabilityMatchers {
            fix_available: Some(false),
            ..Default::default()
        });
        let with_fix = vuln("CVE-1", "HIGH", None, "1.2.3");
        let without_fix = vuln("CVE-1", "HIGH", None, "");
        assert!(fixable.matches(&with_fix));
        assert!(!fixable.matches(&without_fix));
        assert!(unfixed.matches(&without_fix));
        assert!(!unfixed.matches(&with_fix));
    }

    #[test]
    fn every_set_matcher_must_hold() {
        let m = CompiledMatchers::compile(&Matchers {
            package_name: Some("log4j-core".to_string()),
            version_expr: Some("<2.17.0".to_string()),
            vulnerability: Some(VulnerabilityMatchers {
                severities: vec![Severity::Critical],
                min_score: Some(9.0),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
        assert!(m.matches(&vuln("CVE-2021-44228", "CRITICAL", Some(10.0), "")));
        assert!(!m.matches(&vuln("CVE-2021-44228", "HIGH", Some(10.0), "")));
        let mut other_package = vuln("CVE-2021-44228", "CRITICAL", Some(10.0), "");
        other_package.package = "spring-core".to_string();
        assert!(!m.matches(&other_package));
    }

    #[test]
    fn scope_is_clusters_and_namespace() {
        let m = CompiledMatchers::compile(&Matchers {
            clusters: vec!["prod".to_string()],
            namespace: Some("payments".to_string()),
            ..Default::default()
        })
        .unwrap();
        assert!(m.in_scope("prod", "payments"));
        assert!(!m.in_scope("stage", "payments"));
        assert!(!m.in_scope("prod", "default"));
        let any = CompiledMatchers::compile(&Matchers::default()).unwrap();
        assert!(any.in_scope("stage", "default"));
    }

    #[test]
    fn compile_errors_name_the_field_and_reason() {
        let bad_expr = CompiledMatchers::compile(&Matchers {
            version_expr: Some(">=".to_string()),
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(bad_expr.reason, REASON_INVALID_VERSION_EXPR);
        assert!(bad_expr.to_string().starts_with("version_expr:"));

        for v in [
            VulnerabilityMatchers {
                min_score: Some(10.1),
                ..Default::default()
            },
            VulnerabilityMatchers {
                min_score: Some(f64::NAN),
                ..Default::default()
            },
            VulnerabilityMatchers {
                ids: vec!["  ".to_string()],
                ..Default::default()
            },
        ] {
            let err = CompiledMatchers::compile(&Matchers {
                vulnerability: Some(v),
                ..Default::default()
            })
            .unwrap_err();
            assert_eq!(err.reason, REASON_INVALID_MATCHERS);
            assert!(err.field.starts_with("vulnerability."));
        }
    }

    #[test]
    fn the_sql_filter_carries_every_pushdown_field() {
        let m = CompiledMatchers::compile(&Matchers {
            clusters: vec!["prod".to_string()],
            namespace: Some("payments".to_string()),
            package_name: Some("log4j-core".to_string()),
            vulnerability: Some(VulnerabilityMatchers {
                ids: vec!["cve-2021-44228".to_string()],
                severities: vec![Severity::Critical],
                min_score: Some(9.0),
                fix_available: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            m.vulnerability_filter(),
            VulnerabilityMatchFilter {
                clusters: vec!["prod".to_string()],
                namespace: Some("payments".to_string()),
                package_name: Some("log4j-core".to_string()),
                ids: vec!["CVE-2021-44228".to_string()],
                severities: vec!["CRITICAL".to_string()],
                min_score: Some(9.0),
                fix_available: Some(true),
            }
        );
    }

    fn component_term(package: &str) -> Matchers {
        Matchers {
            package_name: Some(package.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn a_rule_sets_exactly_one_of_matchers_and_terms() {
        let one = component_term("axios");
        let both = CompiledRule::compile(Some(&one), std::slice::from_ref(&one)).unwrap_err();
        assert_eq!(both.field, "matchers");
        let neither = CompiledRule::compile(None, &[]).unwrap_err();
        assert_eq!(neither.reason, REASON_INVALID_MATCHERS);
        let too_many = vec![one; MAX_TERMS + 1];
        let err = CompiledRule::compile(None, &too_many).unwrap_err();
        assert_eq!(err.field, "matcher_terms");
    }

    #[test]
    fn a_term_error_names_the_term() {
        let terms = vec![
            component_term("axios"),
            Matchers {
                version_expr: Some(">=".to_string()),
                ..Default::default()
            },
        ];
        let err = CompiledRule::compile(None, &terms).unwrap_err();
        assert_eq!(err.field, "matcher_terms[1].version_expr");
        assert_eq!(err.reason, REASON_INVALID_VERSION_EXPR);
    }

    /// `(log4j-core component) OR (CVE-2021-44228) OR (CRITICAL with a fix)`.
    #[test]
    fn terms_are_ored_and_fields_inside_a_term_are_anded() {
        let rule = CompiledRule::compile(
            None,
            &[
                component_term("log4j-core"),
                Matchers {
                    vulnerability: Some(VulnerabilityMatchers {
                        ids: vec!["CVE-2021-44228".to_string()],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Matchers {
                    vulnerability: Some(VulnerabilityMatchers {
                        severities: vec![Severity::Critical],
                        fix_available: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
        )
        .unwrap();
        assert!(rule.matches("prod", "ns", &component("log4j-core", "2.14.1")));
        assert!(!rule.matches("prod", "ns", &component("axios", "1.6.0")));
        assert!(rule.matches("prod", "ns", &vuln("CVE-2021-44228", "LOW", None, "")));
        assert!(rule.matches("prod", "ns", &vuln("CVE-9", "CRITICAL", None, "1.0")));
        assert!(
            !rule.matches("prod", "ns", &vuln("CVE-9", "CRITICAL", None, "")),
            "the third term needs both a CRITICAL severity and a fix"
        );
        assert_eq!(rule.applicable(ReportKind::Sbom, "prod", "ns").len(), 1);
        assert_eq!(
            rule.applicable(ReportKind::Vulnerability, "prod", "ns")
                .len(),
            2
        );
    }

    #[test]
    fn scope_is_per_term() {
        let rule = CompiledRule::compile(
            None,
            &[
                Matchers {
                    clusters: vec!["prod".to_string()],
                    ..component_term("axios")
                },
                Matchers {
                    clusters: vec!["stage".to_string()],
                    ..component_term("lodash")
                },
            ],
        )
        .unwrap();
        assert!(rule.matches("prod", "ns", &component("axios", "1.0.0")));
        assert!(!rule.matches("prod", "ns", &component("lodash", "1.0.0")));
        assert!(rule.matches("stage", "ns", &component("lodash", "1.0.0")));
        assert!(rule.applicable(ReportKind::Sbom, "dev", "ns").is_empty());
    }

    #[test]
    fn exclusions_veto_what_the_term_would_match() {
        let m = CompiledMatchers::compile(&Matchers {
            exclude_namespaces: vec!["kube-system".to_string()],
            exclude_packages: vec!["OpenSSL".to_string()],
            vulnerability: Some(VulnerabilityMatchers {
                severities: vec![Severity::Critical],
                exclude_ids: vec!["cve-2023-0001".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
        assert!(m.in_scope("prod", "payments"));
        assert!(!m.in_scope("prod", "kube-system"));

        assert!(m.matches(&vuln("CVE-2021-44228", "CRITICAL", None, "")));
        assert!(!m.matches(&vuln("CVE-2023-0001", "CRITICAL", None, "")));
        let mut openssl = vuln("CVE-2021-44228", "CRITICAL", None, "");
        openssl.package = "openssl".to_string();
        assert!(!m.matches(&openssl));
    }

    #[test]
    fn a_blank_exclusion_is_rejected() {
        let err = CompiledMatchers::compile(&Matchers {
            exclude_packages: vec![String::new()],
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err.field, "exclude_packages");
    }
}
