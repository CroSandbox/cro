// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Embedded ACID Storage Engine powered by `redb`.
//!
//! Replaces loose `state/*.json` files with crash-safe, transactional B-tree tables
//! for instances, snapshots, and runtime metadata, with automatic legacy migration.

use crate::state::types::{SnapshotManifest, VmState, VmStatus};
use anyhow::{Context, Result};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

pub const INSTANCES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("instances");
pub const SNAPSHOTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("snapshots");
pub const METADATA_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");

/// Embedded transactional ACID key-value store for microVM state.
#[derive(Clone)]
pub struct RedbStore {
    db: Arc<Database>,
    db_path: PathBuf,
}

impl RedbStore {
    /// Opens an existing `redb` database or creates a new one at `db_path`.
    pub fn open_or_create(db_path: impl Into<PathBuf>) -> Result<Self> {
        let db_path = db_path.into();
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let db = Database::create(&db_path)
            .with_context(|| format!("Failed to open or create redb database at {:?}", db_path))?;

        // Initialize tables within a write transaction
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(INSTANCES_TABLE)?;
            let _ = write_txn.open_table(SNAPSHOTS_TABLE)?;
            let _ = write_txn.open_table(METADATA_TABLE)?;
        }
        write_txn.commit()?;

        Ok(Self {
            db: Arc::new(db),
            db_path,
        })
    }

    /// Returns the filesystem path to the database file.
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    /// Saves or updates a `VmState` inside an ACID transaction.
    pub fn save_instance(&self, state: &VmState) -> Result<()> {
        let data = serde_json::to_vec(state)
            .with_context(|| format!("Failed to serialize VmState '{}'", state.id))?;

        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(INSTANCES_TABLE)?;
            table.insert(state.id.as_str(), data.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Fetches a `VmState` by exact instance ID.
    pub fn get_instance(&self, id: &str) -> Result<Option<VmState>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(INSTANCES_TABLE)?;

        if let Some(val) = table.get(id)? {
            let state: VmState = serde_json::from_slice(val.value())
                .with_context(|| format!("Failed to deserialize VmState for '{}'", id))?;
            Ok(Some(state))
        } else {
            Ok(None)
        }
    }

    /// Removes an instance state by ID in an ACID transaction.
    pub fn delete_instance(&self, id: &str) -> Result<Option<VmState>> {
        let write_txn = self.db.begin_write()?;
        let mut table = write_txn.open_table(INSTANCES_TABLE)?;
        let removed_state = match table.remove(id)? {
            Some(val) => {
                let state: VmState = serde_json::from_slice(val.value())?;
                Some(state)
            }
            None => None,
        };
        drop(table);
        write_txn.commit()?;
        Ok(removed_state)
    }

    /// Lists all stored instances, automatically populating process liveness and resources.
    pub fn list_instances(&self) -> Result<Vec<VmState>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(INSTANCES_TABLE)?;

        let mut states = Vec::new();
        let iter = table.iter()?;

        for item in iter {
            let (_key, val) = item?;
            if let Ok(mut vm) = serde_json::from_slice::<VmState>(val.value()) {
                vm.status = if vm.is_process_alive() {
                    if vm.status == VmStatus::Paused {
                        VmStatus::Paused
                    } else {
                        VmStatus::Running
                    }
                } else {
                    VmStatus::Stopped
                };

                if vm.vcpus.is_none() || vm.memory_mib.is_none() {
                    for cfg_name in &["runner_config.json", ".krun_config.json"] {
                        let cfg_p = vm.instance_dir.join(cfg_name);
                        if let Ok(cfg_data) = fs::read_to_string(&cfg_p) {
                            if let Ok(runner_cfg) =
                                serde_json::from_str::<crate::types::RunnerConfig>(&cfg_data)
                            {
                                if vm.vcpus.is_none() {
                                    vm.vcpus = Some(runner_cfg.num_vcpus);
                                }
                                if vm.memory_mib.is_none() {
                                    vm.memory_mib = Some(runner_cfg.ram_mib);
                                }
                                break;
                            }
                        }
                    }
                }

                states.push(vm);
            }
        }

        states.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        Ok(states)
    }

    /// Saves a snapshot manifest.
    pub fn save_snapshot(&self, key: &str, manifest: &SnapshotManifest) -> Result<()> {
        let data = serde_json::to_vec(manifest)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SNAPSHOTS_TABLE)?;
            table.insert(key, data.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Retrieves a snapshot manifest by key.
    pub fn get_snapshot(&self, key: &str) -> Result<Option<SnapshotManifest>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SNAPSHOTS_TABLE)?;

        if let Some(val) = table.get(key)? {
            let manifest: SnapshotManifest = serde_json::from_slice(val.value())?;
            Ok(Some(manifest))
        } else {
            Ok(None)
        }
    }

    /// Removes a snapshot manifest by key.
    pub fn delete_snapshot(&self, key: &str) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let mut table = write_txn.open_table(SNAPSHOTS_TABLE)?;
        let removed = table.remove(key)?.is_some();
        drop(table);
        write_txn.commit()?;
        Ok(removed)
    }

    /// Transparently migrates any legacy `*.json` state files from `json_dir` into redb.
    pub fn migrate_from_json_dir(&self, json_dir: &Path) -> Result<usize> {
        if !json_dir.exists() {
            return Ok(0);
        }

        let mut migrated = 0;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(INSTANCES_TABLE)?;

            for entry in fs::read_dir(json_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(vm) = serde_json::from_str::<VmState>(&content) {
                            if table.get(vm.id.as_str())?.is_none() {
                                table.insert(vm.id.as_str(), content.as_bytes())?;
                                migrated += 1;
                            }
                        }
                    }
                }
            }

            if migrated > 0 {
                let mut meta_table = write_txn.open_table(METADATA_TABLE)?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let info = format!("migrated_{}_instances_at_{}", migrated, now);
                meta_table.insert("legacy_json_migration", info.as_bytes())?;
            }
        }
        write_txn.commit()?;

        if migrated > 0 {
            info!(
                "📦 Transparently migrated {} legacy JSON states into redb ACID storage",
                migrated
            );
        }

        Ok(migrated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_redb_store_crud_lifecycle() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.redb");

        let store = RedbStore::open_or_create(&db_path).unwrap();

        let vm = VmState {
            id: "vm-redb-test-1".to_string(),
            pid: std::process::id(),
            image: "alpine:latest".to_string(),
            created_at: 12345,
            port_forwards: vec![],
            instance_dir: dir.path().join("instances/vm-redb-test-1"),
            status: VmStatus::Running,
            vcpus: Some(4),
            memory_mib: Some(2048),
        };

        // 1. Insert
        store.save_instance(&vm).unwrap();

        // 2. Read back
        let loaded = store.get_instance("vm-redb-test-1").unwrap().unwrap();
        assert_eq!(loaded.id, "vm-redb-test-1");
        assert_eq!(loaded.vcpus, Some(4));
        assert_eq!(loaded.memory_mib, Some(2048));

        // 3. List
        let list = store.list_instances().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "vm-redb-test-1");

        // 4. Delete
        let removed = store.delete_instance("vm-redb-test-1").unwrap();
        assert!(removed.is_some());
        assert!(store.get_instance("vm-redb-test-1").unwrap().is_none());
    }

    #[test]
    fn test_redb_transparent_migration_from_json() {
        let dir = tempdir().unwrap();
        let json_dir = dir.path().join("state");
        fs::create_dir_all(&json_dir).unwrap();

        // Create 3 legacy JSON files
        for i in 1..=3 {
            let vm = VmState {
                id: format!("legacy-vm-{}", i),
                pid: 0,
                image: "alpine:latest".to_string(),
                created_at: 1000 * i,
                port_forwards: vec![],
                instance_dir: dir.path().join(format!("instances/legacy-vm-{}", i)),
                status: VmStatus::Stopped,
                vcpus: Some(2),
                memory_mib: Some(1024),
            };
            let content = serde_json::to_string_pretty(&vm).unwrap();
            fs::write(json_dir.join(format!("legacy-vm-{}.json", i)), content).unwrap();
        }

        let store = RedbStore::open_or_create(dir.path().join("state.redb")).unwrap();
        let migrated = store.migrate_from_json_dir(&json_dir).unwrap();
        assert_eq!(migrated, 3);

        // Verify all 3 exist in redb
        let list = store.list_instances().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, "legacy-vm-3"); // newest first
        assert_eq!(list[2].id, "legacy-vm-1");

        // Running migration again is idempotent (0 additional migrated)
        let remigrated = store.migrate_from_json_dir(&json_dir).unwrap();
        assert_eq!(remigrated, 0);
    }
}
