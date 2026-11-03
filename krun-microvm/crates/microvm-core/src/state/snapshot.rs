// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use super::types::{SnapshotManifest, VmState, VmStatus};
use super::StateManager;
use anyhow::{bail, Result};
use std::fs;
use std::path::Path;

pub(crate) fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_p = entry.path();
        let dst_p = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_all(&src_p, &dst_p)?;
        } else {
            fs::copy(&src_p, &dst_p)?;
        }
    }
    Ok(())
}

impl StateManager {
    /// Captures a live or stopped microVM's state, configuration, and instance rootfs into a snapshot package.
    pub fn snapshot(
        data_dir: &Path,
        id_or_pid: &str,
        output_path: &Path,
    ) -> Result<SnapshotManifest> {
        let vm = match Self::find(data_dir, id_or_pid)? {
            Some(v) => v,
            None => bail!("MicroVM with ID or PID '{}' not found", id_or_pid),
        };

        let rootfs = vm.rootfs_path();
        if !rootfs.exists() {
            bail!("MicroVM rootfs path does not exist at {}", rootfs.display());
        }

        let runner_config = [
            vm.instance_dir.join("runner_config.json"),
            vm.instance_dir.join(".krun_config.json"),
        ]
        .into_iter()
        .find(|p| p.exists())
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|data| serde_json::from_str::<crate::types::RunnerConfig>(&data).ok());

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let manifest = SnapshotManifest {
            version: 1,
            original_id: vm.id.clone(),
            image: vm.image.clone(),
            created_at: now,
            port_forwards: vm.port_forwards.clone(),
            runner_config,
        };

        if output_path
            .extension()
            .is_some_and(|e| e == "tar" || e == "gz")
        {
            let tar_file = fs::File::create(output_path)?;
            let mut tar_builder = tar::Builder::new(tar_file);

            let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
            let mut header = tar::Header::new_gnu();
            header.set_size(manifest_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append_data(&mut header, "snapshot.json", &manifest_bytes[..])?;
            tar_builder.append_dir_all("rootfs", &rootfs)?;
            tar_builder.finish()?;
        } else {
            fs::create_dir_all(output_path)?;
            let manifest_path = output_path.join("snapshot.json");
            let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
            fs::write(manifest_path, manifest_bytes)?;

            let target_rootfs = output_path.join("rootfs");
            crate::rootfs::clone_rootfs(&rootfs, &target_rootfs)?;
        }

        Ok(manifest)
    }

    /// Restores a snapshot into a new microVM instance ready for immediate execution.
    pub fn restore(data_dir: &Path, snapshot_path: &Path, new_id: Option<&str>) -> Result<VmState> {
        if !snapshot_path.exists() {
            bail!("Snapshot path '{}' does not exist", snapshot_path.display());
        }

        let gen_id = format!(
            "vm-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let target_id = new_id.unwrap_or(&gen_id);
        let instances_dir = data_dir.join("instances").join(target_id);
        fs::create_dir_all(&instances_dir)?;

        let target_rootfs = instances_dir.join("rootfs");

        let manifest: SnapshotManifest = if snapshot_path.is_file() {
            let file = fs::File::open(snapshot_path)?;
            let mut archive = tar::Archive::new(file);
            let mut found_manifest = None;

            for entry_res in archive.entries()? {
                let mut entry = entry_res?;
                let path = entry.path()?.to_path_buf();
                if path == Path::new("snapshot.json") {
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut entry, &mut s)?;
                    found_manifest = serde_json::from_str::<SnapshotManifest>(&s).ok();
                } else if path.starts_with("rootfs") {
                    let rel = path.strip_prefix("rootfs")?;
                    let dest = crate::rootfs::tar_security::sanitize_tar_path(&target_rootfs, rel)?;
                    crate::rootfs::tar_security::ensure_no_symlink_parents(&target_rootfs, &dest)?;
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    entry.unpack(&dest)?;
                }
            }

            found_manifest.ok_or_else(|| anyhow::anyhow!("snapshot.json not found in archive"))?
        } else {
            let manifest_path = snapshot_path.join("snapshot.json");
            if !manifest_path.exists() {
                bail!("snapshot.json not found in {}", snapshot_path.display());
            }
            let data = fs::read_to_string(manifest_path)?;
            let m: SnapshotManifest = serde_json::from_str(&data)?;

            let src_rootfs = snapshot_path.join("rootfs");
            if src_rootfs.exists() {
                crate::rootfs::clone_rootfs(&src_rootfs, &target_rootfs)?;
            }
            m
        };

        if let Some(ref cfg) = manifest.runner_config {
            let json = serde_json::to_string_pretty(cfg)?;
            fs::write(instances_dir.join("runner_config.json"), &json)?;
            fs::write(instances_dir.join(".krun_config.json"), &json)?;
        }

        let new_state = VmState {
            id: target_id.to_string(),
            pid: 0,
            image: manifest.image,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            port_forwards: manifest.port_forwards,
            instance_dir: instances_dir,
            status: VmStatus::Stopped,
            vcpus: manifest.runner_config.as_ref().map(|c| c.num_vcpus),
            memory_mib: manifest.runner_config.as_ref().map(|c| c.ram_mib),
        };

        Self::save(data_dir, &new_state)?;
        Ok(new_state)
    }
}
