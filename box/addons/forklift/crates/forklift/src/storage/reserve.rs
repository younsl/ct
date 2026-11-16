//! Protects a freshly written blob from garbage collection on another pod.
//!
//! On SQLite one process writes and sweeps, and an in-process lock keeps the
//! sweeper away from bytes that are stored but not referenced yet. On
//! PostgreSQL the sweeper runs on another pod, so that lock cannot help. This
//! wrapper makes every `put` reserve the digest in the metadata store, which
//! restarts the blob's garbage collection grace period, before the caller goes
//! on to reference it.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::AsyncRead;

use super::{BlobStore, Error, Result, SeekableStore, WalkableStore};
use crate::meta;

/// A blob store whose writes are reserved in the metadata store.
pub struct ReservingStore {
    inner: Arc<dyn WalkableStore>,
    meta: Arc<meta::Store>,
}

impl ReservingStore {
    pub fn new(inner: Arc<dyn WalkableStore>, meta: Arc<meta::Store>) -> Self {
        Self { inner, meta }
    }
}

#[async_trait]
impl BlobStore for ReservingStore {
    /// Stores the bytes, reserves the digest, then confirms the bytes are
    /// still there.
    ///
    /// The sweeper deletes a blob's record and its bytes inside one
    /// transaction, so the reservation either lands first and makes the
    /// sweeper skip the digest, or waits for the sweeper to finish. In the
    /// second case the bytes written above are gone, and the last check turns
    /// that into an error instead of a reference to nothing.
    async fn put(&self, r: Pin<Box<dyn AsyncRead + Send>>) -> Result<(String, i64)> {
        let (digest, size) = self.inner.put(r).await?;
        self.meta
            .reserve_blob(&digest, size)
            .await
            .map_err(|e| Error::Other(format!("reserve blob: {e}")))?;
        if !self.inner.exists(&digest).await? {
            return Err(Error::Other(
                "blob was garbage collected while it was being stored, retry the upload".into(),
            ));
        }
        Ok((digest, size))
    }

    async fn open(&self, digest: &str) -> Result<(Box<dyn AsyncRead + Send + Unpin>, i64)> {
        self.inner.open(digest).await
    }

    async fn exists(&self, digest: &str) -> Result<bool> {
        self.inner.exists(digest).await
    }

    async fn delete(&self, digest: &str) -> Result<()> {
        self.inner.delete(digest).await
    }

    async fn size(&self, digest: &str) -> Result<Option<i64>> {
        self.inner.size(digest).await
    }

    fn as_seekable(&self) -> Option<&dyn SeekableStore> {
        self.inner.as_seekable()
    }
}

#[async_trait]
impl WalkableStore for ReservingStore {
    async fn walk_digests(
        &self,
        f: &mut (dyn for<'a> FnMut(&'a str) -> Result<()> + Send),
    ) -> Result<()> {
        self.inner.walk_digests(f).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FsStore;

    async fn env() -> (
        ReservingStore,
        Arc<meta::Store>,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let (store, db_dir) = crate::testing::meta::test_store().await;
        let store = Arc::new(store);
        let blob_dir = tempfile::tempdir().expect("blob dir");
        let fs = FsStore::new(blob_dir.path()).expect("fs store");
        (
            ReservingStore::new(Arc::new(fs), Arc::clone(&store)),
            store,
            db_dir,
            blob_dir,
        )
    }

    fn body(s: &'static str) -> Pin<Box<dyn AsyncRead + Send>> {
        Box::pin(std::io::Cursor::new(s.as_bytes()))
    }

    /// A stored blob gets a record right away, stamped as unreferenced now, so
    /// a sweep with any cutoff in the past leaves it alone.
    #[tokio::test]
    async fn put_reserves_the_digest() {
        let (blobs, store, _db, _dir) = env().await;
        let (digest, size) = blobs.put(body("reserved")).await.expect("put");
        assert_eq!(size, 8);
        assert_eq!(store.get_blob(&digest).await.expect("record").ref_count, 0);

        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        assert!(
            store
                .list_unreferenced_blobs(10, past)
                .await
                .expect("list")
                .is_empty(),
            "a just-reserved blob is inside the grace period"
        );
        assert!(blobs.exists(&digest).await.expect("exists"));
    }

    /// Storing content again restarts the grace period of an unreferenced
    /// blob that was about to be reclaimed.
    #[tokio::test]
    async fn put_again_restarts_the_grace_period() {
        let (blobs, store, _db, _dir) = env().await;
        let (digest, _) = blobs.put(body("again")).await.expect("put");
        let future = chrono::Utc::now() + chrono::Duration::hours(48);
        assert_eq!(
            store
                .list_unreferenced_blobs(10, future)
                .await
                .expect("list"),
            vec![digest.clone()],
            "past its grace period from the future's point of view"
        );
        // A real reclaim at this point would remove it. A second put instead
        // re-stamps it, which the delete guard below observes.
        blobs.put(body("again")).await.expect("put again");
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(1);
        assert!(
            !store
                .delete_blob_record(&digest, cutoff)
                .await
                .expect("guard"),
            "re-reserved blob must not be deleted"
        );
    }

    #[tokio::test]
    async fn reads_and_walks_forward_to_the_inner_store() {
        let (blobs, _store, _db, _dir) = env().await;
        let (digest, size) = blobs.put(body("forward")).await.expect("put");
        assert_eq!(blobs.size(&digest).await.expect("size"), Some(size));
        let (_, n) = blobs.open(&digest).await.expect("open");
        assert_eq!(n, size);
        assert!(blobs.as_seekable().is_some());
        let mut seen = Vec::new();
        blobs
            .walk_digests(&mut |d| {
                seen.push(d.to_string());
                Ok(())
            })
            .await
            .expect("walk");
        assert_eq!(seen, vec![digest.clone()]);
        blobs.delete(&digest).await.expect("delete");
        assert!(!blobs.exists(&digest).await.expect("exists"));
    }
}
