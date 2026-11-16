//! Shared staging for chunked OCI blob uploads.
//!
//! A chunked push spans several requests (start, append, finish). With one
//! serving pod the bytes accumulate in a local session file. When several pods
//! serve, each request can land on a different pod, so the chunks are staged
//! in the object store all pods share and stitched together on finish.

use std::collections::VecDeque;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio_util::io::StreamReader;

use crate::objstore::{ObjectApi, PutBody, PutObjectInput};

/// Where the parts of an upload session live when pods do not share a disk.
#[async_trait]
pub trait PartStaging: Send + Sync {
    /// Stores the bytes of `file` as the part starting at `offset`. Storing a
    /// part again at the same offset replaces it, which is what a client retry
    /// of a lost chunk needs.
    async fn put_part(
        &self,
        session: &str,
        offset: i64,
        file: &Path,
        len: i64,
    ) -> std::io::Result<()>;
    /// Reads every part of the session in offset order as one stream.
    async fn open(&self, session: &str) -> std::io::Result<Pin<Box<dyn AsyncRead + Send + Unpin>>>;
    /// Deletes every part of the session.
    async fn remove(&self, session: &str) -> std::io::Result<()>;
}

/// [`PartStaging`] over the object store, one object per chunk.
pub struct ObjectStaging {
    api: Arc<dyn ObjectApi>,
    bucket: String,
    prefix: String,
}

impl ObjectStaging {
    /// `prefix` is the blob store's key prefix. The parts go under
    /// `<prefix>/oci-uploads/`, next to the blobs and never among them.
    pub fn new(api: Arc<dyn ObjectApi>, bucket: &str, prefix: &str) -> Self {
        let prefix = prefix.trim_matches('/');
        let prefix = if prefix.is_empty() {
            "oci-uploads".to_string()
        } else {
            format!("{prefix}/oci-uploads")
        };
        Self {
            api,
            bucket: bucket.to_string(),
            prefix,
        }
    }

    fn session_prefix(&self, session: &str) -> String {
        format!("{}/{session}/", self.prefix)
    }
}

/// What the stitched reader carries between chunks.
struct Parts {
    api: Arc<dyn ObjectApi>,
    bucket: String,
    keys: VecDeque<String>,
    current: Option<Box<dyn AsyncRead + Send + Unpin>>,
}

const READ_CHUNK: usize = 64 * 1024;

#[async_trait]
impl PartStaging for ObjectStaging {
    async fn put_part(
        &self,
        session: &str,
        offset: i64,
        file: &Path,
        len: i64,
    ) -> std::io::Result<()> {
        // Zero-padded so the listing's lexicographic order is offset order.
        let key = format!("{}{offset:020}", self.session_prefix(session));
        self.api
            .put_object(PutObjectInput {
                bucket: self.bucket.clone(),
                key,
                body: PutBody::File(file.to_path_buf()),
                content_length: len,
                metadata: std::collections::HashMap::new(),
                if_match: None,
                if_none_match: None,
            })
            .await
            .map(|_| ())
            .map_err(std::io::Error::other)
    }

    async fn open(&self, session: &str) -> std::io::Result<Pin<Box<dyn AsyncRead + Send + Unpin>>> {
        let keys = self
            .api
            .list_keys(&self.bucket, &self.session_prefix(session))
            .await
            .map_err(std::io::Error::other)?;
        let state = Parts {
            api: Arc::clone(&self.api),
            bucket: self.bucket.clone(),
            keys: keys.into(),
            current: None,
        };
        let stream = futures_util::stream::try_unfold(state, |mut st| async move {
            loop {
                if st.current.is_none() {
                    let Some(key) = st.keys.pop_front() else {
                        return Ok::<_, std::io::Error>(None);
                    };
                    let part = st
                        .api
                        .get_object(&st.bucket, &key)
                        .await
                        .map_err(std::io::Error::other)?;
                    st.current = Some(part.body);
                }
                let mut buf = BytesMut::with_capacity(READ_CHUNK);
                let n = match st.current.as_mut() {
                    Some(body) => body.read_buf(&mut buf).await?,
                    None => 0,
                };
                if n == 0 {
                    st.current = None;
                    continue;
                }
                return Ok(Some((buf.freeze(), st)));
            }
        });
        Ok(Box::pin(StreamReader::new(Box::pin(stream))))
    }

    async fn remove(&self, session: &str) -> std::io::Result<()> {
        let keys = self
            .api
            .list_keys(&self.bucket, &self.session_prefix(session))
            .await
            .map_err(std::io::Error::other)?;
        for key in keys {
            self.api
                .delete_object(&self.bucket, &key)
                .await
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::objstore::{
        Error, GetObjectOutput, HeadObjectOutput, PutObjectOutput, Result as ObjResult,
    };

    /// An in-memory bucket.
    #[derive(Default)]
    pub(crate) struct MemBucket {
        pub(crate) objects: parking_lot::Mutex<BTreeMap<String, Vec<u8>>>,
    }

    #[async_trait]
    impl ObjectApi for MemBucket {
        async fn put_object(&self, input: PutObjectInput) -> ObjResult<PutObjectOutput> {
            let bytes = match &input.body {
                PutBody::Bytes(b) => b.clone(),
                PutBody::File(p) => tokio::fs::read(p)
                    .await
                    .map_err(|e| Error::ObjectStore(e.to_string()))?,
            };
            self.objects.lock().insert(input.key, bytes);
            Ok(PutObjectOutput::default())
        }

        async fn get_object(&self, _bucket: &str, key: &str) -> ObjResult<GetObjectOutput> {
            let bytes = self
                .objects
                .lock()
                .get(key)
                .cloned()
                .ok_or(Error::NotFound)?;
            Ok(GetObjectOutput {
                content_length: Some(bytes.len() as i64),
                body: Box::new(std::io::Cursor::new(bytes)),
                e_tag: None,
            })
        }

        async fn head_object(&self, _bucket: &str, _key: &str) -> ObjResult<HeadObjectOutput> {
            Ok(HeadObjectOutput::default())
        }

        async fn delete_object(&self, _bucket: &str, key: &str) -> ObjResult<()> {
            self.objects.lock().remove(key);
            Ok(())
        }

        async fn list_keys(&self, _bucket: &str, prefix: &str) -> ObjResult<Vec<String>> {
            Ok(self
                .objects
                .lock()
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }
    }

    async fn put(staging: &ObjectStaging, dir: &Path, session: &str, offset: i64, data: &[u8]) {
        let file = dir.join("part");
        tokio::fs::write(&file, data).await.expect("write part");
        staging
            .put_part(session, offset, &file, data.len() as i64)
            .await
            .expect("put part");
    }

    async fn read_all(staging: &ObjectStaging, session: &str) -> Vec<u8> {
        let mut out = Vec::new();
        staging
            .open(session)
            .await
            .expect("open")
            .read_to_end(&mut out)
            .await
            .expect("read");
        out
    }

    /// Parts come back in offset order whatever order they were stored in,
    /// including offsets whose decimal forms would sort differently as text.
    #[tokio::test]
    async fn parts_are_stitched_in_offset_order() {
        let bucket = Arc::new(MemBucket::default());
        let staging = ObjectStaging::new(Arc::clone(&bucket) as Arc<dyn ObjectApi>, "b", "blobs/");
        let dir = tempfile::tempdir().expect("dir");

        put(&staging, dir.path(), "aa", 10, b"-third").await;
        put(&staging, dir.path(), "aa", 0, b"first").await;
        put(&staging, dir.path(), "aa", 5, b"-next").await;
        put(&staging, dir.path(), "bb", 0, b"other session").await;

        assert_eq!(read_all(&staging, "aa").await, b"first-next-third");
        assert!(
            bucket
                .objects
                .lock()
                .keys()
                .all(|k| k.starts_with("blobs/oci-uploads/")),
            "parts live under the staging prefix"
        );
    }

    /// A retried chunk replaces the part at its offset.
    #[tokio::test]
    async fn retried_part_replaces_the_earlier_one() {
        let bucket = Arc::new(MemBucket::default());
        let staging = ObjectStaging::new(bucket as Arc<dyn ObjectApi>, "b", "");
        let dir = tempfile::tempdir().expect("dir");
        put(&staging, dir.path(), "aa", 0, b"lost").await;
        put(&staging, dir.path(), "aa", 0, b"retried").await;
        assert_eq!(read_all(&staging, "aa").await, b"retried");
    }

    #[tokio::test]
    async fn remove_deletes_only_that_session() {
        let bucket = Arc::new(MemBucket::default());
        let staging = ObjectStaging::new(Arc::clone(&bucket) as Arc<dyn ObjectApi>, "b", "");
        let dir = tempfile::tempdir().expect("dir");
        put(&staging, dir.path(), "aa", 0, b"a").await;
        put(&staging, dir.path(), "bb", 0, b"b").await;

        staging.remove("aa").await.expect("remove");
        assert_eq!(read_all(&staging, "aa").await, Vec::<u8>::new());
        assert_eq!(read_all(&staging, "bb").await, b"b");
        assert_eq!(bucket.objects.lock().len(), 1);
    }

    /// A part larger than one read buffer is streamed whole.
    #[tokio::test]
    async fn large_parts_stream_across_read_chunks() {
        let bucket = Arc::new(MemBucket::default());
        let staging = ObjectStaging::new(bucket as Arc<dyn ObjectApi>, "b", "");
        let dir = tempfile::tempdir().expect("dir");
        let big = vec![7u8; READ_CHUNK * 3 + 11];
        put(&staging, dir.path(), "aa", 0, &big).await;
        put(&staging, dir.path(), "aa", big.len() as i64, b"tail").await;
        let got = read_all(&staging, "aa").await;
        assert_eq!(got.len(), big.len() + 4);
        assert!(got.ends_with(b"tail"));
    }
}
