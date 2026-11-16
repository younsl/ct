//! Blob reference records: lookup, aggregate stats, GC candidate listing, replication paging
//! and the guarded record delete.

use chrono::{DateTime, Utc};
use forklift_metadb::params;

use super::repository::query_all;
use super::time::sql_time_le;
use super::{
    Blob, BlobRefDrift, Error, FormatUsage, Result, Store, format_time, now_rfc3339, parse_time,
};

impl Store {
    /// Returns blob metadata by digest.
    pub async fn get_blob(&self, sha: &str) -> Result<Blob> {
        let sha = sha.to_string();
        self.read(move |conn| {
            conn.query_row(
                "SELECT sha256, size, ref_count, created_at FROM blobs WHERE sha256 = ?",
                params![sha],
                |r| {
                    let created: String = r.get(3)?;
                    Ok(Blob {
                        sha256: r.get(0)?,
                        size: r.get(1)?,
                        ref_count: r.get(2)?,
                        created_at: parse_time(&created),
                    })
                },
            )
            .map_err(|e| Error::sqlite("get blob", e))
        })
        .await
    }

    /// Returns the number of stored blobs and their total physical size in
    /// bytes, as `(count, bytes)`. Because blobs are content-addressed and
    /// deduplicated, this reflects actual disk usage rather than the logical
    /// artifact size.
    pub async fn blob_stats(&self) -> Result<(i64, i64)> {
        self.read(|conn| {
            conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM blobs",
                [],
                |r| {
                    let c: Option<i64> = r.get(0)?;
                    let b: Option<i64> = r.get(1)?;
                    Ok((c.unwrap_or(0), b.unwrap_or(0)))
                },
            )
            .map_err(|e| Error::sqlite("blob stats", e))
        })
        .await
    }

    /// Returns the stored blob footprint split by the format of the
    /// repositories referencing each blob, largest first. A blob shared across
    /// formats counts once, under the first format by name, so the parts never
    /// add up to more than [`Store::blob_stats`]. Blobs no artifact references
    /// (awaiting garbage collection) are not in any part.
    pub async fn blob_usage_by_format(&self) -> Result<Vec<FormatUsage>> {
        self.read(|conn| {
            query_all(
                conn,
                "blob usage by format",
                "SELECT format, COUNT(*), SUM(size) FROM (
                   SELECT b.size AS size, MIN(r.format) AS format
                     FROM blobs b
                     JOIN artifacts a ON a.blob_sha256 = b.sha256
                     JOIN repositories r ON r.id = a.repo_id
                    GROUP BY b.sha256
                 )
                 GROUP BY format
                 ORDER BY SUM(size) DESC, format ASC",
                [],
                |r| {
                    Ok(FormatUsage {
                        format: r.get(0)?,
                        blob_count: r.get(1)?,
                        bytes: r.get(2)?,
                    })
                },
            )
        })
        .await
    }

    /// Returns digests of blobs that have been unreferenced since before the
    /// cutoff, capped by limit. The cache sweeper uses this to delete bytes
    /// from the blob store.
    ///
    /// The cutoff enforces the GC grace period: bytes are reclaimed only once
    /// the digest has been unreferenced for long enough that an asynchronous
    /// metadata rollback (s3 backend, see `objstore`) can no longer resurrect a
    /// row pointing at them. `created_at` is checked too, so a blob written
    /// moments ago -- which a snapshot may have captured after `blobs.put` but
    /// before the artifact row took its reference -- is never a GC candidate.
    pub async fn list_unreferenced_blobs(
        &self,
        limit: i64,
        before: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        let cutoff = format_time(before);
        self.read(move |conn| {
            let pg = conn.is_postgres();
            let sql = format!(
                "SELECT sha256 FROM blobs
           WHERE ref_count <= 0
             AND unreferenced_since IS NOT NULL AND {}
             AND {}
           LIMIT ?",
                sql_time_le(pg, "unreferenced_since", "?"),
                sql_time_le(pg, "created_at", "?"),
            );
            query_all(
                conn,
                "list unreferenced blobs",
                &sql,
                params![cutoff, cutoff, limit],
                |r| r.get(0),
            )
        })
        .await
    }

    /// Returns blob digests ordered by sha256, starting strictly after the
    /// cursor, capped by limit. Replication standbys page through this to
    /// mirror the leader's blob set.
    pub async fn list_blob_digests(&self, after: &str, limit: i64) -> Result<Vec<String>> {
        let after = after.to_string();
        self.read(move |conn| {
            query_all(
                conn,
                "list blob digests",
                "SELECT sha256 FROM blobs WHERE sha256 > ? ORDER BY sha256 LIMIT ?",
                params![after, limit],
                |r| r.get(0),
            )
        })
        .await
    }

    /// Removes a blob row, but only while it is still unreferenced
    /// (`ref_count <= 0`) and past the grace cutoff, and reports whether a row
    /// was actually deleted. The sweeper deletes the record before the bytes
    /// and reclaims the bytes only when the result is `true`: if a concurrent
    /// upload or proxy fetch of identical (content-addressed) content
    /// re-referenced this digest since it was listed, the guarded delete
    /// removes no row, the result is `false`, and the now-live bytes are left
    /// in place. The cutoff conditions mirror [`Store::list_unreferenced_blobs`]
    /// so a digest re-referenced and re-freed between the two calls cannot skip
    /// its grace period.
    pub async fn delete_blob_record(&self, sha: &str, before: DateTime<Utc>) -> Result<bool> {
        let sha = sha.to_string();
        let cutoff = format_time(before);
        self.write(move |conn| {
            let pg = conn.is_postgres();
            let sql = format!(
                "DELETE FROM blobs
           WHERE sha256 = ?
             AND ref_count <= 0
             AND unreferenced_since IS NOT NULL AND {}
             AND {}",
                sql_time_le(pg, "unreferenced_since", "?"),
                sql_time_le(pg, "created_at", "?"),
            );
            let n = conn
                .execute(&sql, params![sha, cutoff, cutoff])
                .map_err(|e| Error::sqlite("delete blob record", e))?;
            Ok(n > 0)
        })
        .await
    }

    /// [`Store::delete_blob_record`] for a store several pods write to: the
    /// guarded delete and `delete_bytes` run inside one transaction, and the
    /// record only goes if the bytes did.
    ///
    /// The row lock taken by the delete is what excludes writers on other
    /// pods. [`Store::reserve_blob`] for the same digest waits on it until the
    /// bytes are gone, so a writer can never keep a record whose bytes this
    /// call is about to remove. A failed byte delete rolls the record back and
    /// the next sweep retries it.
    pub async fn reclaim_blob_record<F>(
        &self,
        sha: &str,
        before: DateTime<Utc>,
        delete_bytes: F,
    ) -> Result<bool>
    where
        F: FnOnce() -> std::result::Result<(), String> + Send + 'static,
    {
        let sha = sha.to_string();
        let cutoff = format_time(before);
        self.write(move |conn| {
            let pg = conn.is_postgres();
            let sql = format!(
                "DELETE FROM blobs
           WHERE sha256 = ?
             AND ref_count <= 0
             AND unreferenced_since IS NOT NULL AND {}
             AND {}",
                sql_time_le(pg, "unreferenced_since", "?"),
                sql_time_le(pg, "created_at", "?"),
            );
            let tx = conn
                .transaction_unlocked()
                .map_err(|e| Error::sqlite("begin blob reclaim", e))?;
            let n = tx
                .execute(&sql, params![sha, cutoff, cutoff])
                .map_err(|e| Error::sqlite("delete blob record", e))?;
            if n == 0 {
                return Ok(false);
            }
            delete_bytes().map_err(|e| Error::Other(format!("delete blob bytes: {e}")))?;
            tx.commit()
                .map_err(|e| Error::sqlite("commit blob reclaim", e))?;
            Ok(true)
        })
        .await
    }

    /// Returns blobs whose `ref_count` disagrees with the rows referencing
    /// them, capped by `limit`. Every code path changes the count and the
    /// referencing row in one transaction, so a committed state never
    /// disagrees and anything listed here is real drift.
    pub async fn list_blob_ref_drift(&self, limit: i64) -> Result<Vec<BlobRefDrift>> {
        self.read(move |conn| {
            query_all(
                conn,
                "list blob ref drift",
                "SELECT b.sha256, b.ref_count, COALESCE(a.n, 0) + COALESCE(g.n, 0)
                   FROM blobs b
                   LEFT JOIN (SELECT blob_sha256, COUNT(*) AS n FROM artifacts GROUP BY blob_sha256) a
                     ON a.blob_sha256 = b.sha256
                   LEFT JOIN (SELECT blob_sha256, COUNT(*) AS n FROM group_metadata_cache GROUP BY blob_sha256) g
                     ON g.blob_sha256 = b.sha256
                  WHERE b.ref_count <> COALESCE(a.n, 0) + COALESCE(g.n, 0)
                  ORDER BY b.sha256
                  LIMIT ?",
                params![limit],
                |r| {
                    Ok(BlobRefDrift {
                        sha256: r.get(0)?,
                        recorded: r.get(1)?,
                        actual: r.get(2)?,
                    })
                },
            )
        })
        .await
    }

    /// Raises a blob's `ref_count` to the number of rows referencing it and
    /// reports whether it changed. Only an undercount is repaired: it is the
    /// direction that lets the sweeper delete bytes still in use. The count
    /// is taken again inside the transaction, which excludes every writer, so
    /// a reference added since the drift was listed is not lost.
    pub async fn raise_blob_ref_count(&self, sha: &str) -> Result<bool> {
        const ACTUAL: &str = "(SELECT COUNT(*) FROM artifacts WHERE blob_sha256 = blobs.sha256)
             + (SELECT COUNT(*) FROM group_metadata_cache WHERE blob_sha256 = blobs.sha256)";
        let sha = sha.to_string();
        self.write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| Error::sqlite("begin raise blob ref", e))?;
            let n = tx
                .execute(
                    &format!(
                        "UPDATE blobs SET ref_count = {ACTUAL}, unreferenced_since = NULL
                          WHERE sha256 = ? AND ref_count < {ACTUAL}"
                    ),
                    params![sha],
                )
                .map_err(|e| Error::sqlite("raise blob ref", e))?;
            tx.commit()
                .map_err(|e| Error::sqlite("commit raise blob ref", e))?;
            Ok(n > 0)
        })
        .await
    }

    /// Records that `sha` was just written to the blob store and may be
    /// referenced shortly. A new digest gets a record, and an unreferenced one
    /// has its grace period restarted, so the sweeper leaves it alone until an
    /// artifact row takes its reference. A referenced blob is untouched.
    pub async fn reserve_blob(&self, sha: &str, size: i64) -> Result<()> {
        let sha = sha.to_string();
        self.write(move |conn| {
            let now = now_rfc3339();
            conn.execute(
                "INSERT INTO blobs(sha256, size, ref_count, created_at, unreferenced_since)
                 VALUES(?, ?, 0, ?, ?)
                 ON CONFLICT(sha256) DO UPDATE SET unreferenced_since = excluded.unreferenced_since
                 WHERE blobs.ref_count <= 0",
                params![sha, size, now, now],
            )
            .map(|_| ())
            .map_err(|e| Error::sqlite("reserve blob", e))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{Artifact, FORMAT_NPM, FORMAT_RAW, Repository, TYPE_HOSTED};
    use crate::testing::meta::test_store;

    #[tokio::test]
    async fn blob_usage_splits_by_repository_format() {
        let (s, _dir) = test_store().await;
        let mut repo_ids = Vec::new();
        for (name, format) in [
            ("npm-a", FORMAT_NPM),
            ("npm-b", FORMAT_NPM),
            ("files", FORMAT_RAW),
        ] {
            let repo = s
                .create_repository(Repository {
                    name: name.to_string(),
                    format: format.to_string(),
                    r#type: TYPE_HOSTED.to_string(),
                    ..Default::default()
                })
                .await
                .unwrap();
            repo_ids.push(repo.id);
        }
        let now = Utc::now();
        // The npm blob is shared by two npm repositories and also stored in the
        // raw one, so it must count once, under npm.
        for (repo_id, path, sha, size) in [
            (repo_ids[0], "a/-/a-1.tgz", "shared", 100),
            (repo_ids[1], "a/-/a-1.tgz", "shared", 100),
            (repo_ids[2], "copy.tgz", "shared", 100),
            (repo_ids[2], "big.bin", "raw-only", 300),
        ] {
            s.put_artifact(Artifact {
                repo_id,
                path: path.to_string(),
                blob_sha256: sha.to_string(),
                size,
                cached_at: now,
                last_accessed_at: now,
                ..Default::default()
            })
            .await
            .unwrap();
        }

        let parts = s.blob_usage_by_format().await.unwrap();
        assert_eq!(
            parts,
            vec![
                FormatUsage {
                    format: FORMAT_RAW.to_string(),
                    blob_count: 1,
                    bytes: 300,
                },
                FormatUsage {
                    format: FORMAT_NPM.to_string(),
                    blob_count: 1,
                    bytes: 100,
                },
            ]
        );
        let (_, total) = s.blob_stats().await.unwrap();
        assert_eq!(parts.iter().map(|p| p.bytes).sum::<i64>(), total);
    }

    /// A failed byte delete keeps the record, so the next sweep retries the
    /// digest instead of leaking bytes nothing points at.
    #[tokio::test]
    async fn reclaim_keeps_the_record_when_the_bytes_survive() {
        let (s, _dir) = test_store().await;
        s.reserve_blob("sha-keep", 3).await.unwrap();
        let cutoff = chrono::Utc::now() + chrono::Duration::hours(48);

        let err = s
            .reclaim_blob_record("sha-keep", cutoff, || Err("disk on fire".into()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("disk on fire"), "{err}");
        assert_eq!(s.get_blob("sha-keep").await.unwrap().size, 3);

        assert!(
            s.reclaim_blob_record("sha-keep", cutoff, || Ok(()))
                .await
                .unwrap()
        );
        assert!(s.get_blob("sha-keep").await.unwrap_err().is_not_found());
        // Nothing left to reclaim: the byte delete must not run.
        assert!(
            !s.reclaim_blob_record("sha-keep", cutoff, || Err("must not run".into()))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn reserve_restarts_grace_only_while_unreferenced() {
        let (s, _dir) = test_store().await;
        s.reserve_blob("sha-res", 1).await.unwrap();
        let soon = chrono::Utc::now() + chrono::Duration::hours(48);
        assert_eq!(
            s.list_unreferenced_blobs(10, soon).await.unwrap(),
            vec!["sha-res".to_string()]
        );
        // Reserving again re-stamps it, so a cutoff before now no longer matches.
        s.reserve_blob("sha-res", 1).await.unwrap();
        let past = chrono::Utc::now() - chrono::Duration::seconds(1);
        assert!(!s.delete_blob_record("sha-res", past).await.unwrap());
    }

    /// The cross-pod guarantee: while a sweeper holds a digest's record inside
    /// its reclaim transaction, a writer's reservation of the same digest
    /// waits, and only proceeds once the bytes are gone. Row locks provide it,
    /// so it is PostgreSQL only.
    #[tokio::test(flavor = "multi_thread")]
    async fn reservation_waits_for_an_in_flight_reclaim() {
        if !crate::testing::meta::on_postgres() {
            return;
        }
        let (s, _dir) = test_store().await;
        let s = std::sync::Arc::new(s);
        s.reserve_blob("sha-race", 1).await.unwrap();
        let cutoff = chrono::Utc::now() + chrono::Duration::hours(48);

        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let sweeper = std::sync::Arc::clone(&s);
        let reclaim = tokio::spawn(async move {
            sweeper
                .reclaim_blob_record("sha-race", cutoff, move || {
                    entered_tx.send(()).map_err(|e| e.to_string())?;
                    release_rx.recv().map_err(|e| e.to_string())
                })
                .await
        });
        tokio::task::spawn_blocking(move || entered_rx.recv())
            .await
            .unwrap()
            .unwrap();

        let writer = std::sync::Arc::clone(&s);
        let reserve = tokio::spawn(async move { writer.reserve_blob("sha-race", 1).await });
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(
            !reserve.is_finished(),
            "the reservation must wait for the sweeper's row lock"
        );

        release_tx.send(()).unwrap();
        assert!(
            reclaim.await.unwrap().unwrap(),
            "sweeper reclaimed the digest"
        );
        reserve.await.unwrap().unwrap();
        // The writer ends up with a fresh record inside its grace period.
        assert_eq!(s.get_blob("sha-race").await.unwrap().ref_count, 0);
        let past = chrono::Utc::now() - chrono::Duration::seconds(1);
        assert!(!s.delete_blob_record("sha-race", past).await.unwrap());
    }
}
