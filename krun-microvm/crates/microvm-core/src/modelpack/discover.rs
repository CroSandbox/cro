// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Host-wide local model discovery across common AI ecosystems.

use super::spec::ModelFormat;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Origin ecosystem where a model was discovered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelSource {
    HuggingFace,
    LmStudio,
    Ollama,
    KrunCache,
    LocalDir,
}

impl std::fmt::Display for ModelSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HuggingFace => write!(f, "Hugging Face"),
            Self::LmStudio => write!(f, "LM Studio"),
            Self::Ollama => write!(f, "Ollama"),
            Self::KrunCache => write!(f, "ModelPack Cache"),
            Self::LocalDir => write!(f, "Local Directory"),
        }
    }
}

/// A discovered local model ready for inference serving.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredModel {
    pub name: String,
    pub path: PathBuf,
    pub format: ModelFormat,
    pub size_bytes: u64,
    pub source: ModelSource,
}

pub struct ModelDiscovery;

impl ModelDiscovery {
    /// Discovers all local models across standard host cache paths and optional custom directories.
    pub fn discover(data_dir: &Path, custom_dirs: &[PathBuf]) -> Vec<DiscoveredModel> {
        let mut results = Vec::new();
        let mut seen_paths = HashSet::new();

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let home_path = PathBuf::from(home);

        // 1. cro ModelPack cache directory
        let krun_models = data_dir.join("models");
        if krun_models.exists() {
            Self::scan_krun_cache(&krun_models, &mut results, &mut seen_paths);
        }

        // 2. Hugging Face Hub cache (~/.cache/huggingface/hub/)
        let hf_hub = home_path.join(".cache/huggingface/hub");
        if hf_hub.exists() {
            Self::scan_directory_for_models(
                &hf_hub,
                ModelSource::HuggingFace,
                &mut results,
                &mut seen_paths,
                4,
            );
        }

        // 3. LM Studio model directories (~/.lmstudio/models, ~/.cache/lm-studio/models)
        for lm_path in &[
            home_path.join(".lmstudio/models"),
            home_path.join(".cache/lm-studio/models"),
        ] {
            if lm_path.exists() {
                Self::scan_directory_for_models(
                    lm_path,
                    ModelSource::LmStudio,
                    &mut results,
                    &mut seen_paths,
                    4,
                );
            }
        }

        // 4. Ollama models directory (~/.ollama/models)
        let ollama_path = home_path.join(".ollama/models/blobs");
        if ollama_path.exists() {
            Self::scan_directory_for_models(
                &ollama_path,
                ModelSource::Ollama,
                &mut results,
                &mut seen_paths,
                2,
            );
        }

        // 5. Standard user models directory (~/models)
        let user_models = home_path.join("models");
        if user_models.exists() {
            Self::scan_directory_for_models(
                &user_models,
                ModelSource::LocalDir,
                &mut results,
                &mut seen_paths,
                3,
            );
        }

        // 6. User-provided custom directories
        for custom in custom_dirs {
            if custom.exists() {
                Self::scan_directory_for_models(
                    custom,
                    ModelSource::LocalDir,
                    &mut results,
                    &mut seen_paths,
                    4,
                );
            }
        }

        // Sort: largest size first
        results.sort_by_key(|a| std::cmp::Reverse(a.size_bytes));
        results
    }

    /// Scans the cro ModelPack cache for models containing .modelpack.json.
    fn scan_krun_cache(
        models_dir: &Path,
        results: &mut Vec<DiscoveredModel>,
        seen_paths: &mut HashSet<PathBuf>,
    ) {
        if let Ok(entries) = fs::read_dir(models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let meta_file = path.join(".modelpack.json");
                    if meta_file.exists() {
                        if let Ok(data) = fs::read(&meta_file) {
                            if let Ok(meta) =
                                serde_json::from_slice::<super::engine::CachedModelMetadata>(&data)
                            {
                                if seen_paths.insert(path.clone()) {
                                    results.push(DiscoveredModel {
                                        name: meta.name,
                                        path: path.clone(),
                                        format: meta.format,
                                        size_bytes: meta.total_size,
                                        source: ModelSource::KrunCache,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Recursively scans a filesystem directory up to `max_depth` levels looking for model files.
    pub fn scan_directory_for_models(
        dir: &Path,
        source: ModelSource,
        results: &mut Vec<DiscoveredModel>,
        seen_paths: &mut HashSet<PathBuf>,
        max_depth: usize,
    ) {
        if max_depth == 0 {
            return;
        }

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();

                if file_name.starts_with('.') {
                    continue;
                }

                if path.is_dir() {
                    // Check if directory itself is a model package (e.g. has config.json and tokenizer.json)
                    if path.join("config.json").exists()
                        && (path.join("model.safetensors").exists()
                            || path.join("pytorch_model.bin").exists())
                    {
                        if seen_paths.insert(path.clone()) {
                            let size = Self::directory_size(&path);
                            results.push(DiscoveredModel {
                                name: file_name,
                                path: path.clone(),
                                format: ModelFormat::Safetensors,
                                size_bytes: size,
                                source: source.clone(),
                            });
                        }
                        continue;
                    }

                    Self::scan_directory_for_models(
                        &path,
                        source.clone(),
                        results,
                        seen_paths,
                        max_depth - 1,
                    );
                } else if path.is_file() {
                    if let Some(format) = Self::detect_file_format(&path) {
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        // Skip very small files that cannot be model weights (< 5 MB)
                        if size > 5 * 1024 * 1024 && seen_paths.insert(path.clone()) {
                            results.push(DiscoveredModel {
                                name: file_name,
                                path: path.clone(),
                                format,
                                size_bytes: size,
                                source: source.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    /// Detects model format based on file extension and magic headers.
    pub fn detect_file_format(path: &Path) -> Option<ModelFormat> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ext.as_str() {
            "gguf" => Some(ModelFormat::Gguf),
            "safetensors" => Some(ModelFormat::Safetensors),
            "bin" | "pt" | "pth" => Some(ModelFormat::PyTorch),
            "onnx" => Some(ModelFormat::Onnx),
            _ => {
                // If no extension or unknown (common in Ollama blobs), inspect file magic
                if let Ok(mut file) = fs::File::open(path) {
                    use std::io::Read;
                    let mut magic = [0u8; 4];
                    if file.read_exact(&mut magic).is_ok() && &magic == b"GGUF" {
                        return Some(ModelFormat::Gguf);
                    }
                }
                None
            }
        }
    }

    fn directory_size(path: &Path) -> u64 {
        let mut total = 0;
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total += meta.len();
                    } else if meta.is_dir() {
                        total += Self::directory_size(&entry.path());
                    }
                }
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_detect_file_format() {
        assert_eq!(
            ModelDiscovery::detect_file_format(Path::new("llama.gguf")),
            Some(ModelFormat::Gguf)
        );
        assert_eq!(
            ModelDiscovery::detect_file_format(Path::new("model.safetensors")),
            Some(ModelFormat::Safetensors)
        );
        assert_eq!(
            ModelDiscovery::detect_file_format(Path::new("weights.bin")),
            Some(ModelFormat::PyTorch)
        );
        assert_eq!(
            ModelDiscovery::detect_file_format(Path::new("readme.txt")),
            None
        );
    }

    #[test]
    fn test_scan_directory_for_models() {
        let dir = tempdir().unwrap();
        let gguf_path = dir.path().join("test_model.gguf");
        // Create 6 MB mock file
        let data = vec![0u8; 6 * 1024 * 1024];
        fs::write(&gguf_path, data).unwrap();

        let mut results = Vec::new();
        let mut seen = HashSet::new();

        ModelDiscovery::scan_directory_for_models(
            dir.path(),
            ModelSource::LocalDir,
            &mut results,
            &mut seen,
            2,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "test_model.gguf");
        assert_eq!(results[0].format, ModelFormat::Gguf);
        assert_eq!(results[0].source, ModelSource::LocalDir);
        assert_eq!(results[0].size_bytes, 6 * 1024 * 1024);
    }
}
