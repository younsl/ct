//! `Database` as a `ReportStore`, so the scraper's internal API and the
//! alerts subsystem read it through the same trait the server reads remotely.

use anyhow::Result;
use async_trait::async_trait;

use trivy_collector_core::trivy::ReportPayload;
use trivy_collector_core::{
    ClusterInfo, ComponentSearchResult, FullReport, HydrationStatus, QueryParams, ReportMeta,
    ReportStore, SbomComponentMatch, Stats, TrendResponse, VulnSearchResult, VulnerabilityMatch,
    VulnerabilityMatchFilter,
};

use crate::Database;

#[async_trait]
impl ReportStore for Database {
    async fn query_reports(
        &self,
        report_type: &str,
        params: &QueryParams,
    ) -> Result<(Vec<ReportMeta>, i64)> {
        Self::query_reports(self, report_type, params).await
    }

    async fn get_report(
        &self,
        cluster: &str,
        namespace: &str,
        name: &str,
        report_type: &str,
    ) -> Result<Option<FullReport>> {
        Self::get_report(self, cluster, namespace, name, report_type).await
    }

    async fn get_stats(&self) -> Result<Stats> {
        Self::get_stats(self).await
    }

    async fn list_clusters(&self) -> Result<Vec<ClusterInfo>> {
        Self::list_clusters(self).await
    }

    async fn list_namespaces(&self, cluster: Option<&str>) -> Result<Vec<String>> {
        Self::list_namespaces(self, cluster).await
    }

    async fn search_vulnerabilities(
        &self,
        query: &str,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<VulnSearchResult>, i64)> {
        Self::search_vulnerabilities(self, query, limit, offset).await
    }

    async fn search_sbom_components(
        &self,
        component: &str,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<ComponentSearchResult>, i64)> {
        Self::search_sbom_components(self, component, limit, offset).await
    }

    async fn suggest_vulnerability_ids(&self, query: &str, limit: i64) -> Result<Vec<String>> {
        Self::suggest_vulnerability_ids(self, query, limit).await
    }

    async fn suggest_component_names(&self, query: &str, limit: i64) -> Result<Vec<String>> {
        Self::suggest_component_names(self, query, limit).await
    }

    async fn list_sbom_component_matches(
        &self,
        clusters: &[String],
        namespace: Option<&str>,
        package_name: Option<&str>,
    ) -> Result<Vec<SbomComponentMatch>> {
        Self::list_sbom_component_matches(self, clusters, namespace, package_name).await
    }

    async fn list_vulnerability_matches(
        &self,
        filter: &VulnerabilityMatchFilter,
    ) -> Result<Vec<VulnerabilityMatch>> {
        Self::list_vulnerability_matches(self, filter).await
    }

    async fn get_live_trends(
        &self,
        start_date: &str,
        end_date: &str,
        cluster: Option<&str>,
        granularity: &str,
    ) -> Result<TrendResponse> {
        Self::get_live_trends(self, start_date, end_date, cluster, granularity).await
    }

    async fn get_reports_data_range(&self) -> Result<(Option<String>, Option<String>)> {
        Self::get_reports_data_range(self).await
    }

    async fn hydration(&self) -> Result<HydrationStatus> {
        Ok(HydrationStatus::complete())
    }

    async fn upsert_report(&self, payload: &ReportPayload) -> Result<()> {
        Self::upsert_report(self, payload).await
    }

    async fn delete_report(
        &self,
        cluster: &str,
        namespace: &str,
        name: &str,
        report_type: &str,
    ) -> Result<bool> {
        Self::delete_report(self, cluster, namespace, name, report_type).await
    }

    async fn delete_reports_for_cluster(&self, cluster: &str) -> Result<u64> {
        Self::delete_reports_for_cluster(self, cluster).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn database_satisfies_report_store() {
        let db = Database::new(":memory:").await.unwrap();
        let store: Arc<dyn ReportStore> = Arc::new(db);

        assert!(store.list_clusters().await.unwrap().is_empty());
        assert_eq!(store.get_stats().await.unwrap().total_clusters, 0);
        assert!(store.hydration().await.unwrap().hydrated);
    }
}
