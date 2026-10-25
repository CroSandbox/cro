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

/// VirtioFS directory mount rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtioFsMount {
    pub tag: String,
    pub path: PathBuf,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub dax_window_size_bytes: Option<u64>,
}

impl VirtioFsMount {
    pub fn new(tag: impl Into<String>, path: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            tag: tag.into(),
            path: path.into(),
            read_only,
            dax_window_size_bytes: None,
        }
    }

    pub fn with_dax(mut self, bytes: u64) -> Self {
        self.dax_window_size_bytes = Some(bytes);
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
        assert_eq!(DiskFormat::from_path(Path::new("ubuntu.qcow2")), DiskFormat::Qcow2);
        assert_eq!(DiskFormat::from_path(Path::new("arch.qcow")), DiskFormat::Qcow2);
        assert_eq!(DiskFormat::from_path(Path::new("vmware.vmdk")), DiskFormat::Vmdk);
        assert_eq!(DiskFormat::from_path(Path::new("alpine.raw")), DiskFormat::Raw);
        assert_eq!(DiskFormat::from_path(Path::new("data.img")), DiskFormat::Raw);
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
            .with_direct_io(true);
        assert_eq!(customized.sync_mode, DiskSyncMode::Full);
        assert!(customized.direct_io);
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

