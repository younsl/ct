//! `ReportStore` over the scraper's internal HTTP API.
//!
//! This is what makes the server pod disposable: it holds no database, no
//! volume, and no connection pool, and every report read becomes a call to the
//! one process that owns the SQLite file. Response bodies are the same `serde`
//! models the local store returns, so the two implementations cannot drift in
//! shape.
//!
//! Every request carries the shared internal token. The port is also fenced by
//! a NetworkPolicy admitting only the server pods, and is never added to the
//! HTTPRoute or any ServiceMonitor.

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use std::time::Duration;
use tracing::debug;

use trivy_collector_core::trivy::{ReportEvent, ReportEventType, ReportPayload};

use trivy_collector_core::wire::{INTERNAL_API_PREFIX, INTERNAL_TOKEN_HEADER};
use trivy_collector_core::{
    ClusterInfo, ComponentSearchResult, DataRangeResponse, FullReport, HydrationStatus,
    PagedResponse, QueryParams, ReportMeta, ReportStore, SbomComponentMatch, Stats, TrendResponse,
    VulnSearchResult, VulnerabilityMatch, VulnerabilityMatchFilter,
};

const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Clone)]
pub struct RemoteStore {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl RemoteStore {
    /// `base_url` is the scraper's internal endpoint, e.g.
    /// `http://trivy-collector-scraper:8081`.
    pub fn new(base_url: &str, token: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .context("Failed to build internal API HTTP client")?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        })
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}{}", self.base_url, INTERNAL_API_PREFIX, path)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let url = self.url(path);
        debug!(url = %url, "Internal API request");
        let resp = self
            .client
            .get(&url)
            .header(INTERNAL_TOKEN_HEADER, &self.token)
            .query(query)
            .send()
            .await
            .with_context(|| format!("Internal API request to {url} failed"))?;
        Self::decode(resp, &url).await
    }

    async fn decode<T: DeserializeOwned>(resp: reqwest::Response, url: &str) -> Result<T> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Internal API {} returned {}: {}",
                url,
                status,
                body.chars().take(256).collect::<String>()
            ));
        }
        resp.json::<T>()
            .await
            .with_context(|| format!("Failed to decode internal API response from {url}"))
    }

    /// Path segment encoding for the report-identity routes. Names and
    /// namespaces are Kubernetes identifiers, but cluster names come from a
    /// registration Secret and are not guaranteed to be path-safe.
    fn report_path(cluster: &str, report_type: &str, namespace: &str, name: &str) -> String {
        format!(
            "/reports/{}/{}/{}/{}",
            urlencoding::encode(cluster),
            urlencoding::encode(report_type),
            urlencoding::encode(namespace),
            urlencoding::encode(name)
        )
    }
}

/// Flatten `QueryParams` into the internal API's query string. Only the fields
/// the SQL layer actually reads are sent.
#[must_use]
pub fn query_params_to_pairs(
    report_type: &str,
    params: &QueryParams,
) -> Vec<(&'static str, String)> {
    let mut pairs = vec![("report_type", report_type.to_string())];
    if let Some(v) = &params.cluster {
        pairs.push(("cluster", v.clone()));
    }
    if let Some(v) = &params.namespace {
        pairs.push(("namespace", v.clone()));
    }
    if let Some(v) = &params.app {
        pairs.push(("app", v.clone()));
    }
    if let Some(v) = &params.image {
        pairs.push(("image", v.clone()));
    }
    if let Some(v) = &params.component {
        pairs.push(("component", v.clone()));
    }
    if let Some(v) = &params.cve {
        pairs.push(("cve", v.clone()));
    }
    if let Some(v) = &params.severity
        && !v.is_empty()
    {
        pairs.push(("severity", v.join(",")));
    }
    if let Some(v) = params.limit {
        pairs.push(("limit", v.to_string()));
    }
    if let Some(v) = params.offset {
        pairs.push(("offset", v.to_string()));
    }
    pairs
}

#[async_trait]
impl ReportStore for RemoteStore {
    async fn query_reports(
        &self,
        report_type: &str,
        params: &QueryParams,
    ) -> Result<(Vec<ReportMeta>, i64)> {
        let resp: PagedResponse<ReportMeta> = self
            .get("/reports", &query_params_to_pairs(report_type, params))
            .await?;
        Ok((resp.items, resp.total))
    }

    async fn get_report(
        &self,
        cluster: &str,
        namespace: &str,
        name: &str,
        report_type: &str,
    ) -> Result<Option<FullReport>> {
        let path = Self::report_path(cluster, report_type, namespace, name);
        let url = self.url(&path);
        let resp = self
            .client
            .get(&url)
            .header(INTERNAL_TOKEN_HEADER, &self.token)
            .send()
            .await
            .with_context(|| format!("Internal API request to {url} failed"))?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(Self::decode(resp, &url).await?))
    }

    async fn get_stats(&self) -> Result<Stats> {
        self.get("/stats", &[]).await
    }

    async fn list_clusters(&self) -> Result<Vec<ClusterInfo>> {
        let resp: PagedResponse<ClusterInfo> = self.get("/clusters", &[]).await?;
        Ok(resp.items)
    }

    async fn list_namespaces(&self, cluster: Option<&str>) -> Result<Vec<String>> {
        let query: Vec<(&str, String)> =
            cluster.map_or_default(|c| vec![("cluster", c.to_string())]);
        let resp: PagedResponse<String> = self.get("/namespaces", &query).await?;
        Ok(resp.items)
    }

    async fn search_vulnerabilities(
        &self,
        query: &str,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<VulnSearchResult>, i64)> {
        let resp: PagedResponse<VulnSearchResult> = self
            .get(
                "/search/vulnerabilities",
                &[
                    ("q", query.to_string()),
                    ("limit", limit.to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?;
        Ok((resp.items, resp.total))
    }

    async fn search_sbom_components(
        &self,
        component: &str,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<ComponentSearchResult>, i64)> {
        let resp: PagedResponse<ComponentSearchResult> = self
            .get(
                "/search/components",
                &[
                    ("component", component.to_string()),
                    ("limit", limit.to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?;
        Ok((resp.items, resp.total))
    }

    async fn suggest_vulnerability_ids(&self, query: &str, limit: i64) -> Result<Vec<String>> {
        let resp: PagedResponse<String> = self
            .get(
                "/suggest/vulnerability-ids",
                &[("q", query.to_string()), ("limit", limit.to_string())],
            )
            .await?;
        Ok(resp.items)
    }

    async fn suggest_component_names(&self, query: &str, limit: i64) -> Result<Vec<String>> {
        let resp: PagedResponse<String> = self
            .get(
                "/suggest/component-names",
                &[("q", query.to_string()), ("limit", limit.to_string())],
            )
            .await?;
        Ok(resp.items)
    }

    async fn list_sbom_component_matches(
        &self,
        clusters: &[String],
        namespace: Option<&str>,
        package_name: Option<&str>,
    ) -> Result<Vec<SbomComponentMatch>> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if !clusters.is_empty() {
            query.push(("clusters", clusters.join(",")));
        }
        if let Some(ns) = namespace {
            query.push(("namespace", ns.to_string()));
        }
        if let Some(pkg) = package_name {
            query.push(("package_name", pkg.to_string()));
        }
        let resp: PagedResponse<SbomComponentMatch> =
            self.get("/sbom/component-matches", &query).await?;
        Ok(resp.items)
    }

    async fn list_vulnerability_matches(
        &self,
        filter: &VulnerabilityMatchFilter,
    ) -> Result<Vec<VulnerabilityMatch>> {
        let mut query: Vec<(&str, String)> = Vec::new();
        for (key, values) in [
            ("clusters", &filter.clusters),
            ("ids", &filter.ids),
            ("severities", &filter.severities),
        ] {
            if !values.is_empty() {
                query.push((key, values.join(",")));
            }
        }
        if let Some(ns) = &filter.namespace {
            query.push(("namespace", ns.clone()));
        }
        if let Some(pkg) = &filter.package_name {
            query.push(("package_name", pkg.clone()));
        }
        if let Some(min) = filter.min_score {
            query.push(("min_score", min.to_string()));
        }
        if let Some(fix) = filter.fix_available {
            query.push(("fix_available", fix.to_string()));
        }
        let resp: PagedResponse<VulnerabilityMatch> =
            self.get("/vulnerabilities/matches", &query).await?;
        Ok(resp.items)
    }

    async fn get_live_trends(
        &self,
        start_date: &str,
        end_date: &str,
        cluster: Option<&str>,
        granularity: &str,
    ) -> Result<TrendResponse> {
        let mut query = vec![
            ("start_date", start_date.to_string()),
            ("end_date", end_date.to_string()),
            ("granularity", granularity.to_string()),
        ];
        if let Some(c) = cluster {
            query.push(("cluster", c.to_string()));
        }
        self.get("/dashboard/trends", &query).await
    }

    async fn get_reports_data_range(&self) -> Result<(Option<String>, Option<String>)> {
        let resp: DataRangeResponse = self.get("/dashboard/data-range", &[]).await?;
        Ok((resp.data_from, resp.data_to))
    }

    async fn hydration(&self) -> Result<HydrationStatus> {
        self.get("/hydration", &[]).await
    }

    async fn upsert_report(&self, payload: &ReportPayload) -> Result<()> {
        let url = self.url("/reports");
        let event = ReportEvent {
            event_type: ReportEventType::Apply,
            payload: payload.clone(),
        };
        let resp = self
            .client
            .post(&url)
            .header(INTERNAL_TOKEN_HEADER, &self.token)
            .json(&event)
            .send()
            .await
            .with_context(|| format!("Internal API request to {url} failed"))?;
        let _: serde_json::Value = Self::decode(resp, &url).await?;
        Ok(())
    }

    async fn delete_report(
        &self,
        cluster: &str,
        namespace: &str,
        name: &str,
        report_type: &str,
    ) -> Result<bool> {
        let path = Self::report_path(cluster, report_type, namespace, name);
        let url = self.url(&path);
        let resp = self
            .client
            .delete(&url)
            .header(INTERNAL_TOKEN_HEADER, &self.token)
            .send()
            .await
            .with_context(|| format!("Internal API request to {url} failed"))?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        let body: serde_json::Value = Self::decode(resp, &url).await?;
        Ok(body
            .get("deleted")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true))
    }

    async fn delete_reports_for_cluster(&self, cluster: &str) -> Result<u64> {
        let url = self.url(&format!("/clusters/{}", urlencoding::encode(cluster)));
        let resp = self
            .client
            .delete(&url)
            .header(INTERNAL_TOKEN_HEADER, &self.token)
            .send()
            .await
            .with_context(|| format!("Internal API request to {url} failed"))?;
        let body: serde_json::Value = Self::decode(resp, &url).await?;
        Ok(body
            .get("deleted")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> RemoteStore {
        RemoteStore::new("http://scraper:8081/", "shh").unwrap()
    }

    #[test]
    fn base_url_loses_its_trailing_slash() {
        assert_eq!(store().base_url(), "http://scraper:8081");
    }

    #[test]
    fn urls_sit_under_the_internal_prefix() {
        assert_eq!(
            store().url("/stats"),
            "http://scraper:8081/internal/v1/stats"
        );
    }

    #[test]
    fn report_paths_encode_every_segment() {
        let p = RemoteStore::report_path("prod/eu", "sbomreport", "kube-system", "a b/c");
        assert_eq!(p, "/reports/prod%2Feu/sbomreport/kube-system/a%20b%2Fc");
    }

    #[test]
    fn query_params_carry_only_what_is_set() {
        let params = QueryParams {
            cluster: Some("prod".into()),
            severity: Some(vec!["critical".into(), "high".into()]),
            limit: Some(50),
            ..Default::default()
        };
        let pairs = query_params_to_pairs("vulnerabilityreport", &params);

        assert!(pairs.contains(&("report_type", "vulnerabilityreport".to_string())));
        assert!(pairs.contains(&("cluster", "prod".to_string())));
        assert!(pairs.contains(&("severity", "critical,high".to_string())));
        assert!(pairs.contains(&("limit", "50".to_string())));
        assert!(!pairs.iter().any(|(k, _)| *k == "namespace"));
        assert!(!pairs.iter().any(|(k, _)| *k == "offset"));
    }

    #[test]
    fn empty_severity_list_is_not_sent() {
        let params = QueryParams {
            severity: Some(vec![]),
            ..Default::default()
        };
        let pairs = query_params_to_pairs("sbomreport", &params);
        assert!(!pairs.iter().any(|(k, _)| *k == "severity"));
    }
}
