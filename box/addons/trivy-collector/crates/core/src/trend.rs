//! Dashboard trend shapes, shared by the SQLite queries that build them and
//! the HTTP responses that carry them.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Aggregated trend data point
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TrendDataPoint {
    pub date: String,
    pub clusters_count: i64,
    pub vuln_reports: i64,
    pub sbom_reports: i64,
    pub critical: i64,
    pub high: i64,
    pub medium: i64,
    pub low: i64,
    pub unknown: i64,
    pub components: i64,
}

/// Trend response metadata
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TrendMeta {
    pub range_start: String,
    pub range_end: String,
    pub granularity: String,
    pub clusters: Vec<String>,
    /// Earliest date with data (data retention start)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_from: Option<String>,
    /// Latest date with data (data retention end)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_to: Option<String>,
}

/// Full trend response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TrendResponse {
    pub meta: TrendMeta,
    pub series: Vec<TrendDataPoint>,
}
