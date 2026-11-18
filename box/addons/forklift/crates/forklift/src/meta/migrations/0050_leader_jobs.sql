-- On-demand runs of leader-only jobs (coverage scan, insights analysis) when
-- several pods serve. A pod that does not lead records the request here, the
-- leader claims it, and the leader publishes its progress with a heartbeat so
-- every pod reports the same run and how the last one ended. One row per job.
CREATE TABLE leader_jobs (
    job          TEXT PRIMARY KEY,
    requested_by TEXT NOT NULL DEFAULT '',
    requested_at TEXT NOT NULL DEFAULT '',
    progress     TEXT NOT NULL DEFAULT '',
    heartbeat_at TEXT NOT NULL DEFAULT '',
    last_error   TEXT NOT NULL DEFAULT ''
);
