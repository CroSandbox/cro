// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use super::kernel::detect_kernel_format;
use crate::PortForward;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub use crate::types::{
    ArtifactMount, BootPayload, CowWorkspace, DiskAttachment, FirmwarePayload,
    FreePageReportingMode, FsOverlayEntry, KernelPayload, ModelAttachment, RunnerConfig,
    VirtioFsMount, VsockPort,
};

#[must_use = "MicroVmBuilder must be configured and executed by calling .run()"]
pub struct MicroVmBuilder {
    pub(crate) image: String,
    pub(crate) vcpus: u8,
    pub(crate) ram_mib: u32,
    pub(crate) cmd_override: Option<Vec<String>>,
    pub(crate) env_vars: Vec<String>,
    pub(crate) workdir: Option<String>,
    pub(crate) port_forwards: Vec<PortForward>,
    pub(crate) virtiofs_mounts: Vec<VirtioFsMount>,
    pub(crate) artifacts: Vec<ArtifactMount>,
    pub(crate) models: Vec<ModelAttachment>,
    pub(crate) cow_workspaces: Vec<CowWorkspace>,
    pub(crate) vsock_ports: Vec<VsockPort>,
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) runner_path: Option<PathBuf>,
    pub(crate) console_log_path: Option<PathBuf>,
    pub(crate) perform_preflight: bool,
    pub(crate) log_level: Option<u32>,
    pub(crate) interactive: bool,
    pub(crate) tty: bool,
    pub(crate) detach: bool,
    pub(crate) no_network: bool,
    pub(crate) rlimits: Option<String>,
    pub(crate) rootfs_override: Option<PathBuf>,
    pub(crate) bundle_dir: Option<PathBuf>,
    pub(crate) net_sock_path: Option<String>,
    pub(crate) dns_servers: Vec<String>,
    pub(crate) hostname: Option<String>,
    pub(crate) network_mode: crate::net::NetworkMode,
    pub(crate) file_mounts: Vec<(PathBuf, PathBuf)>,
    pub(crate) dax_window_size: Option<u64>,
    pub(crate) image_acceleration: Option<crate::acceleration::ImageAcceleration>,
    pub(crate) gpu: bool,
    pub(crate) gpu_shm_size: Option<u64>,
    pub(crate) gpu_flags: Option<u32>,
    pub(crate) sandbox: bool,
    pub(crate) allow_hosts: Vec<String>,
    pub(crate) default_deny: bool,
    pub(crate) secrets: Vec<(String, String)>,
    pub(crate) max_tokens: Option<u64>,
    pub(crate) boot_payload: Option<BootPayload>,
    pub(crate) disks: Vec<DiskAttachment>,
    pub(crate) mac_address: Option<String>,
    pub(crate) mtu: Option<usize>,
    pub(crate) extra_hosts: Vec<(String, String)>,
    pub(crate) shm_size: Option<String>,
    pub(crate) approval_policy: Option<crate::net::ApprovalPolicy>,
    pub(crate) approval_store: Option<std::sync::Arc<crate::net::ApprovalStore>>,
    pub(crate) fs_overlays: Vec<FsOverlayEntry>,
    pub(crate) smbios_oem_strings: Vec<String>,
    pub(crate) balloon_reporting_mode: Option<FreePageReportingMode>,
    pub(crate) rosetta: bool,
    pub(crate) platform: Option<String>,
    pub(crate) instance_id: Option<String>,
}

impl MicroVmBuilder {
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            vcpus: 2,
            ram_mib: 512,
            cmd_override: None,
            env_vars: Vec::new(),
            workdir: None,
            port_forwards: Vec::new(),
            virtiofs_mounts: Vec::new(),
            artifacts: Vec::new(),
            models: Vec::new(),
            cow_workspaces: Vec::new(),
            vsock_ports: Vec::new(),
            data_dir: None,
            runner_path: None,
            console_log_path: None,
            perform_preflight: true,
            log_level: None,
            interactive: false,
            tty: false,
            detach: false,
            no_network: false,
            rlimits: None,
            rootfs_override: None,
            bundle_dir: None,
            net_sock_path: None,
            dns_servers: Vec::new(),
            hostname: None,
            network_mode: crate::net::NetworkMode::Tsi,
            file_mounts: Vec::new(),
            dax_window_size: None,
            image_acceleration: None,
            gpu: false,
            gpu_shm_size: None,
            gpu_flags: None,
            sandbox: true,
            allow_hosts: Vec::new(),
            default_deny: false,
            secrets: Vec::new(),
            max_tokens: None,
            boot_payload: None,
            disks: Vec::new(),
            mac_address: None,
            mtu: None,
            extra_hosts: Vec::new(),
            shm_size: None,
            approval_policy: None,
            approval_store: None,
            fs_overlays: Vec::new(),
            smbios_oem_strings: Vec::new(),
            balloon_reporting_mode: None,
            rosetta: false,
            platform: None,
            instance_id: None,
        }
    }

    /// Creates a MicroVmBuilder from an unpacked OCI runtime bundle directory (containing config.json and rootfs/).
    pub fn from_bundle(bundle_dir: impl AsRef<Path>) -> Result<Self> {
        let bundle = crate::bundle::OciBundle::load(bundle_dir)?;
        let mut builder = Self::new(format!("bundle:{}", bundle.bundle_dir.display()))
            .virtiofs_mounts(bundle.virtiofs_mounts);

        if let Some(cpus) = bundle.vcpus {
            builder = builder.cpus(cpus);
        }
        if let Some(ram) = bundle.ram_mib {
            builder = builder.memory_mb(ram);
        }
        builder.file_mounts = bundle.file_mounts;

        if !bundle.cmd.is_empty() {
            builder = builder.cmd(bundle.cmd);
        }
        if !bundle.env.is_empty() {
            builder.env_vars = bundle.env;
        }
        if let Some(w) = bundle.workdir {
            builder = builder.workdir(w);
        }
        if let Some(lim) = bundle.rlimits {
            builder = builder.rlimits(lim);
        }
        let network_annotation = bundle.spec.annotations().as_ref().and_then(|a| {
            a.get("krun.network")
                .or_else(|| a.get("krun.io/network-mode"))
                .or_else(|| a.get("io.katacontainers.config.hypervisor.network_model"))
        });

        if let Some(net_model) = network_annotation {
            if net_model == "gvproxy" {
                builder = builder.network_mode(crate::net::NetworkMode::Gvproxy);
            } else if net_model == "tsi" {
                builder = builder.network_mode(crate::net::NetworkMode::Tsi);
            } else if net_model == "none" {
                builder = builder.network_mode(crate::net::NetworkMode::None);
            }
        } else if let Some(netns) = bundle.netns_path {
            builder = builder.network_mode(crate::net::NetworkMode::Cni {
                netns,
                socket_path: None,
            });
        } else {
            // Default to gvproxy for rootless / desktop execution when CNI netns is omitted
            builder = builder.network_mode(crate::net::NetworkMode::Gvproxy);
        }

        if let Some(allow_str) = bundle.spec.annotations().as_ref().and_then(|a| {
            a.get("krun.network.allow")
                .or_else(|| a.get("krun.io/allow-egress"))
        }) {
            for target in allow_str.split(',') {
                let trimmed = target.trim();
                if !trimmed.is_empty() {
                    builder = builder.allow_host(trimmed);
                }
            }
        }
        for pf in bundle.port_forwards {
            builder = builder.port_forward(pf.host, pf.guest);
        }
        if let Some(ref ann) = bundle.spec.annotations() {
            if let Some(gpu_str) = ann.get("krun.gpu").or_else(|| ann.get("krun.io/gpu")) {
                if gpu_str == "true" || gpu_str == "1" {
                    builder = builder.gpu(true);
                }
            }
            if let Some(dax_str) = ann
                .get("krun.dax")
                .or_else(|| ann.get("krun.io/dax-window-size"))
            {
                if dax_str == "true" || dax_str == "1" {
                    builder = builder.dax_window_size(2 * 1024 * 1024 * 1024);
                } else if !dax_str.is_empty() && dax_str != "false" {
                    if let Ok(bytes) = crate::config::parse_size_to_bytes(dax_str) {
                        builder = builder.dax_window_size(bytes);
                    }
                }
            }
            if let Some(sb_str) = ann
                .get("krun.sandbox")
                .or_else(|| ann.get("krun.io/sandbox"))
            {
                if sb_str == "false" || sb_str == "0" {
                    builder = builder.sandbox(false);
                }
            }
            if let Some(balloon_str) = ann
                .get("krun.balloon.reporting")
                .or_else(|| ann.get("krun.io/balloon-reporting"))
            {
                if let Ok(mode) = balloon_str.parse() {
                    builder = builder.balloon_reporting_mode(mode);
                }
            }
            if let Some(net_policy) = ann
                .get("krun.net.policy")
                .or_else(|| ann.get("krun.io/net-policy"))
            {
                if net_policy == "default-deny" || net_policy == "deny-all" {
                    builder = builder.default_deny(true);
                }
            }
        }
        builder.rootfs_override = Some(bundle.rootfs_path);
        builder.bundle_dir = Some(bundle.bundle_dir);
        Ok(builder)
    }

    pub fn file_mount(mut self, src: impl Into<PathBuf>, dest: impl Into<PathBuf>) -> Self {
        self.file_mounts.push((src.into(), dest.into()));
        self
    }

    pub fn file_mounts(mut self, mounts: Vec<(PathBuf, PathBuf)>) -> Self {
        self.file_mounts.extend(mounts);
        self
    }

    pub fn get_image(&self) -> &str {
        &self.image
    }

    pub fn get_vcpus(&self) -> u8 {
        self.vcpus
    }

    pub fn get_ram_mib(&self) -> u32 {
        self.ram_mib
    }

    pub fn get_port_forwards(&self) -> &[PortForward] {
        &self.port_forwards
    }

    pub fn get_file_mounts(&self) -> &[(PathBuf, PathBuf)] {
        &self.file_mounts
    }

    pub fn get_gpu(&self) -> bool {
        self.gpu
    }

    pub fn get_gpu_shm_size(&self) -> Option<u64> {
        self.gpu_shm_size
    }

    pub fn net_sock_path(mut self, path: impl Into<String>) -> Self {
        self.net_sock_path = Some(path.into());
        self
    }

    pub fn virtiofs_mounts(mut self, mounts: Vec<VirtioFsMount>) -> Self {
        self.virtiofs_mounts.extend(mounts);
        self
    }

    pub fn virtiofs_mount(mut self, mount: VirtioFsMount) -> Self {
        self.virtiofs_mounts.push(mount);
        self
    }

    pub fn balloon_reporting_mode(mut self, mode: FreePageReportingMode) -> Self {
        self.balloon_reporting_mode = Some(mode);
        self
    }

    /// Attaches an arbitrary OCI artifact (e.g. model weights, dataset, toolchain) from an OCI registry,
    /// caching it locally and mounting it via VirtioFS with the specified tag.
    pub fn attach_artifact(
        mut self,
        reference: impl Into<String>,
        tag: impl Into<String>,
        read_only: bool,
    ) -> Self {
        self.artifacts.push(ArtifactMount {
            reference: reference.into(),
            tag: tag.into(),
            read_only,
        });
        self
    }

    /// Attaches an AI/ML model conforming to the CNCF ModelPack Specification.
    ///
    /// Automatically mounts weights via VirtioFS, injects `MODEL_*` environment variables,
    /// and provisions a VirtioFS DAX window for zero-copy shared memory access.
    pub fn model(mut self, reference: impl Into<String>, mount_path: Option<String>) -> Self {
        self.models
            .push(ModelAttachment::new(reference, mount_path));
        self
    }

    /// Mounts a host directory as an instant Copy-on-Write (CoW) sandbox via VirtioFS.
    /// Any modifications made inside the microVM are isolated and will not affect the host directory.
    pub fn workspace_cow(mut self, host_path: impl AsRef<Path>, tag: impl Into<String>) -> Self {
        self.cow_workspaces.push(CowWorkspace {
            host_path: host_path.as_ref().to_path_buf(),
            tag: tag.into(),
        });
        self
    }

    pub fn cpus(mut self, vcpus: u8) -> Self {
        self.vcpus = vcpus;
        self
    }

    pub fn memory_mb(mut self, ram_mib: u32) -> Self {
        self.ram_mib = ram_mib;
        self
    }

    pub fn cmd(mut self, cmd: Vec<String>) -> Self {
        self.cmd_override = Some(cmd);
        self
    }

    pub fn env(mut self, key: &str, val: &str) -> Self {
        self.env_vars.push(format!("{key}={val}"));
        self
    }

    pub fn workdir(mut self, dir: impl Into<String>) -> Self {
        self.workdir = Some(dir.into());
        self
    }

    pub fn port_forward(mut self, host: u16, guest: u16) -> Self {
        self.port_forwards.push(PortForward::new(host, guest));
        self
    }

    pub fn virtiofs(mut self, tag: &str, path: impl Into<PathBuf>, read_only: bool) -> Self {
        self.virtiofs_mounts
            .push(VirtioFsMount::new(tag, path, read_only));
        self
    }

    /// Sets the VirtioFS DAX (Direct Access) shared memory window size in bytes.
    pub fn dax_window_size(mut self, bytes: u64) -> Self {
        self.dax_window_size = Some(bytes);
        self
    }

    /// Sets the VirtioFS DAX window size from a human-readable string (e.g. "4G", "512M", "1024K").
    pub fn dax_window_size_str(mut self, s: &str) -> Result<Self> {
        self.dax_window_size = Some(crate::config::parse_size_to_bytes(s)?);
        Ok(self)
    }

    /// Configures Dragonfly Nydus RAFSv6 / EROFS image acceleration and lazy loading.
    pub fn image_acceleration(
        mut self,
        acceleration: crate::acceleration::ImageAcceleration,
    ) -> Self {
        self.image_acceleration = Some(acceleration);
        self
    }

    /// Toggles on-demand lazy loading (RAFSv6 chunk streaming).
    pub fn lazy_load(mut self, enabled: bool) -> Self {
        let mut acc = self.image_acceleration.take().unwrap_or_default();
        acc.lazy_load = enabled;
        self.image_acceleration = Some(acc);
        self
    }

    /// Configures explicit path to a local RAFS v6 or EROFS metadata bootstrap image.
    pub fn nydus_bootstrap(mut self, path: impl Into<PathBuf>) -> Self {
        let mut acc = self.image_acceleration.take().unwrap_or_default();
        acc.bootstrap_path = Some(path.into());
        self.image_acceleration = Some(acc);
        self
    }

    /// Sets the directory where downloaded chunk blobs are cached and deduplicated.
    pub fn chunk_cache_dir(mut self, path: impl Into<PathBuf>) -> Self {
        let mut acc = self.image_acceleration.take().unwrap_or_default();
        acc.chunk_cache_dir = Some(path.into());
        self.image_acceleration = Some(acc);
        self
    }

    /// Sets the chunk or macro-chunk size for streaming (e.g. "4M", "64M").
    pub fn chunk_size_str(mut self, s: &str) -> Result<Self> {
        let bytes = crate::config::parse_size_to_bytes(s)?;
        let mut acc = self.image_acceleration.take().unwrap_or_default();
        acc.chunk_size_bytes = Some(bytes);
        self.image_acceleration = Some(acc);
        Ok(self)
    }

    pub fn get_image_acceleration(&self) -> Option<&crate::acceleration::ImageAcceleration> {
        self.image_acceleration.as_ref()
    }

    pub fn vsock_port(mut self, port: u32, path: impl Into<PathBuf>) -> Self {
        self.vsock_ports.push(VsockPort::new(port, path));
        self
    }

    pub fn data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(path.into());
        self
    }

    pub fn runner_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.runner_path = Some(path.into());
        self
    }

    pub fn console_log(mut self, path: impl Into<PathBuf>) -> Self {
        self.console_log_path = Some(path.into());
        self
    }

    pub fn preflight(mut self, enable: bool) -> Self {
        self.perform_preflight = enable;
        self
    }

    pub fn log_level(mut self, level: u32) -> Self {
        self.log_level = Some(level);
        self
    }

    pub fn interactive(mut self, enabled: bool) -> Self {
        self.interactive = enabled;
        self
    }

    pub fn tty(mut self, enabled: bool) -> Self {
        self.tty = enabled;
        self
    }

    pub fn detach(mut self, enabled: bool) -> Self {
        self.detach = enabled;
        self
    }

    pub fn no_network(mut self, enabled: bool) -> Self {
        self.no_network = enabled;
        self
    }

    pub fn rlimits(mut self, rlimits: impl Into<String>) -> Self {
        self.rlimits = Some(rlimits.into());
        self
    }

    /// Enable or disable hardware-accelerated virtio-gpu (Metal on Apple Silicon, DRM on Linux).
    pub fn gpu(mut self, enabled: bool) -> Self {
        self.gpu = enabled;
        self
    }

    /// Configure shared memory vRAM host window size in bytes for virtio-gpu device.
    pub fn gpu_shm_size(mut self, bytes: u64) -> Self {
        self.gpu_shm_size = Some(bytes);
        self
    }

    /// Custom virglrenderer flags (defaults to Venus/Metal acceleration flags).
    pub fn gpu_flags(mut self, flags: u32) -> Self {
        self.gpu_flags = Some(flags);
        self
    }

    /// Enables or disables zero-trust host sandboxing (Landlock LSM and Seccomp syscall filtering on Linux).
    pub fn sandbox(mut self, enabled: bool) -> Self {
        self.sandbox = enabled;
        self
    }

    /// Configures the size of POSIX shared memory (/dev/shm) mounted inside the guest (e.g. "64m", "512m", "2g").
    pub fn shm_size(mut self, size: impl Into<String>) -> Self {
        self.shm_size = Some(size.into());
        self
    }

    /// Restricts outbound network egress to explicitly permitted `host:port` destinations (default-deny egress).
    pub fn allow_host(mut self, host: impl Into<String>) -> Self {
        self.allow_hosts.push(host.into());
        self
    }

    /// Adds multiple allowed outbound destinations.
    pub fn allow_hosts(mut self, hosts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        for h in hosts {
            self.allow_hosts.push(h.into());
        }
        self
    }

    /// Enforces an explicit default-deny network egress policy.
    pub fn default_deny(mut self, enabled: bool) -> Self {
        self.default_deny = enabled;
        self
    }

    /// Injects a secret using zero-trust in-flight substitution:
    /// The guest sees `KEY=krun-secret:KEY`, while the host proxy substitutes the real secret on outbound requests.
    pub fn secret(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.secrets.push((key.into(), value.into()));
        self
    }

    /// Sets a hard ceiling on cumulative LLM tokens (prompt + completion) consumed by the microVM.
    pub fn max_tokens(mut self, limit: u64) -> Self {
        self.max_tokens = Some(limit);
        self
    }

    /// Configures Human-in-the-Loop (HITL) approval policy for the microVM.
    pub fn approval_policy(mut self, policy: crate::net::ApprovalPolicy) -> Self {
        self.approval_policy = Some(policy);
        self
    }

    /// Sets a shared ApprovalStore for tracking and deciding on intercepted requests.
    pub fn approval_store(mut self, store: std::sync::Arc<crate::net::ApprovalStore>) -> Self {
        self.approval_store = Some(store);
        self
    }

    /// Enables Human-in-the-Loop (HITL) manual approval for all mutating / destructive agent operations.
    pub fn hitl(mut self, enabled: bool) -> Self {
        if enabled {
            self.approval_policy = Some(
                crate::net::ApprovalPolicy::new()
                    .with_trigger(crate::net::ApprovalTrigger::AllMutating)
                    .with_trigger(crate::net::ApprovalTrigger::DestructiveKeywords),
            );
        } else {
            self.approval_policy = None;
        }
        self
    }

    /// Appends a DNS nameserver (or comma/semicolon-separated nameservers, e.g. "8.8.8.8,1.1.1.1").
    pub fn dns(mut self, dns: impl Into<String>) -> Self {
        self.dns_servers.push(dns.into());
        self
    }

    /// Sets multiple DNS nameservers.
    pub fn dns_servers(mut self, servers: Vec<String>) -> Self {
        self.dns_servers.extend(servers);
        self
    }

    /// Sets the guest microVM hostname.
    pub fn hostname(mut self, name: impl Into<String>) -> Self {
        self.hostname = Some(name.into());
        self
    }

    /// Sets the network mode (TSI, UnixStream, or None) for high-performance guest isolation.
    pub fn network_mode(mut self, mode: crate::net::NetworkMode) -> Self {
        match &mode {
            crate::net::NetworkMode::None => {
                self.no_network = true;
            }
            crate::net::NetworkMode::UnixStream(path) => {
                self.no_network = false;
                self.net_sock_path = Some(path.to_string_lossy().to_string());
            }
            crate::net::NetworkMode::Cni { netns, socket_path } => {
                self.no_network = false;
                if let Some(sock) = socket_path {
                    self.net_sock_path = Some(sock.to_string_lossy().to_string());
                } else {
                    let default_sock = netns.parent().unwrap_or(netns).join("cni-krun.sock");
                    if default_sock.exists() {
                        self.net_sock_path = Some(default_sock.to_string_lossy().to_string());
                    }
                }
            }
            crate::net::NetworkMode::Tsi | crate::net::NetworkMode::Gvproxy => {
                self.no_network = false;
                self.net_sock_path = None;
            }
        }
        self.network_mode = mode;
        self
    }

    /// Sets a custom MAC address for the guest virtio-net interface (e.g. 5a:94:ef:e4:0c:ee).
    pub fn mac_address(mut self, mac: impl Into<String>) -> Self {
        self.mac_address = Some(mac.into());
        self
    }

    /// Sets the MTU for the guest virtual network interface (default: 1500).
    pub fn mtu(mut self, mtu: usize) -> Self {
        self.mtu = Some(mtu);
        self
    }

    /// Adds extra hostname-to-IP alias mappings to `/etc/hosts` inside guest (e.g. for Compose inter-service communication).
    pub fn extra_hosts(mut self, hosts: Vec<(String, String)>) -> Self {
        self.extra_hosts.extend(hosts);
        self
    }

    /// Sets explicit multi-boot payload configuration.
    pub fn boot_payload(mut self, payload: BootPayload) -> Self {
        self.boot_payload = Some(payload);
        self
    }

    /// Configures direct kernel boot (e.g. Linux direct bzImage/ELF, NetBSD, FreeBSD, Asahi Linux kernel).
    /// Format is automatically detected from file magic if not specified.
    pub fn kernel(
        self,
        kernel_path: impl Into<PathBuf>,
        initramfs: Option<PathBuf>,
        cmdline: Option<String>,
    ) -> Self {
        let p = kernel_path.into();
        let fmt = detect_kernel_format(&p);
        self.kernel_with_format(p, fmt, initramfs, cmdline)
    }

    /// Configures direct kernel boot with an explicit kernel format.
    pub fn kernel_with_format(
        mut self,
        kernel_path: impl Into<PathBuf>,
        kernel_format: u32,
        initramfs: Option<PathBuf>,
        cmdline: Option<String>,
    ) -> Self {
        self.boot_payload = Some(BootPayload::Kernel(KernelPayload {
            kernel_path: kernel_path.into(),
            kernel_format,
            initramfs,
            cmdline,
        }));
        self
    }

    /// Configures UEFI firmware boot (e.g. booting full disk images via EDK2 / KRUN_EFI.fd).
    pub fn firmware(mut self, firmware_path: impl Into<PathBuf>) -> Self {
        self.boot_payload = Some(BootPayload::Firmware(FirmwarePayload {
            firmware_path: firmware_path.into(),
        }));
        self
    }

    /// Configures unikernel boot (e.g. Unikraft, Nanos, OSv, Solo5/Mirage).
    pub fn unikernel(mut self, kernel_path: impl Into<PathBuf>, cmdline: Option<String>) -> Self {
        self.boot_payload = Some(BootPayload::Unikernel(KernelPayload {
            kernel_path: kernel_path.into(),
            kernel_format: krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_ELF,
            initramfs: None,
            cmdline,
        }));
        self
    }

    /// Attaches an additional block disk device to the microVM.
    pub fn disk(
        mut self,
        id: impl Into<String>,
        path: impl Into<PathBuf>,
        read_only: bool,
    ) -> Self {
        self.disks.push(DiskAttachment::new(id, path, read_only));
        self
    }

    /// Attaches an existing DiskAttachment directly (with custom format or sync mode).
    pub fn disk_attachment(mut self, attachment: DiskAttachment) -> Self {
        self.disks.push(attachment);
        self
    }

    /// Attaches an in-memory virtual filesystem overlay entry.
    pub fn fs_overlay(mut self, entry: FsOverlayEntry) -> Self {
        self.fs_overlays.push(entry);
        self
    }

    /// Injects an in-memory virtual overlay file into the guest filesystem.
    pub fn fs_overlay_file(
        mut self,
        fs_tag: impl Into<String>,
        path: impl Into<String>,
        data: impl Into<Vec<u8>>,
        mode: u32,
        one_shot: bool,
    ) -> Self {
        self.fs_overlays
            .push(FsOverlayEntry::new(fs_tag, path, data, mode, one_shot));
        self
    }

    /// Injects an SMBIOS OEM string into guest BIOS/firmware tables.
    pub fn smbios_oem_string(mut self, s: impl Into<String>) -> Self {
        self.smbios_oem_strings.push(s.into());
        self
    }

    /// Injects multiple SMBIOS OEM strings.
    pub fn smbios_oem_strings(mut self, strings: Vec<String>) -> Self {
        self.smbios_oem_strings.extend(strings);
        self
    }

    /// Enables or disables Apple Silicon Rosetta for Linux (x86_64 emulation).
    pub fn rosetta(mut self, enabled: bool) -> Self {
        self.rosetta = enabled;
        self
    }

    /// Returns whether Rosetta emulation is requested.
    pub fn get_rosetta(&self) -> bool {
        self.rosetta
    }

    /// Sets the target architecture platform (e.g. "linux/amd64", "linux/arm64").
    pub fn platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    /// Returns the configured platform if any.
    pub fn get_platform(&self) -> Option<&str> {
        self.platform.as_deref()
    }

    /// Explicitly assigns a fixed instance ID instead of auto-generating one.
    pub fn instance_id(mut self, id: impl Into<String>) -> Self {
        self.instance_id = Some(id.into());
        self
    }

    /// Returns the configured instance ID if explicitly set.
    pub fn get_instance_id(&self) -> Option<&str> {
        self.instance_id.as_deref()
    }

    pub fn get_virtiofs_mounts(&self) -> &[VirtioFsMount] {
        &self.virtiofs_mounts
    }

    pub fn get_extra_hosts(&self) -> &[(String, String)] {
        &self.extra_hosts
    }
}
