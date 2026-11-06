use super::cache::{CacheEntry, CacheKind};
use super::digest::{sha256_hex, verify_sha256};
use super::manifest::{parse_oci_config_from_json, SingleManifest};
use crate::config::OciConfig;
use crate::rootfs::extract::extract_layer;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct OciIndex {
    #[serde(rename = "schemaVersion")]
    #[allow(dead_code)]
    schema_version: Option<u32>,
    manifests: Vec<OciIndexManifest>,
}

#[derive(Debug, Deserialize)]
struct OciIndexManifest {
    #[serde(rename = "mediaType")]
    #[allow(dead_code)]
    media_type: Option<String>,
    digest: String,
    annotations: Option<std::collections::HashMap<String, String>>,
}

pub struct OciLayout;

impl OciLayout {
    /// Loads an image directly from an offline OCI Image Layout directory on disk.
    /// Returns the path to the extracted rootfs and the OciConfig.
    pub fn load(
        layout_dir: &Path,
        tag: Option<&str>,
        cache_base: &Path,
    ) -> Result<(PathBuf, OciConfig)> {
        let layout_marker = layout_dir.join("oci-layout");
        if !layout_marker.exists() {
            bail!(
                "Invalid OCI Image Layout: 'oci-layout' marker file missing at {}",
                layout_dir.display()
            );
        }

        let index_file = layout_dir.join("index.json");
        if !index_file.exists() {
            bail!(
                "Invalid OCI Image Layout: 'index.json' missing at {}",
                layout_dir.display()
            );
        }

        let index_data = fs::read_to_string(&index_file)
            .with_context(|| format!("Failed to read index.json at {}", index_file.display()))?;
        let index: OciIndex =
            serde_json::from_str(&index_data).context("Failed to parse OCI layout index.json")?;

        if index.manifests.is_empty() {
            bail!("OCI layout index.json has no manifests");
        }

        // Match manifest by tag annotation, or use the first manifest
        let manifest_desc = if let Some(target_tag) = tag {
            index
                .manifests
                .iter()
                .find(|m| {
                    m.annotations.as_ref().is_some_and(|a| {
                        a.get("org.opencontainers.image.ref.name")
                            .is_some_and(|v| v == target_tag)
                    })
                })
                .with_context(|| format!("OCI layout tag not found: {target_tag}"))?
        } else {
            &index.manifests[0]
        };

        let manifest_hash = sha256_hex(&manifest_desc.digest)?;

        let manifest_blob_path = layout_dir.join("blobs").join("sha256").join(manifest_hash);

        if !manifest_blob_path.exists() {
            bail!(
                "Manifest blob not found at {}",
                manifest_blob_path.display()
            );
        }

        let manifest_bytes = fs::read(&manifest_blob_path)?;
        verify_sha256(&manifest_desc.digest, &manifest_bytes)?;
        let manifest: SingleManifest = serde_json::from_slice(&manifest_bytes)
            .context("Failed to parse manifest JSON in OCI layout")?;

        let cache = CacheEntry::acquire(cache_base, CacheKind::Image, &manifest_desc.digest)?;
        let rootfs_dir = cache.path().to_path_buf();
        if let Some(cfg) = cache.read_metadata::<OciConfig>()? {
            tracing::info!(
                "RootFS cache hit for OCI layout manifest {}",
                manifest_desc.digest
            );
            return Ok((rootfs_dir, cfg));
        }

        // Read config blob
        let config_hash = sha256_hex(&manifest.config.digest)?;
        let config_blob_path = layout_dir.join("blobs").join("sha256").join(config_hash);
        let config_bytes = fs::read(&config_blob_path).context("Missing OCI config blob")?;
        verify_sha256(&manifest.config.digest, &config_bytes)?;
        let full_config: serde_json::Value = serde_json::from_slice(&config_bytes)?;
        let oci_config = parse_oci_config_from_json(&full_config);

        let transaction = cache.stage()?;
        let staging_rootfs = transaction.path();

        for (i, layer) in manifest.layers.iter().enumerate() {
            let layer_hash = sha256_hex(&layer.digest)?;
            let layer_blob_path = layout_dir.join("blobs").join("sha256").join(layer_hash);

            if !layer_blob_path.exists() {
                bail!("Layer blob not found: {}", layer_blob_path.display());
            }

            tracing::info!(
                "Extracting OCI layout layer [{}/{}] from blobs/sha256/{}...",
                i + 1,
                manifest.layers.len(),
                layer_hash
            );

            let layer_bytes = fs::read(&layer_blob_path)?;
            verify_sha256(&layer.digest, &layer_bytes)?;
            let cursor = std::io::Cursor::new(layer_bytes);
            let is_gz = layer
                .media_type
                .as_ref()
                .is_none_or(|m| m.contains("gzip") || m.ends_with(".tar+gzip"));

            extract_layer(cursor, staging_rootfs, is_gz)?;
        }

        transaction.commit(&oci_config)?;

        Ok((rootfs_dir, oci_config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use tempfile::tempdir;

    #[test]
    fn test_load_oci_layout_basic() {
        let dir = tempdir().unwrap();
        let layout_dir = dir.path();

        // 1. Create oci-layout
        fs::write(
            layout_dir.join("oci-layout"),
            r#"{"imageLayoutVersion": "1.0.0"}"#,
        )
        .unwrap();

        let config = br#"{"config":{"Cmd":["echo","offline layout works"]}}"#;
        let config_hash = format!("{:x}", sha2::Sha256::digest(config));
        let manifest = serde_json::to_vec(&serde_json::json!({
            "config": {"digest": format!("sha256:{config_hash}"), "size": config.len()},
            "layers": []
        }))
        .unwrap();
        let manifest_hash = format!("{:x}", sha2::Sha256::digest(&manifest));
        let blobs_dir = layout_dir.join("blobs/sha256");
        fs::create_dir_all(&blobs_dir).unwrap();
        fs::write(blobs_dir.join(config_hash), config).unwrap();
        fs::write(blobs_dir.join(&manifest_hash), manifest).unwrap();
        fs::write(
            layout_dir.join("index.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "manifests": [{"digest": format!("sha256:{manifest_hash}"),
                    "annotations": {"org.opencontainers.image.ref.name": "test-v1"}}]
            }))
            .unwrap(),
        )
        .unwrap();

        let cache_dir = tempdir().unwrap();
        let (rootfs, config) =
            OciLayout::load(layout_dir, Some("test-v1"), cache_dir.path()).unwrap();

        assert!(rootfs.exists());
        assert_eq!(config.cmd, vec!["echo", "offline layout works"]);
    }

    #[test]
    fn failed_layout_extraction_leaves_no_partial_cache() {
        let temp = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let blobs = temp.path().join("blobs/sha256");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(temp.path().join("oci-layout"), b"{}").unwrap();
        let config = b"{}";
        let config_hash = format!("{:x}", sha2::Sha256::digest(config));
        fs::write(blobs.join(&config_hash), config).unwrap();
        let manifest = serde_json::to_vec(&serde_json::json!({
            "config": {"digest": format!("sha256:{config_hash}"), "size": 2},
            "layers": [{"digest": format!("sha256:{}", "b".repeat(64)), "size": 1}]
        }))
        .unwrap();
        let manifest_hash = format!("{:x}", sha2::Sha256::digest(&manifest));
        fs::write(blobs.join(&manifest_hash), &manifest).unwrap();
        fs::write(
            temp.path().join("index.json"),
            serde_json::to_vec(&serde_json::json!({
                "manifests": [{"digest": format!("sha256:{manifest_hash}")}]
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(OciLayout::load(temp.path(), None, cache.path()).is_err());
        assert_eq!(
            fs::read_dir(cache.path().join("staging")).unwrap().count(),
            0
        );
        assert!(!cache
            .path()
            .join("rootfs")
            .join(format!("sha256_{manifest_hash}"))
            .exists());
        assert!(!cache
            .path()
            .join("configs")
            .join(format!("sha256_{manifest_hash}.json"))
            .exists());
    }

    #[test]
    fn layout_rejects_untrusted_manifest_paths() {
        let temp = tempdir().unwrap();
        let cache = tempdir().unwrap();
        fs::write(temp.path().join("oci-layout"), b"{}").unwrap();
        for digest in [
            "sha256:../../outside",
            "sha256:/tmp/outside",
            "sha512:abcdef",
        ] {
            fs::write(
                temp.path().join("index.json"),
                serde_json::to_vec(&serde_json::json!({"manifests": [{"digest": digest}]}))
                    .unwrap(),
            )
            .unwrap();
            let error = OciLayout::load(temp.path(), None, cache.path()).unwrap_err();
            assert!(error.to_string().contains("digest"));
        }
        assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 0);
    }

    #[test]
    fn layout_rejects_tampered_manifest() {
        let temp = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let hex = "a".repeat(64);
        fs::write(temp.path().join("oci-layout"), b"{}").unwrap();
        fs::create_dir_all(temp.path().join("blobs/sha256")).unwrap();
        fs::write(temp.path().join("blobs/sha256").join(&hex), b"{}").unwrap();
        fs::write(
            temp.path().join("index.json"),
            serde_json::to_vec(
                &serde_json::json!({"manifests": [{"digest": format!("sha256:{hex}")}]}),
            )
            .unwrap(),
        )
        .unwrap();
        let error = OciLayout::load(temp.path(), None, cache.path()).unwrap_err();
        assert!(error.to_string().contains("digest mismatch"));
    }
}
