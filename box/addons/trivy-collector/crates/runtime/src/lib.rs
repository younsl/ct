//! Runtime plumbing both binaries share: command line basics, logging, the
//! health and metrics listener, the Kubernetes client, process lifecycle, and
//! build info.

pub mod build_info;
pub mod cli;
pub mod health;
pub mod kube_env;
pub mod lifecycle;
pub mod logging;
