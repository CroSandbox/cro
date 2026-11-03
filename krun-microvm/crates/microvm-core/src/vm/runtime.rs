// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::tty::RawModeGuard;
use anyhow::{bail, Context, Result};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;
use tokio::process::Child;

pub struct MicroVm {
    pub(crate) id: String,
    pub(crate) child: Child,
    pub(crate) instance_dir: PathBuf,
    pub(crate) data_dir: PathBuf,
    pub(crate) is_detached: bool,
    pub(crate) supervisor_sock: PathBuf,
    pub(crate) _raw_guard: Option<RawModeGuard>,
    pub(crate) _proxy: Option<crate::net::EgressProxyServer>,
    pub(crate) _gvproxy: Option<crate::net::GvproxyInstance>,
}

impl MicroVm {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn proxy_port(&self) -> Option<u16> {
        self._proxy.as_ref().map(|p| p.port())
    }

    pub fn gvproxy(&self) -> Option<&crate::net::GvproxyInstance> {
        self._gvproxy.as_ref()
    }

    pub fn egress_blocked_count(&self) -> u64 {
        self._proxy.as_ref().map(|p| p.blocked_count()).unwrap_or(0)
    }

    pub fn llm_tokens_consumed(&self) -> u64 {
        self._proxy
            .as_ref()
            .map(|p| p.tokens_consumed())
            .unwrap_or(0)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn is_detached(&self) -> bool {
        self.is_detached
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Asynchronously waits for the microVM to exit and cleans up ephemeral state.
    pub async fn wait(&mut self) -> Result<ExitStatus> {
        let status = self.child.wait().await?;
        self.cleanup();
        Ok(status)
    }

    /// Asynchronously waits for the microVM to exit within the specified timeout.
    ///
    /// If the timeout expires before the VM halts, initiates a graceful stop (`stop()`)
    /// and returns a timeout error.
    pub async fn wait_timeout(&mut self, timeout: Duration) -> Result<ExitStatus> {
        match tokio::time::timeout(timeout, self.child.wait()).await {
            Ok(status_res) => {
                let status = status_res?;
                self.cleanup();
                Ok(status)
            }
            Err(_) => {
                tracing::warn!(
                    "MicroVM '{}' exceeded timeout of {:?}; terminating...",
                    self.id,
                    timeout
                );
                let _ = self.stop().await;
                bail!(
                    "MicroVM '{}' execution timed out after {:?}",
                    self.id,
                    timeout
                );
            }
        }
    }

    pub fn supervisor_sock_path(&self) -> &Path {
        &self.supervisor_sock
    }

    /// Sends a heartbeat ping to the supervisor and measures round-trip latency.
    pub async fn ping(&self) -> Result<Duration> {
        if !self.supervisor_sock.exists() {
            bail!("Supervisor socket does not exist");
        }
        let mut stream = tokio::net::UnixStream::connect(&self.supervisor_sock).await?;
        let ping_msg =
            crate::protocol::ControlMessage::new(1, crate::protocol::MessagePayload::Ping);
        let start = std::time::Instant::now();
        crate::protocol::write_frame_async(&mut stream, &ping_msg).await?;
        let resp = tokio::time::timeout(
            Duration::from_secs(3),
            crate::protocol::read_frame_async(&mut stream),
        )
        .await
        .context("Ping timed out")??;
        let elapsed = start.elapsed();
        if matches!(resp.payload, crate::protocol::MessagePayload::Pong { .. }) {
            Ok(elapsed)
        } else {
            bail!("Unexpected response to Ping: {:?}", resp.payload);
        }
    }

    /// Queries the supervisor process for live resource usage statistics.
    pub async fn stats(&self) -> Result<crate::metrics::ProcessStats> {
        if self.supervisor_sock.exists() {
            if let Ok(mut stream) = tokio::net::UnixStream::connect(&self.supervisor_sock).await {
                let stats_msg =
                    crate::protocol::ControlMessage::new(1, crate::protocol::MessagePayload::Stats);
                if crate::protocol::write_frame_async(&mut stream, &stats_msg)
                    .await
                    .is_ok()
                {
                    if let Ok(Ok(resp)) = tokio::time::timeout(
                        Duration::from_secs(3),
                        crate::protocol::read_frame_async(&mut stream),
                    )
                    .await
                    {
                        if let crate::protocol::MessagePayload::StatsResponse(s) = resp.payload {
                            return Ok(s);
                        }
                    }
                }
            }
        }
        // Fallback to host process inspection
        let pid = self.pid().unwrap_or(0);
        crate::metrics::collect_process_stats(pid)
            .context("Failed to collect host process telemetry")
    }

    /// Suspends vCPUs and pauses the microVM.
    pub async fn pause(&mut self) -> Result<()> {
        if self.supervisor_sock.exists() {
            if let Ok(mut stream) = tokio::net::UnixStream::connect(&self.supervisor_sock).await {
                let pause_msg =
                    crate::protocol::ControlMessage::new(1, crate::protocol::MessagePayload::Pause);
                let _ = crate::protocol::write_frame_async(&mut stream, &pause_msg).await;
            }
        } else if let Some(pid) = self.pid() {
            let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGSTOP);
        }
        if let Err(e) = crate::state::StateManager::pause(&self.data_dir, &self.id) {
            tracing::warn!(instance_id = %self.id, error = %e, "Failed to update state to paused");
        }
        Ok(())
    }

    /// Resumes suspended vCPUs and unpauses the microVM.
    pub async fn resume(&mut self) -> Result<()> {
        if self.supervisor_sock.exists() {
            if let Ok(mut stream) = tokio::net::UnixStream::connect(&self.supervisor_sock).await {
                let resume_msg = crate::protocol::ControlMessage::new(
                    1,
                    crate::protocol::MessagePayload::Resume,
                );
                let _ = crate::protocol::write_frame_async(&mut stream, &resume_msg).await;
            }
        } else if let Some(pid) = self.pid() {
            let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGCONT);
        }
        if let Err(e) = crate::state::StateManager::resume(&self.data_dir, &self.id) {
            tracing::warn!(instance_id = %self.id, error = %e, "Failed to update state to resumed");
        }
        Ok(())
    }

    /// Gracefully stops the microVM (via supervisor socket with fallback to SIGTERM and SIGKILL).
    pub async fn stop(&mut self) -> Result<()> {
        // 1. Try graceful stop over supervisor socket
        if self.supervisor_sock.exists() {
            if let Ok(mut stream) = tokio::net::UnixStream::connect(&self.supervisor_sock).await {
                let stop_msg = crate::protocol::ControlMessage::new(
                    1,
                    crate::protocol::MessagePayload::Stop {
                        timeout_secs: 5,
                        force: false,
                    },
                );
                let _ = crate::protocol::write_frame_async(&mut stream, &stop_msg).await;
            }
        }

        // 2. Poll for termination or fallback to OS signals
        if let Some(pid) = self.pid() {
            let nix_pid = Pid::from_raw(pid as i32);
            let timeout = Duration::from_secs(5);
            let start = std::time::Instant::now();

            while start.elapsed() < timeout {
                if !self.is_alive() {
                    self.cleanup();
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            // Fallback to SIGTERM if still running
            let _ = signal::kill(nix_pid, Signal::SIGTERM);
            let term_start = std::time::Instant::now();
            while term_start.elapsed() < Duration::from_secs(2) {
                if !self.is_alive() {
                    self.cleanup();
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            // Force kill if not stopped
            let _ = signal::kill(nix_pid, Signal::SIGKILL);
            let _ = self.child.wait().await;
        }

        self.cleanup();
        Ok(())
    }

    /// Copies a host file or directory into the guest rootfs.
    pub fn copy_to_guest(&self, src_host: &Path, guest_path: &str) -> Result<()> {
        crate::state::StateManager::copy_into(&self.data_dir, &self.id, src_host, guest_path)
    }

    /// Copies a file or directory from the guest rootfs to the host.
    pub fn copy_from_guest(&self, guest_path: &str, dst_host: &Path) -> Result<()> {
        crate::state::StateManager::copy_from(&self.data_dir, &self.id, guest_path, dst_host)
    }

    pub fn instance_dir(&self) -> &Path {
        &self.instance_dir
    }

    pub fn console_log_path(&self) -> PathBuf {
        self.instance_dir.join("console.log")
    }

    /// Explicitly purges instance state and directory from disk.
    pub fn purge(&self) {
        if let Err(e) = crate::state::StateManager::remove(&self.data_dir, &self.id) {
            tracing::warn!(instance_id = %self.id, error = %e, "Failed to remove VM state during purge");
        }
        if self.instance_dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&self.instance_dir) {
                tracing::warn!(instance_dir = ?self.instance_dir, error = %e, "Failed to remove instance directory during purge");
            }
        }
    }

    fn cleanup(&self) {
        if self.is_detached {
            return;
        }
        self.purge();
    }
}

impl Drop for MicroVm {
    fn drop(&mut self) {
        if !self.is_detached {
            if let Ok(None) = self.child.try_wait() {
                if let Some(pid) = self.child.id() {
                    let nix_pid = Pid::from_raw(pid as i32);
                    let _ = signal::kill(nix_pid, Signal::SIGKILL);
                }
            }
            self.cleanup();
        }
    }
}

pub(crate) fn find_runner_binary() -> Result<PathBuf> {
    // 1. Next to current executable
    if let Ok(curr) = std::env::current_exe() {
        if let Some(dir) = curr.parent() {
            let candidate = dir.join("microvm-runner");
            if candidate.exists() {
                ensure_runner_signed(&candidate)?;
                return Ok(candidate);
            }
        }
    }

    // 2. Check target directory of current workspace
    let candidates = [
        PathBuf::from("target/debug/microvm-runner"),
        PathBuf::from("target/release/microvm-runner"),
        PathBuf::from("../target/debug/microvm-runner"),
        PathBuf::from("../target/release/microvm-runner"),
    ];

    for c in &candidates {
        if c.exists() {
            let path = c.canonicalize()?;
            ensure_runner_signed(&path)?;
            return Ok(path);
        }
    }

    // 3. System PATH
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("microvm-runner");
            if candidate.exists() {
                ensure_runner_signed(&candidate)?;
                return Ok(candidate);
            }
        }
    }

    bail!("microvm-runner binary not found. Please build microvm-runner or pass runner_path()");
}

#[cfg(target_os = "macos")]
pub(crate) fn ensure_runner_signed(runner_path: &Path) -> Result<()> {
    let check = std::process::Command::new("codesign")
        .args(["-d", "--entitlements", "-"])
        .arg(runner_path)
        .output();

    let needs_signing = match check {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            !stdout.contains("com.apple.security.hypervisor")
                && !stderr.contains("com.apple.security.hypervisor")
        }
        Err(_) => true,
    };

    if needs_signing {
        const PLIST_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.hypervisor</key>
    <true/>
</dict>
</plist>
"#;
        let mut tmp_plist = tempfile::NamedTempFile::new()
            .context("Failed to create temporary plist file for codesign")?;
        std::io::Write::write_all(&mut tmp_plist, PLIST_XML.as_bytes())?;

        let status = std::process::Command::new("codesign")
            .arg("--entitlements")
            .arg(tmp_plist.path())
            .args(["--force", "-s", "-"])
            .arg(runner_path)
            .status()
            .context("Failed to execute codesign command")?;

        if !status.success() {
            tracing::warn!("Failed to codesign runner binary with hypervisor entitlement");
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn ensure_runner_signed(_runner_path: &Path) -> Result<()> {
    Ok(())
}
