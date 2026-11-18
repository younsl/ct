CREATE TABLE leader_jobs (
    job TEXT PRIMARY KEY,
    requested_by TEXT NOT NULL DEFAULT '',
    requested_at TEXT NOT NULL DEFAULT '',
    progress TEXT NOT NULL DEFAULT '',
    heartbeat_at TEXT NOT NULL DEFAULT '',
    last_error TEXT NOT NULL DEFAULT ''
);
