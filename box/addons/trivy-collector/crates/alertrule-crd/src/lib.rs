//! The `AlertRule` custom resource: its Kubernetes type, the HTTP API shape it
//! converts to, and the chart CRD rendered from it. Types only, so `crdgen`
//! and anything reading rules link no controller or delivery code.

pub mod chart;
pub mod crd;
pub mod types;
