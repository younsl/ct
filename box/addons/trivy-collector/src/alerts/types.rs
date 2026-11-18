//! Alert rule schema in the HTTP API's snake_case shape.
//!
//! A rule is one `matchers` term or a `matcher_terms` list, ORed like
//! `nodeSelectorTerms`. Fields inside a term are ANDed. A term without a
//! `vulnerability` block fires when a matching component lands in a
//! workload's SBOM. With one, it fires when a matching vulnerability lands in
//! a workload's vulnerability report.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, ToSchema)]
pub struct Matchers {
    /// SBOM component name, or the vulnerable package for a vulnerability
    /// rule. Absent matches every package.
    pub package_name: Option<String>,
    /// Optional version constraint (e.g. `<2.17.0`, `>=1.0.0,<2.0.0`).
    pub version_expr: Option<String>,
    /// Empty = match any cluster.
    #[serde(default)]
    pub clusters: Vec<String>,
    pub namespace: Option<String>,
    /// Workload namespaces the term never matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_namespaces: Vec<String>,
    /// Packages the term never matches, compared case-insensitively.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_packages: Vec<String>,
    /// Present switches the term from SBOM components to vulnerabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vulnerability: Option<VulnerabilityMatchers>,
}

/// Conditions on a vulnerability finding. Every set field must hold, and an
/// empty block matches every vulnerability in scope.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, ToSchema)]
pub struct VulnerabilityMatchers {
    /// Vulnerability IDs (`CVE-2021-44228`, `GHSA-...`), compared
    /// case-insensitively. Empty matches any ID.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    /// Vulnerability IDs the term never matches, such as an accepted risk.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_ids: Vec<String>,
    /// Trivy severities to match. Empty matches any severity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub severities: Vec<Severity>,
    /// Lowest CVSS score that matches, inclusive. A finding without a score
    /// never matches when this is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f64>,
    /// `true` matches only findings with a fixed version, `false` only those
    /// without. Absent matches both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_available: Option<bool>,
}

/// Trivy's severity scale, spelled the way Trivy reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Unknown,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct SlackReceiver {
    pub webhook_url: String,
    pub channel: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct Receiver {
    pub name: String,
    pub slack: Option<SlackReceiver>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct AlertRule {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// A single term. Exactly one of `matchers` and `matcher_terms` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matchers: Option<Matchers>,
    /// Terms ORed together: the rule fires on a finding any term matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matcher_terms: Vec<Matchers>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
    pub receivers: Vec<Receiver>,
    #[serde(default)]
    pub cooldown_secs: Option<u64>,
    pub created_at: String,
    pub created_by: String,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
    /// `metadata.generation` of the stored object. Read-only, and absent for a
    /// draft that has never been applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<i64>,
    /// What the evaluator has observed. Read-only: a value sent on a create or
    /// update is ignored, because status is a Kubernetes subresource that only
    /// the scraper writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::alerts::crd::AlertRuleStatus>,
}

const fn default_true() -> bool {
    true
}

impl AlertRule {
    /// Every term, whichever field holds them.
    pub fn terms(&self) -> impl Iterator<Item = &Matchers> {
        self.matchers.iter().chain(&self.matcher_terms)
    }
}

pub fn validate_rule_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("rule name must not be empty");
    }
    if name.len() > 253 {
        return Err("rule name must be 253 characters or fewer");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("rule name may only contain alphanumeric, '-', '_', '.'");
    }
    Ok(())
}

/// Validate a Slack webhook URL. Restricts the destination to Slack's
/// canonical webhook host so an operator can't (accidentally or
/// maliciously) point the alerts subsystem at an arbitrary internal URL,
/// turning the trivy-collector pod into an SSRF probe.
///
/// The trailing slash on the prefix anchors the host boundary, so e.g.
/// `https://hooks.slack.com.attacker.com/...` and
/// `https://hooks.slack.com:8080@evil.com/...` both fail the check.
pub fn validate_webhook_url(url: &str) -> Result<(), &'static str> {
    if url.is_empty() {
        return Err("webhook URL must not be empty");
    }
    if !url.starts_with("https://hooks.slack.com/") {
        return Err("webhook URL must start with https://hooks.slack.com/");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rule_name_ok() {
        assert!(validate_rule_name("log4j-critical").is_ok());
        assert!(validate_rule_name("rule_1.v2").is_ok());
    }

    #[test]
    fn validate_rule_name_rejects_invalid() {
        assert!(validate_rule_name("").is_err());
        assert!(validate_rule_name("has spaces").is_err());
        assert!(validate_rule_name("slash/in/name").is_err());
    }

    #[test]
    fn validate_webhook_url_accepts_canonical() {
        assert!(validate_webhook_url("https://hooks.slack.com/services/T0/B0/secretXXXX").is_ok());
    }

    #[test]
    fn validate_webhook_url_rejects_non_slack_host() {
        // Bare HTTP scheme
        assert!(validate_webhook_url("http://hooks.slack.com/services/foo").is_err());
        // Different host entirely
        assert!(validate_webhook_url("https://evil.com/services/foo").is_err());
        // Subdomain trickery — trailing slash on the prefix prevents
        // `hooks.slack.com.attacker.com` from passing.
        assert!(validate_webhook_url("https://hooks.slack.com.attacker.com/services/foo").is_err());
        // Userinfo trickery — `https://userinfo@host/` style.
        assert!(validate_webhook_url("https://user@hooks.slack.com/services/foo").is_err());
        // Empty / non-URL strings
        assert!(validate_webhook_url("").is_err());
        assert!(validate_webhook_url("not a url").is_err());
    }
}
