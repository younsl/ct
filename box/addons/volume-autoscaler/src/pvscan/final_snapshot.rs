//! Final snapshot: an opt-in Velero `Backup` of a claim taken right before
//! orphan PVC cleanup deletes it.
//!
//! The addon never snapshots a volume by itself. It creates one Velero
//! `Backup` per claim and deletes the claim only after that backup reports
//! `Completed` with at least one volume snapshot. A backup that fails, or
//! completes without a snapshot, keeps the claim.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::types::Finding;

pub const BACKUP_API_VERSION: &str = "velero.io/v1";
pub const BACKUP_KIND: &str = "Backup";
pub const BACKUP_PLURAL: &str = "backups";

/// The claim label a backup selects on. Its value is the backup name, so one
/// backup holds exactly one claim.
pub const LABEL_FINAL_SNAPSHOT: &str = "final-snapshot";

const NAME_PREFIX: &str = "orphan-pvc";
const MANAGED_BY: &str = "volume-autoscaler";

/// How the final backup is taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The namespace Velero watches for `Backup` objects.
    pub namespace: String,
    /// How long Velero keeps the backup and its snapshots.
    pub ttl: Duration,
    /// Empty uses Velero's default `BackupStorageLocation`.
    pub storage_location: String,
    /// Empty uses Velero's default `VolumeSnapshotLocations`.
    pub volume_snapshot_locations: Vec<String>,
    /// Uploads the snapshot data to the backup storage location.
    pub snapshot_move_data: bool,
}

/// Where a claim's final backup stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending(String),
    Completed,
    Failed(String),
}

/// Names the backup after the claim UID and its deadline, so a claim that is
/// canceled and scheduled again never reuses a backup taken for the earlier
/// schedule.
#[must_use]
pub fn backup_name(uid: &str, delete_after: DateTime<Utc>) -> String {
    format!("{NAME_PREFIX}-{uid}-{}", delete_after.timestamp())
}

impl Settings {
    /// Builds the `Backup` that holds one claim and the volume behind it.
    /// File system backup is off because no Pod mounts an orphan claim.
    #[must_use]
    pub fn backup(&self, f: &Finding, name: &str, label: &str) -> Value {
        let mut spec = json!({
            "includedNamespaces": [f.namespace],
            "labelSelector": {"matchLabels": {label: name}},
            "snapshotVolumes": true,
            "defaultVolumesToFsBackup": false,
            "ttl": format!("{}s", self.ttl.as_secs()),
        });
        if !self.storage_location.is_empty() {
            spec["storageLocation"] = self.storage_location.clone().into();
        }
        if !self.volume_snapshot_locations.is_empty() {
            spec["volumeSnapshotLocations"] = self.volume_snapshot_locations.clone().into();
        }
        if self.snapshot_move_data {
            spec["snapshotMoveData"] = true.into();
        }
        json!({
            "apiVersion": BACKUP_API_VERSION,
            "kind": BACKUP_KIND,
            "metadata": {
                "name": name,
                "namespace": self.namespace,
                "labels": {"app.kubernetes.io/managed-by": MANAGED_BY},
                "annotations": {
                    format!("{label}-claim"): f.key(),
                    format!("{label}-volume"): f.volume_name,
                    format!("{label}-volume-id"): f.volume_id,
                },
            },
            "spec": spec,
        })
    }
}

/// Reads a `Backup`'s progress. Only `Completed` with a volume snapshot
/// counts: a backup that finished without one holds the claim manifest and
/// none of its data.
#[must_use]
pub fn progress(backup: &Value) -> Progress {
    let status = &backup["status"];
    let phase = status["phase"].as_str().unwrap_or_default();
    match phase {
        "Completed" => {
            let snapshots: u64 = [
                "volumeSnapshotsCompleted",
                "csiVolumeSnapshotsCompleted",
                "backupItemOperationsCompleted",
            ]
            .iter()
            .filter_map(|k| status[*k].as_u64())
            .sum();
            if snapshots == 0 {
                Progress::Failed("backup completed without a volume snapshot".into())
            } else {
                Progress::Completed
            }
        }
        "Failed"
        | "PartiallyFailed"
        | "FailedValidation"
        | "WaitingForPluginOperationsPartiallyFailed"
        | "FinalizingPartiallyFailed"
        | "Deleting" => {
            let detail = status["failureReason"]
                .as_str()
                .map(ToString::to_string)
                .or_else(|| {
                    status["validationErrors"].as_array().map(|errs| {
                        errs.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                })
                .filter(|d| !d.is_empty());
            Progress::Failed(detail.map_or_else(
                || format!("backup phase {phase}"),
                |d| format!("backup phase {phase}: {d}"),
            ))
        }
        "" => Progress::Pending("New".into()),
        other => Progress::Pending(other.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pvscan::types::KIND_PVC;

    fn settings() -> Settings {
        Settings {
            namespace: "velero".into(),
            ttl: Duration::from_hours(720),
            storage_location: String::new(),
            volume_snapshot_locations: vec![],
            snapshot_move_data: false,
        }
    }

    fn claim() -> Finding {
        Finding {
            kind: KIND_PVC.into(),
            namespace: "legacy".into(),
            name: "uploads".into(),
            uid: "0b1563dc-9412-4cbb-8658-a4d4d019ddfa".into(),
            volume_name: "pv-uploads".into(),
            volume_id: "vol-0fedcba9876543210".into(),
            ..Finding::default()
        }
    }

    #[test]
    fn backup_name_fits_a_label_value_and_follows_the_deadline() {
        let t = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let name = backup_name(&claim().uid, t);
        assert_eq!(
            name,
            "orphan-pvc-0b1563dc-9412-4cbb-8658-a4d4d019ddfa-1700000000"
        );
        assert!(name.len() <= 63);
        assert_ne!(
            name,
            backup_name(&claim().uid, t + chrono::TimeDelta::hours(1))
        );
    }

    #[test]
    fn backup_selects_one_claim() {
        let label = "volume-autoscaler.sh/final-snapshot";
        let b = settings().backup(&claim(), "orphan-pvc-x", label);
        assert_eq!(b["apiVersion"], "velero.io/v1");
        assert_eq!(b["metadata"]["namespace"], "velero");
        assert_eq!(b["metadata"]["name"], "orphan-pvc-x");
        assert_eq!(
            b["metadata"]["annotations"]["volume-autoscaler.sh/final-snapshot-claim"],
            "legacy/uploads"
        );
        assert_eq!(
            b["spec"],
            json!({
                "includedNamespaces": ["legacy"],
                "labelSelector": {"matchLabels": {label: "orphan-pvc-x"}},
                "snapshotVolumes": true,
                "defaultVolumesToFsBackup": false,
                "ttl": "2592000s",
            })
        );

        let s = Settings {
            storage_location: "primary".into(),
            volume_snapshot_locations: vec!["aws-default".into()],
            snapshot_move_data: true,
            ..settings()
        };
        let b = s.backup(&claim(), "orphan-pvc-x", label);
        assert_eq!(b["spec"]["storageLocation"], "primary");
        assert_eq!(b["spec"]["volumeSnapshotLocations"], json!(["aws-default"]));
        assert_eq!(b["spec"]["snapshotMoveData"], true);
    }

    #[test]
    fn progress_by_phase() {
        let cases = [
            (json!({}), Progress::Pending("New".into())),
            (
                json!({"status": {"phase": "InProgress"}}),
                Progress::Pending("InProgress".into()),
            ),
            (
                json!({"status": {"phase": "WaitingForPluginOperations"}}),
                Progress::Pending("WaitingForPluginOperations".into()),
            ),
            (
                json!({"status": {"phase": "Completed", "volumeSnapshotsCompleted": 1}}),
                Progress::Completed,
            ),
            (
                json!({"status": {"phase": "Completed", "csiVolumeSnapshotsCompleted": 1}}),
                Progress::Completed,
            ),
            (
                json!({"status": {"phase": "Completed", "backupItemOperationsCompleted": 1}}),
                Progress::Completed,
            ),
            (
                json!({"status": {"phase": "Completed", "volumeSnapshotsCompleted": 0}}),
                Progress::Failed("backup completed without a volume snapshot".into()),
            ),
            (
                json!({"status": {"phase": "PartiallyFailed"}}),
                Progress::Failed("backup phase PartiallyFailed".into()),
            ),
            (
                json!({"status": {"phase": "Failed", "failureReason": "no location"}}),
                Progress::Failed("backup phase Failed: no location".into()),
            ),
            (
                json!({"status": {"phase": "FailedValidation", "validationErrors": ["a", "b"]}}),
                Progress::Failed("backup phase FailedValidation: a, b".into()),
            ),
            (
                json!({"status": {"phase": "Deleting"}}),
                Progress::Failed("backup phase Deleting".into()),
            ),
        ];
        for (backup, want) in cases {
            assert_eq!(progress(&backup), want, "{backup}");
        }
    }
}
