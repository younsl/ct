//! Application state for the server tier.
//!
//! Nothing here owns a database. Reports arrive through a `ReportStore` (a
//! `RemoteStore` in production), notes and API tokens come from watched
//! Kubernetes objects, and request logs go to stdout. That is what makes the
//! server pod disposable and lets it run more than one replica.

use std::sync::Arc;
use std::time::Instant;

use crate::auth::AuthState;
use crate::auth::rbac::RbacPolicy;
use crate::config::Config;
use crate::metrics::Metrics;
use crate::notes::NotesStore;
use crate::token_store::TokenStore;
use trivy_collector_alerts::AlertEvaluator;
use trivy_collector_core::ReportStore;

/// Runtime configuration info (subset of Config for API exposure)
#[derive(Clone)]
pub struct ConfigInfo {
    pub log_format: String,
    pub log_level: String,
    pub health_port: u16,
    pub server_port: u16,
    pub scraper_url: String,
    pub hub_secret_namespace: String,
    pub auth_mode: Option<String>,
    pub mcp_enabled: bool,
}

impl From<&Config> for ConfigInfo {
    fn from(config: &Config) -> Self {
        let auth_mode = if config.auth_mode == "none" {
            None
        } else {
            Some(config.auth_mode.clone())
        };

        Self {
            log_format: config.common.log_format.clone(),
            log_level: config.common.log_level.clone(),
            health_port: config.common.health_port,
            server_port: config.server_port,
            scraper_url: config.scraper_url.clone(),
            hub_secret_namespace: config.hub_secret_namespace.clone(),
            auth_mode,
            mcp_enabled: config.mcp_enabled,
        }
    }
}

/// Runtime information collected at server startup
#[derive(Clone)]
pub struct RuntimeInfo {
    pub start_time: Instant,
    pub hostname: String,
}

impl RuntimeInfo {
    #[must_use]
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            hostname: hostname::get().map_or_else(
                |_| "unknown".to_string(),
                |h| h.to_string_lossy().to_string(),
            ),
        }
    }

    /// Get uptime as human-readable string
    #[must_use]
    pub fn uptime_string(&self) -> String {
        format_uptime(self.start_time.elapsed().as_secs())
    }
}

/// Render a duration in seconds as `1d 2h 3m 4s`, dropping leading zero units.
fn format_uptime(total_secs: u64) -> String {
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if days > 0 {
        format!("{days}d {hours}h {minutes}m {seconds}s")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

impl Default for RuntimeInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    /// Report reads and writes. `RemoteStore` in production.
    pub store: Arc<dyn ReportStore>,
    pub config: Arc<ConfigInfo>,
    pub runtime: Arc<RuntimeInfo>,
    /// Authentication state (None when auth_mode == "none")
    pub auth: Option<Arc<AuthState>>,
    /// RBAC policy engine
    pub rbac: Arc<RbacPolicy>,
    /// Prometheus metrics
    pub metrics: Arc<Metrics>,
    /// Alert evaluator (None when Kubernetes API unavailable or alerts disabled)
    pub alerts: Option<Arc<AlertEvaluator>>,
    /// ConfigMap-backed report notes (None when the Kubernetes API is
    /// unavailable, in which case notes are read-only and empty).
    pub notes: Option<Arc<NotesStore>>,
    /// Secret-backed API tokens (None when the Kubernetes API is unavailable,
    /// in which case Bearer-token auth is unavailable).
    pub tokens: Option<Arc<TokenStore>>,
}

impl AppState {
    /// Join stored notes onto a page of report metadata. A missing notes store
    /// leaves the fields empty rather than failing the read.
    pub fn merge_notes(&self, metas: &mut [trivy_collector_core::ReportMeta]) {
        if let Some(notes) = &self.notes {
            notes.cache().merge_all(metas);
        }
    }

    /// Join a stored note onto one report's metadata.
    pub fn merge_note(&self, meta: &mut trivy_collector_core::ReportMeta) {
        if let Some(notes) = &self.notes {
            notes.cache().merge_meta(meta);
        }
    }

    /// Publish how many API tokens the Secret-backed store holds.
    pub fn record_token_count(&self) {
        if let Some(tokens) = &self.tokens {
            self.metrics
                .api_tokens_total
                .set(tokens.cache().len() as i64);
        }
    }

    /// Publish the notes ConfigMap's current size. The 1MiB object limit is a
    /// hard wall, so its headroom is worth a gauge rather than a surprise.
    pub fn record_notes_size(&self) {
        if let Some(notes) = &self.notes {
            self.metrics
                .notes_configmap_bytes
                .set(notes.cache().bytes() as i64);
        }
    }
}

/// Allow axum-extra PrivateCookieJar to extract the cookie Key from AppState
impl axum::extract::FromRef<AppState> for cookie::Key {
    fn from_ref(state: &AppState) -> Self {
        state
            .auth
            .as_ref()
            .map_or_else(Self::generate, |a| a.cookie_key.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_uptime_drops_leading_zero_units() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(45), "45s");
        assert_eq!(format_uptime(90), "1m 30s");
        assert_eq!(format_uptime(3661), "1h 1m 1s");
        assert_eq!(format_uptime(90061), "1d 1h 1m 1s");
    }

    #[test]
    fn runtime_info_starts_at_zero_with_a_hostname() {
        let runtime = RuntimeInfo::new();
        assert_eq!(runtime.uptime_string(), "0s");
        assert_ne!(runtime.hostname, "");
    }

    #[test]
    fn runtime_info_default_matches_new() {
        let runtime = RuntimeInfo::default();
        assert_ne!(runtime.hostname, "");
        assert_eq!(runtime.uptime_string(), "0s");
    }

    #[test]
    fn config_info_carries_the_servers_own_settings() {
        let mut c = Config::for_test();
        c.common.log_level = "debug".to_string();
        c.common.health_port = 9090;
        c.server_port = 8080;
        c.scraper_url = "http://scraper:8081".to_string();
        c.auth_mode = "keycloak".to_string();

        let info = ConfigInfo::from(&c);
        assert_eq!(info.log_level, "debug");
        assert_eq!(info.health_port, 9090);
        assert_eq!(info.server_port, 8080);
        assert_eq!(info.scraper_url, "http://scraper:8081");
        assert_eq!(info.auth_mode, Some("keycloak".to_string()));
    }

    #[test]
    fn auth_mode_none_is_reported_as_absent() {
        let info = ConfigInfo::from(&Config::for_test());
        assert!(info.auth_mode.is_none());
    }
}
