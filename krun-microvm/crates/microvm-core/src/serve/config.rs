// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Serving configuration and inference engine auto-dispatch for `cro serve` and `cro vllm`.

use crate::config::parse_size_to_bytes;
use crate::modelpack::{CachedModelMetadata, ModelFormat};
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
    /// Trailing custom arguments passed directly to the inference binary.
    pub extra_args: Vec<String>,
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
            extra_args: Vec::new(),
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
            ServeEngine::LlamaCpp => "ghcr.io/ggerganov/llama.cpp:server".to_string(),
            ServeEngine::Uzu => "trymirai/uzu:latest".to_string(),
        }
    }

    /// Builds the execution command vector for the active engine.
    pub fn build_container_cmd(
        &self,
        active_engine: ServeEngine,
        meta: &CachedModelMetadata,
    ) -> Vec<String> {
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
                    "/server".to_string(),
                    "-m".to_string(),
                    gguf_file,
                    "--host".to_string(),
                    "0.0.0.0".to_string(),
                    "--port".to_string(),
                    "8000".to_string(),
                ];
                if let Some(ctx) = self.max_model_len.or(meta.context_length) {
                    cmd.push("-c".to_string());
                    cmd.push(ctx.to_string());
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

        let mut builder = MicroVmBuilder::new(image)
            .cpus(self.cpus)
            .memory_mb(self.memory_mb)
            .cmd(cmd)
            .port_forward(self.host_port, 8000)
            .model(
                model_path.to_string_lossy(),
                Some(format!("/models/{}", meta.name)),
            );

        if self.gpu {
            builder = builder.gpu(true);
            let shm_bytes = parse_size_to_bytes(&self.gpu_shm_size)
                .map_err(|e| anyhow::anyhow!("Invalid --gpu-shm-size: {}", e))?;
            builder = builder.gpu_shm_size(shm_bytes);
        }

        if let Some(dax_bytes) = self.dax_window_size {
            builder = builder.dax_window_size(dax_bytes);
        }

        if let Some(ref ddir) = self.data_dir {
            builder = builder.data_dir(ddir);
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
        assert_eq!(image, "ghcr.io/ggerganov/llama.cpp:server");

        let cmd = config.build_container_cmd(engine, &meta);
        assert_eq!(cmd[0], "/server");
        assert_eq!(cmd[1], "-m");
        assert_eq!(cmd[2], "/models/llama-gguf/tiny.Q4_K_M.gguf");
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
}
