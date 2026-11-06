use super::cache::{CacheEntry, CacheKind};
use super::digest::{sha256_hex, verify_sha256};
use super::manifest::{parse_oci_config_from_json, ManifestIndex, SingleManifest};
use super::reference::ImageReference;
use crate::config::OciConfig;
use crate::rootfs::extract::extract_layer;
use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, WWW_AUTHENTICATE};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct OciClient {
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct DockerTokenResponse {
    token: Option<String>,
    access_token: Option<String>,
}

impl Default for OciClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OciClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("krun-microvm/0.1.0")
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    /// Pulls an arbitrary OCI artifact (e.g. model weights, dataset, toolchain) into cache_base/artifacts/<digest>.
    pub async fn pull_artifact(
        &self,
        reference: &ImageReference,
        cache_base: &Path,
    ) -> Result<PathBuf> {
        super::artifact::pull_artifact_impl(self, reference, cache_base).await
    }

    /// Pulls a CNCF ModelPack artifact into cache_base/models/<digest>.
    pub async fn pull_model(
        &self,
        reference: &ImageReference,
        cache_base: &Path,
    ) -> Result<PathBuf> {
        crate::modelpack::ModelEngine::pull_model(self, reference, cache_base).await
    }

    /// Obtains an anonymous bearer token for pulling from any OCI compliant registry.
    pub(crate) async fn get_token(&self, reference: &ImageReference) -> Result<Option<String>> {
        // 1. Probe the registry /v2/ endpoint to receive standard Www-Authenticate challenge
        let ping_url = format!("https://{}/v2/", reference.registry);
        if let Ok(resp) = self.http.get(&ping_url).send().await {
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                if let Some(auth_hdr) = resp.headers().get(WWW_AUTHENTICATE) {
                    if let Ok(hdr_str) = auth_hdr.to_str() {
                        if let Ok(Some(tok)) =
                            self.token_from_www_authenticate(hdr_str, reference).await
                        {
                            return Ok(Some(tok));
                        }
                    }
                }
            }
        }

        // 2. Direct Docker Hub fallback if probe didn't return challenge
        if reference.registry == "registry-1.docker.io" {
            let url = format!(
                "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
                reference.repository
            );
            if let Ok(resp) = self.http.get(&url).send().await {
                if resp.status().is_success() {
                    let body: DockerTokenResponse = resp.json().await?;
                    return Ok(body.token.or(body.access_token));
                }
            }
        }

        // 2b. Direct GHCR fallback if probe didn't return challenge
        if reference.registry == "ghcr.io" {
            let url = format!(
                "https://ghcr.io/token?service=ghcr.io&scope=repository:{}:pull",
                reference.repository
            );
            if let Ok(resp) = self.http.get(&url).send().await {
                if resp.status().is_success() {
                    let body: DockerTokenResponse = resp.json().await?;
                    return Ok(body.token.or(body.access_token));
                }
            }
        }

        Ok(None)
    }

    /// Parses RFC 7235 Www-Authenticate header and fetches a bearer token from the indicated realm.
    async fn token_from_www_authenticate(
        &self,
        auth_hdr: &str,
        reference: &ImageReference,
    ) -> Result<Option<String>> {
        let auth_hdr = auth_hdr.trim();
        if !auth_hdr.starts_with("Bearer ") && !auth_hdr.starts_with("bearer ") {
            return Ok(None);
        }
        let params_part = &auth_hdr[7..];
        let mut realm = None;
        let mut service = None;
        let mut scope = None;

        for part in params_part.split(',') {
            if let Some((k, v)) = part.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"');
                match key {
                    "realm" => realm = Some(val.to_string()),
                    "service" => service = Some(val.to_string()),
                    "scope" => scope = Some(val.to_string()),
                    _ => {}
                }
            }
        }

        let realm_url = match realm {
            Some(r) => r,
            None => return Ok(None),
        };

        let mut req = self.http.get(&realm_url);
        if let Some(s) = service {
            req = req.query(&[("service", s)]);
        }
        let final_scope =
            scope.unwrap_or_else(|| format!("repository:{}:pull", reference.repository));
        req = req.query(&[("scope", final_scope)]);

        let resp = req.send().await?;
        if resp.status().is_success() {
            let body: DockerTokenResponse = resp.json().await?;
            return Ok(body.token.or(body.access_token));
        }

        Ok(None)
    }

    fn auth_headers(&self, token: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(t) = token {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", t)).unwrap(),
            );
        }
        headers
    }

    /// Pulls the image and returns the path to the cached, extracted rootfs along with its OCI configuration.
    pub async fn pull_and_unpack(
        &self,
        reference: &ImageReference,
        cache_base: &Path,
    ) -> Result<(PathBuf, OciConfig)> {
        // 0. Try local Docker / Podman daemon first
        let stripped_repo = reference
            .repository
            .strip_prefix("library/")
            .unwrap_or(&reference.repository);
        let local_names = [
            format!("{}:{}", stripped_repo, reference.tag),
            format!("{}:{}", reference.repository, reference.tag),
            stripped_repo.to_string(),
            reference.repository.clone(),
            reference.canonical_name(),
        ];
        // 0a. Try locally built images first
        for name in &local_names {
            if let Ok(Some((rootfs, cfg))) = crate::builder::try_fetch_local_tag(name, cache_base) {
                tracing::info!("Using locally built image '{}'", name);
                return Ok((rootfs, cfg));
            }
        }

        // 0b. Try local Docker / Podman daemon
        for name in &local_names {
            if let Ok(Some((rootfs, cfg))) =
                super::daemon::DockerDaemon::try_fetch_and_unpack(name, cache_base)
            {
                tracing::info!("Using local image '{}' from Docker daemon", name);
                return Ok((rootfs, cfg));
            }
        }

        let token = self.get_token(reference).await?;
        let token_ref = token.as_deref();

        // 1. Fetch Manifest
        let (manifest, manifest_digest) = self.fetch_manifest(reference, token_ref).await?;

        let cache =
            CacheEntry::acquire_async(cache_base, CacheKind::Image, &manifest_digest).await?;
        let rootfs_dir = cache.path().to_path_buf();
        if let Some(cfg) = cache.read_metadata::<OciConfig>()? {
            tracing::info!("RootFS cache hit for manifest {}", manifest_digest);
            return Ok((rootfs_dir, cfg));
        }

        // 2. Fetch OCI Config blob (verifies digest)
        let config_bytes = self
            .fetch_blob(reference, &manifest.config.digest, token_ref)
            .await?;
        let full_config: serde_json::Value =
            serde_json::from_slice(&config_bytes).context("Failed to parse image config JSON")?;

        let oci_config = parse_oci_config_from_json(&full_config);

        // A dropped or failed pull cleans up its private staging directory.
        let transaction = cache.stage()?;
        let staging_rootfs = transaction.path();

        tracing::info!(
            "Extracting {} layers for {} (streaming)...",
            manifest.layers.len(),
            reference.canonical_name()
        );
        for (i, layer) in manifest.layers.iter().enumerate() {
            tracing::info!(
                "Extracting layer [{}/{}] ({})",
                i + 1,
                manifest.layers.len(),
                layer.digest
            );

            let layer_hash = sha256_hex(&layer.digest)?;
            let blob_cache_file = cache_base.join("blobs").join("sha256").join(layer_hash);

            let blob_bytes = if blob_cache_file.exists() {
                tracing::debug!("Layer blob cache hit: {}", layer.digest);
                let cached = std::fs::read(&blob_cache_file)?;
                verify_sha256(&layer.digest, &cached)?;
                bytes::Bytes::from(cached)
            } else {
                let downloaded = self
                    .fetch_blob_bytes(reference, &layer.digest, token_ref)
                    .await?;
                super::cache::atomic_write(&blob_cache_file, &downloaded)?;
                downloaded
            };

            let cursor = std::io::Cursor::new(blob_bytes);
            extract_layer(cursor, staging_rootfs, true)?;
        }

        transaction.commit(&oci_config)?;

        Ok((rootfs_dir, oci_config))
    }

    pub(crate) async fn fetch_manifest(
        &self,
        reference: &ImageReference,
        token: Option<&str>,
    ) -> Result<(SingleManifest, String)> {
        let target_tag_or_digest = if let Some(ref d) = reference.digest {
            d.as_str()
        } else {
            reference.tag.as_str()
        };

        let manifest_url = format!(
            "https://{}/v2/{}/manifests/{}",
            reference.registry, reference.repository, target_tag_or_digest
        );

        let mut headers = self.auth_headers(token);
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "application/vnd.docker.distribution.manifest.v2+json, \
                 application/vnd.docker.distribution.manifest.list.v2+json, \
                 application/vnd.oci.image.manifest.v1+json, \
                 application/vnd.oci.image.index.v1+json",
            ),
        );

        let mut resp = self
            .http
            .get(&manifest_url)
            .headers(headers.clone())
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(auth_hdr) = resp.headers().get(WWW_AUTHENTICATE) {
                if let Ok(hdr_str) = auth_hdr.to_str() {
                    if let Ok(Some(tok)) =
                        self.token_from_www_authenticate(hdr_str, reference).await
                    {
                        let mut retry_headers = self.auth_headers(Some(&tok));
                        retry_headers.insert(
                            ACCEPT,
                            HeaderValue::from_static(
                                "application/vnd.docker.distribution.manifest.v2+json, \
                                 application/vnd.docker.distribution.manifest.list.v2+json, \
                                 application/vnd.oci.image.manifest.v1+json, \
                                 application/vnd.oci.image.index.v1+json",
                            ),
                        );
                        resp = self
                            .http
                            .get(&manifest_url)
                            .headers(retry_headers)
                            .send()
                            .await?;
                    }
                }
            }
        }
        if !resp.status().is_success() {
            bail!("Failed to fetch manifest: HTTP {}", resp.status());
        }

        let body_bytes = resp.bytes().await?;
        let digest_str = format!("sha256:{:x}", Sha256::digest(&body_bytes));

        // If pinned by digest, verify digest matches
        if let Some(ref expected_digest) = reference.digest {
            if &digest_str != expected_digest && target_tag_or_digest.starts_with("sha256:") {
                bail!(
                    "Cryptographic digest mismatch for manifest: expected {}, got {}",
                    expected_digest,
                    digest_str
                );
            }
        }

        // Check if this is an image index / multi-arch manifest list
        if let Ok(index) = serde_json::from_slice::<ManifestIndex>(&body_bytes) {
            if let Some(entries) = index.manifests {
                if entries.is_empty() {
                    bail!("Registry returned an empty manifest index");
                }
                let target_arch = match std::env::consts::ARCH {
                    "aarch64" => "arm64",
                    "x86_64" => "amd64",
                    other => other,
                };

                let matched_entry = entries.iter().find(|e| {
                    if let Some(ref p) = e.platform {
                        p.os == "linux" && p.architecture == target_arch
                    } else {
                        false
                    }
                });

                let chosen_digest = match matched_entry {
                    Some(entry) => &entry.digest,
                    None => {
                        // fallback to first linux manifest
                        entries
                            .iter()
                            .find(|e| e.platform.as_ref().is_some_and(|p| p.os == "linux"))
                            .map(|e| &e.digest)
                            .unwrap_or(&entries[0].digest)
                    }
                };

                sha256_hex(chosen_digest)?;
                // Fetch the concrete single manifest for this platform
                let child_url = format!(
                    "https://{}/v2/{}/manifests/{}",
                    reference.registry, reference.repository, chosen_digest
                );
                let child_resp = self.http.get(&child_url).headers(headers).send().await?;
                let child_resp = child_resp.error_for_status()?;
                let child_bytes = child_resp.bytes().await?;
                verify_sha256(chosen_digest, &child_bytes)?;
                let single: SingleManifest = serde_json::from_slice(&child_bytes)
                    .context("Failed to parse concrete single manifest")?;
                return Ok((single, chosen_digest.clone()));
            }
        }

        let single: SingleManifest =
            serde_json::from_slice(&body_bytes).context("Failed to parse single manifest")?;
        Ok((single, digest_str))
    }

    async fn fetch_blob(
        &self,
        reference: &ImageReference,
        digest: &str,
        token: Option<&str>,
    ) -> Result<Vec<u8>> {
        let bytes = self.fetch_blob_bytes(reference, digest, token).await?;
        Ok(bytes.to_vec())
    }

    pub(crate) async fn fetch_blob_bytes(
        &self,
        reference: &ImageReference,
        digest: &str,
        token: Option<&str>,
    ) -> Result<bytes::Bytes> {
        sha256_hex(digest)?;
        let url = format!(
            "https://{}/v2/{}/blobs/{}",
            reference.registry, reference.repository, digest
        );

        let headers = self.auth_headers(token);
        let mut resp = self.http.get(&url).headers(headers).send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(auth_hdr) = resp.headers().get(WWW_AUTHENTICATE) {
                if let Ok(hdr_str) = auth_hdr.to_str() {
                    if let Ok(Some(tok)) =
                        self.token_from_www_authenticate(hdr_str, reference).await
                    {
                        let retry_headers = self.auth_headers(Some(&tok));
                        resp = self.http.get(&url).headers(retry_headers).send().await?;
                    }
                }
            }
        }
        if !resp.status().is_success() {
            bail!("Failed to download blob {}: HTTP {}", digest, resp.status());
        }

        let bytes = resp.bytes().await?;

        verify_sha256(digest, &bytes)?;

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_parse_www_authenticate_header() {
        let hdr = r#"Bearer realm="https://quay.io/v2/auth",service="quay.io""#;
        let mut realm = None;
        let mut service = None;
        for part in hdr["Bearer ".len()..].split(',') {
            if let Some((k, v)) = part.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"');
                if key == "realm" {
                    realm = Some(val.to_string());
                }
                if key == "service" {
                    service = Some(val.to_string());
                }
            }
        }
        assert_eq!(realm.as_deref(), Some("https://quay.io/v2/auth"));
        assert_eq!(service.as_deref(), Some("quay.io"));
    }
}
