// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CNCF ModelPack Engine for pulling, caching, inspecting, and listing models.

use super::spec::*;
use crate::oci::digest::{sha256_hex, verify_sha256};
use crate::oci::{ImageReference, OciClient};
use crate::rootfs::extract::extract_layer;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Descriptor metadata for a locally cached or inspected ModelPack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedModelMetadata {
    /// Canonical OCI reference or source path.
    pub reference: String,
    /// Cryptographic digest of the model manifest (e.g. `sha256:...`).
    pub digest: String,
    /// Model name.
    pub name: String,
    /// Model weight serialization format.
    pub format: ModelFormat,
    /// Architecture descriptor (e.g. `llama`, `mistral`).
    #[serde(default)]
    pub architecture: Option<String>,
    /// Precision/quantization level (e.g. `q4_k_m`, `fp16`).
    #[serde(default)]
    pub precision: Option<String>,
    /// Maximum context length.
    #[serde(default)]
    pub context_length: Option<u64>,
    /// Model file manifest entries.
    #[serde(default)]
    pub files: Vec<ModelFileEntry>,
    /// Total unpacked size on disk in bytes.
    #[serde(default)]
    pub total_size: u64,
    /// Timestamp when cached (seconds since UNIX epoch).
    pub created_at: u64,
    /// Absolute or relative path to the unpacked model directory.
    pub path: PathBuf,
}

pub struct ModelEngine;

impl ModelEngine {
    /// Pulls a CNCF ModelPack artifact from an OCI registry into `cache_base/models/<safe_digest>`.
    pub async fn pull_model(
        client: &OciClient,
        reference: &ImageReference,
        cache_base: &Path,
    ) -> Result<PathBuf> {
        let token = client.get_token(reference).await?;
        let token_ref = token.as_deref();

        // 1. Fetch manifest
        let (manifest, manifest_digest) = client.fetch_manifest(reference, token_ref).await?;
        if let Some(ref at) = manifest.artifact_type {
            if at != MODELPACK_MANIFEST_ARTIFACT_TYPE && at != MODELPACK_LEGACY_ARTIFACT_TYPE {
                tracing::debug!(
                    "Manifest artifactType '{at}' differs from standard ModelPack manifest"
                );
            }
        }
        let safe_digest = manifest_digest.replace(':', "_");
        let model_dir = cache_base.join("models").join(&safe_digest);
        let meta_path = model_dir.join(".modelpack.json");

        // Cache hit check
        if model_dir.exists() && meta_path.exists() {
            tracing::info!(
                "Model cache hit for '{}' ({})",
                reference.canonical_name(),
                manifest_digest
            );
            return Ok(model_dir);
        }

        let staging_dir = cache_base
            .join("models")
            .join(format!(".staging_{}", safe_digest));
        if staging_dir.exists() {
            let _ = fs::remove_dir_all(&staging_dir);
        }
        fs::create_dir_all(&staging_dir).with_context(|| {
            format!(
                "Failed to create model staging directory: {}",
                staging_dir.display()
            )
        })?;

        tracing::info!(
            "Downloading ModelPack {} ({} layers)...",
            reference.canonical_name(),
            manifest.layers.len()
        );

        // 2. Fetch and parse ModelConfig
        let config_bytes = client
            .fetch_blob_bytes(reference, &manifest.config.digest, token_ref)
            .await?;
        let mut model_config = match ModelConfig::from_json_slice(&config_bytes) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(
                    "Could not parse ModelConfig JSON ({e}); defaulting to inferred metadata"
                );
                let default_name = reference
                    .repository
                    .split('/')
                    .next_back()
                    .unwrap_or("model")
                    .to_string();
                ModelConfig::new(default_name, ModelFormat::Other("unknown".to_string()))
            }
        };

        // 3. Download and extract layers
        let total_layers = manifest.layers.len();
        for (i, layer) in manifest.layers.iter().enumerate() {
            tracing::info!(
                "Fetching ModelPack layer [{}/{}] ({})",
                i + 1,
                total_layers,
                layer.digest
            );
            let blob_bytes = client
                .fetch_blob_bytes(reference, &layer.digest, token_ref)
                .await?;

            let is_tar = layer
                .media_type
                .as_ref()
                .is_some_and(|m| m.contains("tar") || m.contains("layer"))
                || layer.digest.ends_with(".tar");

            if is_tar {
                let is_gz = layer
                    .media_type
                    .as_ref()
                    .is_some_and(|m| m.contains("gzip"))
                    || layer.digest.ends_with(".tar.gz");
                let cursor = std::io::Cursor::new(blob_bytes);
                extract_layer(cursor, &staging_dir, is_gz).with_context(|| {
                    format!("Failed to unpack ModelPack layer {}", layer.digest)
                })?;
            } else {
                // Raw blob fallback
                let filename = layer
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get("org.opencontainers.image.title"))
                    .cloned()
                    .unwrap_or_else(|| {
                        let short = &layer.digest[..layer.digest.len().min(12)];
                        format!("blob_{short}.bin")
                    });

                crate::rootfs::tar_security::write_file_in_root(
                    &staging_dir,
                    Path::new(&filename),
                    &blob_bytes,
                )?;
            }
        }

        // 4. Index files and compute total size
        let (scanned_files, total_size, inferred_format) =
            Self::scan_directory_files(&staging_dir)?;
        if model_config.files.is_empty() {
            model_config.files = scanned_files;
        }
        if model_config.format == ModelFormat::Other("unknown".to_string()) {
            if let Some(fmt) = inferred_format {
                model_config.format = fmt;
            }
        }

        // 5. Store metadata descriptor
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let metadata = CachedModelMetadata {
            reference: reference.canonical_name(),
            digest: manifest_digest,
            name: model_config.name,
            format: model_config.format,
            architecture: model_config.architecture,
            precision: model_config.precision,
            context_length: model_config.context_length,
            files: model_config.files,
            total_size,
            created_at: now,
            path: model_dir.clone(),
        };

        crate::rootfs::tar_security::write_file_in_root(
            &staging_dir,
            Path::new(".modelpack.json"),
            serde_json::to_string_pretty(&metadata)?.as_bytes(),
        )?;

        // 6. Commit staging directory to model_dir
        if model_dir.exists() {
            let _ = fs::remove_dir_all(&model_dir);
        }
        if let Some(parent) = model_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&staging_dir, &model_dir).with_context(|| {
            format!(
                "Failed to commit model directory from {} to {}",
                staging_dir.display(),
                model_dir.display()
            )
        })?;

        tracing::info!(
            "Successfully cached ModelPack '{}' at {}",
            metadata.name,
            model_dir.display()
        );

        Ok(model_dir)
    }

    /// Lists all locally cached ModelPack instances from `cache_base/models/`.
    pub fn list_cached(cache_base: &Path) -> Result<Vec<CachedModelMetadata>> {
        let models_dir = cache_base.join("models");
        if !models_dir.exists() {
            return Ok(Vec::new());
        }

        let mut list = Vec::new();
        for entry in fs::read_dir(&models_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let meta_file = path.join(".modelpack.json");
                if meta_file.exists() {
                    if let Ok(content) = fs::read(&meta_file) {
                        if let Ok(mut meta) =
                            serde_json::from_slice::<CachedModelMetadata>(&content)
                        {
                            meta.path = path.clone();
                            list.push(meta);
                        }
                    }
                }
            }
        }

        list.sort_by_key(|m| std::cmp::Reverse(m.created_at));
        Ok(list)
    }

    /// Searches for a cached model by reference, name, or digest.
    pub fn find_cached(
        reference_or_name: &str,
        cache_base: &Path,
    ) -> Result<Option<CachedModelMetadata>> {
        let models = Self::list_cached(cache_base)?;
        for m in models {
            if m.reference == reference_or_name
                || m.name == reference_or_name
                || m.digest == reference_or_name
                || m.digest.strip_prefix("sha256:") == Some(reference_or_name)
                || m.path.to_string_lossy() == reference_or_name
            {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// Inspects a model given either a local filesystem path or an OCI reference/name.
    pub fn inspect_model(path_or_ref: &str, cache_base: &Path) -> Result<CachedModelMetadata> {
        let local_path = Path::new(path_or_ref);
        if local_path.exists() {
            return Self::inspect_local_path(local_path);
        }

        // Try finding in local cache
        if let Some(cached) = Self::find_cached(path_or_ref, cache_base)? {
            return Ok(cached);
        }

        bail!(
            "Model '{}' not found in cache or local filesystem. Run 'cro model pull {}' first.",
            path_or_ref,
            path_or_ref
        )
    }

    /// Inspects a local filesystem directory or model artifact.
    pub fn inspect_local_path(path: &Path) -> Result<CachedModelMetadata> {
        // Check 1: Cached ModelPack descriptor exists
        let meta_path = path.join(".modelpack.json");
        if meta_path.exists() {
            let content = fs::read(&meta_path)?;
            let mut meta = serde_json::from_slice::<CachedModelMetadata>(&content)?;
            meta.path = path.to_path_buf();
            return Ok(meta);
        }

        // Check 2: OCI Image Layout format (output of cro model pack)
        let oci_layout = path.join("oci-layout");
        let index_json = path.join("index.json");
        if oci_layout.exists() && index_json.exists() {
            return Self::inspect_oci_layout(path);
        }

        // Check 3: Raw directory containing model files
        let (files, total_size, inferred_format) = Self::scan_directory_files(path)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("model")
            .to_string();

        let format = inferred_format.unwrap_or(ModelFormat::Other("unknown".to_string()));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Ok(CachedModelMetadata {
            reference: format!("local://{}", path.display()),
            digest: "local".to_string(),
            name,
            format,
            architecture: None,
            precision: None,
            context_length: None,
            files,
            total_size,
            created_at: now,
            path: path.to_path_buf(),
        })
    }

    /// Inspects an OCI Image Layout directory.
    fn inspect_oci_layout(layout_dir: &Path) -> Result<CachedModelMetadata> {
        let index_data = fs::read(layout_dir.join("index.json"))?;
        let index: serde_json::Value = serde_json::from_slice(&index_data)?;

        let manifest_entry = index
            .get("manifests")
            .and_then(|m| m.as_array())
            .and_then(|arr| arr.first())
            .context("No manifests found in index.json")?;

        let manifest_digest = manifest_entry
            .get("digest")
            .and_then(|d| d.as_str())
            .context("Missing manifest digest in index.json")?;

        let manifest_hex = sha256_hex(manifest_digest)?;
        let manifest_blob = layout_dir.join("blobs").join("sha256").join(manifest_hex);
        let manifest_bytes = fs::read(&manifest_blob)
            .with_context(|| format!("Manifest blob not found: {}", manifest_blob.display()))?;

        verify_sha256(manifest_digest, &manifest_bytes)?;
        let manifest = ModelManifest::from_json_slice(&manifest_bytes)?;

        let config_hex = sha256_hex(&manifest.config.digest)?;
        let config_blob = layout_dir.join("blobs").join("sha256").join(config_hex);
        let config_bytes = fs::read(&config_blob)
            .with_context(|| format!("Config blob not found: {}", config_blob.display()))?;

        verify_sha256(&manifest.config.digest, &config_bytes)?;
        let config = ModelConfig::from_json_slice(&config_bytes)?;
        let total_size = manifest.layers.iter().map(|l| l.size).sum();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Ok(CachedModelMetadata {
            reference: format!("local://{}", layout_dir.display()),
            digest: manifest_digest.to_string(),
            name: config.name,
            format: config.format,
            architecture: config.architecture,
            precision: config.precision,
            context_length: config.context_length,
            files: config.files,
            total_size,
            created_at: now,
            path: layout_dir.to_path_buf(),
        })
    }

    /// Scans a directory to classify files, compute total size, and infer the model format.
    pub fn scan_directory_files(
        dir: &Path,
    ) -> Result<(Vec<ModelFileEntry>, u64, Option<ModelFormat>)> {
        let mut entries = Vec::new();
        let mut total_size: u64 = 0;
        let mut inferred_format: Option<ModelFormat> = None;

        if !dir.exists() {
            return Ok((entries, 0, None));
        }

        Self::scan_recursive(
            dir,
            dir,
            &mut entries,
            &mut total_size,
            &mut inferred_format,
        )?;

        Ok((entries, total_size, inferred_format))
    }

    fn scan_recursive(
        base: &Path,
        current: &Path,
        entries: &mut Vec<ModelFileEntry>,
        total_size: &mut u64,
        inferred_format: &mut Option<ModelFormat>,
    ) -> Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if file_name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                Self::scan_recursive(base, &path, entries, total_size, inferred_format)?;
            } else if path.is_file() {
                let size = entry.metadata()?.len();
                *total_size += size;

                let rel_path = path
                    .strip_prefix(base)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| file_name.clone());

                let lower = file_name.to_ascii_lowercase();
                let role = if lower.ends_with(".gguf") {
                    if inferred_format.is_none() {
                        *inferred_format = Some(ModelFormat::Gguf);
                    }
                    ModelFileRole::Weight
                } else if lower.ends_with(".safetensors") {
                    if inferred_format.is_none() {
                        *inferred_format = Some(ModelFormat::Safetensors);
                    }
                    ModelFileRole::Weight
                } else if lower.ends_with(".onnx") {
                    if inferred_format.is_none() {
                        *inferred_format = Some(ModelFormat::Onnx);
                    }
                    ModelFileRole::Weight
                } else if lower.ends_with(".bin") || lower.ends_with(".pt") {
                    if inferred_format.is_none() {
                        *inferred_format = Some(ModelFormat::PyTorch);
                    }
                    ModelFileRole::Weight
                } else if lower.ends_with(".uzu") {
                    if inferred_format.is_none() {
                        *inferred_format = Some(ModelFormat::Other("uzu".to_string()));
                    }
                    ModelFileRole::Weight
                } else if lower.contains("tokenizer") || lower.contains("vocab") {
                    ModelFileRole::Tokenizer
                } else if lower.ends_with(".json") || lower.contains("config") {
                    ModelFileRole::Config
                } else if lower.ends_with(".md")
                    || lower.contains("readme")
                    || lower.contains("license")
                {
                    ModelFileRole::Doc
                } else {
                    ModelFileRole::Weight
                };

                entries.push(ModelFileEntry {
                    path: rel_path,
                    role,
                    size,
                    digest: None,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modelpack::pack::{ModelPackOptions, ModelPacker};

    #[test]
    fn test_inspect_local_model_directory() {
        let temp = tempfile::tempdir().unwrap();
        let model_dir = temp.path().join("llama3");
        fs::create_dir_all(&model_dir).unwrap();

        fs::write(model_dir.join("model.Q4_K_M.gguf"), b"MOCK_GGUF_BYTES").unwrap();
        fs::write(
            model_dir.join("config.json"),
            b"{\"architectures\":[\"LlamaForCausalLM\"]}",
        )
        .unwrap();
        fs::write(model_dir.join("tokenizer.json"), b"{\"type\":\"BPE\"}").unwrap();

        let meta = ModelEngine::inspect_local_path(&model_dir).unwrap();
        assert_eq!(meta.name, "llama3");
        assert_eq!(meta.format, ModelFormat::Gguf);
        assert_eq!(meta.files.len(), 3);
        assert!(meta.total_size > 0);
    }

    #[test]
    fn test_inspect_and_list_cached_model() {
        let temp = tempfile::tempdir().unwrap();
        let cache_base = temp.path().join("cache");
        let model_dir = cache_base.join("models").join("sha256_1234567890abcdef");
        fs::create_dir_all(&model_dir).unwrap();

        let cached_meta = CachedModelMetadata {
            reference: "ghcr.io/org/llama-3.2-1b:latest".to_string(),
            digest: "sha256:1234567890abcdef".to_string(),
            name: "llama-3.2-1b".to_string(),
            format: ModelFormat::Safetensors,
            architecture: Some("llama".to_string()),
            precision: Some("fp16".to_string()),
            context_length: Some(8192),
            files: vec![ModelFileEntry {
                path: "model.safetensors".to_string(),
                role: ModelFileRole::Weight,
                size: 2048,
                digest: None,
            }],
            total_size: 2048,
            created_at: 1700000000,
            path: model_dir.clone(),
        };

        fs::write(
            model_dir.join(".modelpack.json"),
            serde_json::to_vec_pretty(&cached_meta).unwrap(),
        )
        .unwrap();

        // Test listing
        let list = ModelEngine::list_cached(&cache_base).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "llama-3.2-1b");
        assert_eq!(list[0].format, ModelFormat::Safetensors);

        // Test finding
        let found = ModelEngine::find_cached("llama-3.2-1b", &cache_base).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().precision.as_deref(), Some("fp16"));

        // Test inspecting
        let inspected =
            ModelEngine::inspect_model("ghcr.io/org/llama-3.2-1b:latest", &cache_base).unwrap();
        assert_eq!(inspected.name, "llama-3.2-1b");
    }

    #[test]
    fn test_inspect_packed_oci_layout() {
        let temp = tempfile::tempdir().unwrap();
        let src_dir = temp.path().join("src");
        let oci_dir = temp.path().join("packed_oci");
        fs::create_dir_all(&src_dir).unwrap();

        fs::write(src_dir.join("weights.onnx"), b"ONNX_MOCK_123456").unwrap();
        fs::write(src_dir.join("README.md"), b"# Model Card").unwrap();

        let opts = ModelPackOptions {
            source_dir: src_dir,
            output_dir: oci_dir.clone(),
            name: "onnx-bert".to_string(),
            format: ModelFormat::Onnx,
            version: Some("2.0.0".to_string()),
            architecture: Some("bert".to_string()),
            precision: Some("fp32".to_string()),
            context_length: Some(512),
            tag: Some("v2".to_string()),
        };

        ModelPacker::pack(&opts).unwrap();

        let inspected = ModelEngine::inspect_local_path(&oci_dir).unwrap();
        assert_eq!(inspected.name, "onnx-bert");
        assert_eq!(inspected.format, ModelFormat::Onnx);
        assert_eq!(inspected.architecture.as_deref(), Some("bert"));
        assert_eq!(inspected.precision.as_deref(), Some("fp32"));
        assert_eq!(inspected.context_length, Some(512));
    }
}
