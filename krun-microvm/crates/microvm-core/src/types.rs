use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Port forwarding rule (host:guest).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForward {
    pub host: u16,
    pub guest: u16,
}

impl PortForward {
    pub fn new(host: u16, guest: u16) -> Self {
        Self { host, guest }
    }
}

/// VirtioFS cache policy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VirtioFsCachePolicy {
    Never = 0,
    #[default]
    Auto = 1,
    Always = 2,
}

impl VirtioFsCachePolicy {
    pub fn to_krun_sys(&self) -> u32 {
        match self {
            Self::Never => krun_sys::cache_policies::KRUN_CACHE_POLICY_NEVER,
            Self::Auto => krun_sys::cache_policies::KRUN_CACHE_POLICY_AUTO,
            Self::Always => krun_sys::cache_policies::KRUN_CACHE_POLICY_ALWAYS,
        }
    }
}

impl std::str::FromStr for VirtioFsCachePolicy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "never" | "none" => Ok(Self::Never),
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            other => Err(format!(
                "Unknown cache policy '{other}', expected 'never', 'auto', or 'always'"
            )),
        }
    }
}

impl std::fmt::Display for VirtioFsCachePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Never => write!(f, "never"),
            Self::Auto => write!(f, "auto"),
            Self::Always => write!(f, "always"),
        }
    }
}

/// VirtioFS permission semantics configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VirtioFsPermissionSemantics {
    #[default]
    Complete = 0,
    Simplified = 1,
}

impl VirtioFsPermissionSemantics {
    pub fn to_krun_sys(&self) -> u32 {
        match self {
            Self::Complete => {
                krun_sys::permission_semantics::KRUN_PERMISSION_SEMANTICS_LINUX_COMPLETE
            }
            Self::Simplified => {
                krun_sys::permission_semantics::KRUN_PERMISSION_SEMANTICS_LINUX_SIMPLIFIED
            }
        }
    }
}

impl std::str::FromStr for VirtioFsPermissionSemantics {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "complete" | "linux-complete" => Ok(Self::Complete),
            "simplified" | "linux-simplified" => Ok(Self::Simplified),
            other => Err(format!(
                "Unknown permission semantics '{other}', expected 'complete' or 'simplified'"
            )),
        }
    }
}

impl std::fmt::Display for VirtioFsPermissionSemantics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete => write!(f, "complete"),
            Self::Simplified => write!(f, "simplified"),
        }
    }
}

/// Memory balloon free page reporting mode configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FreePageReportingMode {
    #[default]
    Default = 0,
    Replace = 1,
}

impl FreePageReportingMode {
    pub fn to_krun_sys(&self) -> u32 {
        match self {
            Self::Default => {
                krun_sys::free_page_reporting_modes::KRUN_FREE_PAGE_REPORTING_MODE_DEFAULT
            }
            Self::Replace => {
                krun_sys::free_page_reporting_modes::KRUN_FREE_PAGE_REPORTING_MODE_REPLACE
            }
        }
    }
}

impl std::str::FromStr for FreePageReportingMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "default" => Ok(Self::Default),
            "replace" | "reclaim" => Ok(Self::Replace),
            other => Err(format!(
                "Unknown balloon reporting mode '{other}', expected 'default' or 'replace'"
            )),
        }
    }
}

impl std::fmt::Display for FreePageReportingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::Replace => write!(f, "replace"),
        }
    }
}

/// VirtioFS directory mount rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtioFsMount {
    pub tag: String,
    pub path: PathBuf,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub dax_window_size_bytes: Option<u64>,
    #[serde(default)]
    pub cache_policy: Option<VirtioFsCachePolicy>,
    #[serde(default)]
    pub permission_semantics: Option<VirtioFsPermissionSemantics>,
}

impl VirtioFsMount {
    pub fn new(tag: impl Into<String>, path: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            tag: tag.into(),
            path: path.into(),
            read_only,
            dax_window_size_bytes: None,
            cache_policy: None,
            permission_semantics: None,
        }
    }

    pub fn with_dax(mut self, bytes: u64) -> Self {
        self.dax_window_size_bytes = Some(bytes);
        self
    }

    pub fn with_cache_policy(mut self, policy: VirtioFsCachePolicy) -> Self {
        self.cache_policy = Some(policy);
        self
    }

    pub fn with_permission_semantics(mut self, semantics: VirtioFsPermissionSemantics) -> Self {
        self.permission_semantics = Some(semantics);
        self
    }
}

/// Vsock port mapping to a host UNIX socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VsockPort {
    pub port: u32,
    pub socket_path: PathBuf,
}

impl VsockPort {
    pub fn new(port: u32, socket_path: impl Into<PathBuf>) -> Self {
        Self {
            port,
            socket_path: socket_path.into(),
        }
    }
}

/// Supported disk formats for block devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiskFormat {
    #[default]
    Raw = 0,
    Qcow2 = 1,
    Vmdk = 2,
}

impl DiskFormat {
    pub fn from_path(path: &std::path::Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "qcow2" | "qcow" => Self::Qcow2,
            "vmdk" => Self::Vmdk,
            _ => Self::Raw,
        }
    }
}

/// Supported sync modes for block devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiskSyncMode {
    None = 0,
    #[default]
    Relaxed = 1,
    Full = 2,
}

/// Block disk device attachment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskAttachment {
    pub id: String,
    pub path: PathBuf,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub format: DiskFormat,
    #[serde(default)]
    pub sync_mode: DiskSyncMode,
    #[serde(default)]
    pub direct_io: bool,
    #[serde(default)]
    pub parallel_reads: bool,
    #[serde(default)]
    pub read_pool_size: Option<usize>,
}

impl DiskAttachment {
    pub fn new(id: impl Into<String>, path: impl Into<PathBuf>, read_only: bool) -> Self {
        let p = path.into();
        let format = DiskFormat::from_path(&p);
        Self {
            id: id.into(),
            path: p,
            read_only,
            format,
            sync_mode: DiskSyncMode::Relaxed,
            direct_io: false,
            parallel_reads: false,
            read_pool_size: None,
        }
    }

    pub fn with_format(mut self, format: DiskFormat) -> Self {
        self.format = format;
        self
    }

    pub fn with_sync_mode(mut self, sync_mode: DiskSyncMode) -> Self {
        self.sync_mode = sync_mode;
        self
    }

    pub fn with_direct_io(mut self, direct_io: bool) -> Self {
        self.direct_io = direct_io;
        self
    }

    pub fn with_parallel_reads(mut self, parallel_reads: bool) -> Self {
        self.parallel_reads = parallel_reads;
        self
    }

    pub fn with_read_pool_size(mut self, size: usize) -> Self {
        self.read_pool_size = Some(size);
        self
    }
}

/// In-memory overlay file entry for zero-disk virtual filesystem injection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsOverlayEntry {
    pub fs_tag: String,
    pub path: String,
    pub data: Vec<u8>,
    pub mode: u32,
    pub one_shot: bool,
}

impl FsOverlayEntry {
    pub fn new(
        fs_tag: impl Into<String>,
        path: impl Into<String>,
        data: impl Into<Vec<u8>>,
        mode: u32,
        one_shot: bool,
    ) -> Self {
        Self {
            fs_tag: fs_tag.into(),
            path: path.into(),
            data: data.into(),
            mode,
            one_shot,
        }
    }
}

/// Payload specification for direct kernel boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelPayload {
    pub kernel_path: PathBuf,
    #[serde(default)]
    pub kernel_format: u32,
    #[serde(default)]
    pub initramfs: Option<PathBuf>,
    #[serde(default)]
    pub cmdline: Option<String>,
}

/// Payload specification for UEFI firmware boot (e.g. FreeBSD/NetBSD with KRUN_EFI.fd).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwarePayload {
    pub firmware_path: PathBuf,
}

/// Multi-boot payload configuration for the microVM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum BootPayload {
    #[serde(rename = "oci")]
    Oci,
    #[serde(rename = "kernel")]
    Kernel(KernelPayload),
    #[serde(rename = "firmware")]
    Firmware(FirmwarePayload),
    #[serde(rename = "unikernel")]
    Unikernel(KernelPayload),
}

/// Configuration passed to microvm-runner as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub root_path: PathBuf,
    pub num_vcpus: u8,
    pub ram_mib: u32,
    #[serde(default)]
    pub boot_payload: Option<BootPayload>,
    #[serde(default)]
    pub disks: Vec<DiskAttachment>,
    #[serde(default)]
    pub port_forwards: Vec<PortForward>,
    #[serde(default)]
    pub net_sock_path: Option<String>,
    #[serde(default)]
    pub virtiofs_mounts: Vec<VirtioFsMount>,
    #[serde(default)]
    pub vsock_ports: Vec<VsockPort>,
    #[serde(default)]
    pub console_log_path: Option<PathBuf>,
    #[serde(default)]
    pub log_level: Option<u32>,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub tty: bool,
    #[serde(default)]
    pub no_network: bool,
    #[serde(default)]
    pub gvproxy: bool,
    #[serde(default)]
    pub netns: Option<PathBuf>,
    #[serde(default)]
    pub rlimits: Option<String>,
    #[serde(default)]
    pub detach: bool,
    #[serde(default)]
    pub dax_window_size_bytes: Option<u64>,
    #[serde(default)]
    pub image_acceleration: Option<crate::acceleration::ImageAcceleration>,
    #[serde(default)]
    pub gpu: bool,
    #[serde(default)]
    pub gpu_shm_size_bytes: Option<u64>,
    #[serde(default)]
    pub gpu_flags: Option<u32>,
    #[serde(default = "default_true")]
    pub sandbox: bool,
    #[serde(default)]
    pub allow_hosts: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<(String, String)>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub mac_address: Option<String>,
    #[serde(default)]
    pub mtu: Option<usize>,
    #[serde(default)]
    pub dns_servers: Vec<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub supervisor_sock_path: Option<PathBuf>,
    #[serde(default)]
    pub fs_overlays: Vec<FsOverlayEntry>,
    #[serde(default)]
    pub smbios_oem_strings: Vec<String>,
    #[serde(default)]
    pub balloon_reporting_mode: Option<FreePageReportingMode>,
}

fn default_true() -> bool {
    true
}

/// Specification for attaching an OCI artifact to a microVM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactMount {
    pub reference: String,
    pub tag: String,
    pub read_only: bool,
}

impl ArtifactMount {
    pub fn new(reference: impl Into<String>, tag: impl Into<String>, read_only: bool) -> Self {
        Self {
            reference: reference.into(),
            tag: tag.into(),
            read_only,
        }
    }
}

/// Specification for attaching a CNCF ModelPack AI model to a microVM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAttachment {
    pub reference: String,
    pub mount_path: Option<String>,
}

impl ModelAttachment {
    pub fn new(reference: impl Into<String>, mount_path: Option<String>) -> Self {
        Self {
            reference: reference.into(),
            mount_path,
        }
    }
}

/// Specification for attaching an isolated Copy-on-Write workspace to a microVM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CowWorkspace {
    pub host_path: PathBuf,
    pub tag: String,
}

impl CowWorkspace {
    pub fn new(host_path: impl Into<PathBuf>, tag: impl Into<String>) -> Self {
        Self {
            host_path: host_path.into(),
            tag: tag.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_disk_format_detection() {
        assert_eq!(
            DiskFormat::from_path(Path::new("ubuntu.qcow2")),
            DiskFormat::Qcow2
        );
        assert_eq!(
            DiskFormat::from_path(Path::new("arch.qcow")),
            DiskFormat::Qcow2
        );
        assert_eq!(
            DiskFormat::from_path(Path::new("vmware.vmdk")),
            DiskFormat::Vmdk
        );
        assert_eq!(
            DiskFormat::from_path(Path::new("alpine.raw")),
            DiskFormat::Raw
        );
        assert_eq!(
            DiskFormat::from_path(Path::new("data.img")),
            DiskFormat::Raw
        );
        assert_eq!(DiskFormat::from_path(Path::new("no_ext")), DiskFormat::Raw);
    }

    #[test]
    fn test_disk_attachment_defaults_and_builder() {
        let disk = DiskAttachment::new("disk0", "/var/lib/images/cloud.qcow2", false);
        assert_eq!(disk.id, "disk0");
        assert_eq!(disk.format, DiskFormat::Qcow2);
        assert_eq!(disk.sync_mode, DiskSyncMode::Relaxed);
        assert!(!disk.read_only);
        assert!(!disk.direct_io);

        let customized = disk
            .with_sync_mode(DiskSyncMode::Full)
            .with_direct_io(true)
            .with_parallel_reads(true)
            .with_read_pool_size(8);
        assert_eq!(customized.sync_mode, DiskSyncMode::Full);
        assert!(customized.direct_io);
        assert!(customized.parallel_reads);
        assert_eq!(customized.read_pool_size, Some(8));
    }

    #[test]
    fn test_virtiofs_mount_builder_and_options() {
        let mount = VirtioFsMount::new("shared", "/data", false)
            .with_dax(1024 * 1024 * 512)
            .with_cache_policy(VirtioFsCachePolicy::Always)
            .with_permission_semantics(VirtioFsPermissionSemantics::Simplified);

        assert_eq!(mount.tag, "shared");
        assert_eq!(mount.dax_window_size_bytes, Some(536870912));
        assert_eq!(mount.cache_policy, Some(VirtioFsCachePolicy::Always));
        assert_eq!(
            mount.permission_semantics,
            Some(VirtioFsPermissionSemantics::Simplified)
        );
    }

    #[test]
    fn test_cache_policy_and_permission_semantics_parsing() {
        use std::str::FromStr;

        assert_eq!(
            VirtioFsCachePolicy::from_str("never").unwrap(),
            VirtioFsCachePolicy::Never
        );
        assert_eq!(
            VirtioFsCachePolicy::from_str("auto").unwrap(),
            VirtioFsCachePolicy::Auto
        );
        assert_eq!(
            VirtioFsCachePolicy::from_str("always").unwrap(),
            VirtioFsCachePolicy::Always
        );
        assert!(VirtioFsCachePolicy::from_str("invalid").is_err());
        assert_eq!(VirtioFsCachePolicy::Always.to_string(), "always");

        assert_eq!(
            VirtioFsPermissionSemantics::from_str("complete").unwrap(),
            VirtioFsPermissionSemantics::Complete
        );
        assert_eq!(
            VirtioFsPermissionSemantics::from_str("simplified").unwrap(),
            VirtioFsPermissionSemantics::Simplified
        );
        assert!(VirtioFsPermissionSemantics::from_str("invalid").is_err());
        assert_eq!(
            VirtioFsPermissionSemantics::Complete.to_string(),
            "complete"
        );

        assert_eq!(
            FreePageReportingMode::from_str("default").unwrap(),
            FreePageReportingMode::Default
        );
        assert_eq!(
            FreePageReportingMode::from_str("replace").unwrap(),
            FreePageReportingMode::Replace
        );
        assert_eq!(
            FreePageReportingMode::from_str("reclaim").unwrap(),
            FreePageReportingMode::Replace
        );
        assert!(FreePageReportingMode::from_str("unknown").is_err());
        assert_eq!(FreePageReportingMode::Replace.to_string(), "replace");
    }

    #[test]
    fn test_runner_config_json_roundtrip_new_fields() {
        let json = r#"{
            "root_path": "/tmp/root",
            "num_vcpus": 2,
            "ram_mib": 2048,
            "disks": [{
                "id": "disk0",
                "path": "/tmp/disk.raw",
                "parallel_reads": true,
                "read_pool_size": 4
            }],
            "virtiofs_mounts": [{
                "tag": "fs0",
                "path": "/tmp/fs",
                "cache_policy": "always",
                "permission_semantics": "simplified"
            }],
            "balloon_reporting_mode": "replace"
        }"#;

        let cfg: RunnerConfig = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(
            cfg.balloon_reporting_mode,
            Some(FreePageReportingMode::Replace)
        );
        assert!(cfg.disks[0].parallel_reads);
        assert_eq!(cfg.disks[0].read_pool_size, Some(4));
        assert_eq!(
            cfg.virtiofs_mounts[0].cache_policy,
            Some(VirtioFsCachePolicy::Always)
        );
        assert_eq!(
            cfg.virtiofs_mounts[0].permission_semantics,
            Some(VirtioFsPermissionSemantics::Simplified)
        );

        let reserialized = serde_json::to_string(&cfg).expect("should serialize");
        let cfg2: RunnerConfig =
            serde_json::from_str(&reserialized).expect("should deserialize again");
        assert_eq!(
            cfg2.balloon_reporting_mode,
            Some(FreePageReportingMode::Replace)
        );
    }

    #[test]
    fn test_fs_overlay_entry_creation() {
        let entry = FsOverlayEntry::new("/dev/root", "etc/secret.token", b"token_abc", 0o600, true);
        assert_eq!(entry.fs_tag, "/dev/root");
        assert_eq!(entry.path, "etc/secret.token");
        assert_eq!(entry.data, b"token_abc");
        assert_eq!(entry.mode, 0o600);
        assert!(entry.one_shot);
    }
}
