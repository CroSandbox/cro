// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use super::builder::MicroVmBuilder;
use super::runtime::{ensure_runner_signed, find_runner_binary, MicroVm};
use crate::config::KrunConfig;
use crate::oci::{ImageReference, OciClient};
use crate::preflight::Preflight;
use crate::rootfs::clone_rootfs;
use crate::tty::RawModeGuard;
use crate::types::{RunnerConfig, VirtioFsMount};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};

pub(crate) static INSTANCE_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

impl MicroVmBuilder {
    /// Resolves the effective base data directory for the microVM.
    pub(crate) fn resolve_data_dir(&self) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        self.data_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(home).join(".cache/krun-microvm"))
    }

    /// Runs host virtualization and system preflight checks if enabled.
    pub(crate) fn run_preflight(&self, data_dir: &Path) -> Result<()> {
        let host_ports: Vec<u16> = self.port_forwards.iter().map(|p| p.host).collect();
        let reports = Preflight::run_all(&host_ports, data_dir);
        for r in &reports {
            if !r.passed {
                bail!("Preflight check '{}' failed: {}", r.name, r.message);
            }
        }
        Ok(())
    }

    /// Generates a globally unique instance ID and ensures instance directories exist.
    pub(crate) fn generate_instance(&self, data_dir: &Path) -> Result<(String, PathBuf, PathBuf)> {
        let instance_id = if let Some(ref custom_id) = self.instance_id {
            custom_id.clone()
        } else {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
            let millis = now.as_millis();
            let nanos = now.subsec_nanos();
            let counter = INSTANCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("vm-{millis}-{nanos:06}-{counter:03}")
        };
        let instance_dir = data_dir.join("instances").join(&instance_id);
        let instance_rootfs = instance_dir.join("rootfs");
        Ok((instance_id, instance_dir, instance_rootfs))
    }

    /// Resolves rootfs layers, performs CoW cloning, injects single-file mounts, and writes krun config.
    pub(crate) async fn prepare_rootfs(
        &self,
        instance_id: &str,
        instance_rootfs: &Path,
        data_dir: &Path,
        is_direct_boot: bool,
    ) -> Result<()> {
        if is_direct_boot {
            let _ = fs::create_dir_all(instance_rootfs);
            return Ok(());
        }

        // Resolve rootfs (from bundle, local OCI layout, or OCI client pull)
        let (cached_rootfs, oci_config) = if let Some(ref custom_rootfs) = self.rootfs_override {
            let empty_cfg = crate::config::OciConfig {
                entrypoint: Vec::new(),
                cmd: Vec::new(),
                env: Vec::new(),
                working_dir: None,
                user: None,
            };
            (custom_rootfs.clone(), empty_cfg)
        } else {
            let reference = ImageReference::parse(&self.image)?;
            if reference.is_local_layout {
                let layout_path = reference
                    .layout_path
                    .as_ref()
                    .context("Local OCI layout path missing in ImageReference")?;
                tracing::info!(
                    "Loading image from local OCI layout at {} (tag: {})...",
                    layout_path.display(),
                    reference.tag
                );
                crate::oci::OciLayout::load(layout_path, Some(&reference.tag), data_dir)?
            } else {
                let oci_client = OciClient::new();
                tracing::info!(
                    "Ensuring image '{}' is ready...",
                    reference.canonical_name()
                );
                oci_client.pull_and_unpack(&reference, data_dir).await?
            }
        };

        if !instance_rootfs.exists() {
            tracing::info!("Cloning rootfs to instance {} (CoW)...", instance_id);
            clone_rootfs(&cached_rootfs, instance_rootfs)
                .context("Failed to perform CoW clone of rootfs")?;
        } else {
            tracing::info!("Reusing existing rootfs for instance {}...", instance_id);
        }

        // Inject single-file mounts (e.g. ConfigMaps, Secrets, service account tokens)
        for (src, dest) in &self.file_mounts {
            let relative_dest = dest.strip_prefix("/").unwrap_or(dest);
            let target_path = instance_rootfs.join(relative_dest);
            if let Some(parent) = target_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::copy(src, &target_path) {
                tracing::warn!(
                    "Failed to copy file mount from {} to {}: {}",
                    src.display(),
                    target_path.display(),
                    e
                );
            } else {
                tracing::debug!(
                    "Injected file mount {} -> {}",
                    src.display(),
                    target_path.display()
                );
            }
        }

        // Resolve Cmd, Env, and standard guest rootfs PATH
        let mut final_cmd = oci_config.resolve_cmd(self.cmd_override.clone());

        if let Some(first) = final_cmd.first_mut() {
            if !first.starts_with('/') {
                let search_paths = [
                    "/usr/local/sbin",
                    "/usr/local/bin",
                    "/usr/sbin",
                    "/usr/bin",
                    "/sbin",
                    "/bin",
                ];
                for sp in search_paths {
                    let rel = sp.trim_start_matches('/');
                    let candidate = instance_rootfs.join(rel).join(&first);
                    if candidate.exists() {
                        *first = format!("{sp}/{first}");
                        break;
                    }
                }
            }
        }

        // Guarantee enterprise /tmp & /var/tmp sticky permissions (0o1777)
        for tmp_rel in &["tmp", "var/tmp"] {
            let p = instance_rootfs.join(tmp_rel);
            let _ = fs::create_dir_all(&p);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o1777));
            }
        }

        if !final_cmd.is_empty() {
            let shm_sz = self.shm_size.as_deref().unwrap_or("64m");
            let mut init_lines = Vec::new();
            init_lines.push("#!/bin/sh".to_string());
            init_lines.push(
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$PATH"
                    .to_string(),
            );
            init_lines.push("chmod 1777 /tmp /var/tmp 2>/dev/null || true".to_string());
            init_lines.push("mkdir -p /dev/shm 2>/dev/null || true".to_string());
            init_lines.push(format!(
                "mount -t tmpfs -o rw,nosuid,nodev,mode=1777,size={shm_sz} tmpfs /dev/shm 2>/dev/null || true"
            ));

            if self.network_mode.is_gvproxy() {
                init_lines.push("ip link set eth0 up 2>/dev/null || true".to_string());
                init_lines.push("(udhcpc -i eth0 -q -n -t 2 || (ip addr add 192.168.127.2/24 dev eth0 && ip route add default via 192.168.127.1)) 2>/dev/null || true".to_string());
            }

            init_lines.push("exec \"$@\"".to_string());
            init_lines.push("".to_string());

            let init_script = init_lines.join("\n");
            let script_path = instance_rootfs.join("krun-init.sh");
            let _ = std::fs::write(&script_path, init_script);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755));
            }
            final_cmd.insert(0, "/krun-init.sh".to_string());
        }

        let mut final_env = oci_config.env;
        final_env.extend(self.env_vars.clone());
        if self.network_mode.is_gvproxy() {
            final_env.push("KRUN_DHCP=1".to_string());
        }
        for (k, _) in &self.secrets {
            final_env.push(format!("{k}=krun-secret:{k}"));
        }
        let final_workdir = self.workdir.clone().or(oci_config.working_dir);

        let krun_cfg = KrunConfig::new(final_cmd, final_env, final_workdir);
        krun_cfg.write_to(instance_rootfs)?;
        Ok(())
    }

    /// Sets up VirtioFS mounts including OCI artifacts, CoW workspaces, and Nydus chunk cache.
    pub async fn prepare_virtiofs_mounts(
        &self,
        instance_dir: &Path,
        instance_rootfs: &Path,
        data_dir: &Path,
    ) -> Result<Vec<VirtioFsMount>> {
        let mut final_virtiofs_mounts = self.virtiofs_mounts.clone();

        // Attach requested OCI artifacts
        if !self.artifacts.is_empty() {
            let oci_client = OciClient::new();
            for art in &self.artifacts {
                let art_ref = ImageReference::parse(&art.reference)?;
                tracing::info!(
                    "Ensuring OCI artifact '{}' is ready...",
                    art_ref.canonical_name()
                );
                let art_path = oci_client.pull_artifact(&art_ref, data_dir).await?;
                final_virtiofs_mounts.push(VirtioFsMount::new(
                    art.tag.clone(),
                    art_path,
                    art.read_only,
                ));
            }
        }

        // Attach requested CNCF ModelPack models
        if !self.models.is_empty() {
            let oci_client = OciClient::new();
            for (idx, m) in self.models.iter().enumerate() {
                let (host_model_dir, meta) = if Path::new(&m.reference).exists() {
                    let path = PathBuf::from(&m.reference);
                    let meta = crate::modelpack::ModelEngine::inspect_local_path(&path)?;
                    (path, meta)
                } else {
                    let model_ref = ImageReference::parse(&m.reference)?;
                    let model_path =
                        match crate::modelpack::ModelEngine::find_cached(&m.reference, data_dir)? {
                            Some(cached) => cached.path,
                            None => {
                                tracing::info!(
                                    "ModelPack '{}' not found in cache; pulling from registry...",
                                    model_ref.canonical_name()
                                );
                                oci_client.pull_model(&model_ref, data_dir).await?
                            }
                        };
                    let meta = crate::modelpack::ModelEngine::inspect_local_path(&model_path)?;
                    (model_path, meta)
                };

                let safe_name: String = meta
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let tag = format!("model_{safe_name}_{idx}");
                let guest_mount = m
                    .mount_path
                    .clone()
                    .unwrap_or_else(|| format!("/models/{}", meta.name));

                // Provision VirtioFS DAX window for weights
                let dax_window = self.dax_window_size.unwrap_or_else(|| {
                    if meta.format == crate::modelpack::ModelFormat::Gguf
                        || meta.format == crate::modelpack::ModelFormat::Safetensors
                    {
                        let chunk = 256 * 1024 * 1024;
                        if meta.total_size > 0 {
                            (meta.total_size.div_ceil(chunk) * chunk).max(512 * 1024 * 1024)
                        } else {
                            2 * 1024 * 1024 * 1024
                        }
                    } else {
                        0
                    }
                });

                let mut virtio_mount = VirtioFsMount::new(tag.clone(), host_model_dir, true);
                if dax_window > 0 {
                    virtio_mount = virtio_mount.with_dax(dax_window);
                }
                final_virtiofs_mounts.push(virtio_mount);

                // Prepare guest rootfs directory & symlinks
                let tag_dir = instance_rootfs.join(&tag);
                let _ = fs::create_dir_all(&tag_dir);

                let clean_mount = guest_mount.trim_start_matches('/');
                let target_link = instance_rootfs.join(clean_mount);
                if let Some(parent) = target_link.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                #[cfg(unix)]
                {
                    if !target_link.exists() {
                        let _ = std::os::unix::fs::symlink(format!("/{}", tag), &target_link);
                    }
                }

                // Inject MODEL_* environment variables into .krun_config.json
                let krun_cfg_path = instance_rootfs.join(crate::config::KrunConfig::FILE_NAME);
                if krun_cfg_path.exists() {
                    if let Ok(content) = fs::read(&krun_cfg_path) {
                        if let Ok(mut krun_cfg) =
                            serde_json::from_slice::<crate::config::KrunConfig>(&content)
                        {
                            krun_cfg.env.push(format!("MODEL_NAME={}", meta.name));
                            krun_cfg.env.push(format!("MODEL_PATH={}", guest_mount));
                            krun_cfg.env.push(format!("MODEL_FORMAT={}", meta.format));
                            if let Some(ref arch) = meta.architecture {
                                krun_cfg.env.push(format!("MODEL_ARCHITECTURE={}", arch));
                            }
                            if let Some(ref prec) = meta.precision {
                                krun_cfg.env.push(format!("MODEL_PRECISION={}", prec));
                            }
                            let _ = krun_cfg.write_to(instance_rootfs);
                        }
                    }
                }
            }
        }

        // Setup CoW workspaces (APFS clonefile / FICLONE snapshot)
        for ws in &self.cow_workspaces {
            let cow_dir = instance_dir.join("workspaces").join(&ws.tag);
            tracing::info!(
                "Creating CoW workspace snapshot for '{}' at {}...",
                ws.tag,
                cow_dir.display()
            );
            clone_rootfs(&ws.host_path, &cow_dir).with_context(|| {
                format!(
                    "Failed to create CoW workspace snapshot for '{}'",
                    ws.host_path.display()
                )
            })?;
            final_virtiofs_mounts.push(VirtioFsMount::new(ws.tag.clone(), cow_dir, false));
        }

        // Attach accelerated Nydus chunk blob cache directory if lazy loading is enabled
        if let Some(ref acc) = self.image_acceleration {
            if acc.lazy_load {
                let cache_dir = acc
                    .chunk_cache_dir
                    .clone()
                    .unwrap_or_else(|| data_dir.join("nydus-cache"));
                let _ = fs::create_dir_all(&cache_dir);
                tracing::info!(
                    "Configuring accelerated chunk cache at {} (format: {:?})...",
                    cache_dir.display(),
                    acc.format
                );
                let mut chunk_mount = VirtioFsMount::new("nydus-blobs", &cache_dir, true);
                if let Some(dax_bytes) = self.dax_window_size {
                    chunk_mount = chunk_mount.with_dax(dax_bytes);
                }
                final_virtiofs_mounts.push(chunk_mount);
            }
        }

        // Attach Apple Silicon Rosetta for Linux runtime if requested or if targeting amd64
        let rosetta_requested = self.rosetta
            || self
                .platform
                .as_deref()
                .map(|p| p.contains("amd64") || p.contains("x86_64"))
                .unwrap_or(false);

        if rosetta_requested {
            if let Some(rosetta_dir) = super::kernel::find_rosetta_path() {
                tracing::info!(
                    "Attaching Rosetta for Linux runtime from {}...",
                    rosetta_dir.display()
                );
                final_virtiofs_mounts.push(VirtioFsMount::new("rosetta", rosetta_dir, true));
            } else if self.rosetta {
                bail!("Rosetta for Linux is not installed on this host. On macOS, install with: softwareupdate --install-rosetta");
            }
        }

        // Ensure mount directory exists in guest rootfs for all virtiofs tags
        for m in &final_virtiofs_mounts {
            if m.tag.is_empty()
                || !m
                    .tag
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                bail!(
                    "Invalid VirtioFS mount tag '{}'. Tags must be non-empty and alphanumeric",
                    m.tag
                );
            }
            let _ = fs::create_dir_all(instance_rootfs.join(&m.tag));
        }

        // Inject in-guest virtiofs mount commands into krun-init.sh before exec "$@"
        let script_path = instance_rootfs.join("krun-init.sh");
        if script_path.exists() {
            if let Ok(content) = fs::read_to_string(&script_path) {
                let mut mount_cmds = Vec::new();
                for m in &final_virtiofs_mounts {
                    let ro_flag = if m.read_only { "-o ro " } else { "" };
                    mount_cmds.push(format!("mkdir -p /{} 2>/dev/null || true", m.tag));
                    mount_cmds.push(format!(
                        "mount -t virtiofs {ro_flag}{} /{} 2>/dev/null || true",
                        m.tag, m.tag
                    ));
                }

                // If rosetta is mounted, register ELF handler with binfmt_misc
                if final_virtiofs_mounts.iter().any(|m| m.tag == "rosetta") {
                    mount_cmds.push("if [ -f /rosetta/rosetta ]; then".to_string());
                    mount_cmds.push(
                        "  mkdir -p /proc/sys/fs/binfmt_misc 2>/dev/null || true".to_string(),
                    );
                    mount_cmds.push("  mount -t binfmt_misc binfmt_misc /proc/sys/fs/binfmt_misc 2>/dev/null || true".to_string());
                    mount_cmds
                        .push("  if [ -f /proc/sys/fs/binfmt_misc/register ]; then".to_string());
                    mount_cmds.push("    echo ':rosetta:M::\\x7fELF\\x02\\x01\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x02\\x00\\x3e\\x00:\\xff\\xff\\xff\\xff\\xff\\xfe\\xfe\\x00\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xfe\\xff\\xff\\xff:/rosetta/rosetta:CF' > /proc/sys/fs/binfmt_misc/register 2>/dev/null || true".to_string());
                    mount_cmds.push("  fi".to_string());
                    mount_cmds.push("fi".to_string());
                }

                if !mount_cmds.is_empty() {
                    let mount_block = mount_cmds.join("\n");
                    let updated = if content.contains("exec \"$@\"") {
                        content.replace("exec \"$@\"", &format!("{}\nexec \"$@\"", mount_block))
                    } else {
                        format!("{}\n{}", content, mount_block)
                    };
                    let _ = fs::write(&script_path, updated);
                }
            }
        }

        Ok(final_virtiofs_mounts)
    }

    /// Configures guest network files and starts the host egress proxy server if filtering is active.
    pub(crate) async fn configure_networking(
        &self,
        instance_id: &str,
        instance_dir: &Path,
        instance_rootfs: &Path,
    ) -> Result<(
        String,
        Option<crate::net::EgressProxyServer>,
        Option<u16>,
        Option<String>,
    )> {
        let guest_hostname = self
            .hostname
            .clone()
            .unwrap_or_else(|| instance_id.to_string());
        if !self.no_network {
            let nameservers = crate::net::DnsConfig::resolve_nameservers(&self.dns_servers);
            tracing::info!(
                "Configuring guest network files (hostname: {}, DNS: {:?})...",
                guest_hostname,
                nameservers
            );
            crate::net::DnsConfig::write_network_files_with_aliases(
                instance_rootfs,
                &guest_hostname,
                &nameservers,
                &self.extra_hosts,
            )?;
        } else {
            crate::net::DnsConfig::write_network_files_with_aliases(
                instance_rootfs,
                &guest_hostname,
                &[],
                &[],
            )?;
        }

        // Start host egress proxy server if filtering, secrets, token budget, approvals, or default-deny are specified
        let (egress_proxy, proxy_port) = if !self.allow_hosts.is_empty()
            || self.default_deny
            || !self.secrets.is_empty()
            || self.max_tokens.is_some()
            || self.approval_policy.is_some()
        {
            let policy = crate::net::EgressPolicy::new(self.allow_hosts.clone())
                .with_default_deny(self.default_deny);
            let secrets = crate::net::SecretSubstitution::new(&self.secrets);
            let budget = crate::net::LlmTokenBudget::new(self.max_tokens);
            let proxy_server =
                crate::net::EgressProxyServer::start(policy, secrets, budget).await?;
            let port = proxy_server.port();
            let url = proxy_server.proxy_url();
            crate::net::DnsConfig::inject_proxy_environment(instance_rootfs, &url)?;
            tracing::info!("Egress proxy active on port {port} for microVM {instance_id}");
            (Some(proxy_server), Some(port))
        } else {
            (None, None)
        };

        let final_net_sock_path = if self.network_mode.is_gvproxy() {
            let gvproxy_sock = instance_dir.join("gvproxy.sock");
            Some(gvproxy_sock.to_string_lossy().to_string())
        } else if let Some(p) = self.network_mode.unix_socket_path() {
            Some(p.to_string_lossy().to_string())
        } else {
            self.net_sock_path.clone()
        };

        Ok((
            guest_hostname,
            egress_proxy,
            proxy_port,
            final_net_sock_path,
        ))
    }

    /// Prepares kernel payload transparently compressing raw AArch64 payloads on macOS Apple Silicon.
    pub(crate) fn prepare_boot_payload(
        &self,
        instance_dir: &Path,
    ) -> Option<crate::types::BootPayload> {
        let mut final_boot_payload = self.boot_payload.clone();
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        if let Some(crate::types::BootPayload::Kernel(ref mut k)) = final_boot_payload {
            let is_gz = if let Ok(mut f) = std::fs::File::open(&k.kernel_path) {
                use std::io::Read;
                let mut magic = [0u8; 2];
                f.read_exact(&mut magic).is_ok() && magic == [0x1f, 0x8b]
            } else {
                false
            };

            if !is_gz {
                let compressed_path = instance_dir.join("kernel_payload.gz");
                if let Ok(mut src) = std::fs::File::open(&k.kernel_path) {
                    if let Ok(dst) = std::fs::File::create(&compressed_path) {
                        use flate2::write::GzEncoder;
                        use flate2::Compression;
                        let mut enc = GzEncoder::new(dst, Compression::fast());
                        if std::io::copy(&mut src, &mut enc).is_ok() && enc.finish().is_ok() {
                            k.kernel_path = compressed_path;
                            k.kernel_format = krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ;
                        }
                    }
                }
            } else {
                k.kernel_format = krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ;
            }
        }
        final_boot_payload
    }

    /// Generates runner configuration JSON and locates the signed runner binary.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_runner_config(
        &self,
        instance_dir: &Path,
        instance_rootfs: &Path,
        final_boot_payload: Option<crate::types::BootPayload>,
        final_net_sock_path: Option<String>,
        final_virtiofs_mounts: Vec<VirtioFsMount>,
        final_console_log: Option<PathBuf>,
        proxy_port: Option<u16>,
        guest_hostname: String,
    ) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let runner_binary = match &self.runner_path {
            Some(p) => {
                ensure_runner_signed(p)?;
                p.clone()
            }
            None => find_runner_binary()?,
        };

        let supervisor_sock = instance_dir.join("supervisor.sock");
        let runner_cfg = RunnerConfig {
            root_path: instance_rootfs.to_path_buf(),
            num_vcpus: self.vcpus,
            ram_mib: self.ram_mib,
            boot_payload: final_boot_payload,
            disks: self.disks.clone(),
            port_forwards: self.port_forwards.clone(),
            net_sock_path: final_net_sock_path,
            virtiofs_mounts: final_virtiofs_mounts,
            vsock_ports: self.vsock_ports.clone(),
            console_log_path: final_console_log,
            log_level: self.log_level,
            interactive: self.interactive,
            tty: self.tty,
            no_network: self.no_network,
            gvproxy: self.network_mode.is_gvproxy(),
            netns: self.network_mode.netns_path().map(|p| p.to_path_buf()),
            rlimits: self.rlimits.clone(),
            detach: self.detach,
            dax_window_size_bytes: self.dax_window_size,
            image_acceleration: self.image_acceleration.clone(),
            gpu: self.gpu,
            gpu_shm_size_bytes: self.gpu_shm_size,
            gpu_flags: self.gpu_flags,
            sandbox: self.sandbox,
            allow_hosts: self.allow_hosts.clone(),
            secrets: self.secrets.clone(),
            max_tokens: self.max_tokens,
            proxy_port,
            mac_address: self.mac_address.clone(),
            mtu: self.mtu,
            dns_servers: self.dns_servers.clone(),
            hostname: Some(guest_hostname),
            supervisor_sock_path: Some(supervisor_sock.clone()),
            fs_overlays: self.fs_overlays.clone(),
            smbios_oem_strings: self.smbios_oem_strings.clone(),
            balloon_reporting_mode: self.balloon_reporting_mode,
        };

        let runner_cfg_path = instance_dir.join("runner_config.json");
        let cfg_json = serde_json::to_string_pretty(&runner_cfg)?;
        std::fs::write(&runner_cfg_path, &cfg_json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&runner_cfg_path, std::fs::Permissions::from_mode(0o600));
        }

        Ok((runner_binary, runner_cfg_path, supervisor_sock))
    }

    /// Spawns the microVM runner child process with configured I/O and process credentials.
    pub(crate) fn spawn_runner_process(
        &self,
        runner_binary: &Path,
        runner_cfg_path: &Path,
        final_console_log: Option<&Path>,
    ) -> Result<(Child, Option<RawModeGuard>)> {
        tracing::info!("Spawning microvm runner (PID isolated)...");

        let mut cmd = Command::new(runner_binary);
        cmd.arg("--config").arg(runner_cfg_path);
        if !self.detach {
            cmd.kill_on_drop(true);
        }

        if self.detach {
            cmd.stdin(std::process::Stdio::null());
            if let Some(log_path) = final_console_log {
                if let Some(parent) = log_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(log_path)?;
                let err_file = file.try_clone()?;
                cmd.stdout(file);
                cmd.stderr(err_file);
            } else {
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
            }

            #[cfg(unix)]
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        } else {
            if self.interactive || self.tty {
                cmd.stdin(std::process::Stdio::inherit());
            } else {
                cmd.stdin(std::process::Stdio::null());
            }
            cmd.stdout(std::process::Stdio::inherit());
            cmd.stderr(std::process::Stdio::inherit());
        }

        if !self.detach {
            cmd.kill_on_drop(true);
        }

        let raw_guard = if self.tty && !self.detach {
            Some(RawModeGuard::new()?)
        } else {
            None
        };

        let child = cmd.spawn().with_context(|| {
            format!("Failed to spawn runner binary {}", runner_binary.display())
        })?;

        Ok((child, raw_guard))
    }

    /// Performs supervisor socket readiness handshake, registers VM instance state, and returns the running `MicroVm`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn wait_supervisor_ready(
        &self,
        mut child: Child,
        supervisor_sock: &Path,
        instance_id: &str,
        instance_dir: &Path,
        data_dir: &Path,
        raw_guard: Option<RawModeGuard>,
        egress_proxy: Option<crate::net::EgressProxyServer>,
    ) -> Result<MicroVm> {
        let connect_timeout = Duration::from_secs(3);
        let start_time = std::time::Instant::now();
        let mut supervisor_ready = false;
        while start_time.elapsed() < connect_timeout {
            if supervisor_sock.exists() {
                if let Ok(mut stream) = tokio::net::UnixStream::connect(supervisor_sock).await {
                    if let Ok(Ok(msg)) = tokio::time::timeout(
                        Duration::from_millis(500),
                        crate::protocol::read_frame_async(&mut stream),
                    )
                    .await
                    {
                        if matches!(msg.payload, crate::protocol::MessagePayload::Ready { .. }) {
                            supervisor_ready = true;
                            break;
                        }
                    }
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                if status.success() {
                    let pid = child.id().unwrap_or(0);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    let vm_state = crate::state::VmState {
                        id: instance_id.to_string(),
                        pid,
                        image: self.image.clone(),
                        created_at: now,
                        port_forwards: self.port_forwards.clone(),
                        instance_dir: instance_dir.to_path_buf(),
                        status: crate::state::VmStatus::Stopped,
                        vcpus: Some(self.vcpus),
                        memory_mib: Some(self.ram_mib),
                    };
                    if let Err(e) = crate::state::StateManager::save(data_dir, &vm_state) {
                        tracing::warn!(instance_id = %instance_id, error = %e, "Failed to persist stopped VM state to state manager");
                    }

                    return Ok(MicroVm {
                        id: instance_id.to_string(),
                        child,
                        instance_dir: instance_dir.to_path_buf(),
                        data_dir: data_dir.to_path_buf(),
                        is_detached: self.detach,
                        supervisor_sock: supervisor_sock.to_path_buf(),
                        _raw_guard: raw_guard,
                        _proxy: egress_proxy,
                        _gvproxy: None,
                    });
                } else {
                    bail!("microvm-runner exited with error status: {status}");
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !supervisor_ready {
            tracing::debug!(
                "Supervisor socket did not signal Ready within timeout; proceeding in fallback mode"
            );
        }

        let pid = child.id().unwrap_or(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let vm_state = crate::state::VmState {
            id: instance_id.to_string(),
            pid,
            image: self.image.clone(),
            created_at: now,
            port_forwards: self.port_forwards.clone(),
            instance_dir: instance_dir.to_path_buf(),
            status: crate::state::VmStatus::Running,
            vcpus: Some(self.vcpus),
            memory_mib: Some(self.ram_mib),
        };
        if let Err(e) = crate::state::StateManager::save(data_dir, &vm_state) {
            tracing::warn!(instance_id = %instance_id, error = %e, "Failed to persist running VM state to state manager");
        }

        Ok(MicroVm {
            id: instance_id.to_string(),
            child,
            instance_dir: instance_dir.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            is_detached: self.detach,
            supervisor_sock: supervisor_sock.to_path_buf(),
            _raw_guard: raw_guard,
            _proxy: egress_proxy,
            _gvproxy: None,
        })
    }

    /// Pulls the image (if not cached), creates a CoW rootfs snapshot, prepares .krun_config.json,
    /// and spawns the microVM runner subprocess.
    pub async fn run(self) -> Result<MicroVm> {
        let data_dir = self.resolve_data_dir();
        if self.perform_preflight {
            self.run_preflight(&data_dir)?;
        }

        let is_direct_boot = matches!(
            self.boot_payload,
            Some(crate::types::BootPayload::Kernel(_))
                | Some(crate::types::BootPayload::Firmware(_))
                | Some(crate::types::BootPayload::Unikernel(_))
        );

        let (instance_id, instance_dir, instance_rootfs) = self.generate_instance(&data_dir)?;
        self.prepare_rootfs(&instance_id, &instance_rootfs, &data_dir, is_direct_boot)
            .await?;

        let final_virtiofs_mounts = self
            .prepare_virtiofs_mounts(&instance_dir, &instance_rootfs, &data_dir)
            .await?;

        let (guest_hostname, egress_proxy, proxy_port, final_net_sock_path) = self
            .configure_networking(&instance_id, &instance_dir, &instance_rootfs)
            .await?;

        let final_boot_payload = self.prepare_boot_payload(&instance_dir);

        let final_console_log = if self.detach {
            Some(
                self.console_log_path
                    .clone()
                    .unwrap_or_else(|| instance_dir.join("console.log")),
            )
        } else {
            self.console_log_path.clone()
        };

        let (runner_binary, runner_cfg_path, supervisor_sock) = self.prepare_runner_config(
            &instance_dir,
            &instance_rootfs,
            final_boot_payload,
            final_net_sock_path,
            final_virtiofs_mounts,
            final_console_log.clone(),
            proxy_port,
            guest_hostname,
        )?;

        let (child, raw_guard) = self.spawn_runner_process(
            &runner_binary,
            &runner_cfg_path,
            final_console_log.as_deref(),
        )?;

        self.wait_supervisor_ready(
            child,
            &supervisor_sock,
            &instance_id,
            &instance_dir,
            &data_dir,
            raw_guard,
            egress_proxy,
        )
        .await
    }
}
