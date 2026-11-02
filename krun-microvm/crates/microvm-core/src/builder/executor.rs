use super::dockerignore::Dockerignore;
use super::parser::{Dockerfile, Instruction};
use crate::config::OciConfig;
use crate::exec::{exec_in_guest_rootfs, ExecRequest};
use crate::oci::client::OciClient;
use crate::oci::reference::ImageReference;
use crate::rootfs::clone_rootfs;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

/// Normalizes a path lexically and verifies that it does not escape `base` via `..`.
pub fn safe_normalize_relative(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(c) => normalized.push(c),
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!(
                        "Path containment violation: path '{}' escapes base directory via '..'",
                        path.display()
                    );
                }
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(normalized)
}

/// Securely resolves a path within a base directory, verifying both lexical containment
/// and canonical symlink containment.
pub fn secure_resolve_in_base(base: &Path, rel_or_abs: &Path) -> Result<PathBuf> {
    let normalized_rel = safe_normalize_relative(rel_or_abs)?;
    let candidate = base.join(normalized_rel);

    let canonical_base = base
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize base directory '{}'", base.display()))?;

    if candidate.exists() {
        let canonical_candidate = candidate.canonicalize().with_context(|| {
            format!(
                "Failed to canonicalize candidate path '{}'",
                candidate.display()
            )
        })?;
        if !canonical_candidate.starts_with(&canonical_base) {
            bail!(
                "Path containment violation: candidate '{}' resolves to '{}' outside base '{}'",
                candidate.display(),
                canonical_candidate.display(),
                base.display()
            );
        }
        Ok(candidate)
    } else {
        // Find closest existing ancestor and verify containment
        let mut curr = candidate.as_path();
        while let Some(parent) = curr.parent() {
            if parent.exists() {
                let canonical_parent = parent.canonicalize().with_context(|| {
                    format!("Failed to canonicalize ancestor '{}'", parent.display())
                })?;
                if !canonical_parent.starts_with(&canonical_base) {
                    bail!(
                        "Path containment violation: ancestor directory '{}' resolves to '{}' outside base '{}'",
                        parent.display(),
                        canonical_parent.display(),
                        base.display()
                    );
                }
                break;
            }
            curr = parent;
        }
        Ok(candidate)
    }
}

/// Configuration options for building an image.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Tag to assign to the built image (e.g. `myapp:latest`).
    pub tag: Option<String>,
    /// Optional explicit path to the Dockerfile. If None, `<context_dir>/Dockerfile` is used.
    pub dockerfile_path: Option<PathBuf>,
    /// Path to the build context directory.
    pub context_dir: PathBuf,
    /// Disable layer caching.
    pub no_cache: bool,
    /// Base directory for caching images, layers, and artifacts.
    pub cache_base: PathBuf,
    /// Build-time arguments (e.g. `ARG`).
    pub build_args: HashMap<String, String>,
}

impl BuildOptions {
    pub fn new(context_dir: impl Into<PathBuf>, cache_base: impl Into<PathBuf>) -> Self {
        Self {
            tag: None,
            dockerfile_path: None,
            context_dir: context_dir.into(),
            no_cache: false,
            cache_base: cache_base.into(),
            build_args: HashMap::new(),
        }
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn with_dockerfile(mut self, path: impl Into<PathBuf>) -> Self {
        self.dockerfile_path = Some(path.into());
        self
    }
}

/// Result of an image build operation.
#[derive(Debug, Clone)]
pub struct BuildResult {
    pub digest: String,
    pub rootfs_path: PathBuf,
    pub config: OciConfig,
    pub tag: Option<String>,
}

/// Metadata stored in `local_images.json` for locally built images.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalImageRecord {
    pub tag: String,
    pub digest: String,
    pub rootfs_path: PathBuf,
    pub config_path: PathBuf,
    pub created_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalImageRegistry {
    pub images: HashMap<String, LocalImageRecord>,
}

impl LocalImageRegistry {
    pub fn registry_file(cache_base: &Path) -> PathBuf {
        cache_base.join("local_images.json")
    }

    pub fn load(cache_base: &Path) -> Self {
        let path = Self::registry_file(cache_base);
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(reg) = serde_json::from_str::<LocalImageRegistry>(&data) {
                    return reg;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self, cache_base: &Path) -> Result<()> {
        let path = Self::registry_file(cache_base);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }

    pub fn register(&mut self, tag: &str, digest: &str, rootfs_path: &Path, config_path: &Path) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let record = LocalImageRecord {
            tag: tag.to_string(),
            digest: digest.to_string(),
            rootfs_path: rootfs_path.to_path_buf(),
            config_path: config_path.to_path_buf(),
            created_at: now,
        };

        self.images.insert(tag.to_string(), record.clone());
        // Also register under stripped prefix if applicable
        if let Some(stripped) = tag.strip_prefix("library/") {
            self.images.insert(stripped.to_string(), record.clone());
        }
    }

    pub fn lookup(&self, tag_or_name: &str) -> Option<&LocalImageRecord> {
        self.images.get(tag_or_name).or_else(|| {
            let tagged = if !tag_or_name.contains(':') {
                format!("{tag_or_name}:latest")
            } else {
                tag_or_name.to_string()
            };
            self.images.get(&tagged)
        })
    }
}

/// Tries to resolve a locally built image by tag/name.
pub fn try_fetch_local_tag(
    tag_or_name: &str,
    cache_base: &Path,
) -> Result<Option<(PathBuf, OciConfig)>> {
    let reg = LocalImageRegistry::load(cache_base);
    if let Some(record) = reg.lookup(tag_or_name) {
        if record.rootfs_path.exists() && record.config_path.exists() {
            let config_data = fs::read_to_string(&record.config_path)?;
            if let Ok(config) = serde_json::from_str::<OciConfig>(&config_data) {
                return Ok(Some((record.rootfs_path.clone(), config)));
            }
        }
    }
    Ok(None)
}

/// Represents an intermediate build stage.
struct BuildStage {
    rootfs: PathBuf,
    config: OciConfig,
    workdir: PathBuf,
    env: HashMap<String, String>,
}

/// Primary build engine executing Dockerfiles.
pub struct BuildEngine {
    options: BuildOptions,
    oci_client: OciClient,
}

impl BuildEngine {
    pub fn new(options: BuildOptions) -> Self {
        Self {
            options,
            oci_client: OciClient::new(),
        }
    }

    /// Executes the build process.
    pub async fn build(&self) -> Result<BuildResult> {
        let df_path = self
            .options
            .dockerfile_path
            .clone()
            .unwrap_or_else(|| self.options.context_dir.join("Dockerfile"));

        tracing::info!("Parsing Dockerfile: {}", df_path.display());
        let dockerfile = Dockerfile::from_file(&df_path)?;

        let ignore = Dockerignore::load_from_context(&self.options.context_dir);

        let mut stages: HashMap<String, BuildStage> = HashMap::new();
        let mut stage_indices: Vec<String> = Vec::new();
        let mut active_stage: Option<BuildStage> = None;
        let mut active_alias: Option<String> = None;

        let total_steps = dockerfile.instructions.len();

        for (step_idx, instruction) in dockerfile.instructions.iter().enumerate() {
            let step_num = step_idx + 1;
            match instruction {
                Instruction::From {
                    image,
                    platform: _,
                    alias,
                } => {
                    // Finalize current stage if one was open
                    if let Some(prev) = active_stage.take() {
                        let stage_key = active_alias
                            .take()
                            .unwrap_or_else(|| (stage_indices.len() - 1).to_string());
                        stages.insert(stage_key, prev);
                    }

                    tracing::info!(
                        "Step [{}/{}] : FROM {}{}",
                        step_num,
                        total_steps,
                        image,
                        alias
                            .as_ref()
                            .map(|a| format!(" AS {a}"))
                            .unwrap_or_default()
                    );

                    let stage_id = alias
                        .clone()
                        .unwrap_or_else(|| stage_indices.len().to_string());
                    stage_indices.push(stage_id.clone());
                    active_alias = alias.clone();

                    let stage = self.init_stage_from(image, &stages).await?;
                    active_stage = Some(stage);
                }
                Instruction::Workdir(wd) => {
                    tracing::info!("Step [{}/{}] : WORKDIR {}", step_num, total_steps, wd);
                    let stage = active_stage.as_mut().context("No active FROM stage")?;
                    let new_wd = if wd.starts_with('/') {
                        PathBuf::from(wd)
                    } else {
                        stage.workdir.join(wd)
                    };
                    stage.workdir = new_wd.clone();
                    stage.config.working_dir = Some(new_wd.to_string_lossy().to_string());

                    // Ensure directory exists in guest rootfs
                    let rel_wd = new_wd.strip_prefix("/").unwrap_or(&new_wd);
                    let target_dir = stage.rootfs.join(rel_wd);
                    fs::create_dir_all(&target_dir).with_context(|| {
                        format!("Failed to create WORKDIR {}", target_dir.display())
                    })?;
                }
                Instruction::Env(pairs) => {
                    tracing::info!(
                        "Step [{}/{}] : ENV {:?}",
                        step_num,
                        total_steps,
                        pairs
                            .iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                    );
                    let stage = active_stage.as_mut().context("No active FROM stage")?;
                    for (k, v) in pairs {
                        stage.env.insert(k.clone(), v.clone());
                        // Update OciConfig env list
                        let entry = format!("{k}={v}");
                        stage
                            .config
                            .env
                            .retain(|e| !e.starts_with(&format!("{k}=")));
                        stage.config.env.push(entry);
                    }
                }
                Instruction::Copy {
                    sources,
                    dest,
                    from_stage,
                    chown: _,
                } => {
                    tracing::info!(
                        "Step [{}/{}] : COPY {:?} -> {}{}",
                        step_num,
                        total_steps,
                        sources,
                        dest,
                        from_stage
                            .as_ref()
                            .map(|f| format!(" (--from={f})"))
                            .unwrap_or_default()
                    );
                    let stage = active_stage.as_mut().context("No active FROM stage")?;

                    let src_base = if let Some(from_name) = from_stage {
                        let ref_stage = stages
                            .get(from_name)
                            .or_else(|| {
                                // Try numerical index
                                from_name.parse::<usize>().ok().and_then(|idx| {
                                    stage_indices.get(idx).and_then(|key| stages.get(key))
                                })
                            })
                            .with_context(|| format!("Invalid COPY --from stage: '{from_name}'"))?;
                        ref_stage.rootfs.clone()
                    } else {
                        self.options.context_dir.clone()
                    };

                    self.copy_files(
                        &src_base,
                        sources,
                        dest,
                        &stage.workdir,
                        &stage.rootfs,
                        from_stage.is_none().then_some(&ignore),
                    )?;
                }
                Instruction::Add {
                    sources,
                    dest,
                    chown: _,
                } => {
                    tracing::info!(
                        "Step [{}/{}] : ADD {:?} -> {}",
                        step_num,
                        total_steps,
                        sources,
                        dest
                    );
                    let stage = active_stage.as_mut().context("No active FROM stage")?;
                    self.copy_files(
                        &self.options.context_dir,
                        sources,
                        dest,
                        &stage.workdir,
                        &stage.rootfs,
                        Some(&ignore),
                    )?;
                }
                Instruction::Run(cmd_form) => {
                    let cmd_args = cmd_form.to_args();
                    tracing::info!("Step [{}/{}] : RUN {:?}", step_num, total_steps, cmd_args);
                    let stage = active_stage.as_mut().context("No active FROM stage")?;

                    let env_vec: Vec<String> =
                        stage.env.iter().map(|(k, v)| format!("{k}={v}")).collect();

                    let req = ExecRequest::new(cmd_args.clone())
                        .with_env(env_vec)
                        .with_workdir(stage.workdir.to_string_lossy().to_string());

                    let resp = exec_in_guest_rootfs(&stage.rootfs, &req)
                        .await
                        .with_context(|| {
                            format!("Failed to execute RUN command: {:?}", cmd_args)
                        })?;

                    if resp.exit_code != 0 {
                        eprintln!("{}", resp.stdout);
                        eprintln!("{}", resp.stderr);
                        bail!(
                            "The command '{:?}' returned a non-zero code: {}",
                            cmd_args,
                            resp.exit_code
                        );
                    }
                }
                Instruction::Cmd(cmd_form) => {
                    tracing::info!(
                        "Step [{}/{}] : CMD {:?}",
                        step_num,
                        total_steps,
                        cmd_form.to_args()
                    );
                    let stage = active_stage.as_mut().context("No active FROM stage")?;
                    stage.config.cmd = cmd_form.to_args();
                }
                Instruction::Entrypoint(cmd_form) => {
                    tracing::info!(
                        "Step [{}/{}] : ENTRYPOINT {:?}",
                        step_num,
                        total_steps,
                        cmd_form.to_args()
                    );
                    let stage = active_stage.as_mut().context("No active FROM stage")?;
                    stage.config.entrypoint = cmd_form.to_args();
                }
                Instruction::Expose(ports) => {
                    tracing::info!("Step [{}/{}] : EXPOSE {:?}", step_num, total_steps, ports);
                }
                Instruction::User(user) => {
                    tracing::info!("Step [{}/{}] : USER {}", step_num, total_steps, user);
                    let stage = active_stage.as_mut().context("No active FROM stage")?;
                    stage.config.user = Some(user.clone());
                }
                Instruction::Label(labels) => {
                    tracing::info!("Step [{}/{}] : LABEL {:?}", step_num, total_steps, labels);
                    // Labels tracked in config if needed
                }
            }
        }

        let final_stage = active_stage.context("No stages built")?;

        // Invariant from Phase 1 & 2: Ensure /tmp and /var/tmp exist with 0o1777 sticky bits
        crate::rootfs::extract::ensure_tmp_sticky_bit(&final_stage.rootfs)?;

        // Compute image digest based on final config and timestamp
        let config_bytes = serde_json::to_vec_pretty(&final_stage.config)?;
        let mut hasher = Sha256::new();
        hasher.update(&config_bytes);
        let digest_hex = hex::encode(hasher.finalize());
        let digest = format!("sha256:{digest_hex}");
        let safe_digest = digest.replace(':', "_");

        // Commit final rootfs to cache_base/rootfs/<safe_digest>
        let target_rootfs = self.options.cache_base.join("rootfs").join(&safe_digest);
        if target_rootfs.exists() {
            let _ = fs::remove_dir_all(&target_rootfs);
        }
        if let Some(parent) = target_rootfs.parent() {
            fs::create_dir_all(parent)?;
        }
        clone_rootfs(&final_stage.rootfs, &target_rootfs)
            .context("Failed to commit built rootfs to cache")?;

        // Save config to cache_base/configs/<safe_digest>.json
        let target_config_file = self
            .options
            .cache_base
            .join("configs")
            .join(format!("{safe_digest}.json"));
        if let Some(parent) = target_config_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target_config_file, &config_bytes)?;

        // Register local tag if provided
        if let Some(ref tag) = self.options.tag {
            let mut reg = LocalImageRegistry::load(&self.options.cache_base);
            reg.register(tag, &digest, &target_rootfs, &target_config_file);
            reg.save(&self.options.cache_base)?;
            tracing::info!("Successfully tagged image as '{}'", tag);
        }

        Ok(BuildResult {
            digest,
            rootfs_path: target_rootfs,
            config: final_stage.config,
            tag: self.options.tag.clone(),
        })
    }

    async fn init_stage_from(
        &self,
        image: &str,
        stages: &HashMap<String, BuildStage>,
    ) -> Result<BuildStage> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let staging_id = format!("build_stage_{}_{}", std::process::id(), nonce);
        let staging_rootfs = self.options.cache_base.join("staging").join(staging_id);
        fs::create_dir_all(&staging_rootfs)?;

        if image == "scratch" {
            let empty_cfg = OciConfig::default();
            return Ok(BuildStage {
                rootfs: staging_rootfs,
                config: empty_cfg,
                workdir: PathBuf::from("/"),
                env: HashMap::new(),
            });
        }

        // Check if image is a previous stage alias
        if let Some(prev) = stages.get(image) {
            clone_rootfs(&prev.rootfs, &staging_rootfs)?;
            return Ok(BuildStage {
                rootfs: staging_rootfs,
                config: prev.config.clone(),
                workdir: prev.workdir.clone(),
                env: prev.env.clone(),
            });
        }

        // Otherwise check local registry or pull via OCI client
        let (cached_rootfs, oci_config) = if let Ok(Some((local_rootfs, local_cfg))) =
            try_fetch_local_tag(image, &self.options.cache_base)
        {
            (local_rootfs, local_cfg)
        } else {
            let reference = ImageReference::parse(image)?;
            self.oci_client
                .pull_and_unpack(&reference, &self.options.cache_base)
                .await?
        };

        clone_rootfs(&cached_rootfs, &staging_rootfs)?;

        let mut env_map = HashMap::new();
        for e in &oci_config.env {
            if let Some((k, v)) = e.split_once('=') {
                env_map.insert(k.to_string(), v.to_string());
            }
        }

        let initial_workdir = oci_config
            .working_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));

        Ok(BuildStage {
            rootfs: staging_rootfs,
            config: oci_config,
            workdir: initial_workdir,
            env: env_map,
        })
    }

    fn copy_files(
        &self,
        src_base: &Path,
        sources: &[String],
        dest: &str,
        current_workdir: &Path,
        guest_rootfs: &Path,
        dockerignore: Option<&Dockerignore>,
    ) -> Result<()> {
        let dest_path = Path::new(dest);
        let resolved_dest = if dest_path.is_absolute() {
            dest_path.to_path_buf()
        } else {
            current_workdir.join(dest_path)
        };

        // Normalize destination relative to guest rootfs and verify containment
        let guest_dest = secure_resolve_in_base(guest_rootfs, &resolved_dest)?;

        let dest_is_dir = dest.ends_with('/') || dest.ends_with('.');
        if dest_is_dir && !guest_dest.exists() {
            fs::create_dir_all(&guest_dest)?;
        }

        for src_pattern in sources {
            let rel_src = Path::new(src_pattern.strip_prefix("./").unwrap_or(src_pattern));
            // Securely resolve source within src_base (build context or reference stage rootfs)
            let src_path = secure_resolve_in_base(src_base, rel_src)?;

            if !src_path.exists() {
                bail!("Source path does not exist: {}", src_path.display());
            }

            let file_type = fs::symlink_metadata(&src_path)?.file_type();
            if file_type.is_symlink() {
                let canonical_src_base = src_base
                    .canonicalize()
                    .unwrap_or_else(|_| src_base.to_path_buf());
                let canonical_target = src_path.canonicalize().with_context(|| {
                    format!(
                        "Failed to canonicalize symlink target '{}'",
                        src_path.display()
                    )
                })?;
                if !canonical_target.starts_with(&canonical_src_base) {
                    bail!(
                        "Path containment violation: symlink '{}' resolves to '{}' outside source base '{}'",
                        src_path.display(),
                        canonical_target.display(),
                        src_base.display()
                    );
                }
            }

            if src_path.is_file() {
                if let Some(di) = dockerignore {
                    if let Ok(rel) = src_path.strip_prefix(src_base) {
                        if di.is_ignored(rel) {
                            continue;
                        }
                    }
                }

                let target_file = if dest_is_dir {
                    let file_name = src_path.file_name().context("Invalid source filename")?;
                    guest_dest.join(file_name)
                } else {
                    guest_dest.clone()
                };

                // Double check target_file containment
                let target_file = secure_resolve_in_base(
                    guest_rootfs,
                    target_file
                        .strip_prefix(guest_rootfs)
                        .unwrap_or(&target_file),
                )?;

                if let Some(parent) = target_file.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::copy(&src_path, &target_file)?;
            } else if src_path.is_dir() {
                self.copy_dir_filtered(
                    &src_path,
                    &guest_dest,
                    src_base,
                    guest_rootfs,
                    dockerignore,
                )?;
            }
        }

        Ok(())
    }

    fn copy_dir_filtered(
        &self,
        src_dir: &Path,
        dst_dir: &Path,
        context_base: &Path,
        guest_rootfs: &Path,
        dockerignore: Option<&Dockerignore>,
    ) -> Result<()> {
        let dst_dir = secure_resolve_in_base(
            guest_rootfs,
            dst_dir.strip_prefix(guest_rootfs).unwrap_or(dst_dir),
        )?;
        fs::create_dir_all(&dst_dir)?;

        let canonical_context_base = context_base
            .canonicalize()
            .unwrap_or_else(|_| context_base.to_path_buf());

        for entry in fs::read_dir(src_dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;

            // Guard against symlink traversal escaping context_base
            if file_type.is_symlink() {
                let canonical_target = match path.canonicalize() {
                    Ok(p) => p,
                    Err(e) => {
                        bail!(
                            "Path containment violation: dangling or unresolvable symlink '{}': {}",
                            path.display(),
                            e
                        );
                    }
                };
                if !canonical_target.starts_with(&canonical_context_base) {
                    bail!(
                        "Path containment violation: symlink '{}' resolves to '{}' outside build context '{}'",
                        path.display(),
                        canonical_target.display(),
                        context_base.display()
                    );
                }
            }

            if let Some(di) = dockerignore {
                if let Ok(rel) = path.strip_prefix(context_base) {
                    if di.is_ignored(rel) {
                        continue;
                    }
                }
            }

            let file_name = entry.file_name();
            let dest_child = dst_dir.join(&file_name);
            let secure_dest_child = secure_resolve_in_base(
                guest_rootfs,
                dest_child.strip_prefix(guest_rootfs).unwrap_or(&dest_child),
            )?;

            if file_type.is_dir() {
                self.copy_dir_filtered(
                    &path,
                    &secure_dest_child,
                    context_base,
                    guest_rootfs,
                    dockerignore,
                )?;
            } else {
                fs::copy(&path, &secure_dest_child)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_registry_save_and_lookup() {
        let temp = tempfile::tempdir().unwrap();
        let cache_base = temp.path();

        let mut reg = LocalImageRegistry::load(cache_base);
        let rootfs = cache_base.join("rootfs_mock");
        let config = cache_base.join("config_mock.json");
        fs::create_dir_all(&rootfs).unwrap();
        fs::write(&config, "{}").unwrap();

        reg.register("myapp:v1", "sha256:1234", &rootfs, &config);
        reg.save(cache_base).unwrap();

        let loaded = LocalImageRegistry::load(cache_base);
        let hit = loaded.lookup("myapp:v1");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().digest, "sha256:1234");
    }

    #[tokio::test]
    async fn test_build_engine_scratch_copy_workdir() {
        let temp = tempfile::tempdir().unwrap();
        let cache_base = temp.path().join("cache");
        let context_dir = temp.path().join("context");
        fs::create_dir_all(&context_dir).unwrap();

        let df_content = r#"
        FROM scratch
        WORKDIR /app
        COPY hello.txt /app/hello.txt
        ENV PORT=3000
        CMD ["/app/hello.txt"]
        "#;
        fs::write(context_dir.join("Dockerfile"), df_content).unwrap();
        fs::write(context_dir.join("hello.txt"), "hello world").unwrap();

        let options = BuildOptions::new(&context_dir, &cache_base).with_tag("test-app:latest");

        let engine = BuildEngine::new(options);
        let result = engine.build().await.unwrap();

        assert_eq!(result.tag, Some("test-app:latest".to_string()));
        assert!(result.rootfs_path.exists());
        assert!(result.rootfs_path.join("app/hello.txt").exists());
        let read_txt = fs::read_to_string(result.rootfs_path.join("app/hello.txt")).unwrap();
        assert_eq!(read_txt, "hello world");

        // Verify config
        assert_eq!(result.config.cmd, vec!["/app/hello.txt".to_string()]);
        assert!(result.config.env.contains(&"PORT=3000".to_string()));

        // Verify local lookup works
        let local = try_fetch_local_tag("test-app:latest", &cache_base).unwrap();
        assert!(local.is_some());
    }

    #[test]
    fn test_safe_normalize_relative_bounds() {
        assert_eq!(
            safe_normalize_relative(Path::new("a/b/c")).unwrap(),
            PathBuf::from("a/b/c")
        );
        assert_eq!(
            safe_normalize_relative(Path::new("./a/b/../c")).unwrap(),
            PathBuf::from("a/c")
        );
        assert_eq!(
            safe_normalize_relative(Path::new("/var/app")).unwrap(),
            PathBuf::from("var/app")
        );
        assert!(safe_normalize_relative(Path::new("../secret")).is_err());
        assert!(safe_normalize_relative(Path::new("a/../../secret")).is_err());
    }

    #[test]
    fn test_secure_resolve_in_base_containment() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        fs::create_dir_all(&base).unwrap();

        // Valid inside base
        let valid = secure_resolve_in_base(&base, Path::new("sub/file.txt")).unwrap();
        assert!(valid.starts_with(&base));

        // Traversal escaping base rejected
        let escaping = secure_resolve_in_base(&base, Path::new("../outside.txt"));
        assert!(escaping.is_err());
    }

    #[tokio::test]
    async fn test_build_engine_copy_source_traversal_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let cache_base = temp.path().join("cache");
        let context_dir = temp.path().join("context");
        fs::create_dir_all(&context_dir).unwrap();

        let df_content = r#"
        FROM scratch
        COPY ../outside.txt /app/outside.txt
        "#;
        fs::write(context_dir.join("Dockerfile"), df_content).unwrap();

        let options = BuildOptions::new(&context_dir, &cache_base);
        let engine = BuildEngine::new(options);
        let result = engine.build().await;
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Path containment violation") || err_msg.contains("escapes"));
    }

    #[tokio::test]
    async fn test_build_engine_copy_dest_traversal_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let cache_base = temp.path().join("cache");
        let context_dir = temp.path().join("context");
        fs::create_dir_all(&context_dir).unwrap();

        let df_content = r#"
        FROM scratch
        COPY hello.txt /../../outside.txt
        "#;
        fs::write(context_dir.join("Dockerfile"), df_content).unwrap();
        fs::write(context_dir.join("hello.txt"), "hello").unwrap();

        let options = BuildOptions::new(&context_dir, &cache_base);
        let engine = BuildEngine::new(options);
        let result = engine.build().await;
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Path containment violation") || err_msg.contains("escapes"));
    }

    #[tokio::test]
    async fn test_build_engine_copy_symlink_breakout_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let cache_base = temp.path().join("cache");
        let context_dir = temp.path().join("context");
        let outside_dir = temp.path().join("secret_outside");
        fs::create_dir_all(&context_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();

        let secret_file = outside_dir.join("token.txt");
        fs::write(&secret_file, "super_secret").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let evil_symlink = context_dir.join("evil_link");
            let _ = symlink(&secret_file, &evil_symlink);

            let df_content = r#"
            FROM scratch
            COPY evil_link /app/token.txt
            "#;
            fs::write(context_dir.join("Dockerfile"), df_content).unwrap();

            let options = BuildOptions::new(&context_dir, &cache_base);
            let engine = BuildEngine::new(options);
            let result = engine.build().await;
            assert!(result.is_err());
            let err_msg = result.err().unwrap().to_string();
            assert!(err_msg.contains("Path containment violation") || err_msg.contains("outside"));
        }
    }
}
