// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CNCF ModelPack Specification types and media type descriptors.
//!
//! Complies with `https://github.com/modelpack/model-spec` for packaging,
//! distributing, and executing AI/ML models as OCI artifacts.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root OCI Image Manifest media type.
pub const OCI_IMAGE_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// CNCF ModelPack Artifact Type for OCI Image Manifests.
pub const MODELPACK_MANIFEST_ARTIFACT_TYPE: &str = "application/vnd.cncf.model.manifest.v1+json";

/// Alternative / Legacy ModelPack Artifact Type.
pub const MODELPACK_LEGACY_ARTIFACT_TYPE: &str = "application/vnd.modelpack";

/// CNCF ModelPack Configuration Descriptor Media Type.
pub const MODELPACK_CONFIG_MEDIA_TYPE: &str = "application/vnd.cncf.model.config.v1+json";

/// CNCF ModelPack Weight Layer Media Type.
pub const MODELPACK_WEIGHT_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.model.weight.v1.tar";

/// CNCF ModelPack Model Architecture / Weight Configuration Layer Media Type.
pub const MODELPACK_CONFIG_LAYER_MEDIA_TYPE: &str =
    "application/vnd.cncf.model.weight.config.v1.tar";

/// CNCF ModelPack Tokenizer Layer Media Type.
pub const MODELPACK_TOKENIZER_LAYER_MEDIA_TYPE: &str =
    "application/vnd.cncf.model.tokenizer.v1.tar";

/// CNCF ModelPack Documentation & Model Card Layer Media Type.
pub const MODELPACK_DOC_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.model.doc.v1.tar";

/// Recognized AI/ML model binary weight formats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFormat {
    Gguf,
    Safetensors,
    Onnx,
    PyTorch,
    Mlx,
    #[serde(untagged)]
    Other(String),
}

impl std::fmt::Display for ModelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gguf => write!(f, "gguf"),
            Self::Safetensors => write!(f, "safetensors"),
            Self::Onnx => write!(f, "onnx"),
            Self::PyTorch => write!(f, "pytorch"),
            Self::Mlx => write!(f, "mlx"),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

impl std::str::FromStr for ModelFormat {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "gguf" => Ok(Self::Gguf),
            "safetensors" => Ok(Self::Safetensors),
            "onnx" => Ok(Self::Onnx),
            "pytorch" | "pt" | "bin" => Ok(Self::PyTorch),
            "mlx" => Ok(Self::Mlx),
            _ => Ok(Self::Other(s.to_string())),
        }
    }
}

/// Role of a file or layer packaged within a ModelPack artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFileRole {
    Weight,
    Config,
    Tokenizer,
    Doc,
}

impl ModelFileRole {
    pub fn layer_media_type(&self) -> &'static str {
        match self {
            Self::Weight => MODELPACK_WEIGHT_LAYER_MEDIA_TYPE,
            Self::Config => MODELPACK_CONFIG_LAYER_MEDIA_TYPE,
            Self::Tokenizer => MODELPACK_TOKENIZER_LAYER_MEDIA_TYPE,
            Self::Doc => MODELPACK_DOC_LAYER_MEDIA_TYPE,
        }
    }
}

/// Metadata entry for a single file contained within the ModelPack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFileEntry {
    pub path: String,
    pub role: ModelFileRole,
    pub size: u64,
    #[serde(default)]
    pub digest: Option<String>,
}

/// CNCF ModelPack Model Configuration Schema (`application/vnd.cncf.model.config.v1+json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Model name or identifier (e.g. `llama-3.2-1b-instruct`, `mistral-7b-v0.3`).
    pub name: String,
    /// Semantic version of the model release.
    #[serde(default)]
    pub version: Option<String>,
    /// Model weight serialization format.
    pub format: ModelFormat,
    /// Base neural network architecture (e.g. `llama`, `mistral`, `qwen`, `gemma`).
    #[serde(default)]
    pub architecture: Option<String>,
    /// Quantization or precision level (e.g. `q4_k_m`, `q8_0`, `fp16`, `bf16`).
    #[serde(default)]
    pub precision: Option<String>,
    /// Maximum context length supported by the model (in tokens).
    #[serde(default)]
    pub context_length: Option<u64>,
    /// File manifest detailing packaged layers and roles.
    #[serde(default)]
    pub files: Vec<ModelFileEntry>,
    /// Arbitrary key-value metadata (license, author, source repository, chat template).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl ModelConfig {
    pub fn new(name: impl Into<String>, format: ModelFormat) -> Self {
        Self {
            name: name.into(),
            version: None,
            format,
            architecture: None,
            precision: None,
            context_length: None,
            files: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_architecture(mut self, arch: impl Into<String>) -> Self {
        self.architecture = Some(arch.into());
        self
    }

    pub fn with_precision(mut self, precision: impl Into<String>) -> Self {
        self.precision = Some(precision.into());
        self
    }

    pub fn with_context_length(mut self, len: u64) -> Self {
        self.context_length = Some(len);
        self
    }

    pub fn add_file(mut self, entry: ModelFileEntry) -> Self {
        self.files.push(entry);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), val.into());
        self
    }

    /// Serializes this model configuration to pretty JSON bytes.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(self).map_err(Into::into)
    }

    /// Deserializes model configuration from JSON bytes.
    pub fn from_json_slice(data: &[u8]) -> Result<Self> {
        serde_json::from_slice(data).map_err(Into::into)
    }
}

/// Generic OCI content descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

/// CNCF ModelPack OCI Image Manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    #[serde(rename = "artifactType")]
    pub artifact_type: String,
    pub config: ModelDescriptor,
    pub layers: Vec<ModelDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

impl ModelManifest {
    pub fn new(config: ModelDescriptor, layers: Vec<ModelDescriptor>) -> Self {
        Self {
            schema_version: 2,
            media_type: OCI_IMAGE_MANIFEST_MEDIA_TYPE.to_string(),
            artifact_type: MODELPACK_MANIFEST_ARTIFACT_TYPE.to_string(),
            config,
            layers,
            annotations: None,
        }
    }

    pub fn is_modelpack(&self) -> bool {
        self.artifact_type == MODELPACK_MANIFEST_ARTIFACT_TYPE
            || self.artifact_type == MODELPACK_LEGACY_ARTIFACT_TYPE
            || self.config.media_type == MODELPACK_CONFIG_MEDIA_TYPE
            || self
                .layers
                .iter()
                .any(|l| l.media_type.contains("cncf.model") || l.media_type.contains("modelpack"))
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(self).map_err(Into::into)
    }

    pub fn from_json_slice(data: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(data)?;
        if !manifest.is_modelpack() {
            bail!(
                "Manifest is not a valid CNCF ModelPack artifact (artifactType: '{}')",
                manifest.artifact_type
            );
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_config_roundtrip() {
        let mut config = ModelConfig::new("llama-3.2-1b", ModelFormat::Gguf)
            .with_architecture("llama")
            .with_precision("q4_k_m")
            .with_context_length(131072)
            .with_metadata("license", "llama3.2")
            .with_metadata("author", "Meta");

        config.files.push(ModelFileEntry {
            path: "llama-3.2-1b-instruct.Q4_K_M.gguf".to_string(),
            role: ModelFileRole::Weight,
            size: 1320000000,
            digest: Some("sha256:11223344".to_string()),
        });

        let json = config.to_json_bytes().unwrap();
        let loaded = ModelConfig::from_json_slice(&json).unwrap();

        assert_eq!(loaded.name, "llama-3.2-1b");
        assert_eq!(loaded.format, ModelFormat::Gguf);
        assert_eq!(loaded.architecture.as_deref(), Some("llama"));
        assert_eq!(loaded.precision.as_deref(), Some("q4_k_m"));
        assert_eq!(loaded.context_length, Some(131072));
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(
            loaded.metadata.get("author").map(|s| s.as_str()),
            Some("Meta")
        );
    }

    #[test]
    fn test_model_manifest_validation() {
        let config_desc = ModelDescriptor {
            media_type: MODELPACK_CONFIG_MEDIA_TYPE.to_string(),
            digest: "sha256:aaaa".to_string(),
            size: 300,
            annotations: None,
        };

        let weight_desc = ModelDescriptor {
            media_type: MODELPACK_WEIGHT_LAYER_MEDIA_TYPE.to_string(),
            digest: "sha256:bbbb".to_string(),
            size: 1048576,
            annotations: None,
        };

        let manifest = ModelManifest::new(config_desc, vec![weight_desc]);
        assert!(manifest.is_modelpack());

        let json = manifest.to_json_bytes().unwrap();
        let parsed = ModelManifest::from_json_slice(&json).unwrap();
        assert_eq!(parsed.artifact_type, MODELPACK_MANIFEST_ARTIFACT_TYPE);
        assert_eq!(parsed.layers.len(), 1);
    }
}
