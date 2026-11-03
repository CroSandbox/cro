// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Serving configuration and inference engine auto-dispatch for `cro serve` and `cro vllm`.

use crate::config::parse_size_to_bytes;
use crate::modelpack::{CachedModelMetadata, HardwareFit, ModelFormat};
use crate::vm::MicroVmBuilder;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Supported local and containerized inference serving engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServeEngine {
    /// Inspect model format and host platform to automatically select the optimal engine.
    Auto,
    /// High-throughput vLLM engine with PagedAttention and continuous batching.
    Vllm,
    /// Universal llama.cpp server with Vulkan/Metal and GGUF quantization support.
    LlamaCpp,
    /// Ultra-fast pure-Rust Apple Silicon engine with speculative decoding.
    Uzu,
}

impl std::str::FromStr for ServeEngine {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "vllm" => Ok(Self::Vllm),
            "llamacpp" | "llama.cpp" | "llama_cpp" | "llama" => Ok(Self::LlamaCpp),
            "uzu" => Ok(Self::Uzu),
            other => Err(format!(
                "Unknown inference engine '{other}', supported: auto, vllm, llamacpp, uzu"
            )),
        }
    }
}

impl std::fmt::Display for ServeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Vllm => write!(f, "vllm"),
            Self::LlamaCpp => write!(f, "llama.cpp"),
            Self::Uzu => write!(f, "uzu"),
        }
    }
}

/// Configuration parameters for serving a model inside a hardware-isolated microVM.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// Model identifier, local directory, or OCI reference.
    pub model: String,
    /// Requested inference engine (default: `ServeEngine::Auto`).
    pub engine: ServeEngine,
    /// Host port to expose the OpenAI-compatible HTTP API on (default: 8000).
    pub host_port: u16,
    /// Host interface to bind the API to (default: `127.0.0.1`).
    pub host: String,
    /// Custom base container image (defaults to engine standard image if `None`).
    pub container_image: Option<String>,
    /// Number of virtual CPUs assigned to the microVM (default: 4).
    pub cpus: u8,
    /// Guest RAM size in MiB (default: 8192 MiB / 8 GB).
    pub memory_mb: u32,
    /// Whether to enable hardware GPU acceleration (default: true).
    pub gpu: bool,
    /// Shared memory vRAM window size for GPU virtualization (default: "4G").
    pub gpu_shm_size: String,
    /// Optional VirtioFS DAX window size override (auto-calculated if `None`).
    pub dax_window_size: Option<u64>,
    /// Maximum model context length (tokens).
    pub max_model_len: Option<u64>,
    /// Tensor parallel ranks (default: 1).
    pub tensor_parallel_size: u32,
    /// Enforce eager execution in vLLM (disables CUDA/Metal graph capture).
    pub enforce_eager: bool,
    /// Optional API key requirement for the exposed OpenAI endpoints.
    pub api_key: Option<String>,
    /// Custom data directory for model caching.
    pub data_dir: Option<PathBuf>,
    /// Optional served model name or public alias (used in OpenAI API responses, e.g. "llama-3.2-1b").
    /// Defaults to the ModelPack name if None.
    pub served_model_name: Option<String>,
    /// Host directory or file paths containing Python packages or plugins to mount into the microVM.
    pub plugins: Vec<PathBuf>,
    /// Explicit vLLM plugin module names to register with VLLM_PLUGINS env var.
    pub vllm_plugins: Vec<String>,
    /// Whether to allow remote code execution in vLLM (--trust-remote-code).
    pub trust_remote_code: bool,
    /// Quantization method override for vLLM (e.g. "awq", "gptq", "fp8", "marlin").
    pub quantization: Option<String>,
    /// Optional custom Jinja chat template file path.
    pub chat_template: Option<PathBuf>,
    /// Custom environment variables to inject into the microVM guest.
    pub env_vars: Vec<(String, String)>,
    /// Automatically capture and freeze a warm-start snapshot when the inference server signals readiness.
    pub snapshot_on_ready: bool,
    /// Restore and instantly resume execution from a pre-warmed snapshot (< 100ms startup).
    pub warm: bool,
    /// Custom warm-start snapshot tag override.
    pub warmup_snapshot_tag: Option<String>,
    /// Trailing custom arguments passed directly to the inference binary.
    pub extra_args: Vec<String>,
    /// Enable scale-to-zero with automatic pausing on idle and sub-2ms wake-on-request.
    pub scale_to_zero: bool,
    /// Inactivity timeout in seconds before auto-pausing the microVM.
    pub idle_timeout_secs: Option<u64>,
    /// Internal backend loopback port forwarded to the guest inference server when scale-to-zero gateway is active.
    pub backend_port: Option<u16>,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            engine: ServeEngine::Auto,
            host_port: 8000,
            host: "127.0.0.1".to_string(),
            container_image: None,
            cpus: 4,
            memory_mb: 8192,
            gpu: true,
            gpu_shm_size: "4G".to_string(),
            dax_window_size: None,
            max_model_len: None,
            tensor_parallel_size: 1,
            enforce_eager: false,
            api_key: None,
            data_dir: None,
            served_model_name: None,
            plugins: Vec::new(),
            vllm_plugins: Vec::new(),
            trust_remote_code: false,
            quantization: None,
            chat_template: None,
            env_vars: Vec::new(),
            snapshot_on_ready: false,
            warm: false,
            warmup_snapshot_tag: None,
            extra_args: Vec::new(),
            scale_to_zero: false,
            idle_timeout_secs: None,
            backend_port: None,
        }
    }
}

impl ServeConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Sets the engine selection.
    pub fn engine(mut self, engine: ServeEngine) -> Self {
        self.engine = engine;
        self
    }

    /// Sets a custom container image override.
    pub fn container_image(mut self, image: impl Into<String>) -> Self {
        self.container_image = Some(image.into());
        self
    }

    /// Sets the host bind address.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Sets the host port to expose.
    pub fn host_port(mut self, port: u16) -> Self {
        self.host_port = port;
        self
    }

    /// Alias for host_port.
    pub fn port(self, port: u16) -> Self {
        self.host_port(port)
    }

    /// Sets the guest memory allocation in MB.
    pub fn memory_mb(mut self, mb: u32) -> Self {
        self.memory_mb = mb;
        self
    }

    /// Sets the guest vCPU allocation.
    pub fn cpus(mut self, cpus: u8) -> Self {
        self.cpus = cpus;
        self
    }

    /// Enables or disables GPU acceleration.
    pub fn gpu(mut self, enabled: bool) -> Self {
        self.gpu = enabled;
        self
    }

    /// Sets the GPU shared memory size.
    pub fn gpu_shm_size(mut self, size: impl Into<String>) -> Self {
        self.gpu_shm_size = size.into();
        self
    }

    /// Sets the DAX window size in bytes.
    pub fn dax_window_size(mut self, bytes: u64) -> Self {
        self.dax_window_size = Some(bytes);
        self
    }

    /// Sets the maximum context length.
    pub fn max_model_len(mut self, len: u64) -> Self {
        self.max_model_len = Some(len);
        self
    }

    /// Sets tensor parallel size.
    pub fn tensor_parallel_size(mut self, size: u32) -> Self {
        self.tensor_parallel_size = size;
        self
    }

    /// Enables or disables eager execution.
    pub fn enforce_eager(mut self, eager: bool) -> Self {
        self.enforce_eager = eager;
        self
    }

    /// Sets an API key for the server.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Sets extra CLI arguments to pass to the engine container.
    pub fn extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Sets custom data/cache directory.
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// Sets the served model name or public alias (e.g. "llama-3.2-1b").
    pub fn served_model_name(mut self, name: impl Into<String>) -> Self {
        self.served_model_name = Some(name.into());
        self
    }

    /// Convenience alias for `served_model_name`.
    pub fn alias(self, alias: impl Into<String>) -> Self {
        self.served_model_name(alias)
    }

    /// Appends a plugin path (file or directory) to mount into the microVM.
    pub fn plugin(mut self, path: impl Into<PathBuf>) -> Self {
        self.plugins.push(path.into());
        self
    }

    /// Appends multiple plugin paths to mount into the microVM.
    pub fn plugins(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.plugins.extend(paths.into_iter().map(Into::into));
        self
    }

    /// Appends an explicit vLLM plugin module name to register with `VLLM_PLUGINS`.
    pub fn vllm_plugin(mut self, plugin: impl Into<String>) -> Self {
        self.vllm_plugins.push(plugin.into());
        self
    }

    /// Appends multiple vLLM plugin module names to register with `VLLM_PLUGINS`.
    pub fn vllm_plugins(mut self, plugins: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.vllm_plugins
            .extend(plugins.into_iter().map(Into::into));
        self
    }

    /// Enables or disables `--trust-remote-code` for vLLM.
    pub fn trust_remote_code(mut self, trust: bool) -> Self {
        self.trust_remote_code = trust;
        self
    }

    /// Sets the quantization method (e.g. "awq", "gptq", "fp8", "marlin").
    pub fn quantization(mut self, method: impl Into<String>) -> Self {
        self.quantization = Some(method.into());
        self
    }

    /// Sets the custom Jinja chat template file path.
    pub fn chat_template(mut self, template: impl Into<PathBuf>) -> Self {
        self.chat_template = Some(template.into());
        self
    }

    /// Adds a custom environment variable to inject into the microVM.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.push((key.into(), value.into()));
        self
    }

    /// Adds multiple custom environment variables to inject into the microVM.
    pub fn env_vars(mut self, vars: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env_vars.extend(vars);
        self
    }

    /// Automatically captures a warm-start snapshot upon reaching readiness.
    pub fn snapshot_on_ready(mut self, enabled: bool) -> Self {
        self.snapshot_on_ready = enabled;
        self
    }

    /// Boots the serving engine from a pre-warmed snapshot (< 100ms startup).
    pub fn warm(mut self, warm: bool) -> Self {
        self.warm = warm;
        self
    }

    /// Sets a custom warm-start snapshot tag override.
    pub fn warmup_snapshot_tag(mut self, tag: impl Into<String>) -> Self {
        self.warmup_snapshot_tag = Some(tag.into());
        self
    }

    /// Enables scale-to-zero with automatic pausing on idle and sub-2ms wake-on-request.
    pub fn scale_to_zero(mut self, enabled: bool) -> Self {
        self.scale_to_zero = enabled;
        self
    }

    /// Sets inactivity timeout in seconds before auto-pausing the microVM.
    pub fn idle_timeout(mut self, timeout_secs: u64) -> Self {
        self.idle_timeout_secs = Some(timeout_secs);
        self.scale_to_zero = true;
        self
    }

    /// Sets internal backend loopback port forwarded to the guest when scale-to-zero gateway is used.
    pub fn backend_port(mut self, port: u16) -> Self {
        self.backend_port = Some(port);
        self
    }

    /// Computes a deterministic SHA-256 fingerprint for a warm snapshot.
    pub fn compute_warm_snapshot_key(&self, model_id: &str) -> String {
        if let Some(ref tag) = self.warmup_snapshot_tag {
            return format!("warm-{}", tag);
        }
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(model_id.as_bytes());
        hasher.update(self.engine.to_string().as_bytes());
        hasher.update(self.cpus.to_le_bytes());
        hasher.update(self.memory_mb.to_le_bytes());
        hasher.update(self.gpu_shm_size.as_bytes());
        if let Some(ref img) = self.container_image {
            hasher.update(img.as_bytes());
        }
        if let Some(len) = self.max_model_len {
            hasher.update(len.to_le_bytes());
        }
        if let Some(ref q) = self.quantization {
            hasher.update(q.as_bytes());
        }
        hasher.update(self.tensor_parallel_size.to_le_bytes());
        hasher.update([if self.trust_remote_code { 1 } else { 0 }]);
        for arg in &self.extra_args {
            hasher.update(arg.as_bytes());
        }
        let hash = format!("{:x}", hasher.finalize());
        format!("warm-{}", &hash[..16])
    }

    /// Returns the effective model name / alias used in the server API.
    pub fn effective_model_name<'a>(&'a self, meta: &'a CachedModelMetadata) -> &'a str {
        self.served_model_name.as_deref().unwrap_or(&meta.name)
    }

    /// Automatically selects the optimal inference engine based on model format and platform.
    pub fn resolve_engine(&self, meta: &CachedModelMetadata) -> ServeEngine {
        if self.engine != ServeEngine::Auto {
            return self.engine;
        }

        match &meta.format {
            ModelFormat::Gguf => ServeEngine::LlamaCpp,
            ModelFormat::Other(name) if name.to_ascii_lowercase().contains("uzu") => {
                ServeEngine::Uzu
            }
            ModelFormat::Safetensors
            | ModelFormat::Onnx
            | ModelFormat::PyTorch
            | ModelFormat::Mlx => ServeEngine::Vllm,
            ModelFormat::Other(_) => {
                // Check if any weight files end with .gguf
                if meta
                    .files
                    .iter()
                    .any(|f| f.path.to_ascii_lowercase().ends_with(".gguf"))
                {
                    ServeEngine::LlamaCpp
                } else {
                    ServeEngine::Vllm
                }
            }
        }
    }

    /// Resolves the base container image for the active serving engine.
    pub fn resolve_image(&self, active_engine: ServeEngine) -> String {
        if let Some(ref img) = self.container_image {
            return img.clone();
        }

        match active_engine {
            ServeEngine::Vllm | ServeEngine::Auto => "vllm/vllm-openai:latest".to_string(),
            ServeEngine::LlamaCpp => "ghcr.io/ggml-org/llama.cpp:server".to_string(),
            ServeEngine::Uzu => "trymirai/uzu:latest".to_string(),
        }
    }

    /// Builds the execution command vector for the active engine.
    pub fn build_container_cmd(
        &self,
        active_engine: ServeEngine,
        meta: &CachedModelMetadata,
    ) -> Vec<String> {
        let model_alias = self.effective_model_name(meta);

        match active_engine {
            ServeEngine::Vllm | ServeEngine::Auto => {
                let mut cmd = vec![
                    "vllm".to_string(),
                    "serve".to_string(),
                    format!("/models/{}", meta.name),
                    "--host".to_string(),
                    "0.0.0.0".to_string(),
                    "--port".to_string(),
                    "8000".to_string(),
                    "--served-model-name".to_string(),
                    model_alias.to_string(),
                ];
                if let Some(len) = self.max_model_len.or(meta.context_length) {
                    cmd.push("--max-model-len".to_string());
                    cmd.push(len.to_string());
                }
                if self.tensor_parallel_size > 1 {
                    cmd.push("--tensor-parallel-size".to_string());
                    cmd.push(self.tensor_parallel_size.to_string());
                }
                if self.enforce_eager {
                    cmd.push("--enforce-eager".to_string());
                }
                if self.trust_remote_code {
                    cmd.push("--trust-remote-code".to_string());
                }
                if let Some(ref q) = self.quantization {
                    cmd.push("--quantization".to_string());
                    cmd.push(q.clone());
                }
                if self.chat_template.is_some() {
                    cmd.push("--chat-template".to_string());
                    cmd.push("/etc/chat_template.jinja".to_string());
                }
                if let Some(ref key) = self.api_key {
                    cmd.push("--api-key".to_string());
                    cmd.push(key.clone());
                }
                cmd.extend(self.extra_args.clone());
                cmd
            }

            ServeEngine::LlamaCpp => {
                let gguf_file = meta
                    .files
                    .iter()
                    .find(|f| f.path.to_ascii_lowercase().ends_with(".gguf"))
                    .map(|f| format!("/models/{}/{}", meta.name, f.path))
                    .unwrap_or_else(|| format!("/models/{}", meta.name));

                let mut cmd = vec![
                    "/app/llama-server".to_string(),
                    "-m".to_string(),
                    gguf_file,
                    "--host".to_string(),
                    "0.0.0.0".to_string(),
                    "--port".to_string(),
                    "8000".to_string(),
                    "--alias".to_string(),
                    model_alias.to_string(),
                ];
                if let Some(ctx) = self.max_model_len.or(meta.context_length) {
                    cmd.push("-c".to_string());
                    cmd.push(ctx.to_string());
                }
                if self.chat_template.is_some() {
                    cmd.push("--chat-template-file".to_string());
                    cmd.push("/etc/chat_template.jinja".to_string());
                }
                if let Some(ref key) = self.api_key {
                    cmd.push("--api-key".to_string());
                    cmd.push(key.clone());
                }
                cmd.extend(self.extra_args.clone());
                cmd
            }

            ServeEngine::Uzu => {
                let mut cmd = vec![
                    "uzu".to_string(),
                    "server".to_string(),
                    "--model".to_string(),
                    format!("/models/{}", meta.name),
                    "--host".to_string(),
                    "0.0.0.0".to_string(),
                    "--port".to_string(),
                    "8000".to_string(),
                ];
                if let Some(ref key) = self.api_key {
                    cmd.push("--api-key".to_string());
                    cmd.push(key.clone());
                }
                cmd.extend(self.extra_args.clone());
                cmd
            }
        }
    }

    /// Configures and returns a `MicroVmBuilder` ready to execute the model server.
    pub fn to_microvm_builder(
        &self,
        active_engine: ServeEngine,
        meta: &CachedModelMetadata,
        model_path: &Path,
    ) -> Result<MicroVmBuilder> {
        let image = self.resolve_image(active_engine);
        let cmd = self.build_container_cmd(active_engine, meta);

        let effective_memory_mb = if self.memory_mb == 8192 && meta.total_size > 0 {
            let ctx = self.max_model_len.or(meta.context_length).unwrap_or(4096);
            if let Ok(fit) = HardwareFit::profile(model_path, Some(ctx)) {
                if fit.recommended_guest_ram_mib > self.memory_mb {
                    tracing::info!(
                        "Auto-sizing guest RAM to {} MiB based on hardware fit profile (weights: {} MB, KV: {} MB)",
                        fit.recommended_guest_ram_mib,
                        fit.model_size_bytes / (1024 * 1024),
                        fit.estimated_kv_cache_bytes / (1024 * 1024)
                    );
                    fit.recommended_guest_ram_mib
                } else {
                    self.memory_mb
                }
            } else {
                self.memory_mb
            }
        } else {
            self.memory_mb
        };

        let effective_host_port = self.backend_port.unwrap_or(self.host_port);
        let mut builder = MicroVmBuilder::new(image)
            .cpus(self.cpus)
            .memory_mb(effective_memory_mb)
            .cmd(cmd)
            .port_forward(effective_host_port, 8000)
            .model(
                model_path.to_string_lossy(),
                Some(format!("/models/{}", meta.name)),
            );

        if self.gpu {
            builder = builder.gpu(true);
            let mut shm_bytes = parse_size_to_bytes(&self.gpu_shm_size)
                .map_err(|e| anyhow::anyhow!("Invalid --gpu-shm-size: {}", e))?;
            if self.gpu_shm_size == "4G" {
                let ctx = self.max_model_len.or(meta.context_length).unwrap_or(4096);
                if let Ok(fit) = HardwareFit::profile(model_path, Some(ctx)) {
                    let rec_shm_bytes = (fit.recommended_gpu_shm_gib as u64) * 1024 * 1024 * 1024;
                    if rec_shm_bytes > shm_bytes {
                        shm_bytes = rec_shm_bytes;
                    }
                }
            }
            builder = builder.gpu_shm_size(shm_bytes);
        }

        if let Some(dax_bytes) = self.dax_window_size {
            builder = builder.dax_window_size(dax_bytes);
        }

        if let Some(ref ddir) = self.data_dir {
            builder = builder.data_dir(ddir);
        }

        if let Some(ref tpl) = self.chat_template {
            builder = builder.file_mount(tpl.clone(), "/etc/chat_template.jinja");
        }

        // Mount plugins and configure python paths
        let mut python_paths = Vec::new();
        let mut auto_vllm_plugins = self.vllm_plugins.clone();

        for (idx, plugin) in self.plugins.iter().enumerate() {
            if plugin.is_dir() {
                let tag = format!("plugin_{idx}");
                builder = builder.virtiofs(&tag, plugin.clone(), true);
                python_paths.push(format!("/{}", tag));
                if let Some(name) = plugin.file_name().and_then(|n| n.to_str()) {
                    if !auto_vllm_plugins.iter().any(|p| p == name) {
                        auto_vllm_plugins.push(name.to_string());
                    }
                }
            } else if plugin.is_file() {
                if let Some(name) = plugin.file_name().and_then(|n| n.to_str()) {
                    let dest = format!("/plugins/{}", name);
                    builder = builder.file_mount(plugin.clone(), dest);
                    if !python_paths.iter().any(|p| p == "/plugins") {
                        python_paths.push("/plugins".to_string());
                    }
                    if let Some(stem) = plugin.file_stem().and_then(|s| s.to_str()) {
                        if !auto_vllm_plugins.iter().any(|p| p == stem) {
                            auto_vllm_plugins.push(stem.to_string());
                        }
                    }
                }
            }
        }

        if !python_paths.is_empty() {
            let py_val = python_paths.join(":");
            builder = builder.env("PYTHONPATH", &format!("{}:/app", py_val));
        }

        if !auto_vllm_plugins.is_empty() {
            let vllm_plugs = auto_vllm_plugins.join(",");
            builder = builder.env("VLLM_PLUGINS", &vllm_plugs);
        }

        for (k, v) in &self.env_vars {
            builder = builder.env(k, v);
        }

        Ok(builder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modelpack::ModelFileEntry;
    use crate::modelpack::ModelFileRole;

    #[test]
    fn test_serve_engine_auto_dispatch_gguf() {
        let config = ServeConfig::new("my-llama");
        let meta = CachedModelMetadata {
            reference: "local://llama".to_string(),
            digest: "sha256:1111".to_string(),
            name: "llama-gguf".to_string(),
            format: ModelFormat::Gguf,
            architecture: Some("llama".to_string()),
            precision: Some("q4_k_m".to_string()),
            context_length: Some(4096),
            files: vec![ModelFileEntry {
                path: "tiny.Q4_K_M.gguf".to_string(),
                role: ModelFileRole::Weight,
                size: 1024,
                digest: None,
            }],
            total_size: 1024,
            created_at: 1000,
            path: PathBuf::from("/models/llama"),
        };

        let engine = config.resolve_engine(&meta);
        assert_eq!(engine, ServeEngine::LlamaCpp);

        let image = config.resolve_image(engine);
        assert_eq!(image, "ghcr.io/ggml-org/llama.cpp:server");

        let cmd = config.build_container_cmd(engine, &meta);
        assert_eq!(cmd[0], "/app/llama-server");
        assert_eq!(cmd[1], "-m");
        assert_eq!(cmd[2], "/models/llama-gguf/tiny.Q4_K_M.gguf");
        assert!(cmd.contains(&"--alias".to_string()));
        assert!(cmd.contains(&"llama-gguf".to_string()));
        assert!(cmd.contains(&"-c".to_string()));
        assert!(cmd.contains(&"4096".to_string()));
    }

    #[test]
    fn test_serve_engine_auto_dispatch_safetensors_vllm() {
        let mut config = ServeConfig::new("mistral");
        config.api_key = Some("test-key".to_string());
        config.max_model_len = Some(8192);

        let meta = CachedModelMetadata {
            reference: "local://mistral".to_string(),
            digest: "sha256:2222".to_string(),
            name: "mistral-7b".to_string(),
            format: ModelFormat::Safetensors,
            architecture: Some("mistral".to_string()),
            precision: Some("fp16".to_string()),
            context_length: Some(8192),
            files: vec![ModelFileEntry {
                path: "model.safetensors".to_string(),
                role: ModelFileRole::Weight,
                size: 5000,
                digest: None,
            }],
            total_size: 5000,
            created_at: 1000,
            path: PathBuf::from("/models/mistral"),
        };

        let engine = config.resolve_engine(&meta);
        assert_eq!(engine, ServeEngine::Vllm);

        let image = config.resolve_image(engine);
        assert_eq!(image, "vllm/vllm-openai:latest");

        let cmd = config.build_container_cmd(engine, &meta);
        assert_eq!(cmd[0], "vllm");
        assert_eq!(cmd[1], "serve");
        assert_eq!(cmd[2], "/models/mistral-7b");
        assert!(cmd.contains(&"--served-model-name".to_string()));
        assert!(cmd.contains(&"mistral-7b".to_string()));
        assert!(cmd.contains(&"--max-model-len".to_string()));
        assert!(cmd.contains(&"8192".to_string()));
        assert!(cmd.contains(&"--api-key".to_string()));
        assert!(cmd.contains(&"test-key".to_string()));
    }

    #[test]
    fn test_serve_engine_manual_override() {
        let mut config = ServeConfig::new("custom");
        config.engine = ServeEngine::Vllm; // override even if model is GGUF

        let meta = CachedModelMetadata {
            reference: "local://test".to_string(),
            digest: "sha256:3333".to_string(),
            name: "test-model".to_string(),
            format: ModelFormat::Gguf,
            architecture: None,
            precision: None,
            context_length: None,
            files: vec![],
            total_size: 100,
            created_at: 1000,
            path: PathBuf::from("/models/test"),
        };

        let engine = config.resolve_engine(&meta);
        assert_eq!(engine, ServeEngine::Vllm);
    }

    #[test]
    fn test_serve_config_vllm_plugins_and_options() {
        let config = ServeConfig::new("qwen")
            .served_model_name("qwen-chat")
            .trust_remote_code(true)
            .quantization("fp8")
            .chat_template(PathBuf::from("/tmp/custom_template.jinja"))
            .vllm_plugin("my_plugin_module")
            .env("CUSTOM_VAR", "VAL123");

        let meta = CachedModelMetadata {
            reference: "local://qwen".to_string(),
            digest: "sha256:4444".to_string(),
            name: "qwen-7b".to_string(),
            format: ModelFormat::Safetensors,
            architecture: Some("qwen2".to_string()),
            precision: Some("bf16".to_string()),
            context_length: Some(4096),
            files: vec![],
            total_size: 1000,
            created_at: 1000,
            path: PathBuf::from("/models/qwen"),
        };

        let cmd = config.build_container_cmd(ServeEngine::Vllm, &meta);
        assert!(cmd.contains(&"--served-model-name".to_string()));
        assert!(cmd.contains(&"qwen-chat".to_string()));
        assert!(cmd.contains(&"--trust-remote-code".to_string()));
        assert!(cmd.contains(&"--quantization".to_string()));
        assert!(cmd.contains(&"fp8".to_string()));
        assert!(cmd.contains(&"--chat-template".to_string()));
        assert!(cmd.contains(&"/etc/chat_template.jinja".to_string()));

        let builder = config
            .to_microvm_builder(ServeEngine::Vllm, &meta, &PathBuf::from("/tmp/model"))
            .unwrap();
        // Chat template is mounted into guest
        assert!(builder.get_file_mounts().iter().any(|(src, dest)| src
            == &PathBuf::from("/tmp/custom_template.jinja")
            && dest == &PathBuf::from("/etc/chat_template.jinja")));
    }

    #[test]
    fn test_serve_config_llama_alias() {
        let config = ServeConfig::new("deepseek").alias("deepseek-r1-distill");

        let meta = CachedModelMetadata {
            reference: "local://deepseek".to_string(),
            digest: "sha256:5555".to_string(),
            name: "deepseek-raw".to_string(),
            format: ModelFormat::Gguf,
            architecture: Some("llama".to_string()),
            precision: Some("q4_0".to_string()),
            context_length: Some(2048),
            files: vec![],
            total_size: 500,
            created_at: 1000,
            path: PathBuf::from("/models/deepseek"),
        };

        let cmd = config.build_container_cmd(ServeEngine::LlamaCpp, &meta);
        assert_eq!(cmd[0], "/app/llama-server");
        assert!(cmd.contains(&"--alias".to_string()));
        assert!(cmd.contains(&"deepseek-r1-distill".to_string()));
    }

    #[test]
    fn test_to_microvm_builder_auto_memory_fit() {
        let temp_dir = tempfile::tempdir().unwrap();
        let model_file = temp_dir.path().join("model.safetensors");
        // Write 10 MB dummy file
        std::fs::write(&model_file, vec![0u8; 10 * 1024 * 1024]).unwrap();

        let config = ServeConfig::new("test-model");
        let meta = CachedModelMetadata {
            reference: "local://test-model".to_string(),
            digest: "sha256:test".to_string(),
            name: "test-model".to_string(),
            format: ModelFormat::Safetensors,
            architecture: Some("llama".to_string()),
            precision: Some("fp16".to_string()),
            context_length: Some(4096),
            files: vec![],
            total_size: 10 * 1024 * 1024,
            created_at: 1000,
            path: model_file.clone(),
        };

        let builder = config
            .to_microvm_builder(ServeEngine::Vllm, &meta, &model_file)
            .unwrap();
        // Since default is 8192 MB and 10MB model fit doesn't exceed 8192 MB, memory remains 8192 MB
        assert_eq!(builder.get_ram_mib(), 8192);
    }

    #[test]
    fn test_compute_warm_snapshot_key() {
        let config1 = ServeConfig::new("llama3");
        let key1 = config1.compute_warm_snapshot_key("sha256:model1");
        assert!(key1.starts_with("warm-"));

        // Identical config -> identical key
        let config2 = ServeConfig::new("llama3");
        let key2 = config2.compute_warm_snapshot_key("sha256:model1");
        assert_eq!(key1, key2);

        // Different engine -> different key
        let mut config3 = ServeConfig::new("llama3");
        config3.engine = ServeEngine::Vllm;
        let key3 = config3.compute_warm_snapshot_key("sha256:model1");
        assert_ne!(key1, key3);

        // Different quantization -> different key
        let mut config_quant = ServeConfig::new("llama3");
        config_quant.quantization = Some("awq".to_string());
        let key_quant = config_quant.compute_warm_snapshot_key("sha256:model1");
        assert_ne!(key1, key_quant);

        // Different extra_args -> different key
        let mut config_extra = ServeConfig::new("llama3");
        config_extra.extra_args = vec!["--gpu-memory-utilization".to_string(), "0.9".to_string()];
        let key_extra = config_extra.compute_warm_snapshot_key("sha256:model1");
        assert_ne!(key1, key_extra);

        // Custom tag override
        let config_tagged = ServeConfig::new("llama3").warmup_snapshot_tag("prod-v1");
        let key_tagged = config_tagged.compute_warm_snapshot_key("sha256:model1");
        assert_eq!(key_tagged, "warm-prod-v1");
    }
}
