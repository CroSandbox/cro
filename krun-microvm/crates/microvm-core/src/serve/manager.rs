// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Serving lifecycle manager and health check prober for `cro serve` and `cro vllm`.

use super::config::{ServeConfig, ServeEngine};
use super::gateway::{find_available_loopback_port, ServeGateway, ServeGatewayHandle};
use crate::modelpack::ModelEngine;
use crate::state::StateManager;
use crate::vm::MicroVm;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Live handle to a running microVM inference server.
pub struct ServeHandle {
    pub vm_id: String,
    pub engine: ServeEngine,
    pub model_name: String,
    pub base_url: String,
    pub chat_url: String,
    pub models_url: String,
    pub vm: Option<MicroVm>,
    pub gateway: Option<ServeGatewayHandle>,
}

impl ServeHandle {
    /// Returns true if the serving microVM is currently auto-paused via scale-to-zero.
    pub fn is_paused(&self) -> bool {
        self.gateway
            .as_ref()
            .map(|g| g.is_paused())
            .unwrap_or(false)
    }

    /// Awaits the microVM process until it exits or is interrupted.
    pub async fn wait(mut self) -> Result<std::process::ExitStatus> {
        if let Some(ref mut vm) = self.vm {
            let res = vm.wait().await;
            if let Some(gw) = self.gateway.as_ref() {
                gw.shutdown();
            }
            res
        } else {
            bail!("MicroVM was launched in detached mode; no active process handle to await");
        }
    }

    /// Awaits the microVM process while gracefully capturing SIGINT / Ctrl+C.
    pub async fn wait_with_sigint(&mut self) -> Result<()> {
        if let Some(ref mut vm) = self.vm {
            tokio::select! {
                res = vm.wait() => {
                    if let Some(gw) = self.gateway.as_ref() {
                        gw.shutdown();
                    }
                    let status = res?;
                    if !status.success() {
                        bail!("MicroVM exited with error status: {}", status);
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("\n🛑 Interrupted. Gracefully stopping microVM inference server (VM ID: {})...", self.vm_id);
                    if let Some(gw) = self.gateway.as_ref() {
                        gw.shutdown();
                    }
                    let _ = vm.stop().await;
                    println!("✅ MicroVM stopped cleanly.");
                }
            }
            Ok(())
        } else {
            bail!("MicroVM was launched in detached mode; no active process handle to await");
        }
    }
}

pub struct ServeManager;

impl ServeManager {
    /// Launches an inference microVM and probes for readiness.
    pub async fn start(config: &ServeConfig, detach: bool, wait: bool) -> Result<ServeHandle> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default_dir = PathBuf::from(home).join(".cache/krun-microvm");
        let data_dir = config.data_dir.as_deref().unwrap_or(&default_dir);

        let (effective_config, backend_port) =
            if config.scale_to_zero || config.idle_timeout_secs.is_some() {
                let bp = match config.backend_port {
                    Some(p) => p,
                    None => find_available_loopback_port()?,
                };
                let mut c = config.clone();
                c.backend_port = Some(bp);
                (c, Some(bp))
            } else {
                (config.clone(), None)
            };

        // 1. Inspect ModelPack metadata
        let meta = ModelEngine::inspect_model(&effective_config.model, data_dir)
            .with_context(|| format!("Failed to inspect model '{}'", effective_config.model))?;

        // 2. Resolve serving engine
        let active_engine = effective_config.resolve_engine(&meta);

        println!(
            "⏳ Initializing microVM inference server for '{}'...",
            meta.name
        );
        println!("   Engine:           {}", active_engine);
        println!(
            "   Model Format:     {} (Total Size: {} MB)",
            meta.format,
            meta.total_size / (1024 * 1024)
        );
        println!(
            "   Guest Allocation: {} vCPUs, {} MB RAM, Virtio-GPU ({})",
            effective_config.cpus, effective_config.memory_mb, effective_config.gpu_shm_size
        );
        if effective_config.scale_to_zero || effective_config.idle_timeout_secs.is_some() {
            println!(
                "   Scale-to-Zero:    enabled (idle timeout: {}s, wake-on-request: < 2ms)",
                effective_config.idle_timeout_secs.unwrap_or(60)
            );
        }

        // 3. Check for warm-start snapshot if requested
        let model_key = if !meta.digest.is_empty() {
            &meta.digest
        } else {
            &meta.name
        };
        let cache_key = effective_config.compute_warm_snapshot_key(model_key);
        let snapshot_dir = data_dir.join("snapshots/serve").join(&cache_key);

        let (mut vm, is_warm_started) = if effective_config.warm
            && snapshot_dir.exists()
            && snapshot_dir.join("snapshot.json").exists()
        {
            println!(
                "⚡ Warm-start enabled: restoring from pre-warmed snapshot '{}'...",
                cache_key
            );
            let warm_boot_start = std::time::Instant::now();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let target_id = format!("vm-warm-{}-{}", now.as_millis(), now.subsec_nanos());

            StateManager::restore(data_dir, &snapshot_dir, Some(&target_id))
                .with_context(|| format!("Failed to restore warm snapshot '{}'", cache_key))?;

            Self::sanitize_restored_rootfs(
                &data_dir.join("instances").join(&target_id).join("rootfs"),
            );

            let mut builder = effective_config
                .to_microvm_builder(active_engine, &meta, &meta.path)
                .with_context(|| "Failed to construct MicroVmBuilder for warm serving")?
                .instance_id(&target_id);

            if detach {
                builder = builder.detach(true);
            }

            let vm = builder
                .run()
                .await
                .context("Failed to spawn restored warm microVM")?;
            let elapsed = warm_boot_start.elapsed();
            println!(
                "⚡ Warm microVM restored and booted in {:.2}ms (Instance ID: {}).",
                elapsed.as_secs_f64() * 1000.0,
                target_id
            );
            (vm, true)
        } else {
            if effective_config.warm {
                println!(
                    "ℹ️  No warm snapshot found for key '{}'; falling back to cold boot.",
                    cache_key
                );
            }

            let mut builder = effective_config
                .to_microvm_builder(active_engine, &meta, &meta.path)
                .with_context(|| "Failed to construct MicroVmBuilder for serving")?;

            if detach {
                builder = builder.detach(true);
            }

            let vm = builder
                .run()
                .await
                .context("Failed to spawn serving microVM")?;
            (vm, false)
        };
        let vm_id = vm.id().to_string();

        let base_url = format!(
            "http://{}:{}/v1",
            effective_config.host, effective_config.host_port
        );
        let chat_url = format!("{}/chat/completions", base_url);
        let models_url = format!("{}/models", base_url);
        let model_alias = effective_config.effective_model_name(&meta);

        // 5. Health check probing
        let probe_port = backend_port.unwrap_or(effective_config.host_port);
        let probe_host = if backend_port.is_some() {
            "127.0.0.1"
        } else {
            &effective_config.host
        };

        if !detach || wait {
            println!(
                "⏳ Probing inference server readiness on port {}...",
                probe_port
            );
            Self::poll_readiness_with_vm(
                probe_host,
                probe_port,
                Duration::from_secs(120),
                Some(&mut vm),
            )
            .await?;

            if effective_config.snapshot_on_ready && !is_warm_started {
                if let Err(e) = Self::snapshot_warm_serve(data_dir, &vm_id, &cache_key) {
                    eprintln!("⚠️  Failed to capture warm-start snapshot: {}", e);
                }
            }

            Self::print_summary_card(
                model_alias,
                active_engine,
                &base_url,
                &chat_url,
                &models_url,
                effective_config.api_key.as_deref(),
            );
        } else {
            println!("🚀 Server spawned in detached mode (VM ID: {}).", vm_id);
            println!("   Base URL: {}", base_url);
        }

        // 6. Scale-to-Zero Gateway startup if enabled
        let gateway = if let Some(bp) = backend_port {
            let gw = ServeGateway::start(
                &effective_config.host,
                effective_config.host_port,
                bp,
                &vm_id,
                data_dir.to_path_buf(),
                Duration::from_secs(effective_config.idle_timeout_secs.unwrap_or(60)),
            )
            .await
            .with_context(|| "Failed to initialize Scale-to-Zero gateway")?;
            Some(gw)
        } else {
            None
        };

        Ok(ServeHandle {
            vm_id,
            engine: active_engine,
            model_name: model_alias.to_string(),
            base_url,
            chat_url,
            models_url,
            vm: if detach { None } else { Some(vm) },
            gateway,
        })
    }

    /// Polls the inference server readiness endpoint until it responds with HTTP 200 OK.
    pub async fn poll_readiness(host: &str, port: u16, timeout: Duration) -> Result<()> {
        Self::poll_readiness_with_vm(host, port, timeout, None).await
    }

    /// Polls the inference server readiness endpoint with microVM health monitoring.
    pub async fn poll_readiness_with_vm(
        host: &str,
        port: u16,
        timeout: Duration,
        mut vm: Option<&mut MicroVm>,
    ) -> Result<()> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()?;

        let health_urls = [
            format!("http://{}:{}/health", host, port),
            format!("http://{}:{}/v1/models", host, port),
        ];

        let start = std::time::Instant::now();
        let mut interval = Duration::from_millis(500);

        while start.elapsed() < timeout {
            // Check if microVM process is still alive
            if let Some(ref mut v) = vm {
                if !v.is_alive() {
                    let log_snippet = Self::read_console_log_tail(&v.console_log_path(), 30);
                    bail!(
                        "Inference microVM exited prematurely during startup.\n{}",
                        log_snippet
                    );
                }
            }

            for url in &health_urls {
                if let Ok(resp) = client.get(url).send().await {
                    if resp.status().is_success() {
                        return Ok(());
                    }
                }
            }

            tokio::time::sleep(interval).await;
            if interval < Duration::from_secs(3) {
                interval += Duration::from_millis(250);
            }
        }

        bail!(
            "Inference server failed to become ready within {}s at http://{}:{}",
            timeout.as_secs(),
            host,
            port
        );
    }

    /// Helper to read the last `tail_lines` from the console log file for error reporting.
    fn read_console_log_tail(log_path: &std::path::Path, tail_lines: usize) -> String {
        if !log_path.exists() {
            return "(MicroVM was running in interactive mode; check terminal output above for error details)".to_string();
        }
        match std::fs::read_to_string(log_path) {
            Ok(content) if !content.trim().is_empty() => {
                let lines: Vec<&str> = content.lines().collect();
                let start_idx = lines.len().saturating_sub(tail_lines);
                let tail = lines[start_idx..].join("\n");
                format!("Recent console log:\n---\n{}\n---", tail)
            }
            Ok(_) => "(Console log is empty)".to_string(),
            Err(e) => format!("(Failed to read console log: {})", e),
        }
    }

    /// Captures a warm snapshot of a running inference microVM once it reaches ready state.
    pub fn snapshot_warm_serve(data_dir: &Path, vm_id: &str, cache_key: &str) -> Result<PathBuf> {
        let serve_snapshots_dir = data_dir.join("snapshots").join("serve");
        std::fs::create_dir_all(&serve_snapshots_dir)?;

        let target_snapshot = serve_snapshots_dir.join(cache_key);
        let tmp_snapshot = serve_snapshots_dir.join(format!(".tmp-{}", cache_key));

        if tmp_snapshot.exists() {
            let _ = std::fs::remove_dir_all(&tmp_snapshot);
        }

        // If the microVM is actively running, pause it during snapshot capture to ensure consistency
        let is_running = StateManager::find(data_dir, vm_id)?
            .map(|vm| vm.is_process_alive())
            .unwrap_or(false);

        if is_running {
            println!(
                "📸 Freezing microVM '{}' to capture warm-start snapshot...",
                vm_id
            );
            if let Err(e) = StateManager::pause(data_dir, vm_id) {
                tracing::warn!("Failed to pause microVM '{}' before snapshot: {}", vm_id, e);
            }
        }

        let snap_res = StateManager::snapshot(data_dir, vm_id, &tmp_snapshot);

        // Always guarantee the microVM is resumed if it was running
        if is_running {
            if let Err(e) = StateManager::resume(data_dir, vm_id) {
                eprintln!(
                    "⚠️  Warning: Failed to resume microVM after snapshot: {}",
                    e
                );
            }
        }

        snap_res
            .with_context(|| format!("Failed to create snapshot at {}", tmp_snapshot.display()))?;

        // Atomic commit of the snapshot directory
        if target_snapshot.exists() {
            let _ = std::fs::remove_dir_all(&target_snapshot);
        }
        std::fs::rename(&tmp_snapshot, &target_snapshot).with_context(|| {
            format!(
                "Failed to finalize snapshot directory to {}",
                target_snapshot.display()
            )
        })?;

        println!(
            "✅ Warm-start snapshot captured successfully: {}",
            target_snapshot.display()
        );
        Ok(target_snapshot)
    }

    /// Purges stale unix domain sockets, lockfiles, and PID files from a restored rootfs
    /// so the inference server doesn't hit EADDRINUSE on startup.
    pub fn sanitize_restored_rootfs(rootfs: &Path) {
        let check_dirs = [
            rootfs.join("tmp"),
            rootfs.join("run"),
            rootfs.join("var/run"),
        ];
        for dir in &check_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.ends_with(".sock")
                            || name.ends_with(".pid")
                            || name.ends_with(".lock")
                        {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }

    /// Displays the formatted endpoint summary card.
    pub fn print_summary_card(
        model_name: &str,
        engine: ServeEngine,
        base_url: &str,
        chat_url: &str,
        models_url: &str,
        api_key: Option<&str>,
    ) {
        println!();
        println!("============================================================");
        println!("🚀 Model Inference Server is LIVE!");
        println!("============================================================");
        println!("   Active Engine:    {}", engine);
        println!("   Model:            {}", model_name);
        println!("   Base API URL:     {}", base_url);
        println!("   Chat Completions: {}", chat_url);
        println!("   Models Endpoint:  {}", models_url);
        if let Some(key) = api_key {
            println!("   API Key Auth:     Bearer {}", key);
        } else {
            println!("   API Key Auth:     Disabled (Open access on loopback)");
        }
        println!("------------------------------------------------------------");
        println!("💡 Example curl usage:");
        println!(
            "   curl {} -H \"Content-Type: application/json\" \\",
            chat_url
        );
        println!("     -d '{{\"model\": \"{}\", \"messages\": [{{\"role\": \"user\", \"content\": \"Hello!\"}}]}}'", model_name);
        println!("============================================================");
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_summary_card_does_not_panic() {
        ServeManager::print_summary_card(
            "llama-3.2-1b",
            ServeEngine::LlamaCpp,
            "http://127.0.0.1:8000/v1",
            "http://127.0.0.1:8000/v1/chat/completions",
            "http://127.0.0.1:8000/v1/models",
            Some("secret-key"),
        );
    }

    #[test]
    fn test_sanitize_restored_rootfs() {
        let temp = tempfile::tempdir().unwrap();
        let rootfs = temp.path().join("rootfs");
        let tmp_dir = rootfs.join("tmp");
        let run_dir = rootfs.join("run");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        std::fs::create_dir_all(&run_dir).unwrap();

        let sock_file = tmp_dir.join("vllm.sock");
        let pid_file = run_dir.join("vllm.pid");
        let lock_file = tmp_dir.join("inference.lock");
        let regular_file = tmp_dir.join("model_weights.bin");

        std::fs::write(&sock_file, "sock").unwrap();
        std::fs::write(&pid_file, "1234").unwrap();
        std::fs::write(&lock_file, "lock").unwrap();
        std::fs::write(&regular_file, "weights").unwrap();

        assert!(sock_file.exists());
        assert!(pid_file.exists());
        assert!(lock_file.exists());
        assert!(regular_file.exists());

        ServeManager::sanitize_restored_rootfs(&rootfs);

        assert!(!sock_file.exists(), "stale socket should have been removed");
        assert!(
            !pid_file.exists(),
            "stale pid file should have been removed"
        );
        assert!(
            !lock_file.exists(),
            "stale lock file should have been removed"
        );
        assert!(regular_file.exists(), "regular file must be preserved");
    }
}
