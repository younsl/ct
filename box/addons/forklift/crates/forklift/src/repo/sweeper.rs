//! Reclaims blob bytes whose reference count has dropped to zero.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use super::Engine;

/// Blobs one reference check reports. Drift is expected to be zero, so the
/// cap only bounds a pass over a database that is badly out of step.
const REF_CHECK_LIMIT: i64 = 1000;

impl Engine {
    /// Periodically reclaims blob bytes whose reference count has dropped to
    /// zero (after artifact deletion or cache eviction). It is leader-gated by
    /// the caller in HA mode so only one instance mutates the blob store.
    ///
    /// `grace` is how long a digest must have been unreferenced before its bytes
    /// may be reclaimed. Deleting bytes is irreversible while the metadata
    /// database is only asynchronously durable in the s3 backend (see
    /// `src/objstore/`), so the delay must outlast the replication lag:
    /// otherwise a rollback can restore an artifact row whose bytes are already
    /// gone, leaving a dangling reference that no code path can repair.
    pub async fn run_sweeper(
        self: Arc<Self>,
        cancel: CancellationToken,
        interval: Duration,
        grace: Duration,
    ) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticker.tick() => self.sweep_once(grace).await,
            }
        }
    }

    pub(crate) async fn sweep_once(&self, grace: Duration) {
        let cutoff =
            self.now() - chrono::TimeDelta::from_std(grace).unwrap_or(chrono::TimeDelta::zero());
        let shas = match self.store.list_unreferenced_blobs(256, cutoff).await {
            Ok(shas) => shas,
            Err(err) => {
                tracing::error!(err = %err, "sweeper list failed");
                return;
            }
        };
        if shas.is_empty() {
            return;
        }
        let mut reclaimed = 0;
        for sha in &shas {
            if self.reclaim_blob(sha, cutoff).await {
                reclaimed += 1;
            }
        }
        if reclaimed > 0 {
            tracing::debug!(count = reclaimed, "sweeper reclaimed blobs");
        }
    }

    /// Deletes one unreferenced digest's record and then its bytes, and reports
    /// whether the bytes were reclaimed.
    ///
    /// The exclusion against reference-creating writes (`put` -> `put_artifact`,
    /// which hold the `gc_mu` read guard) is per digest, not per batch: the
    /// invariant that needs the lock is that no writer can re-reference a digest
    /// between the guarded record delete and the byte delete, and that window is
    /// one iteration. Locking the whole batch instead would stop every upload
    /// and proxy cache write for as long as a batch of blob deletions takes,
    /// which on an object-store backend is one API round-trip per digest.
    pub(crate) async fn reclaim_blob(&self, sha: &str, cutoff: DateTime<Utc>) -> bool {
        if self.store.is_postgres() {
            return self.reclaim_blob_shared(sha, cutoff).await;
        }
        let _gc = self.gc_mu.write().await;
        // Delete the DB record first, guarded by ref_count <= 0 and the same
        // grace cutoff. If the digest was re-referenced since it was listed, the
        // guarded delete removes no row and we must leave the bytes: a live
        // artifact now points at them. Reclaim the bytes only after the record
        // is gone.
        match self.store.delete_blob_record(sha, cutoff).await {
            Ok(false) => return false,
            Ok(true) => {}
            Err(err) => {
                tracing::error!(sha, err = %err, "sweeper record delete failed");
                return false;
            }
        }
        if let Err(err) = self.blobs.delete(sha).await {
            tracing::error!(sha, err = %err, "sweeper blob delete failed");
            return false;
        }
        true
    }

    /// Periodically compares every blob's reference count with the rows that
    /// reference it. It is the backstop behind the transactional bookkeeping:
    /// drift should never happen, and this is how it would be noticed.
    pub async fn run_ref_check(self: Arc<Self>, cancel: CancellationToken, interval: Duration) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticker.tick() => self.check_refs_once().await,
            }
        }
    }

    /// Runs one reference count check. An undercount is raised at once, since
    /// it would let the sweeper delete bytes still in use. An overcount only
    /// keeps unused bytes around, so it is reported and left for an operator.
    pub(crate) async fn check_refs_once(&self) {
        let drift = match self.store.list_blob_ref_drift(REF_CHECK_LIMIT).await {
            Ok(drift) => drift,
            Err(err) => {
                tracing::error!(err = %err, "blob ref check failed");
                return;
            }
        };
        let (mut under, mut over) = (0_i64, 0_i64);
        for d in &drift {
            if d.recorded > d.actual {
                over += 1;
                tracing::warn!(
                    sha = %d.sha256, recorded = d.recorded, actual = d.actual,
                    "blob ref count too high, its bytes will not be reclaimed"
                );
                continue;
            }
            under += 1;
            match self.store.raise_blob_ref_count(&d.sha256).await {
                Ok(true) => {
                    self.ref_repaired.inc();
                    tracing::warn!(
                        sha = %d.sha256, recorded = d.recorded, actual = d.actual,
                        "blob ref count was too low, raised to the actual count"
                    );
                }
                Ok(false) => {}
                Err(err) => {
                    tracing::error!(sha = %d.sha256, err = %err, "blob ref repair failed");
                }
            }
        }
        self.ref_drift
            .with_label_values(&["under"])
            .set(under as f64);
        self.ref_drift.with_label_values(&["over"]).set(over as f64);
    }

    /// Reclaims one digest when other pods write to the same store. `gc_mu`
    /// only excludes writers in this process, so the exclusion comes from the
    /// database instead: the record delete and the byte delete share one
    /// transaction (see `meta::Store::reclaim_blob_record`).
    async fn reclaim_blob_shared(&self, sha: &str, cutoff: DateTime<Utc>) -> bool {
        let blobs = Arc::clone(&self.blobs);
        let digest = sha.to_string();
        let handle = tokio::runtime::Handle::current();
        let delete_bytes = move || {
            handle
                .block_on(blobs.delete(&digest))
                .map_err(|e| e.to_string())
        };
        match self
            .store
            .reclaim_blob_record(sha, cutoff, delete_bytes)
            .await
        {
            Ok(reclaimed) => reclaimed,
            Err(err) => {
                tracing::error!(sha, err = %err, "sweeper reclaim failed");
                false
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //!
    //! They store the artifact through `Engine::put` instead — the same code path the handler calls
    //! — and read it back through the store rather than the router.

    use std::time::Duration;

    use crate::meta;
    use crate::repoconfig::Config;

    use crate::testing::repo::{body, mk_repo, new_test_manager};

    async fn set_ref_count(store: &meta::Store, sha: &str, n: i64) {
        let sha = sha.to_string();
        store
            .write(move |conn| {
                conn.execute(
                    "UPDATE blobs SET ref_count = ? WHERE sha256 = ?",
                    forklift_metadb::params![n, sha],
                )
                .map(|_| ())
                .map_err(|e| meta::Error::sqlite("force ref count", e))
            })
            .await
            .expect("force ref count");
    }

    /// A consistent store reports no drift. An undercount is raised back to
    /// the real count and counted as repaired. An overcount is reported and
    /// left alone, since lowering a count is what frees bytes.
    #[tokio::test]
    async fn ref_check_repairs_undercounts_and_reports_overcounts() {
        let tm = new_test_manager().await;
        let repo = mk_repo(
            &tm.store,
            "mvn-refs",
            meta::TYPE_HOSTED,
            "",
            Config::default(),
        )
        .await;
        let mut digests = Vec::new();
        for (path, content) in [
            ("a/1/a-1.jar", "LOW"),
            ("b/1/b-1.jar", "HIGH"),
            ("c/1/c-1.jar", "FINE"),
        ] {
            tm.engine
                .put(
                    &repo,
                    path,
                    "1",
                    "application/java-archive",
                    None,
                    body(content),
                    "",
                )
                .await
                .expect("put");
            digests.push(
                tm.store
                    .get_artifact(repo.id, path)
                    .await
                    .expect("get artifact")
                    .blob_sha256,
            );
        }
        let drift = |direction: &str| tm.engine.ref_drift.with_label_values(&[direction]).get();

        tm.engine.check_refs_once().await;
        assert_eq!((drift("under"), drift("over")), (0.0, 0.0), "consistent");
        assert_eq!(tm.engine.ref_repaired.get(), 0.0);

        set_ref_count(&tm.store, &digests[0], 0).await;
        set_ref_count(&tm.store, &digests[1], 5).await;
        let listed = tm.store.list_blob_ref_drift(10).await.expect("list drift");
        assert_eq!(listed.len(), 2, "{listed:?}");

        tm.engine.check_refs_once().await;
        assert_eq!((drift("under"), drift("over")), (1.0, 1.0), "found");
        assert_eq!(tm.engine.ref_repaired.get(), 1.0);
        assert_eq!(
            tm.store.get_blob(&digests[0]).await.expect("low").ref_count,
            1,
            "undercount raised"
        );
        assert_eq!(
            tm.store
                .get_blob(&digests[1])
                .await
                .expect("high")
                .ref_count,
            5,
            "overcount left for an operator"
        );
        // The raised blob is referenced again, so no sweep may touch it.
        assert_eq!(
            tm.store
                .list_unreferenced_blobs(10, chrono::Utc::now() + chrono::Duration::hours(48))
                .await
                .expect("list unreferenced"),
            Vec::<String>::new()
        );

        tm.engine.check_refs_once().await;
        assert_eq!((drift("under"), drift("over")), (0.0, 1.0), "repair held");
        assert_eq!(tm.engine.ref_repaired.get(), 1.0);
    }

    /// The happy path: once an artifact is deleted and its blob's `ref_count` drops
    /// to zero, `sweep_once` removes both the blob record and its bytes.
    #[tokio::test]
    async fn sweeper_reclaims_unreferenced() {
        let tm = new_test_manager().await;
        let repo = mk_repo(
            &tm.store,
            "mvn-sweep",
            meta::TYPE_HOSTED,
            "",
            Config::default(),
        )
        .await;
        let path = "com/example/app/1.0/app-1.0.jar";

        tm.engine
            .put(
                &repo,
                path,
                "1.0",
                "application/java-archive",
                None,
                body("SWEEPBYTES"),
                "",
            )
            .await
            .expect("put");
        let art = tm
            .store
            .get_artifact(repo.id, path)
            .await
            .expect("get artifact");
        let digest = art.blob_sha256.clone();

        // Delete the artifact so the blob becomes unreferenced.
        tm.store
            .delete_artifact(repo.id, path)
            .await
            .expect("delete artifact");
        let shas = tm
            .store
            .list_unreferenced_blobs(10, chrono::Utc::now())
            .await
            .expect("list unreferenced");
        assert_eq!(shas, vec![digest.clone()], "unreferenced blobs");

        tm.engine.sweep_once(Duration::ZERO).await;

        assert!(
            tm.store.get_blob(&digest).await.is_err(),
            "blob record still present after sweep"
        );
        assert!(
            !tm.engine.blobs.exists(&digest).await.expect("exists"),
            "blob bytes still present after sweep"
        );
    }

    /// The sweeper never reclaims a blob that is still referenced: the guarded
    /// `delete_blob_record` removes no row, so the bytes stay and the artifact
    /// remains servable. This is the invariant the reordered delete (record first,
    /// bytes only when a row was actually removed) protects.
    #[tokio::test]
    async fn sweeper_keeps_referenced_blob() {
        let tm = new_test_manager().await;
        let repo = mk_repo(
            &tm.store,
            "mvn-keep",
            meta::TYPE_HOSTED,
            "",
            Config::default(),
        )
        .await;
        let path = "com/example/app/1.0/app-1.0.jar";

        tm.engine
            .put(
                &repo,
                path,
                "1.0",
                "application/java-archive",
                None,
                body("KEEPBYTES"),
                "",
            )
            .await
            .expect("put");
        let art = tm
            .store
            .get_artifact(repo.id, path)
            .await
            .expect("get artifact");

        tm.engine.sweep_once(Duration::ZERO).await;

        tm.store
            .get_blob(&art.blob_sha256)
            .await
            .expect("referenced blob record removed by sweep");
        assert!(
            tm.engine
                .blobs
                .exists(&art.blob_sha256)
                .await
                .expect("exists"),
            "referenced blob bytes removed by sweep"
        );
        // The artifact is still readable end to end.
        let (mut reader, size) = tm
            .engine
            .blobs
            .open(&art.blob_sha256)
            .await
            .expect("open blob");
        let mut buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf)
            .await
            .expect("read blob");
        assert_eq!(buf, b"KEEPBYTES");
        assert_eq!(size, 9);
    }
}
