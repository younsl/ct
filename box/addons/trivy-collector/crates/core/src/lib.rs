//! Types and contracts both binaries share: the Trivy Operator report types,
//! the `ReportStore` trait and its models, the scraper's internal API wire
//! format, the formats of human-authored state, and the Hub cluster registry.
//!
//! | State      | Home                                                   |
//! | ---------- | ------------------------------------------------------ |
//! | reports    | SQLite on the scraper's `emptyDir` (`database` crate)  |
//! | notes      | ConfigMap, watched into memory by the server           |
//! | api tokens | Secret, watched into memory by the server              |

use std::collections::BTreeMap;

pub mod authored;
pub mod hub;
mod models;
mod store;
mod trend;
pub mod trivy;
pub mod wire;

// Re-export public types
pub use models::{
    ClusterInfo, ComponentSearchResult, FullReport, QueryParams, ReportMeta, SbomComponentMatch,
    Stats, TokenInfo, VulnSearchResult, VulnSummary, VulnerabilityMatch, VulnerabilityMatchFilter,
};
pub use store::{ClusterSync, DataRangeResponse, HydrationStatus, PagedResponse, ReportStore};
pub use trend::{TrendDataPoint, TrendMeta, TrendResponse};

pub const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
pub const COMPONENT_LABEL: &str = "app.kubernetes.io/component";

/// Labels stamped on every Kubernetes object this crate creates, so an
/// operator can tell app-managed state from hand-written manifests.
#[must_use]
pub fn managed_labels(component: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (MANAGED_BY_LABEL.to_string(), "trivy-collector".to_string()),
        (COMPONENT_LABEL.to_string(), component.to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_labels_identify_the_owner_and_component() {
        let labels = managed_labels("trivy-collector-notes");
        assert_eq!(labels[MANAGED_BY_LABEL], "trivy-collector");
        assert_eq!(labels[COMPONENT_LABEL], "trivy-collector-notes");
    }
}
