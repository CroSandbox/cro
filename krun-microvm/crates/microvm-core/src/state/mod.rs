// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

pub mod snapshot;
pub mod store;
pub mod types;

#[cfg(test)]
mod tests;

use snapshot::*;
pub use store::*;
pub use types::*;

use anyhow::{bail, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Atomically writes content to a destination file using an ephemeral staging file and atomic rename.
static ATOMIC_WRITE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Atomically writes content to a destination file using an ephemeral staging file and atomic rename.
/// This prevents partial reads by concurrent readers and guards against write interruptions.
pub fn atomic_write<P: AsRef<Path>, C: AsRef<[u8]>>(dest: P, contents: C) -> Result<()> {
    let dest = dest.as_ref();
    let dir = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid destination path: no parent"))?;
    fs::create_dir_all(dir)?;
    let filename = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("target");
    let count = ATOMIC_WRITE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp_path = dir.join(format!(
        ".{}.tmp.{}.{}.{}",
        filename,
        std::process::id(),
        count,
        nonce
    ));

    if let Err(e) = fs::write(&tmp_path, contents) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    if let Err(e) = fs::rename(&tmp_path, dest) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    Ok(())
}

/// Manages persistence, lifecycle transitions, and inspect/prune/snapshot operations for microVM instances.
///
/// Can be instantiated with a specific `data_dir` to manage state with dependency injection,
/// or used via static helper methods taking `&Path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateManager {
    data_dir: PathBuf,
}

impl StateManager {
    /// Creates a new `StateManager` bound to the given base data directory.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Returns the base data directory managed by this instance.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Directory where JSON state files are saved (`<data_dir>/state`).
    pub fn state_dir_path(&self) -> PathBuf {
        Self::state_dir(&self.data_dir)
    }

    /// Saves or updates the state of a microVM instance.
    pub fn save_state(&self, state: &VmState) -> Result<PathBuf> {
        Self::save(&self.data_dir, state)
    }

    /// Removes the state file for a microVM instance by ID.
    pub fn remove_state(&self, id: &str) -> Result<()> {
        Self::remove(&self.data_dir, id)
    }

    /// Lists all persisted microVM instances, sorted by newest first.
    pub fn list_states(&self) -> Result<Vec<VmState>> {
        Self::list(&self.data_dir)
    }

    /// Finds a microVM by exact ID, ID prefix, or PID.
    pub fn find_state(&self, id_or_pid: &str) -> Result<Option<VmState>> {
        Self::find(&self.data_dir, id_or_pid)
    }

    /// Pauses execution of a running microVM.
    pub fn pause_vm(&self, id_or_pid: &str) -> Result<VmState> {
        Self::pause(&self.data_dir, id_or_pid)
    }

    /// Resumes execution of a paused microVM.
    pub fn resume_vm(&self, id_or_pid: &str) -> Result<VmState> {
        Self::resume(&self.data_dir, id_or_pid)
    }

    /// Dynamically resizes CPU and/or RAM limits of a running microVM.
    pub fn resize_vm(
        &self,
        id_or_pid: &str,
        memory_mib: Option<u32>,
        vcpus: Option<u8>,
    ) -> Result<VmState> {
        Self::resize(&self.data_dir, id_or_pid, memory_mib, vcpus)
    }

    /// Deletes a microVM instance state and instance directory.
    pub fn delete_vm(&self, id_or_pid: &str, force: bool) -> Result<VmState> {
        Self::delete(&self.data_dir, id_or_pid, force)
    }

    /// Stops a running microVM.
    pub fn stop_vm(&self, id_or_pid: &str) -> Result<()> {
        Self::stop(&self.data_dir, id_or_pid)
    }

    /// Prunes dead instances and temporary staging directories.
    pub fn prune_vms(&self) -> Result<PruneSummary> {
        Self::prune(&self.data_dir)
    }

    /// Copies a host file or directory into a microVM's rootfs.
    pub fn copy_into_vm(
        &self,
        id_or_pid: &str,
        src_host: &Path,
        guest_rel_path: &str,
    ) -> Result<()> {
        Self::copy_into(&self.data_dir, id_or_pid, src_host, guest_rel_path)
    }

    /// Copies a file or directory from a microVM's rootfs to the host.
    pub fn copy_from_vm(
        &self,
        id_or_pid: &str,
        guest_rel_path: &str,
        dst_host: &Path,
    ) -> Result<()> {
        Self::copy_from(&self.data_dir, id_or_pid, guest_rel_path, dst_host)
    }

    /// Executes a command in a running microVM.
    pub async fn exec_in_vm(
        &self,
        id_or_pid: &str,
        req: &crate::exec::ExecRequest,
    ) -> Result<crate::exec::ExecResponse> {
        Self::exec(&self.data_dir, id_or_pid, req).await
    }

    /// Captures a live or stopped microVM's state and rootfs into a snapshot package.
    pub fn snapshot_vm(&self, id_or_pid: &str, output_path: &Path) -> Result<SnapshotManifest> {
        Self::snapshot(&self.data_dir, id_or_pid, output_path)
    }

    /// Restores a snapshot into a new microVM instance ready for immediate execution.
    pub fn restore_vm(&self, snapshot_path: &Path, new_id: Option<&str>) -> Result<VmState> {
        Self::restore(&self.data_dir, snapshot_path, new_id)
    }

    pub fn state_dir(data_dir: &Path) -> PathBuf {
        data_dir.join("state")
    }

    /// Accesses or opens the redb ACID state database with transparent legacy JSON migration.
    pub fn store(data_dir: &Path) -> Result<RedbStore> {
        let db_path = data_dir.join("state.redb");
        let store = RedbStore::open_or_create(&db_path)?;
        let json_dir = Self::state_dir(data_dir);
        if json_dir.exists() {
            let _ = store.migrate_from_json_dir(&json_dir);
        }
        Ok(store)
    }

    pub fn save(data_dir: &Path, state: &VmState) -> Result<PathBuf> {
        let dir = Self::state_dir(data_dir);
        let file_path = dir.join(format!("{}.json", state.id));
        let data = serde_json::to_string_pretty(state)?;
        atomic_write(&file_path, data.as_bytes())?;

        // Also persist in redb ACID storage
        if let Ok(store) = Self::store(data_dir) {
            let _ = store.save_instance(state);
        }

        Ok(file_path)
    }

    pub fn remove(data_dir: &Path, id: &str) -> Result<()> {
        let file_path = Self::state_dir(data_dir).join(format!("{}.json", id));
        if file_path.exists() {
            let _ = fs::remove_file(file_path);
        }

        // Also remove from redb ACID storage
        if let Ok(store) = Self::store(data_dir) {
            let _ = store.delete_instance(id);
        }

        Ok(())
    }

    pub fn list(data_dir: &Path) -> Result<Vec<VmState>> {
        // Query redb ACID storage first
        if let Ok(store) = Self::store(data_dir) {
            if let Ok(instances) = store.list_instances() {
                if !instances.is_empty() {
                    return Ok(instances);
                }
            }
        }

        let dir = Self::state_dir(data_dir);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut vms = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(mut vm) = serde_json::from_str::<VmState>(&content) {
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
                                        serde_json::from_str::<crate::types::RunnerConfig>(
                                            &cfg_data,
                                        )
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
                        vms.push(vm);
                    }
                }
            }
        }

        vms.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        Ok(vms)
    }

    pub fn find(data_dir: &Path, id_or_pid: &str) -> Result<Option<VmState>> {
        // Direct O(1) B-tree lookup by exact ID from redb
        if let Ok(store) = Self::store(data_dir) {
            if let Ok(Some(mut vm)) = store.get_instance(id_or_pid) {
                vm.status = if vm.is_process_alive() {
                    if vm.status == VmStatus::Paused {
                        VmStatus::Paused
                    } else {
                        VmStatus::Running
                    }
                } else {
                    VmStatus::Stopped
                };
                return Ok(Some(vm));
            }
        }

        let vms = Self::list(data_dir)?;
        let target = vms.into_iter().find(|vm| {
            vm.id == id_or_pid || vm.id.starts_with(id_or_pid) || vm.pid.to_string() == id_or_pid
        });
        Ok(target)
    }

    pub fn pause(data_dir: &Path, id_or_pid: &str) -> Result<VmState> {
        let mut vm = match Self::find(data_dir, id_or_pid)? {
            Some(v) => v,
            None => bail!("MicroVM with ID or PID '{}' not found", id_or_pid),
        };

        if !vm.is_process_alive() {
            bail!("Cannot pause microVM '{}': process is not running", vm.id);
        }

        if vm.status == VmStatus::Paused {
            return Ok(vm);
        }

        let sock_path = vm.instance_dir.join("supervisor.sock");
        let mut ipc_success = false;
        if sock_path.exists() {
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(1)));
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let msg =
                    crate::protocol::ControlMessage::new(1, crate::protocol::MessagePayload::Pause);
                if crate::protocol::write_frame_sync(&mut stream, &msg).is_ok() {
                    if let Ok(ack) = crate::protocol::read_frame_sync(&mut stream) {
                        if matches!(ack.payload, crate::protocol::MessagePayload::Ack { .. }) {
                            ipc_success = true;
                        }
                    }
                }
            }
        }

        if !ipc_success {
            unsafe {
                if libc::kill(vm.pid as i32, libc::SIGSTOP) != 0 {
                    bail!(
                        "Failed to send SIGSTOP to process {}: {}",
                        vm.pid,
                        std::io::Error::last_os_error()
                    );
                }
            }
        }

        vm.status = VmStatus::Paused;
        Self::save(data_dir, &vm)?;
        Ok(vm)
    }

    pub fn resume(data_dir: &Path, id_or_pid: &str) -> Result<VmState> {
        let mut vm = match Self::find(data_dir, id_or_pid)? {
            Some(v) => v,
            None => bail!("MicroVM with ID or PID '{}' not found", id_or_pid),
        };

        if !vm.is_process_alive() {
            bail!("Cannot resume microVM '{}': process is not running", vm.id);
        }

        if vm.pid == 0 || vm.pid > i32::MAX as u32 {
            bail!("Cannot resume microVM '{}': invalid PID {}", vm.id, vm.pid);
        }

        let sock_path = vm.instance_dir.join("supervisor.sock");
        let mut ipc_success = false;
        if sock_path.exists() {
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(1)));
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let msg = crate::protocol::ControlMessage::new(
                    1,
                    crate::protocol::MessagePayload::Resume,
                );
                if crate::protocol::write_frame_sync(&mut stream, &msg).is_ok() {
                    if let Ok(ack) = crate::protocol::read_frame_sync(&mut stream) {
                        if matches!(ack.payload, crate::protocol::MessagePayload::Ack { .. }) {
                            ipc_success = true;
                        }
                    }
                }
            }
        }

        if !ipc_success {
            unsafe {
                if libc::kill(vm.pid as i32, libc::SIGCONT) != 0 {
                    bail!(
                        "Failed to send SIGCONT to process {}: {}",
                        vm.pid,
                        std::io::Error::last_os_error()
                    );
                }
            }
        }

        vm.status = VmStatus::Running;
        Self::save(data_dir, &vm)?;
        Ok(vm)
    }

    /// Dynamically resizes CPU and/or RAM limits of a running microVM in-place.
    pub fn resize(
        data_dir: &Path,
        id_or_pid: &str,
        memory_mib: Option<u32>,
        vcpus: Option<u8>,
    ) -> Result<VmState> {
        if memory_mib.is_none() && vcpus.is_none() {
            bail!("At least one resource (--memory or --cpus) must be specified to resize");
        }

        if let Some(mem) = memory_mib {
            if mem == 0 {
                bail!("Memory must be greater than 0 MiB");
            }
        }

        if let Some(cpus) = vcpus {
            if cpus == 0 {
                bail!("vCPUs must be greater than 0");
            }
        }

        let mut vm = match Self::find(data_dir, id_or_pid)? {
            Some(v) => v,
            None => bail!("MicroVM with ID or PID '{}' not found", id_or_pid),
        };

        if !vm.is_process_alive() {
            bail!("Cannot resize microVM '{}': process is not running", vm.id);
        }

        // Update runner_config.json and/or .krun_config.json if present in instance_dir
        for cfg_name in &["runner_config.json", ".krun_config.json"] {
            let cfg_path = vm.instance_dir.join(cfg_name);
            if cfg_path.exists() {
                if let Ok(content) = fs::read_to_string(&cfg_path) {
                    if let Ok(mut runner_cfg) =
                        serde_json::from_str::<crate::types::RunnerConfig>(&content)
                    {
                        if let Some(mem) = memory_mib {
                            runner_cfg.ram_mib = mem;
                        }
                        if let Some(cpus) = vcpus {
                            runner_cfg.num_vcpus = cpus;
                        }
                        if let Ok(updated) = serde_json::to_string_pretty(&runner_cfg) {
                            let _ = atomic_write(&cfg_path, updated.as_bytes());
                        }
                    }
                }
            }
        }

        if let Some(mem) = memory_mib {
            vm.memory_mib = Some(mem);
        }
        if let Some(cpus) = vcpus {
            vm.vcpus = Some(cpus);
        }

        Self::save(data_dir, &vm)?;
        Ok(vm)
    }

    pub fn delete(data_dir: &Path, id_or_pid: &str, force: bool) -> Result<VmState> {
        let vm = match Self::find(data_dir, id_or_pid)? {
            Some(vm) => vm,
            None => bail!("MicroVM with ID or PID '{}' not found", id_or_pid),
        };

        if vm.is_process_alive() {
            if !force {
                bail!(
                    "MicroVM '{}' (PID {}) is currently running. Stop it first or use --force (-f) to remove.",
                    vm.id,
                    vm.pid
                );
            }
            if vm.pid > 0 && vm.pid <= i32::MAX as u32 {
                unsafe {
                    libc::kill(vm.pid as i32, libc::SIGKILL);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        Self::remove(data_dir, &vm.id)?;
        if vm.instance_dir.exists() {
            let _ = fs::remove_dir_all(&vm.instance_dir);
        }

        Ok(vm)
    }

    pub fn stop(data_dir: &Path, id_or_pid: &str) -> Result<()> {
        let vm = match Self::find(data_dir, id_or_pid)? {
            Some(v) => v,
            None => bail!("MicroVM with ID or PID '{}' not found", id_or_pid),
        };

        if vm.is_process_alive() && vm.pid > 0 && vm.pid <= i32::MAX as u32 {
            unsafe {
                libc::kill(vm.pid as i32, libc::SIGTERM);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            if vm.is_process_alive() {
                unsafe {
                    libc::kill(vm.pid as i32, libc::SIGKILL);
                }
            }
        }
        Self::remove(data_dir, &vm.id)?;
        if vm.instance_dir.exists() {
            let _ = fs::remove_dir_all(&vm.instance_dir);
        }
        Ok(())
    }

    pub fn prune(data_dir: &Path) -> Result<PruneSummary> {
        let mut pruned_instances = 0;
        let mut pruned_layers = 0;

        // 1. Clean dead state files & dead instance directories
        let vms = Self::list(data_dir)?;
        for vm in vms {
            if !vm.is_process_alive() {
                Self::remove(data_dir, &vm.id)?;
                if vm.instance_dir.exists() {
                    let _ = fs::remove_dir_all(&vm.instance_dir);
                }
                pruned_instances += 1;
            }
        }

        // Clean orphaned directories in instances/
        let instances_dir = data_dir.join("instances");
        if instances_dir.exists() {
            for entry in fs::read_dir(&instances_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    let id = entry.file_name().to_string_lossy().to_string();
                    let state_file = Self::state_dir(data_dir).join(format!("{}.json", id));
                    if !state_file.exists() {
                        let _ = fs::remove_dir_all(&path);
                        pruned_instances += 1;
                    }
                }
            }
        }

        // 2. Clean temporary staging directories
        let staging_dir = data_dir.join("staging");
        if staging_dir.exists() {
            let _ = fs::remove_dir_all(&staging_dir);
            pruned_layers += 1;
        }

        Ok(PruneSummary {
            pruned_instances,
            pruned_layers,
        })
    }

    /// Copies a host file or directory into a microVM's rootfs.
    pub fn copy_into(
        data_dir: &Path,
        id_or_pid: &str,
        src_host: &Path,
        guest_rel_path: &str,
    ) -> Result<()> {
        let vms = Self::list(data_dir)?;
        let vm = vms
            .iter()
            .find(|v| {
                v.id == id_or_pid || v.id.starts_with(id_or_pid) || v.pid.to_string() == id_or_pid
            })
            .ok_or_else(|| anyhow::anyhow!("MicroVM '{}' not found", id_or_pid))?;

        let rootfs = vm.instance_dir.join("rootfs");
        let target_guest =
            crate::rootfs::tar_security::sanitize_tar_path(&rootfs, Path::new(guest_rel_path))?;
        crate::rootfs::tar_security::ensure_no_symlink_parents(&rootfs, &target_guest)?;

        // If target_guest itself is a symlink, remove it to prevent writing through to target
        if target_guest.is_symlink() {
            let _ = fs::remove_file(&target_guest);
        }

        if src_host.is_dir() {
            copy_dir_all(src_host, &target_guest)?;
        } else {
            if let Some(parent) = target_guest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(src_host, &target_guest)?;
        }
        Ok(())
    }

    /// Copies a file or directory from a microVM's rootfs to the host.
    pub fn copy_from(
        data_dir: &Path,
        id_or_pid: &str,
        guest_rel_path: &str,
        dst_host: &Path,
    ) -> Result<()> {
        let vms = Self::list(data_dir)?;
        let vm = vms
            .iter()
            .find(|v| {
                v.id == id_or_pid || v.id.starts_with(id_or_pid) || v.pid.to_string() == id_or_pid
            })
            .ok_or_else(|| anyhow::anyhow!("MicroVM '{}' not found", id_or_pid))?;

        let rootfs = vm.instance_dir.join("rootfs");
        let src_guest =
            crate::rootfs::tar_security::sanitize_tar_path(&rootfs, Path::new(guest_rel_path))?;
        crate::rootfs::tar_security::ensure_no_symlink_parents(&rootfs, &src_guest)?;

        let final_src = if src_guest.is_symlink() {
            let target = fs::read_link(&src_guest)?;
            crate::rootfs::tar_security::validate_symlink_target(&rootfs, &src_guest, &target)?;
            // If target is absolute, resolve relative to container rootfs, not host root
            let resolved = if target.is_absolute() {
                let rel = target.strip_prefix("/").unwrap_or(&target);
                rootfs.join(rel)
            } else {
                let parent = src_guest.parent().unwrap_or(&rootfs);
                parent.join(&target)
            };
            if !resolved.starts_with(&rootfs) {
                bail!("Security violation: symlink resolves outside container rootfs");
            }
            resolved
        } else {
            src_guest
        };

        if !final_src.exists() && !final_src.is_symlink() {
            bail!(
                "Path '{}' not found inside microVM '{}'",
                guest_rel_path,
                id_or_pid
            );
        }

        if final_src.is_dir() {
            copy_dir_all(&final_src, dst_host)?;
        } else {
            if let Some(parent) = dst_host.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&final_src, dst_host)?;
        }
        Ok(())
    }

    /// Executes a command in a running microVM.
    pub async fn exec(
        data_dir: &Path,
        id_or_pid: &str,
        req: &crate::exec::ExecRequest,
    ) -> Result<crate::exec::ExecResponse> {
        let vms = Self::list(data_dir)?;
        let vm = vms
            .iter()
            .find(|v| {
                v.id == id_or_pid || v.id.starts_with(id_or_pid) || v.pid.to_string() == id_or_pid
            })
            .ok_or_else(|| anyhow::anyhow!("MicroVM '{}' not found", id_or_pid))?;

        if vm.status != VmStatus::Running || !vm.is_process_alive() {
            bail!(
                "Cannot exec in microVM '{}' because it is not running (status: {:?})",
                id_or_pid,
                vm.status
            );
        }

        crate::exec::exec_in_microvm(&vm.exec_socket_path(), &vm.rootfs_path(), req).await
    }
}
