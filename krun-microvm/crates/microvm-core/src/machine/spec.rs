// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Developer machine configuration, distribution catalog, and host-user parity models.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Supported base Linux developer distributions and their default container images.
pub const DISTRO_CATALOG: &[(&str, &str)] = &[
    ("ubuntu", "ubuntu:24.04"),
    ("debian", "debian:bookworm"),
    ("alpine", "alpine:latest"),
    ("fedora", "fedora:40"),
    ("arch", "archlinux:latest"),
    ("rocky", "rockylinux:9"),
    ("opensuse", "opensuse/tumbleweed:latest"),
];

/// Resolves a distro name or direct image reference to an OCI image string.
pub fn resolve_distro_image(distro_or_image: &str) -> String {
    for (d, img) in DISTRO_CATALOG {
        if d.eq_ignore_ascii_case(distro_or_image) {
            return (*img).to_string();
        }
    }
    distro_or_image.to_string()
}

/// Sanitizes a username string to strictly alphanumeric characters, underscores, and hyphens.
pub fn sanitize_username(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if clean.is_empty() || clean.starts_with('-') {
        "cro".to_string()
    } else {
        clean
    }
}

/// Retrieves host username, UID, and GID for seamless in-guest parity.
pub fn get_host_user_info() -> (String, u32, u32) {
    let raw_user = std::env::var("USER").unwrap_or_else(|_| "cro".to_string());
    let user = sanitize_username(&raw_user);
    #[cfg(unix)]
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    #[cfg(not(unix))]
    let (uid, gid) = (1000, 1000);
    (user, uid, gid)
}

/// Validates that a machine name consists only of alphanumeric characters and dashes.
pub fn validate_machine_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Machine name cannot be empty");
    }
    if name.len() > 64 {
        bail!(
            "Machine name '{}' exceeds maximum allowed length of 64 characters",
            name
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        bail!("Machine name '{}' cannot start or end with a hyphen", name);
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        bail!(
            "Machine name '{}' contains invalid characters. Use letters, numbers, and hyphens only",
            name
        );
    }
    Ok(())
}

/// Generates an in-guest initialization shell script to provision the host user,
/// grant passwordless sudo, mount `/Users` VirtioFS, and run an idle supervisor loop.
pub fn generate_machine_init_script(user: &str, uid: u32, gid: u32) -> String {
    format!(
        r#"#!/bin/sh
set -e

# 1. Provision host user parity if not already present
if ! id -u {user} >/dev/null 2>&1; then
  getent group {gid} >/dev/null 2>&1 || groupadd -g {gid} {user} 2>/dev/null || addgroup -g {gid} {user} 2>/dev/null || true
  g=$(getent group {gid} 2>/dev/null | cut -d: -f1)
  useradd -M -u {uid} -g "${{g:-{gid}}}" -d /Users/{user} -s /bin/sh {user} 2>/dev/null || adduser -D -H -u {uid} -G "${{g:-{user}}}" -h /Users/{user} {user} 2>/dev/null || true
fi

# 2. Configure passwordless sudo for the developer
mkdir -p /etc/sudoers.d
echo '{user} ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/cro
chmod 440 /etc/sudoers.d/cro 2>/dev/null || true

# 3. Mount host /Users via VirtioFS tag 'users' if available
if [ -d /Users ]; then
  mount -t virtiofs users /Users 2>/dev/null || true
fi

# 4. Background installation of sudo if the minimal distro lacks it
command -v sudo >/dev/null 2>&1 || (apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq sudo || apk add -q sudo || dnf -y -q install sudo || pacman -Sy --noconfirm sudo || zypper -n -q in sudo) >/dev/null 2>&1 &

# 5. Persistent idle loop responding cleanly to shutdown signals
trap 'exit 0' TERM INT
while :; do sleep 3600 & wait $!; done
"#,
        user = user,
        uid = uid,
        gid = gid
    )
}

/// Persistent configuration metadata for a Linux developer machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineConfig {
    /// Machine friendly name (e.g. "dev", "ai-lab").
    pub name: String,
    /// Selected distribution tag (e.g. "ubuntu", "debian", "alpine").
    pub distro: String,
    /// Full OCI container image reference.
    pub image: String,
    /// Virtual CPU count.
    pub cpus: u8,
    /// RAM allocation in MiB.
    pub memory_mb: u32,
    /// Whether Apple Silicon Rosetta x86_64 emulation is enabled.
    pub rosetta: bool,
    /// Creation timestamp (Unix epoch seconds).
    pub created_at: u64,
    /// Unique underlying microVM instance ID.
    pub instance_id: String,
    /// Current cached status (e.g. "running", "stopped").
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_distro_image() {
        assert_eq!(resolve_distro_image("ubuntu"), "ubuntu:24.04");
        assert_eq!(resolve_distro_image("UBUNTU"), "ubuntu:24.04");
        assert_eq!(resolve_distro_image("debian"), "debian:bookworm");
        assert_eq!(resolve_distro_image("alpine"), "alpine:latest");
        assert_eq!(resolve_distro_image("fedora"), "fedora:40");
        assert_eq!(resolve_distro_image("arch"), "archlinux:latest");
        assert_eq!(resolve_distro_image("custom/image:v1"), "custom/image:v1");
    }

    #[test]
    fn test_validate_machine_name() {
        assert!(validate_machine_name("dev").is_ok());
        assert!(validate_machine_name("ubuntu-24").is_ok());
        assert!(validate_machine_name("my-dev-vm-1").is_ok());

        assert!(validate_machine_name("").is_err());
        assert!(validate_machine_name("-invalid").is_err());
        assert!(validate_machine_name("invalid-").is_err());
        assert!(validate_machine_name("invalid name").is_err());
        assert!(validate_machine_name("dev@box").is_err());
    }

    #[test]
    fn test_generate_machine_init_script() {
        let script = generate_machine_init_script("alice", 501, 20);
        assert!(script.contains("alice ALL=(ALL) NOPASSWD:ALL"));
        assert!(script.contains("-u 501"));
        assert!(script.contains("mount -t virtiofs users /Users"));
    }
}
