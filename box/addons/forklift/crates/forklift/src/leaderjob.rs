//! On-demand runs of leader-only jobs when several pods serve one PostgreSQL
//! metadata store.
//!
//! A coverage scan or an insights analysis runs only on the pod holding the
//! Lease, but the request asking for one can reach any API pod, and with
//! `FORKLIFT_TARGET=api` it never reaches the leader. A pod that does not lead
//! records the request in the store, the leader claims it, and the leader
//! publishes the run's progress with a heartbeat so every pod reports the same
//! run.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

/// Job name of a coverage scan.
pub const COVERAGE_SCAN: &str = "coverage.scan";
/// Job name of a security insights analysis.
pub const INSIGHTS: &str = "insights.analyze";

/// How often the leader looks for a request, and other pods for progress.
pub const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// How often a running job refreshes its heartbeat.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// A request older than this is dropped instead of run, so one made while no
/// pod was leading does not start a run long after somebody stopped waiting.
pub const REQUEST_TTL: TimeDelta = TimeDelta::minutes(5);
/// Progress whose heartbeat is older than this belongs to a pod that died
/// mid-run.
pub const HEARTBEAT_STALE: TimeDelta = TimeDelta::seconds(60);

/// What an on-demand trigger did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// This pod leads and started the run.
    Started,
    /// A run is already in progress.
    Busy,
    /// This pod does not lead, so the run has to be requested from the leader.
    Standby,
}

/// One job's row in the store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobState {
    /// Who asked for a run the leader has not claimed yet. Empty when none.
    pub requested_by: String,
    pub requested_at: Option<DateTime<Utc>>,
    /// The running job's progress as JSON. Empty when no run is in progress.
    pub progress: String,
    pub heartbeat_at: Option<DateTime<Utc>>,
    /// Why the last run failed. Empty when it succeeded.
    pub last_error: String,
}

impl JobState {
    /// Reports whether a request is waiting for the leader.
    pub fn pending(&self, now: DateTime<Utc>) -> bool {
        !self.requested_by.is_empty() && self.requested_at.is_some_and(|t| now - t < REQUEST_TTL)
    }

    /// Returns the progress of a run whose heartbeat is still fresh.
    pub fn running(&self, now: DateTime<Utc>) -> Option<&str> {
        let fresh = self.heartbeat_at.is_some_and(|t| now - t < HEARTBEAT_STALE);
        (fresh && !self.progress.is_empty()).then_some(self.progress.as_str())
    }

    /// Reports whether a run is requested or in progress anywhere.
    pub fn active(&self, now: DateTime<Utc>) -> bool {
        self.pending(now) || self.running(now).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(requested_ago: Option<i64>, heartbeat_ago: Option<i64>, progress: &str) -> JobState {
        let now = Utc::now();
        JobState {
            requested_by: if requested_ago.is_some() {
                "alice".into()
            } else {
                String::new()
            },
            requested_at: requested_ago.map(|s| now - TimeDelta::seconds(s)),
            progress: progress.into(),
            heartbeat_at: heartbeat_ago.map(|s| now - TimeDelta::seconds(s)),
            last_error: String::new(),
        }
    }

    #[test]
    fn idle_row_is_not_active() {
        assert!(!JobState::default().active(Utc::now()));
    }

    #[test]
    fn fresh_request_is_pending_and_an_expired_one_is_not() {
        let now = Utc::now();
        assert!(state(Some(10), None, "").pending(now));
        assert!(!state(Some(REQUEST_TTL.num_seconds() + 1), None, "").pending(now));
    }

    #[test]
    fn progress_counts_only_while_the_heartbeat_is_fresh() {
        let now = Utc::now();
        assert_eq!(state(None, Some(2), "{}").running(now), Some("{}"));
        assert!(
            state(None, Some(HEARTBEAT_STALE.num_seconds() + 1), "{}")
                .running(now)
                .is_none()
        );
        assert!(state(None, Some(2), "").running(now).is_none());
        assert!(state(None, Some(2), "{}").active(now));
    }
}
