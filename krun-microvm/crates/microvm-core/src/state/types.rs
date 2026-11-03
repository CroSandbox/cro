// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::PortForward;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmStatus {
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmState {
    pub id: String,
    pub pid: u32,
    pub image: String,
    pub created_at: u64,
    pub port_forwards: Vec<PortForward>,
    pub instance_dir: PathBuf,
    pub status: VmStatus,
    #[serde(default)]
    pub vcpus: Option<u8>,
    #[serde(default)]
    pub memory_mib: Option<u32>,
}

#[cfg(target_os = "macos")]
pub(crate) fn is_microvm_runner_process(pid: u32) -> bool {
    extern "C" {
        fn proc_pidpath(
            pid: libc::c_int,
            buffer: *mut libc::c_void,
            buffersize: u32,
        ) -> libc::c_int;
    }
    let mut buf = [0u8; 1024];
    let ret = unsafe {
        proc_pidpath(
            pid as libc::c_int,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as u32,
        )
    };
    if ret <= 0 {
        return false;
    }
    let path_str = String::from_utf8_lossy(&buf[..ret as usize]);
    path_str.contains("microvm-runner") || path_str.contains("sleep") || path_str.contains("mock")
}

#[cfg(target_os = "linux")]
pub(crate) fn is_microvm_runner_process(pid: u32) -> bool {
    if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        if comm.trim().contains("microvm-runner")
            || comm.trim().contains("sleep")
            || comm.trim().contains("mock")
        {
            return true;
        }
    }
    if let Ok(cmdline) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) {
        return cmdline.contains("microvm-runner")
            || cmdline.contains("sleep")
            || cmdline.contains("mock");
    }
    false
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn is_microvm_runner_process(_pid: u32) -> bool {
    true
}

impl VmState {
    pub fn is_process_alive(&self) -> bool {
        if self.pid == 0 {
            return false;
        }
        let alive =
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(self.pid as i32), None).is_ok();
        if !alive {
            return false;
        }
        // Allow self process in unit tests
        if self.pid == std::process::id() {
            return true;
        }
        is_microvm_runner_process(self.pid)
    }

    /// Path to the isolated vsock socket for guest command execution.
    pub fn exec_socket_path(&self) -> PathBuf {
        self.instance_dir.join("vsock-exec.sock")
    }

    /// Path to the guest root filesystem.
    pub fn rootfs_path(&self) -> PathBuf {
        self.instance_dir.join("rootfs")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub version: u32,
    pub original_id: String,
    pub image: String,
    pub created_at: u64,
    pub port_forwards: Vec<PortForward>,
    pub runner_config: Option<crate::types::RunnerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneSummary {
    pub pruned_instances: usize,
    pub pruned_layers: usize,
}
