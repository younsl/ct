//! The scan across pods sharing one store: a pod that does not lead requests a
//! scan, the leader claims and runs it, and every pod reads its progress. See
//! [`crate::leaderjob`].

use std::sync::atomic::Ordering;

use chrono::Utc;

use crate::coverage::{Res, Scanner};
use crate::leaderjob::HEARTBEAT_INTERVAL;

impl Scanner {
    /// Marks the store as shared by several pods. Without it the scanner never
    /// touches the job row, which is the single-pod SQLite case.
    pub fn set_shared(&self, shared: bool) {
        self.shared.store(shared, Ordering::SeqCst);
    }

    pub fn shared(&self) -> bool {
        self.shared.load(Ordering::SeqCst)
    }

    /// Asks the leader for a scan. This pod reports it as requested at once,
    /// before its next sync reads the row back.
    pub async fn request_scan(&self, triggered_by: &str) -> Res<()> {
        self.store.request_scan_job(triggered_by).await?;
        let mut state = self.state.write();
        state.remote.requested_by = triggered_by.to_string();
        state.remote.requested_at = Some(Utc::now());
        Ok(())
    }

    /// Claims a waiting request and returns who asked, or `None` when there is
    /// none or it expired.
    pub async fn take_scan_request(&self) -> Res<Option<String>> {
        let Some(job) = self.store.take_scan_job_request().await? else {
            return Ok(None);
        };
        Ok(job.pending(Utc::now()).then_some(job.requested_by))
    }

    /// Reads the shared scan state. When a scan run elsewhere has just ended,
    /// its result is loaded at once instead of on the next full sync.
    pub async fn sync_scan_state(&self) -> Res<()> {
        if !self.shared() {
            return Ok(());
        }
        let job = self.store.read_scan_job().await?;
        let now = Utc::now();
        let finished = {
            let mut state = self.state.write();
            let was_running = state.remote.running(now).is_some();
            let finished = was_running && job.running(now).is_none() && !state.scanning;
            if !state.scanning {
                state.last_scan_error.clone_from(&job.last_error);
            }
            state.remote = job;
            finished
        };
        if finished {
            self.apply_stored_result().await?;
        }
        Ok(())
    }

    /// Publishes this pod's scan progress every [`HEARTBEAT_INTERVAL`] until
    /// dropped.
    pub(crate) async fn publish_progress(&self) {
        loop {
            let progress = self.state.read().progress.clone();
            if let Some(progress) = progress {
                match serde_json::to_string(&progress) {
                    Ok(json) => {
                        if let Err(e) = self.store.write_scan_job_progress(&json).await {
                            tracing::warn!(err = %e, "coverage: publishing scan progress failed");
                        }
                    }
                    Err(e) => tracing::warn!(err = %e, "coverage: encoding scan progress failed"),
                }
            }
            tokio::time::sleep(HEARTBEAT_INTERVAL).await;
        }
    }

    /// Marks the shared scan finished, with why it failed.
    pub(crate) async fn finish_shared(&self, error: &str) {
        if let Err(e) = self.store.finish_scan_job(error).await {
            tracing::warn!(err = %e, "coverage: clearing scan progress failed");
        }
    }
}
