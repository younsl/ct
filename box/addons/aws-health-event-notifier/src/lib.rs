//! Library behind the `aws-health-event-notifier` binary, shared with the integration tests.

pub mod aws;
pub mod config;
pub mod error;
pub mod filter;
pub mod health;
pub mod interactive;
pub mod k8s;
pub mod leader;
pub mod ledger;
pub mod notify;
pub mod observability;
pub mod poller;
pub mod server;
pub mod slack;
