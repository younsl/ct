//! Build metadata stamped by `build.rs` through vergen.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_SHA: &str = env!("VERGEN_GIT_SHA");
pub const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
pub const RUSTC_SEMVER: &str = env!("VERGEN_RUSTC_SEMVER");
pub const RUSTC_CHANNEL: &str = env!("VERGEN_RUSTC_CHANNEL");
pub const CARGO_TARGET_TRIPLE: &str = env!("VERGEN_CARGO_TARGET_TRIPLE");
pub const RUSTC_LLVM_VERSION: Option<&str> = option_env!("VERGEN_RUSTC_LLVM_VERSION");

#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
struct InfoLabels {
    version: String,
    mode: String,
}

/// Register `trivy_collector_info`, always 1, labelled with the version and
/// which binary is running.
pub fn register_metric(registry: &mut prometheus_client::registry::Registry, mode: &str) {
    let info = prometheus_client::metrics::family::Family::<
        InfoLabels,
        prometheus_client::metrics::gauge::Gauge,
    >::default();
    registry.register("trivy_collector_info", "Build information", info.clone());
    info.get_or_create(&InfoLabels {
        version: VERSION.to_string(),
        mode: mode.to_string(),
    })
    .set(1);
}
