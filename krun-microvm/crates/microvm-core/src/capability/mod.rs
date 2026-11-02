// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Capability policy engine for MicroVM sandboxes.
//!
//! Provides granular authorization rules for guest filesystem boundaries,
//! process binary execution whitelists, network privileges, and state mutation.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Access mode for filesystem operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FsAccessMode {
    Read,
    Write,
}

/// Fine-grained capability policy defining the security posture of a microVM sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPolicy {
    /// Permitted read-only filesystem paths or prefixes.
    #[serde(default)]
    pub fs_read_paths: Vec<PathBuf>,
    /// Permitted read-write filesystem paths or prefixes.
    #[serde(default)]
    pub fs_write_paths: Vec<PathBuf>,
    /// Strictly forbidden filesystem paths or prefixes (denied for both read and write).
    #[serde(default)]
    pub fs_denied_paths: Vec<PathBuf>,
    /// Permitted executable binary names or absolute paths.
    /// If empty, all executables are permitted unless listed in `denied_executables`.
    #[serde(default)]
    pub allowed_executables: Vec<String>,
    /// Strictly forbidden executable binary names or patterns.
    #[serde(default)]
    pub denied_executables: Vec<String>,
    /// Whether outbound and inbound networking is enabled.
    #[serde(default = "default_true")]
    pub allow_network: bool,
    /// Whether the guest is allowed to mutate sandbox configuration or persistent disks.
    #[serde(default = "default_true")]
    pub allow_state_mutation: bool,
}

fn default_true() -> bool {
    true
}

impl Default for CapabilityPolicy {
    fn default() -> Self {
        Self::unrestricted()
    }
}

impl CapabilityPolicy {
    /// Creates an unrestricted capability policy (standard virtualization posture).
    pub fn unrestricted() -> Self {
        Self {
            fs_read_paths: Vec::new(),
            fs_write_paths: Vec::new(),
            fs_denied_paths: Vec::new(),
            allowed_executables: Vec::new(),
            denied_executables: Vec::new(),
            allow_network: true,
            allow_state_mutation: true,
        }
    }

    /// Creates a hardened AI Agent sandbox capability policy:
    /// - System rootfs is read-only
    /// - Only `/tmp`, `/var/tmp`, and `/workspace` are writable
    /// - Sensitive system files (`/etc/shadow`, `/root/.ssh`, `/var/run/secrets`) are denied
    /// - Dangerous host-access utilities are forbidden
    pub fn hardened_agent_sandbox() -> Self {
        Self {
            fs_read_paths: vec![
                PathBuf::from("/usr"),
                PathBuf::from("/bin"),
                PathBuf::from("/lib"),
                PathBuf::from("/lib64"),
                PathBuf::from("/etc"),
                PathBuf::from("/opt"),
                PathBuf::from("/workspace"),
                PathBuf::from("/tmp"),
            ],
            fs_write_paths: vec![
                PathBuf::from("/workspace"),
                PathBuf::from("/tmp"),
                PathBuf::from("/var/tmp"),
            ],
            fs_denied_paths: vec![
                PathBuf::from("/etc/shadow"),
                PathBuf::from("/etc/gshadow"),
                PathBuf::from("/root/.ssh"),
                PathBuf::from("/var/run/secrets"),
            ],
            allowed_executables: Vec::new(),
            denied_executables: vec![
                "gdb".to_string(),
                "strace".to_string(),
                "tcpdump".to_string(),
                "nmap".to_string(),
            ],
            allow_network: true,
            allow_state_mutation: false,
        }
    }

    /// Builder method to add a permitted read path.
    pub fn allow_fs_read(mut self, path: impl Into<PathBuf>) -> Self {
        self.fs_read_paths.push(path.into());
        self
    }

    /// Builder method to add a permitted write path.
    pub fn allow_fs_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.fs_write_paths.push(path.into());
        self
    }

    /// Builder method to add a denied path.
    pub fn deny_fs_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.fs_denied_paths.push(path.into());
        self
    }

    /// Builder method to allow an executable binary.
    pub fn allow_exec(mut self, binary: impl Into<String>) -> Self {
        self.allowed_executables.push(binary.into());
        self
    }

    /// Builder method to deny an executable binary.
    pub fn deny_exec(mut self, binary: impl Into<String>) -> Self {
        self.denied_executables.push(binary.into());
        self
    }

    pub fn with_network(mut self, allowed: bool) -> Self {
        self.allow_network = allowed;
        self
    }

    pub fn with_state_mutation(mut self, allowed: bool) -> Self {
        self.allow_state_mutation = allowed;
        self
    }

    /// Authorizes a filesystem access against this capability policy.
    pub fn authorize_fs(&self, path: &Path, mode: FsAccessMode) -> Result<()> {
        // 1. Strict denied path check
        for denied in &self.fs_denied_paths {
            if path.starts_with(denied) {
                bail!(
                    "Capability violation: access to path '{}' is forbidden by capability policy",
                    path.display()
                );
            }
        }

        match mode {
            FsAccessMode::Write => {
                if !self.fs_write_paths.is_empty() {
                    let matched = self
                        .fs_write_paths
                        .iter()
                        .any(|allowed| path.starts_with(allowed));
                    if !matched {
                        bail!(
                            "Capability violation: write access to '{}' is not permitted (allowed write paths: {:?})",
                            path.display(),
                            self.fs_write_paths
                        );
                    }
                }
            }
            FsAccessMode::Read => {
                if !self.fs_read_paths.is_empty() {
                    let matched = self
                        .fs_read_paths
                        .iter()
                        .any(|allowed| path.starts_with(allowed));
                    if !matched {
                        bail!(
                            "Capability violation: read access to '{}' is not permitted (allowed read paths: {:?})",
                            path.display(),
                            self.fs_read_paths
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Authorizes an executable binary command line against this capability policy.
    pub fn authorize_exec(&self, cmd: &[String]) -> Result<()> {
        if cmd.is_empty() {
            bail!("Capability violation: empty command execution requested");
        }

        let binary_full = &cmd[0];
        let binary_name = Path::new(binary_full)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(binary_full);

        // 1. Denied executables check
        for denied in &self.denied_executables {
            if binary_name.eq_ignore_ascii_case(denied) || binary_full == denied {
                bail!(
                    "Capability violation: execution of binary '{}' is forbidden by capability policy",
                    binary_name
                );
            }
        }

        // 2. Allowed executables whitelist
        if !self.allowed_executables.is_empty() {
            let matched = self
                .allowed_executables
                .iter()
                .any(|allowed| binary_name.eq_ignore_ascii_case(allowed) || binary_full == allowed);
            if !matched {
                bail!(
                    "Capability violation: execution of binary '{}' is not whitelisted (allowed: {:?})",
                    binary_name,
                    self.allowed_executables
                );
            }
        }

        Ok(())
    }

    /// Authorizes network access against this capability policy.
    pub fn authorize_network(&self) -> Result<()> {
        if !self.allow_network {
            bail!("Capability violation: network access is disabled by capability policy");
        }
        Ok(())
    }

    /// Authorizes state mutation operations against this capability policy.
    pub fn authorize_state_mutation(&self) -> Result<()> {
        if !self.allow_state_mutation {
            bail!(
                "Capability violation: state mutation operations are disabled by capability policy"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_policy_fs_authorization() {
        let policy = CapabilityPolicy::hardened_agent_sandbox();

        // Allowed reads
        assert!(policy
            .authorize_fs(Path::new("/workspace/src/main.rs"), FsAccessMode::Read)
            .is_ok());
        assert!(policy
            .authorize_fs(Path::new("/usr/bin/python3"), FsAccessMode::Read)
            .is_ok());

        // Allowed writes
        assert!(policy
            .authorize_fs(Path::new("/workspace/output.txt"), FsAccessMode::Write)
            .is_ok());
        assert!(policy
            .authorize_fs(Path::new("/tmp/test.log"), FsAccessMode::Write)
            .is_ok());

        // Denied write to read-only directory
        assert!(policy
            .authorize_fs(Path::new("/etc/hosts"), FsAccessMode::Write)
            .is_err());
        assert!(policy
            .authorize_fs(Path::new("/usr/bin/bad"), FsAccessMode::Write)
            .is_err());

        // Denied access to sensitive files
        assert!(policy
            .authorize_fs(Path::new("/etc/shadow"), FsAccessMode::Read)
            .is_err());
        assert!(policy
            .authorize_fs(Path::new("/root/.ssh/id_rsa"), FsAccessMode::Read)
            .is_err());
    }

    #[test]
    fn test_capability_policy_exec_authorization() {
        let policy = CapabilityPolicy::unrestricted()
            .allow_exec("python3")
            .allow_exec("pytest")
            .deny_exec("curl");

        // Whitelisted
        assert!(policy
            .authorize_exec(&["python3".to_string(), "app.py".to_string()])
            .is_ok());
        assert!(policy
            .authorize_exec(&["/usr/bin/pytest".to_string()])
            .is_ok());

        // Not in whitelist
        let not_whitelisted = policy.authorize_exec(&["bash".to_string()]);
        assert!(not_whitelisted.is_err());
        assert!(not_whitelisted
            .unwrap_err()
            .to_string()
            .contains("not whitelisted"));

        // Explicitly denied
        let denied = policy.authorize_exec(&["curl".to_string(), "https://evil.com".to_string()]);
        assert!(denied.is_err());
        assert!(denied.unwrap_err().to_string().contains("is forbidden"));
    }

    #[test]
    fn test_capability_policy_network_and_mutation() {
        let policy = CapabilityPolicy::unrestricted()
            .with_network(false)
            .with_state_mutation(false);

        assert!(policy.authorize_network().is_err());
        assert!(policy.authorize_state_mutation().is_err());
    }
}
