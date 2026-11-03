// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Developer machine lifecycle manager.
//!
//! Handles creation, boot, execution, and persistent storage for isolated Linux developer environments.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

use super::spec::{
    generate_machine_init_script, get_host_user_info, resolve_distro_image, validate_machine_name,
    MachineConfig,
};
use crate::state::{StateManager, VmStatus};
use crate::types::VirtioFsMount;
use crate::vm::MicroVmBuilder;

/// Subsystem for creating, inspecting, and managing persistent Linux workstations.
pub struct MachineManager;

impl MachineManager {
    /// Returns the storage directory for a specific machine (`<data_dir>/machines/<name>`).
    pub fn machine_dir(base_data_dir: &Path, name: &str) -> PathBuf {
        base_data_dir.join("machines").join(name)
    }

    /// Returns the path to the machine configuration JSON (`<machine_dir>/config.json`).
    pub fn config_path(base_data_dir: &Path, name: &str) -> PathBuf {
        Self::machine_dir(base_data_dir, name).join("config.json")
    }

    /// Creates and immediately boots a new persistent Linux developer machine.
    pub async fn create(
        base_data_dir: &Path,
        name: &str,
        distro: &str,
        image_override: Option<&str>,
        cpus: Option<u8>,
        memory_mb: Option<u32>,
        rosetta: bool,
    ) -> Result<MachineConfig> {
        validate_machine_name(name)?;

        let mdir = Self::machine_dir(base_data_dir, name);
        let cfg_path = Self::config_path(base_data_dir, name);
        if cfg_path.exists() {
            bail!("Developer machine '{}' already exists", name);
        }

        fs::create_dir_all(&mdir)
            .with_context(|| format!("Failed to create machine directory at {}", mdir.display()))?;

        let image = image_override
            .map(str::to_string)
            .unwrap_or_else(|| resolve_distro_image(distro));

        let cpus_val = cpus.unwrap_or(2);
        let memory_val = memory_mb.unwrap_or(2048);
        let instance_id = format!("machine-{}", name);

        let (user, uid, gid) = get_host_user_info();
        let init_script = generate_machine_init_script(&user, uid, gid);
        let init_path = mdir.join("init.sh");
        fs::write(&init_path, init_script)?;

        info!(
            "Provisioning machine '{}' ({}) with {} vCPUs, {} MiB RAM...",
            name, image, cpus_val, memory_val
        );

        let mut builder = MicroVmBuilder::new(&image)
            .instance_id(&instance_id)
            .hostname(name)
            .cpus(cpus_val)
            .memory_mb(memory_val)
            .data_dir(base_data_dir)
            .detach(true)
            .rosetta(rosetta)
            .file_mount(&init_path, "/krun-machine-init.sh")
            .cmd(vec![
                "/bin/sh".to_string(),
                "/krun-machine-init.sh".to_string(),
            ]);

        // Path parity: Mount /Users on macOS or /home on Linux
        let host_users = if Path::new("/Users").exists() {
            PathBuf::from("/Users")
        } else {
            PathBuf::from("/home")
        };
        if host_users.exists() {
            builder = builder.virtiofs_mount(VirtioFsMount::new("users", host_users, false));
        }

        let vm = builder
            .run()
            .await
            .with_context(|| format!("Failed to start microVM instance for machine '{}'", name))?;

        let config = MachineConfig {
            name: name.to_string(),
            distro: distro.to_string(),
            image,
            cpus: cpus_val,
            memory_mb: memory_val,
            rosetta,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            instance_id: vm.id().to_string(),
            status: "running".to_string(),
        };

        fs::write(&cfg_path, serde_json::to_string_pretty(&config)?)?;
        info!("Machine '{}' is now running (ID: {})", name, vm.id());

        Ok(config)
    }

    /// Loads and updates the status of an existing machine by name.
    pub fn get(base_data_dir: &Path, name: &str) -> Result<MachineConfig> {
        let cfg_path = Self::config_path(base_data_dir, name);
        if !cfg_path.exists() {
            bail!("Developer machine '{}' not found", name);
        }

        let content = fs::read_to_string(&cfg_path)?;
        let mut config: MachineConfig = serde_json::from_str(&content)?;

        // Update live status dynamically
        if let Ok(Some(vm_state)) = StateManager::find(base_data_dir, &config.instance_id) {
            config.status = if vm_state.status == VmStatus::Running && vm_state.is_process_alive() {
                "running".to_string()
            } else {
                "stopped".to_string()
            };
        } else {
            config.status = "stopped".to_string();
        }

        Ok(config)
    }

    /// Lists all persisted developer machines with real-time runtime status.
    pub fn list(base_data_dir: &Path) -> Result<Vec<MachineConfig>> {
        let machines_root = base_data_dir.join("machines");
        if !machines_root.is_dir() {
            return Ok(Vec::new());
        }

        let mut list = Vec::new();
        for entry in fs::read_dir(&machines_root)?.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Ok(cfg) = Self::get(base_data_dir, name) {
                        list.push(cfg);
                    }
                }
            }
        }

        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    /// Starts a stopped developer machine.
    pub async fn start(base_data_dir: &Path, name: &str) -> Result<MachineConfig> {
        let mut config = Self::get(base_data_dir, name)?;
        if config.status == "running" {
            info!("Machine '{}' is already running", name);
            return Ok(config);
        }

        let mdir = Self::machine_dir(base_data_dir, name);
        let init_path = mdir.join("init.sh");

        let mut builder = MicroVmBuilder::new(&config.image)
            .instance_id(&config.instance_id)
            .hostname(name)
            .cpus(config.cpus)
            .memory_mb(config.memory_mb)
            .data_dir(base_data_dir)
            .detach(true)
            .rosetta(config.rosetta)
            .file_mount(&init_path, "/krun-machine-init.sh")
            .cmd(vec![
                "/bin/sh".to_string(),
                "/krun-machine-init.sh".to_string(),
            ]);

        let host_users = if Path::new("/Users").exists() {
            PathBuf::from("/Users")
        } else {
            PathBuf::from("/home")
        };
        if host_users.exists() {
            builder = builder.virtiofs_mount(VirtioFsMount::new("users", host_users, false));
        }

        let vm = builder
            .run()
            .await
            .with_context(|| format!("Failed to start microVM instance for machine '{}'", name))?;

        config.status = "running".to_string();
        config.instance_id = vm.id().to_string();

        let cfg_path = Self::config_path(base_data_dir, name);
        fs::write(&cfg_path, serde_json::to_string_pretty(&config)?)?;

        info!("Machine '{}' started successfully (ID: {})", name, vm.id());
        Ok(config)
    }

    /// Gracefully stops a running developer machine.
    pub fn stop(base_data_dir: &Path, name: &str) -> Result<()> {
        let config = Self::get(base_data_dir, name)?;
        if config.status == "stopped" {
            info!("Machine '{}' is already stopped", name);
            return Ok(());
        }

        info!(
            "Stopping machine '{}' (ID: {})...",
            name, config.instance_id
        );
        StateManager::stop(base_data_dir, &config.instance_id)?;

        let mut updated = config;
        updated.status = "stopped".to_string();
        let cfg_path = Self::config_path(base_data_dir, name);
        let _ = fs::write(&cfg_path, serde_json::to_string_pretty(&updated)?);

        Ok(())
    }

    /// Removes a developer machine, terminating running processes and purging its storage.
    pub fn delete(base_data_dir: &Path, name: &str, force: bool) -> Result<()> {
        let config = Self::get(base_data_dir, name)?;
        if config.status == "running" {
            if !force {
                bail!(
                    "Machine '{}' is currently running. Stop it first or use --force",
                    name
                );
            }
            let _ = StateManager::stop(base_data_dir, &config.instance_id);
        }

        // Remove microVM instance rootfs & state
        let _ = StateManager::delete(base_data_dir, &config.instance_id, true);

        // Remove machine directory
        let mdir = Self::machine_dir(base_data_dir, name);
        if mdir.exists() {
            fs::remove_dir_all(&mdir).with_context(|| {
                format!("Failed to remove machine directory at {}", mdir.display())
            })?;
        }

        info!("Machine '{}' deleted successfully", name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_machine_paths() {
        let temp = tempdir().unwrap();
        let base = temp.path();

        let mdir = MachineManager::machine_dir(base, "dev");
        assert_eq!(mdir, base.join("machines/dev"));

        let cfg = MachineManager::config_path(base, "dev");
        assert_eq!(cfg, base.join("machines/dev/config.json"));
    }

    #[tokio::test]
    async fn test_machine_crud_lifecycle() {
        let temp = tempdir().unwrap();
        let base = temp.path();

        // 1. Initially empty
        let list = MachineManager::list(base).unwrap();
        assert!(list.is_empty());

        // 2. Prepare mock config directly
        let mdir = MachineManager::machine_dir(base, "lab");
        fs::create_dir_all(&mdir).unwrap();
        let cfg = MachineConfig {
            name: "lab".to_string(),
            distro: "ubuntu".to_string(),
            image: "ubuntu:24.04".to_string(),
            cpus: 4,
            memory_mb: 4096,
            rosetta: true,
            created_at: 1000,
            instance_id: "machine-lab".to_string(),
            status: "stopped".to_string(),
        };
        fs::write(
            MachineManager::config_path(base, "lab"),
            serde_json::to_string_pretty(&cfg).unwrap(),
        )
        .unwrap();

        // 3. Inspect machine
        let fetched = MachineManager::get(base, "lab").unwrap();
        assert_eq!(fetched.name, "lab");
        assert_eq!(fetched.cpus, 4);
        assert_eq!(fetched.distro, "ubuntu");
        assert_eq!(fetched.status, "stopped");

        // 4. List machines
        let all = MachineManager::list(base).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "lab");

        // 5. Delete machine
        MachineManager::delete(base, "lab", false).unwrap();
        assert!(MachineManager::list(base).unwrap().is_empty());
    }
}
