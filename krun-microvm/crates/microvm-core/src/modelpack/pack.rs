// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Packaging engine for creating CNCF ModelPack OCI artifacts.

use super::spec::*;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tar::{Builder, Header};

/// Options for packaging local model weights and auxiliary files into a ModelPack.
#[derive(Debug, Clone)]
pub struct ModelPackOptions {
    pub source_dir: PathBuf,
    pub output_dir: PathBuf,
    pub name: String,
    pub format: ModelFormat,
    pub version: Option<String>,
    pub architecture: Option<String>,
    pub precision: Option<String>,
    pub context_length: Option<u64>,
    pub tag: Option<String>,
}

pub struct ModelPacker;

impl ModelPacker {
    /// Packages a local model directory into a standard CNCF ModelPack OCI layout.
    pub fn pack(options: &ModelPackOptions) -> Result<ModelManifest> {
        if !options.source_dir.exists() {
            bail!(
                "Source model directory does not exist: {}",
                options.source_dir.display()
            );
        }

        let blobs_dir = options.output_dir.join("blobs").join("sha256");
        fs::create_dir_all(&blobs_dir).with_context(|| {
            format!("Failed to create blobs directory: {}", blobs_dir.display())
        })?;

        // 1. Scan source directory for files and classify roles
        let mut weight_files = Vec::new();
        let mut config_files = Vec::new();
        let mut tokenizer_files = Vec::new();
        let mut doc_files = Vec::new();
        let mut all_file_entries = Vec::new();

        for entry in fs::read_dir(&options.source_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                let lower = file_name.to_ascii_lowercase();
                let size = entry.metadata()?.len();

                let role = if lower.ends_with(".gguf")
                    || lower.ends_with(".safetensors")
                    || lower.ends_with(".onnx")
                    || lower.ends_with(".bin")
                    || lower.ends_with(".pt")
                {
                    weight_files.push((path.clone(), file_name.clone()));
                    ModelFileRole::Weight
                } else if lower.contains("tokenizer") || lower.contains("vocab") {
                    tokenizer_files.push((path.clone(), file_name.clone()));
                    ModelFileRole::Tokenizer
                } else if lower.ends_with(".json") || lower.contains("config") {
                    config_files.push((path.clone(), file_name.clone()));
                    ModelFileRole::Config
                } else if lower.ends_with(".md")
                    || lower.contains("license")
                    || lower.contains("readme")
                {
                    doc_files.push((path.clone(), file_name.clone()));
                    ModelFileRole::Doc
                } else {
                    weight_files.push((path.clone(), file_name.clone()));
                    ModelFileRole::Weight
                };

                all_file_entries.push(ModelFileEntry {
                    path: file_name,
                    role,
                    size,
                    digest: None,
                });
            }
        }

        if weight_files.is_empty() && all_file_entries.is_empty() {
            bail!(
                "No model files found in source directory: {}",
                options.source_dir.display()
            );
        }

        let mut layer_descriptors = Vec::new();

        // 2. Build layer archives for each role
        if !weight_files.is_empty() {
            let desc = Self::create_tar_layer(
                &blobs_dir,
                &weight_files,
                MODELPACK_WEIGHT_LAYER_MEDIA_TYPE,
            )?;
            layer_descriptors.push(desc);
        }

        if !config_files.is_empty() {
            let desc = Self::create_tar_layer(
                &blobs_dir,
                &config_files,
                MODELPACK_CONFIG_LAYER_MEDIA_TYPE,
            )?;
            layer_descriptors.push(desc);
        }

        if !tokenizer_files.is_empty() {
            let desc = Self::create_tar_layer(
                &blobs_dir,
                &tokenizer_files,
                MODELPACK_TOKENIZER_LAYER_MEDIA_TYPE,
            )?;
            layer_descriptors.push(desc);
        }

        if !doc_files.is_empty() {
            let desc =
                Self::create_tar_layer(&blobs_dir, &doc_files, MODELPACK_DOC_LAYER_MEDIA_TYPE)?;
            layer_descriptors.push(desc);
        }

        // 3. Build ModelConfig JSON
        let mut model_config = ModelConfig::new(options.name.clone(), options.format.clone());
        model_config.version = options.version.clone();
        model_config.architecture = options.architecture.clone();
        model_config.precision = options.precision.clone();
        model_config.context_length = options.context_length;
        model_config.files = all_file_entries;

        let config_bytes = model_config.to_json_bytes()?;
        let (config_digest, config_hex) = Self::compute_sha256(&config_bytes);
        let config_path = blobs_dir.join(&config_hex);
        fs::write(&config_path, &config_bytes)?;

        let config_desc = ModelDescriptor {
            media_type: MODELPACK_CONFIG_MEDIA_TYPE.to_string(),
            digest: config_digest,
            size: config_bytes.len() as u64,
            annotations: None,
        };

        // 4. Build ModelManifest JSON
        let manifest = ModelManifest::new(config_desc, layer_descriptors);
        let manifest_bytes = manifest.to_json_bytes()?;
        let (manifest_digest, manifest_hex) = Self::compute_sha256(&manifest_bytes);
        let manifest_path = blobs_dir.join(&manifest_hex);
        fs::write(&manifest_path, &manifest_bytes)?;

        // 5. Generate OCI layout and index.json
        let oci_layout_path = options.output_dir.join("oci-layout");
        fs::write(&oci_layout_path, r#"{"imageLayoutVersion": "1.0.0"}"#)?;

        let index_json = serde_json::json!({
            "schemaVersion": 2,
            "manifests": [
                {
                    "mediaType": OCI_IMAGE_MANIFEST_MEDIA_TYPE,
                    "artifactType": MODELPACK_MANIFEST_ARTIFACT_TYPE,
                    "digest": manifest_digest,
                    "size": manifest_bytes.len(),
                    "annotations": {
                        "org.opencontainers.image.ref.name": options.tag.as_deref().unwrap_or("latest")
                    }
                }
            ]
        });
        fs::write(
            options.output_dir.join("index.json"),
            serde_json::to_vec_pretty(&index_json)?,
        )?;

        tracing::info!(
            "Successfully packaged ModelPack '{}' -> {}",
            options.name,
            manifest_digest
        );

        Ok(manifest)
    }

    fn create_tar_layer(
        blobs_dir: &Path,
        files: &[(PathBuf, String)],
        media_type: &str,
    ) -> Result<ModelDescriptor> {
        let mut tar_buffer = Vec::new();
        {
            let mut tar = Builder::new(&mut tar_buffer);
            for (path, file_name) in files {
                let mut f = File::open(path)?;
                let metadata = f.metadata()?;
                let mut header = Header::new_gnu();
                header.set_size(metadata.len());
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, file_name, &mut f)?;
            }
            tar.finish()?;
        }

        let (digest, hex_str) = Self::compute_sha256(&tar_buffer);
        let blob_path = blobs_dir.join(&hex_str);
        fs::write(&blob_path, &tar_buffer)?;

        Ok(ModelDescriptor {
            media_type: media_type.to_string(),
            digest,
            size: tar_buffer.len() as u64,
            annotations: None,
        })
    }

    fn compute_sha256(data: &[u8]) -> (String, String) {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hex_str = hex::encode(hasher.finalize());
        (format!("sha256:{}", hex_str), hex_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_packer_gguf_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let src_dir = temp.path().join("src");
        let out_dir = temp.path().join("modelpack_oci");
        fs::create_dir_all(&src_dir).unwrap();

        // Create mock model files
        fs::write(
            src_dir.join("tiny-llama.Q4_K_M.gguf"),
            b"GGUF_MOCK_WEIGHTS_12345",
        )
        .unwrap();
        fs::write(
            src_dir.join("config.json"),
            b"{\"architectures\":[\"LlamaForCausalLM\"]}",
        )
        .unwrap();
        fs::write(
            src_dir.join("tokenizer.json"),
            b"{\"model\":{\"type\":\"BPE\"}}",
        )
        .unwrap();
        fs::write(src_dir.join("README.md"), b"# Tiny Llama Model Card").unwrap();

        let options = ModelPackOptions {
            source_dir: src_dir,
            output_dir: out_dir.clone(),
            name: "tiny-llama".to_string(),
            format: ModelFormat::Gguf,
            version: Some("1.0.0".to_string()),
            architecture: Some("llama".to_string()),
            precision: Some("q4_k_m".to_string()),
            context_length: Some(4096),
            tag: Some("v1".to_string()),
        };

        let manifest = ModelPacker::pack(&options).unwrap();
        assert!(manifest.is_modelpack());
        assert_eq!(manifest.layers.len(), 4); // weight, config, tokenizer, doc

        // Verify layout
        assert!(out_dir.join("oci-layout").exists());
        assert!(out_dir.join("index.json").exists());
        assert!(out_dir.join("blobs").join("sha256").exists());
    }
}
