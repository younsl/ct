//! Every manifest under `examples/` must be a rule the evaluator accepts, and
//! must match what its description says it matches.

use std::path::{Path, PathBuf};

use trivy_collector_alertrule_crd::crd::AlertRule;
use trivy_collector_alertrule_crd::types::validate_webhook_url;
use trivy_collector_alerts::findings::{Finding, VulnerabilityFinding};
use trivy_collector_alerts::matchers::CompiledRule;

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn load(file: &str) -> AlertRule {
    let path = examples_dir().join(file);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {file}: {e}"))
}

fn compile(file: &str) -> CompiledRule {
    CompiledRule::for_rule(&load(file).to_api()).unwrap_or_else(|e| panic!("{file}: {e}"))
}

fn component(package: &str, version: &str) -> Finding {
    Finding {
        package: package.to_string(),
        version: version.to_string(),
        ..Default::default()
    }
}

fn vuln(id: &str, package: &str, severity: &str, score: Option<f64>, fixed: &str) -> Finding {
    Finding {
        package: package.to_string(),
        version: "1.0.0".to_string(),
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

#[test]
fn every_example_is_a_valid_rule() {
    let mut files: Vec<String> = std::fs::read_dir(examples_dir())
        .expect("examples directory")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|f| {
            Path::new(f)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("yaml"))
        })
        .collect();
    files.sort();
    assert_ne!(files.len(), 0, "no examples found");
    for file in &files {
        let cr = load(file);
        assert!(
            !cr.spec.description.is_empty(),
            "{file}: say what the rule is for"
        );
        assert!(!cr.spec.receivers.is_empty(), "{file}: no receiver");
        for r in &cr.spec.receivers {
            if let Some(slack) = &r.slack {
                validate_webhook_url(&slack.webhook_url).unwrap_or_else(|e| panic!("{file}: {e}"));
            }
        }
        compile(file);
    }
}

/// `(file, cluster, namespace, finding, fires)`, covering each example's
/// stated intent and at least one finding it must not fire on.
#[test]
#[allow(clippy::too_many_lines, reason = "one row per case")]
fn examples_match_what_they_describe() {
    let cases: Vec<(&str, &str, &str, Finding, bool)> = vec![
        (
            "component-version.yaml",
            "prod",
            "app",
            component("log4j-core", "2.14.1"),
            true,
        ),
        (
            "component-version.yaml",
            "prod",
            "app",
            component("log4j-core", "2.17.1"),
            false,
        ),
        (
            "component-version.yaml",
            "stage",
            "app",
            component("log4j-core", "2.14.1"),
            false,
        ),
        (
            "vulnerability-cve-id.yaml",
            "stage",
            "app",
            vuln("cve-2021-45046", "log4j-core", "CRITICAL", None, ""),
            true,
        ),
        (
            "vulnerability-cve-id.yaml",
            "prod",
            "app",
            vuln("CVE-2022-0001", "log4j-core", "CRITICAL", None, ""),
            false,
        ),
        (
            "vulnerability-cve-id.yaml",
            "prod",
            "app",
            component("log4j-core", "2.14.1"),
            false,
        ),
        (
            "vulnerability-cvss-threshold.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "CRITICAL", Some(9.0), ""),
            true,
        ),
        (
            "vulnerability-cvss-threshold.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "CRITICAL", Some(8.9), ""),
            false,
        ),
        (
            "vulnerability-cvss-threshold.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "CRITICAL", None, ""),
            false,
        ),
        (
            "vulnerability-critical-fixable.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "CRITICAL", None, "1.2.13"),
            true,
        ),
        (
            "vulnerability-critical-fixable.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "CRITICAL", None, ""),
            false,
        ),
        (
            "vulnerability-critical-fixable.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "HIGH", None, "1.2.13"),
            false,
        ),
        (
            "vulnerability-in-package.yaml",
            "prod",
            "app",
            vuln("CVE-1", "spring-core", "LOW", None, ""),
            true,
        ),
        (
            "vulnerability-in-package.yaml",
            "prod",
            "app",
            vuln("CVE-1", "spring-web", "LOW", None, ""),
            false,
        ),
        (
            "exclusions.yaml",
            "prod",
            "app",
            vuln("CVE-1", "openssl", "HIGH", None, ""),
            true,
        ),
        (
            "exclusions.yaml",
            "prod",
            "kube-system",
            vuln("CVE-1", "openssl", "HIGH", None, ""),
            false,
        ),
        (
            "exclusions.yaml",
            "prod",
            "app",
            vuln("CVE-1", "linux-libc-dev", "HIGH", None, ""),
            false,
        ),
        (
            "exclusions.yaml",
            "prod",
            "app",
            vuln("CVE-2023-44487", "golang.org/x/net", "HIGH", None, ""),
            false,
        ),
        (
            "matcher-terms-or.yaml",
            "prod",
            "app",
            component("log4j-core", "2.14.1"),
            true,
        ),
        (
            "matcher-terms-or.yaml",
            "prod",
            "app",
            vuln("CVE-2021-44228", "log4j-core", "CRITICAL", None, ""),
            true,
        ),
        (
            "matcher-terms-or.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "CRITICAL", Some(9.8), "1.2.13"),
            true,
        ),
        (
            "matcher-terms-or.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "CRITICAL", Some(9.8), ""),
            false,
        ),
        (
            "matcher-terms-per-cluster.yaml",
            "prod",
            "app",
            vuln("CVE-1", "zlib", "HIGH", Some(7.5), ""),
            true,
        ),
        (
            "matcher-terms-per-cluster.yaml",
            "prod",
            "kube-system",
            vuln("CVE-1", "zlib", "HIGH", Some(7.5), ""),
            false,
        ),
        (
            "matcher-terms-per-cluster.yaml",
            "stage",
            "app",
            vuln("CVE-1", "zlib", "HIGH", Some(7.5), ""),
            false,
        ),
        (
            "matcher-terms-per-cluster.yaml",
            "stage",
            "app",
            vuln("CVE-1", "zlib", "CRITICAL", Some(9.1), ""),
            true,
        ),
    ];
    for (file, cluster, namespace, finding, fires) in cases {
        assert_eq!(
            compile(file).matches(cluster, namespace, &finding),
            fires,
            "{file} on {cluster}/{namespace}: {finding:?}"
        );
    }
}
