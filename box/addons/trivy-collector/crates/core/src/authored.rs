//! Wire formats of the state a person authors: report notes in a ConfigMap and
//! API tokens in a Secret. The server owns both objects. The formats live here
//! so the scraper's one-shot `export-state` can write them without linking the
//! server.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A ConfigMap caps at roughly 1MiB in total. That is ample for the current
/// volume but it is a hard wall rather than a soft one, so writes are rejected
/// before the object can become untruncatable.
pub const MAX_NOTE_BYTES: usize = 8 * 1024;
pub const MAX_TOTAL_BYTES: usize = 800 * 1024;

/// Length of the lookup prefix: `tc_` plus 8 hex characters.
pub const TOKEN_PREFIX_LEN: usize = 11;

/// One stored note. The identity tuple is carried in the value because the key
/// is a digest and cannot be reversed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub cluster: String,
    pub report_type: String,
    pub namespace: String,
    pub name: String,
    pub notes: String,
    pub notes_created_at: Option<String>,
    pub notes_updated_at: Option<String>,
}

/// Stable ConfigMap key for a report identity.
#[must_use]
pub fn note_key(cluster: &str, report_type: &str, namespace: &str, name: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [cluster, report_type, namespace, name] {
        hasher.update(part.as_bytes());
        hasher.update(b"/");
    }
    hex::encode(hasher.finalize())
}

/// One stored token, keyed in the Secret by its prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredToken {
    pub user_sub: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub token_hash: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
}

impl StoredToken {
    #[must_use]
    pub fn is_expired(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        chrono::DateTime::parse_from_rfc3339(&self.expires_at).is_ok_and(|exp| now >= exp)
    }
}

/// Compare two hex digests without leaking a match position through timing.
#[must_use]
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}
