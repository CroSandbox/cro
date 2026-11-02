// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Serving lifecycle manager and health check prober for `cro serve` and `cro vllm`.

use super::config::{ServeConfig, ServeEngine};
use crate::modelpack::ModelEngine;
use crate::vm::MicroVm;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
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
}

impl ServeHandle {
    /// Awaits the microVM process until it exits or is interrupted.
    pub async fn wait(mut self) -> Result<std::process::ExitStatus> {
        if let Some(ref mut vm) = self.vm {
            vm.wait().await
        } else {
            bail!("MicroVM was launched in detached mode; no active process handle to await");
        }
    }

    /// Awaits the microVM process while gracefully capturing SIGINT / Ctrl+C.
    pub async fn wait_with_sigint(&mut self) -> Result<()> {
        if let Some(ref mut vm) = self.vm {
            tokio::select! {
                res = vm.wait() => {
                    let status = res?;
                    if !status.success() {
                        bail!("MicroVM exited with error status: {}", status);
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("\n🛑 Interrupted. Gracefully stopping microVM inference server (VM ID: {})...", self.vm_id);
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

        // 1. Inspect ModelPack metadata
        let meta = ModelEngine::inspect_model(&config.model, data_dir)
            .with_context(|| format!("Failed to inspect model '{}'", config.model))?;

        // 2. Resolve serving engine
        let active_engine = config.resolve_engine(&meta);

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
            config.cpus, config.memory_mb, config.gpu_shm_size
        );

        // 3. Build MicroVmBuilder
        let mut builder = config
            .to_microvm_builder(active_engine, &meta, &meta.path)
            .with_context(|| "Failed to construct MicroVmBuilder for serving")?;

        if detach {
            builder = builder.detach(true);
        }

        // 4. Spawn microVM
        let mut vm = builder
            .run()
            .await
            .context("Failed to spawn serving microVM")?;
        let vm_id = vm.id().to_string();

        let base_url = format!("http://{}:{}/v1", config.host, config.host_port);
        let chat_url = format!("{}/chat/completions", base_url);
        let models_url = format!("{}/models", base_url);
        let model_alias = config.effective_model_name(&meta);

        // 5. Health check probing
        if !detach || wait {
            println!(
                "⏳ Probing inference server readiness on port {}...",
                config.host_port
            );
            Self::poll_readiness_with_vm(
                &config.host,
                config.host_port,
                Duration::from_secs(120),
                Some(&mut vm),
            )
            .await?;

            Self::print_summary_card(
                model_alias,
                active_engine,
                &base_url,
                &chat_url,
                &models_url,
                config.api_key.as_deref(),
            );
        } else {
            println!("🚀 Server spawned in detached mode (VM ID: {}).", vm_id);
            println!("   Base URL: {}", base_url);
        }

        Ok(ServeHandle {
            vm_id,
            engine: active_engine,
            model_name: model_alias.to_string(),
            base_url,
            chat_url,
            models_url,
            vm: if detach { None } else { Some(vm) },
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
}
