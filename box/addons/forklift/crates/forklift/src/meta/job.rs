//! Requests and progress of leader-only jobs, see [`crate::leaderjob`].

use forklift_metadb::params;

use crate::leaderjob::JobState;

use super::time::{now_rfc3339, parse_time_opt};
use super::{Error, Result, Store};

impl Store {
    /// Records a request to run `job`, replacing an unclaimed one.
    pub async fn request_leader_job(&self, job: &str, by: &str) -> Result<()> {
        let (job, by) = (job.to_string(), by.to_string());
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO leader_jobs(job, requested_by, requested_at) VALUES(?, ?, ?)
         ON CONFLICT(job) DO UPDATE SET requested_by = excluded.requested_by,
             requested_at = excluded.requested_at",
                params![job, by, now_rfc3339()],
            )
            .map(|_| ())
            .map_err(|e| Error::sqlite("request leader job", e))
        })
        .await
    }

    /// Claims the waiting request for `job` and returns it. The claim clears
    /// only the request it read, so two pods claiming at once run it once.
    pub async fn take_leader_job_request(&self, job: &str) -> Result<Option<JobState>> {
        let job = job.to_string();
        self.write(move |conn| {
            let res = conn.query_row(
                "SELECT requested_by, requested_at FROM leader_jobs WHERE job = ?",
                params![job],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            );
            let (by, at) = match res {
                Ok(row) => row,
                Err(forklift_metadb::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => return Err(Error::sqlite("read leader job request", e)),
            };
            if by.is_empty() {
                return Ok(None);
            }
            let claimed = conn
                .execute(
                    "UPDATE leader_jobs SET requested_by = '', requested_at = ''
             WHERE job = ? AND requested_by = ? AND requested_at = ?",
                    params![job, by, at],
                )
                .map_err(|e| Error::sqlite("claim leader job", e))?;
            Ok((claimed == 1).then(|| JobState {
                requested_by: by,
                requested_at: parse_time_opt(Some(&at)),
                ..JobState::default()
            }))
        })
        .await
    }

    /// Publishes the running job's progress and refreshes its heartbeat. An
    /// empty `progress` marks the run finished.
    pub async fn write_leader_job_progress(&self, job: &str, progress: &str) -> Result<()> {
        let (job, progress) = (job.to_string(), progress.to_string());
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO leader_jobs(job, progress, heartbeat_at) VALUES(?, ?, ?)
         ON CONFLICT(job) DO UPDATE SET progress = excluded.progress,
             heartbeat_at = excluded.heartbeat_at",
                params![job, progress, now_rfc3339()],
            )
            .map(|_| ())
            .map_err(|e| Error::sqlite("write leader job progress", e))
        })
        .await
    }

    /// Marks the running job finished and records why it failed, or an empty
    /// `error` when it succeeded.
    pub async fn finish_leader_job(&self, job: &str, error: &str) -> Result<()> {
        let (job, error) = (job.to_string(), error.to_string());
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO leader_jobs(job, heartbeat_at, last_error) VALUES(?, ?, ?)
         ON CONFLICT(job) DO UPDATE SET progress = '', heartbeat_at = excluded.heartbeat_at,
             last_error = excluded.last_error",
                params![job, now_rfc3339(), error],
            )
            .map(|_| ())
            .map_err(|e| Error::sqlite("finish leader job", e))
        })
        .await
    }

    /// Returns the row of `job`, empty when it was never requested or run.
    pub async fn read_leader_job(&self, job: &str) -> Result<JobState> {
        let job = job.to_string();
        self.read(move |conn| {
            let res = conn.query_row(
                "SELECT requested_by, requested_at, progress, heartbeat_at, last_error
         FROM leader_jobs WHERE job = ?",
                params![job],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            );
            match res {
                Ok((requested_by, requested_at, progress, heartbeat_at, last_error)) => {
                    Ok(JobState {
                        requested_by,
                        requested_at: parse_time_opt(Some(&requested_at)),
                        progress,
                        heartbeat_at: parse_time_opt(Some(&heartbeat_at)),
                        last_error,
                    })
                }
                Err(forklift_metadb::Error::QueryReturnedNoRows) => Ok(JobState::default()),
                Err(e) => Err(Error::sqlite("read leader job", e)),
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::leaderjob::COVERAGE_SCAN;
    use crate::testing::meta::test_store;

    #[tokio::test]
    async fn request_is_claimed_once() {
        let (s, _dir) = test_store().await;
        assert!(
            s.take_leader_job_request(COVERAGE_SCAN)
                .await
                .unwrap()
                .is_none()
        );

        s.request_leader_job(COVERAGE_SCAN, "alice").await.unwrap();
        let state = s.read_leader_job(COVERAGE_SCAN).await.unwrap();
        assert!(state.pending(Utc::now()));

        let claimed = s.take_leader_job_request(COVERAGE_SCAN).await.unwrap();
        assert_eq!(claimed.map(|c| c.requested_by).as_deref(), Some("alice"));
        assert!(
            s.take_leader_job_request(COVERAGE_SCAN)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !s.read_leader_job(COVERAGE_SCAN)
                .await
                .unwrap()
                .active(Utc::now())
        );
    }

    #[tokio::test]
    async fn progress_round_trips_and_clears() {
        let (s, _dir) = test_store().await;
        s.write_leader_job_progress(COVERAGE_SCAN, r#"{"done":1}"#)
            .await
            .unwrap();
        let state = s.read_leader_job(COVERAGE_SCAN).await.unwrap();
        assert_eq!(state.running(Utc::now()), Some(r#"{"done":1}"#));

        s.request_leader_job(COVERAGE_SCAN, "bob").await.unwrap();
        let state = s.read_leader_job(COVERAGE_SCAN).await.unwrap();
        assert_eq!(
            state.progress, r#"{"done":1}"#,
            "a request keeps the progress"
        );

        s.finish_leader_job(COVERAGE_SCAN, "gitlab: 401")
            .await
            .unwrap();
        let state = s.read_leader_job(COVERAGE_SCAN).await.unwrap();
        assert!(state.running(Utc::now()).is_none());
        assert_eq!(state.last_error, "gitlab: 401");
        assert_eq!(state.requested_by, "bob", "finishing keeps a newer request");

        s.write_leader_job_progress(COVERAGE_SCAN, "{}")
            .await
            .unwrap();
        s.finish_leader_job(COVERAGE_SCAN, "").await.unwrap();
        let state = s.read_leader_job(COVERAGE_SCAN).await.unwrap();
        assert_eq!(state.last_error, "", "a success clears the error");
    }
}
