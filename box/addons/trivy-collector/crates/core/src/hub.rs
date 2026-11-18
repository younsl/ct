//! The Hub cluster registry: one Secret per registered edge cluster. The
//! scraper watches these Secrets to open watchers, and the server reads and
//! writes them for the cluster admin pages.

pub mod client;
pub mod types;

use k8s_openapi::api::core::v1::Secret;

use types::IN_CLUSTER_LABEL;

/// Does this Secret carry the `in-cluster=true` marker?
#[must_use]
pub fn is_in_cluster(secret: &Secret) -> bool {
    secret
        .metadata
        .labels
        .as_ref()
        .and_then(|m| m.get(IN_CLUSTER_LABEL))
        .is_some_and(|v| v == "true")
}
