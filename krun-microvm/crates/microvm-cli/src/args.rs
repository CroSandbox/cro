// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CLI command and argument definitions parsed via `clap`.

use crate::utils::parse_secret_arg;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "microvm")]
#[command(
    about = "Run OCI container images in hardware-isolated microVMs with libkrun",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// OCI image reference (e.g. alpine:latest, ubuntu:22.04) or empty if --bundle is used
    #[arg(default_value = "")]
    pub image: String,

    /// Path to an unpacked OCI runtime bundle (containing config.json and rootfs/)
    #[arg(long)]
    pub bundle: Option<PathBuf>,

    /// Path to a declarative YAML manifest (compose or microvm spec)
    #[arg(short = 'f', long = "file")]
    pub file: Option<PathBuf>,

    /// Number of virtual CPUs
    #[arg(short = 'c', long, default_value_t = 2)]
    pub cpus: u8,

    /// RAM in MiB
    #[arg(short = 'm', long, default_value_t = 512)]
    pub memory: u32,

    /// Port forwarding rules in host:guest format (e.g. 8080:80)
    #[arg(short = 'p', long = "port")]
    pub ports: Vec<String>,

    /// Mount host directory via VirtioFS: <host_path>:<tag>[:ro]
    #[arg(short = 'v', long = "volume")]
    pub volumes: Vec<String>,

    /// Attach an OCI artifact (models, datasets, blobs) via VirtioFS: <reference>:<tag>[:ro]
    #[arg(long = "artifact")]
    pub artifacts: Vec<String>,

    /// Attach a CNCF ModelPack AI model via VirtioFS with auto-DAX memory mapping: <model_ref_or_dir>[:<mount_path>]
    #[arg(long = "model")]
    pub models: Vec<String>,

    /// Mount an isolated Copy-on-Write (CoW) sandbox of a host directory: <host_path>:<tag>
    #[arg(long = "workspace-cow")]
    pub workspace_cow: Vec<String>,

    /// Environment variables (KEY=VALUE)
    #[arg(short = 'e', long = "env")]
    pub env: Vec<String>,

    /// Working directory inside guest
    #[arg(short = 'w', long)]
    pub workdir: Option<String>,

    /// Custom data cache directory
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Skip preflight virtualization checks
    #[arg(long)]
    pub no_preflight: bool,

    /// libkrun log level (0=Off, 1=Error, 2=Warn, 3=Info, 4=Debug, 5=Trace)
    #[arg(long)]
    pub log_level: Option<u32>,

    /// Keep STDIN open even if not attached
    #[arg(short = 'i', long)]
    pub interactive: bool,

    /// Allocate a pseudo-TTY
    #[arg(short = 't', long)]
    pub tty: bool,

    /// Run container in background and print container ID
    #[arg(short = 'd', long)]
    pub detach: bool,

    /// Completely isolate guest without network interfaces
    #[arg(long)]
    pub no_network: bool,

    /// Network mode: tsi (default), gvproxy (user-mode virtual network), none (air-gapped), unix:<socket_path>, or cni:<netns_path>
    #[arg(long, alias = "network", default_value = "tsi")]
    pub net: String,

    /// Custom DNS nameservers (e.g. 8.8.8.8,1.1.1.1; defaults to autonomous resilient fallback)
    #[arg(long = "dns")]
    pub dns: Vec<String>,

    /// Custom guest hostname (defaults to microVM instance ID)
    #[arg(long)]
    pub hostname: Option<String>,

    /// Custom MAC address for virtio-net interface (e.g. 5a:94:ef:e4:0c:ee)
    #[arg(long = "mac")]
    pub mac: Option<String>,

    /// Network MTU for virtio-net interface (default: 1500)
    #[arg(long = "mtu")]
    pub mtu: Option<usize>,

    /// Guest resource limits (e.g. RLIMIT_NOFILE=1024:2048)
    #[arg(long)]
    pub rlimits: Option<String>,

    /// VirtioFS DAX (Direct Access) shared memory window size (e.g. 4G, 512M)
    #[arg(long)]
    pub dax: Option<String>,

    /// Enable Dragonfly Nydus RAFSv6 instant on-demand lazy loading
    #[arg(long = "lazy-load")]
    pub lazy_load: bool,

    /// Path to local RAFSv6 / EROFS metadata bootstrap image
    #[arg(long = "nydus-bootstrap")]
    pub nydus_bootstrap: Option<PathBuf>,

    /// Directory for caching downloaded Nydus chunk blobs
    #[arg(long = "nydus-cache")]
    pub nydus_cache: Option<PathBuf>,

    /// Chunk or macro-chunk size for streaming (e.g. 4M, 64M)
    #[arg(long = "chunk-size")]
    pub chunk_size: Option<String>,

    /// Enable hardware-accelerated virtio-gpu (Apple Silicon Metal / Linux DRM Venus)
    #[arg(long)]
    pub gpu: bool,

    /// Shared memory vRAM window size for virtio-gpu (e.g. 2G, 4G, 8G)
    #[arg(long = "gpu-shm-size")]
    pub gpu_shm_size: Option<String>,

    /// Disable zero-trust host sandboxing restrictions
    #[arg(long = "no-sandbox")]
    pub no_sandbox: bool,

    /// Allow outbound network egress to explicitly permitted destination (e.g. api.openai.com:443, *.github.com:443)
    #[arg(long = "allow-host", alias = "allow-net")]
    pub allow_hosts: Vec<String>,

    /// Enforce explicit default-deny network egress policy (blocks all external traffic unless explicitly permitted)
    #[arg(long = "default-deny", alias = "deny-all")]
    pub default_deny: bool,

    /// Inject secret via in-flight substitution (KEY=VALUE, KEY=env:VAR_NAME, or KEY=file:/path)
    #[arg(long = "secret")]
    pub secrets: Vec<String>,

    /// Hard ceiling on cumulative LLM tokens (prompt + completion) consumed by the microVM
    #[arg(long = "max-tokens")]
    pub max_tokens: Option<u64>,

    /// Direct kernel boot: path to kernel binary (ELF, RAW, bzImage, Image.gz, m1n1)
    #[arg(long = "kernel")]
    pub kernel: Option<PathBuf>,

    /// Direct kernel format override (raw/bin/m1n1, elf/vmlinux, gz/image.gz/vmlinuz, bz2, zstd, pe)
    #[arg(long = "kernel-format", alias = "kformat")]
    pub kernel_format: Option<String>,

    /// Direct kernel boot: optional initramfs/initrd path
    #[arg(long = "initrd")]
    pub initrd: Option<PathBuf>,

    /// Direct kernel boot: kernel command line arguments
    #[arg(long = "cmdline")]
    pub cmdline: Option<String>,

    /// UEFI firmware boot: path to firmware blob (e.g. KRUN_EFI.fd)
    #[arg(long = "firmware")]
    pub firmware: Option<PathBuf>,

    /// Attach block disk image: <path> or <id>:<path>[:ro]
    #[arg(long = "disk")]
    pub disks: Vec<String>,

    /// VirtioFS cache policy (never, auto, always; default: auto)
    #[arg(long = "fs-cache")]
    pub fs_cache: Option<String>,

    /// VirtioFS permission semantics (complete, simplified; default: complete)
    #[arg(long = "fs-permissions")]
    pub fs_permissions: Option<String>,

    /// Enable parallel reads on attached block disks
    #[arg(long = "disk-parallel-reads")]
    pub disk_parallel_reads: bool,

    /// Worker thread pool size for block disk parallel reads (default: libkrun automatic pool)
    #[arg(long = "disk-read-pool-size")]
    pub disk_read_pool_size: Option<usize>,

    /// Memory balloon free page reporting mode (default, replace; default: default)
    #[arg(long = "balloon-reporting")]
    pub balloon_reporting: Option<String>,

    /// Size of POSIX shared memory (/dev/shm) mounted inside the guest (e.g. 64m, 512m, 2g; default: 64m)
    #[arg(long = "shm-size")]
    pub shm_size: Option<String>,

    /// Enable Apple Silicon Rosetta 2 for Linux x86_64 ELF execution
    #[arg(long)]
    pub rosetta: bool,

    /// Target architecture platform (e.g. linux/amd64, linux/arm64)
    #[arg(long)]
    pub platform: Option<String>,

    /// Optional command to override ENTRYPOINT/CMD
    #[arg(last = true)]
    pub cmd: Vec<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Run an OCI container image, direct kernel, or firmware disk as a microVM
    Run(Box<RunArgs>),

    /// Boot a unikernel image (Unikraft, Nanos, OSv, Solo5) directly in a microVM
    Unikernel {
        /// Path to the unikernel binary (ELF)
        kernel: PathBuf,

        /// Number of virtual CPUs
        #[arg(short = 'c', long, default_value_t = 1)]
        cpus: u8,

        /// RAM in MiB
        #[arg(short = 'm', long, default_value_t = 512)]
        memory: u32,

        /// Unikernel command line arguments
        #[arg(long = "cmdline", alias = "params")]
        cmdline: Option<String>,

        /// Attach block disk image: <path> or <id>:<path>[:ro]
        #[arg(long = "disk")]
        disks: Vec<String>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Run an AI coding agent or secure developer workspace in an isolated microVM sandbox
    Sandbox {
        /// Agent or profile to launch (e.g. claude, gemini, codex, dev)
        #[arg(default_value = "dev")]
        agent: String,

        /// Host directory to mount as isolated Copy-on-Write (CoW) workspace
        #[arg(short = 'w', long = "workspace", default_value = ".")]
        workspace: PathBuf,

        /// Git repository URL to clone into workspace on boot
        #[arg(long = "repo")]
        repo: Option<String>,

        /// Git commit or reference to cherry-pick into the workspace on launch
        #[arg(long = "cherry-pick", alias = "pick")]
        cherry_pick: Option<String>,

        /// Automatically cherry-pick sandbox commits back to host workspace on session exit
        #[arg(long = "apply-to-host", alias = "sync-back")]
        apply_to_host: bool,

        /// Container base image
        #[arg(short = 'i', long = "image", default_value = "alpine:latest")]
        image: String,

        /// Number of virtual CPUs
        #[arg(short = 'c', long, default_value_t = 2)]
        cpus: u8,

        /// RAM in MiB
        #[arg(short = 'm', long, default_value_t = 1024)]
        memory: u32,

        /// Secrets to inject into guest (KEY=VAL, KEY (reads from host env), or @/path/to/file)
        #[arg(long = "secret", value_name = "KEY=VAL|KEY|@FILE")]
        secrets: Vec<String>,

        /// Allow outbound network egress to explicitly permitted destination (e.g. api.openai.com:443, *.github.com:443)
        #[arg(long = "allow-host", alias = "allow-net")]
        allow_hosts: Vec<String>,

        /// Enforce explicit default-deny network egress policy (blocks all external traffic unless explicitly permitted)
        #[arg(long = "default-deny", alias = "deny-all")]
        default_deny: bool,

        /// Hard ceiling on cumulative LLM tokens (prompt + completion) consumed by the microVM
        #[arg(long = "max-tokens")]
        max_tokens: Option<u64>,

        /// Enable Human-in-the-Loop (HITL) approval pause/resume for mutating agent requests (OneCLI model)
        #[arg(long = "hitl", alias = "approvals")]
        hitl: bool,

        /// Enable hardware-accelerated virtio-gpu (Apple Silicon Metal / Linux DRM Venus)
        #[arg(long)]
        gpu: bool,

        /// Shared memory vRAM window size for virtio-gpu (e.g. 2G, 4G, 8G)
        #[arg(long = "gpu-shm-size")]
        gpu_shm_size: Option<String>,

        /// Size of POSIX shared memory (/dev/shm) mounted inside the guest (e.g. 64m, 512m, 2g; default: 64m)
        #[arg(long = "shm-size")]
        shm_size: Option<String>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,

        /// Optional command to execute inside the sandbox
        #[arg(last = true)]
        cmd: Vec<String>,
    },

    /// Execute a command inside a running microVM
    Exec {
        /// ID or PID of the running microVM
        id: String,

        /// Environment variables (KEY=VALUE)
        #[arg(short = 'e', long = "env")]
        env: Vec<String>,

        /// Working directory inside guest
        #[arg(short = 'w', long)]
        workdir: Option<String>,

        /// Allocate a pseudo-TTY
        #[arg(short = 't', long)]
        tty: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,

        /// Command and arguments to execute
        #[arg(
            required = true,
            trailing_var_arg = true,
            allow_hyphen_values = true,
            num_args = 1..
        )]
        cmd: Vec<String>,
    },

    /// List running and recent microVMs
    Ps {
        /// Show all microVMs (including stopped)
        #[arg(short = 'a', long)]
        all: bool,

        /// Do not truncate IDs and image names
        #[arg(long = "no-trunc")]
        no_trunc: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Stop a running microVM by ID or PID
    Stop {
        /// ID or PID of the microVM
        id: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Pause a running microVM (freezes all vCPUs)
    Pause {
        /// ID or PID of the microVM
        id: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Resume execution of a paused microVM
    Resume {
        /// ID or PID of the microVM
        id: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Remove one or more stopped microVMs
    Rm {
        /// ID or PID of the microVM
        id: String,

        /// Force removal of a running microVM (stops it first)
        #[arg(short = 'f', long)]
        force: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Return low-level configuration and runtime details on a microVM
    Inspect {
        /// ID or PID of the microVM
        id: String,

        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Display live resource usage statistics for running microVMs
    Stats {
        /// ID or PID of a specific microVM (omit to display all running microVMs)
        id: Option<String>,

        /// Disable streaming live stats and only output the current snapshot
        #[arg(long = "no-stream")]
        no_stream: bool,

        /// Output telemetry formatted as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Display running supervisor and thread statistics of a microVM
    Top {
        /// ID or PID of the microVM
        id: Option<String>,

        /// Run in interactive full-screen cockpit mode
        #[arg(short = 'i', long)]
        interactive: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Copy files/folders between host and a microVM
    Cp {
        /// Source path (e.g. ./file.txt or <vm-id>:<path>)
        src: String,

        /// Destination path (e.g. <vm-id>:<path> or ./file.txt)
        dest: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// View console logs of a microVM
    Logs {
        /// ID or PID of the microVM
        id: String,

        /// Follow log output continuously
        #[arg(short = 'f', long)]
        follow: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Remove stopped instances and temporary cache directories
    Prune {
        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Pull and cache an OCI image rootfs locally
    Pull {
        /// OCI image reference
        image: String,

        /// Custom cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Run preflight checks to verify hypervisor support
    Preflight {
        /// Ports to check availability for
        #[arg(short, long)]
        port: Vec<u16>,
    },

    /// Print system, virtualization, and resource cache status
    Info,

    /// Manage detached OCI artifacts (models, datasets, toolchains)
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommands,
    },

    /// Manage AI/ML models conforming to the CNCF ModelPack Specification (pull, list, info, pack)
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },

    /// Serve an AI model using a high-throughput inference engine in an isolated microVM
    Serve(Box<ServeArgs>),

    /// Manage and run vLLM inference microVMs (alias to cro serve --engine vllm)
    Vllm {
        #[command(subcommand)]
        command: VllmCommands,
    },

    /// Capture an instant Copy-on-Write snapshot of a microVM
    Snapshot {
        /// ID or PID of the microVM
        id: String,

        /// Output path for snapshot (directory or .tar archive)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Restore a microVM from a snapshot for instant warm-start
    Restore {
        /// Path to the snapshot package (directory or .tar archive)
        snapshot: PathBuf,

        /// Optional new microVM instance ID
        #[arg(short, long)]
        name: Option<String>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Dynamically resize CPU and memory resources of a running microVM
    Resize {
        /// ID or PID of the running microVM
        id: String,

        /// New memory allocation in MiB
        #[arg(short = 'm', long)]
        memory: Option<u32>,

        /// New virtual CPU count
        #[arg(short = 'c', long)]
        cpus: Option<u8>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Export Prometheus metrics or start a live scrape HTTP server
    Metrics {
        /// Address to listen on for Prometheus scrapes (e.g. 127.0.0.1:9090 or 0.0.0.0:9090)
        #[arg(short, long)]
        listen: Option<String>,

        /// Format single-shot metrics output as JSON instead of Prometheus format
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Manage the containerd v2 runtime shim integration
    Containerd {
        #[command(subcommand)]
        command: ContainerdCommands,
    },

    /// Multi-microVM orchestration using declarative compose manifests (docker-compose compatible)
    Compose {
        #[command(subcommand)]
        command: ComposeCommands,
    },

    /// Apply a declarative YAML manifest (Docker Compose or Kubernetes CRD)
    Apply {
        /// Path to YAML manifest file (e.g. krun-compose.yaml, microvm.yaml)
        #[arg(short = 'f', long = "file")]
        file: PathBuf,

        /// Run microVM(s) in background
        #[arg(short = 'd', long)]
        detach: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Cherry-pick commits from an isolated sandbox workspace back to the host git repository
    CherryPick {
        /// ID or PID of the sandbox microVM instance (from microvm ps -a)
        id: String,

        /// Target host workspace directory to apply commits to (defaults to current directory)
        #[arg(short = 'w', long = "workspace", default_value = ".")]
        workspace: PathBuf,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Manage microVM virtual networking, port mappings, and egress policies
    #[command(subcommand)]
    Network(NetworkCommands),

    /// Build an OCI image from a Dockerfile using the native in-process engine
    Build {
        /// Name and optionally a tag in the 'name:tag' format
        #[arg(short = 't', long = "tag")]
        tag: Option<String>,

        /// Name of the Dockerfile (Default is 'PATH/Dockerfile')
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Do not use cache when building the image
        #[arg(long = "no-cache")]
        no_cache: bool,

        /// Path to the build context directory (defaults to current directory)
        #[arg(default_value = ".")]
        context: PathBuf,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Manage Kubernetes Pod specifications with hardware microVM isolation
    Kube {
        #[command(subcommand)]
        command: KubeCommands,
    },

    /// Manage Human-in-the-Loop (HITL) approval requests for intercepted agent actions
    Approval {
        #[command(subcommand)]
        command: ApprovalCommands,
    },

    /// Manage automatic local domain ingress routing (*.cro.local)
    Domain {
        #[command(subcommand)]
        command: DomainCommands,
    },

    /// Manage persistent Linux developer workstations
    Machine {
        #[command(subcommand)]
        command: MachineCommands,
    },

    /// Open the interactive multi-tab terminal cockpit
    #[command(alias = "cockpit", alias = "dashboard")]
    Ui {
        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ApprovalCommands {
    /// List pending or historical human approval requests
    #[command(name = "ls", alias = "list")]
    List {
        /// Show full history (including approved, denied, and timed-out requests)
        #[arg(long, short = 'a')]
        all: bool,

        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Approve an intercepted pending request
    Approve {
        /// ID of the pending approval (e.g. appr_000001)
        id: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Deny an intercepted pending request
    Deny {
        /// ID of the pending approval (e.g. appr_000001)
        id: String,

        /// Optional reason for denial
        #[arg(long, short = 'r')]
        reason: Option<String>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum KubeCommands {
    /// Play/launch containers defined in a Kubernetes Pod manifest
    Play {
        /// Path to the Kubernetes Pod YAML file
        file: PathBuf,

        /// Run microVMs in background (detached mode)
        #[arg(short = 'd', long)]
        detach: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Stop and tear down all containers in a Kubernetes Pod manifest
    Down {
        /// Path to the Kubernetes Pod YAML file
        file: PathBuf,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum NetworkCommands {
    /// List active microVM network configurations, modes, and interfaces
    #[command(name = "ls", alias = "list")]
    Ls {
        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Inspect detailed network topology, IPs, DNS, and egress rules of a microVM
    Inspect {
        /// ID or PID of the microVM
        id: String,

        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Display active host-to-guest port forwards across microVMs
    Ports {
        /// Optional microVM ID (if omitted, lists port forwards for all microVMs)
        id: Option<String>,

        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Test network connectivity, DNS resolution, and egress security from inside a microVM
    Test {
        /// ID or PID of the running microVM
        id: String,

        /// Target host or IP to test connectivity against (e.g. 1.1.1.1, api.openai.com)
        #[arg(default_value = "1.1.1.1")]
        target: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Launch a Cloudflare Pingora L7 Reverse Proxy Gateway or Zero-Trust Egress service
    Pingora {
        /// Listening host:port address (e.g. 127.0.0.1:8080)
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        listen: String,

        /// Forward route rules in format: /path=IP:PORT (e.g. /api=127.0.0.1:3000)
        #[arg(short, long)]
        route: Vec<String>,

        /// Run as Zero-Trust Egress proxy enforcing domain allowlist
        #[arg(long)]
        egress: bool,

        /// Permitted outbound domain destinations (e.g. api.openai.com:443, *.github.com:443)
        #[arg(long = "allow-host")]
        allow_hosts: Vec<String>,

        /// Strip matched path prefix before forwarding to upstream backend
        #[arg(long)]
        strip_prefix: bool,

        /// Secret mapping in KEY=VALUE format for in-flight replacement (egress mode)
        #[arg(long = "secret", value_parser = parse_secret_arg)]
        secrets: Vec<(String, String)>,

        /// Enforce hard LLM token ceiling budget (egress mode)
        #[arg(long)]
        max_tokens: Option<u64>,
    },

    /// Launch the high-performance zero-trust egress proxy standalone
    Proxy {
        /// Listening host:port address (default: 127.0.0.1:8080)
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        listen: String,

        /// Permitted outbound domain destinations (e.g. api.openai.com:443, *.github.com:443, 10.0.0.0/8)
        #[arg(long = "allow-host")]
        allow_hosts: Vec<String>,

        /// Secret mapping in KEY=VALUE format for in-flight replacement
        #[arg(long = "secret", value_parser = parse_secret_arg)]
        secrets: Vec<(String, String)>,

        /// Enforce hard LLM token ceiling budget
        #[arg(long)]
        max_tokens: Option<u64>,
    },

    /// Diagnose host network environment, DNS health, gvproxy runtime, and firewall status
    Doctor {
        /// Format output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ComposeCommands {
    /// Create and start microVM containers defined in compose manifest
    Up {
        /// Path to compose manifest file (defaults to auto-discovering krun-compose.yaml, compose.yaml, etc.)
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Run containers in background
        #[arg(short = 'd', long)]
        detach: bool,

        /// Specific services to start (defaults to all)
        #[arg(value_name = "SERVICE")]
        services: Vec<String>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Stop and remove microVM containers and networks
    Down {
        /// Path to compose manifest file
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Remove named volumes declared in the volumes section
        #[arg(short = 'v', long = "volumes")]
        volumes: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// List running microVM containers for the compose project
    Ps {
        /// Path to compose manifest file
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// View output logs from microVM services
    Logs {
        /// Path to compose manifest file
        #[arg(long = "file")]
        file: Option<PathBuf>,

        /// Specific service name to view logs for
        #[arg(value_name = "SERVICE")]
        service: Option<String>,

        /// Follow log output continuously
        #[arg(short = 'f', long)]
        follow: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Stop running microVM services without removing state
    Stop {
        /// Path to compose manifest file
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Specific services to stop (defaults to all)
        #[arg(value_name = "SERVICE")]
        services: Vec<String>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Start stopped microVM services
    Start {
        /// Path to compose manifest file
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Specific services to start (defaults to all)
        #[arg(value_name = "SERVICE")]
        services: Vec<String>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Restart microVM services
    Restart {
        /// Path to compose manifest file
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Specific services to restart (defaults to all)
        #[arg(value_name = "SERVICE")]
        services: Vec<String>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Validate and view the resolved compose configuration
    Config {
        /// Path to compose manifest file
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ArtifactCommands {
    /// Pull and cache an OCI artifact locally
    Pull {
        /// OCI artifact reference (e.g. ghcr.io/owner/model:latest)
        artifact: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// List locally cached OCI artifacts
    List {
        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ModelCommands {
    /// Pull and unpack a CNCF ModelPack artifact from an OCI registry
    Pull {
        /// Model reference (e.g. ghcr.io/org/llama-3.2-1b:latest)
        model: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// List all locally cached ModelPack models
    List {
        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Inspect metadata, format, architecture, and layers of a ModelPack model
    Info {
        /// Model reference or local directory
        model: String,

        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Package a local directory of weights and configs into a CNCF ModelPack OCI layout
    Pack {
        /// Directory containing model weight files and configs
        #[arg(short = 's', long)]
        source: PathBuf,

        /// Destination directory for the packaged OCI Image Layout
        #[arg(short = 'o', long)]
        output: PathBuf,

        /// Model name (e.g. llama-3.2-1b-instruct)
        #[arg(short = 'n', long)]
        name: String,

        /// Model weight format (gguf, safetensors, onnx, pytorch, mlx)
        #[arg(short = 'f', long)]
        format: String,

        /// Semantic version (e.g. 1.0.0)
        #[arg(short = 'v', long)]
        version: Option<String>,

        /// Base neural network architecture (e.g. llama, mistral, qwen)
        #[arg(short = 'a', long)]
        architecture: Option<String>,

        /// Quantization / precision level (e.g. q4_k_m, fp16, int8)
        #[arg(short = 'p', long)]
        precision: Option<String>,

        /// Maximum context length in tokens
        #[arg(long)]
        context_length: Option<u64>,

        /// Tag annotation for the OCI layout index (default: latest)
        #[arg(short = 't', long)]
        tag: Option<String>,
    },

    /// Discover locally available models across Hugging Face, LM Studio, Ollama, and local directories
    Discover {
        /// Optional custom search directories
        #[arg(long = "path")]
        custom_paths: Vec<PathBuf>,

        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Pre-flight hardware memory profiling and suitability estimate for a model
    Fit {
        /// Path to model file, directory, or model name
        model: String,

        /// Target context length in tokens (default: 4096)
        #[arg(long, default_value_t = 4096)]
        context_length: u64,

        /// Format output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ContainerdCommands {
    /// Generate containerd config.toml runtime snippet and Kubernetes RuntimeClass YAML
    GenerateConfig,

    /// Install (symlink) containerd-shim-krun-v2 into a system path
    Install {
        /// Target directory for the shim binary
        #[arg(short, long, default_value = "/usr/local/bin")]
        target: PathBuf,
    },

    /// Check containerd daemon status and shim binary availability
    Status,
}

#[derive(Args, Debug, Clone)]
pub struct ServeArgs {
    /// Model reference (OCI ModelPack or local directory)
    pub model: String,

    /// Inference engine (auto, vllm, llamacpp, uzu; default: auto)
    #[arg(long, default_value = "auto")]
    pub engine: String,

    /// Host port to expose the OpenAI-compatible API on (default: 8000)
    #[arg(short = 'p', long, default_value_t = 8000)]
    pub port: u16,

    /// Host interface to bind the API to (default: 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Base container image for the inference engine
    #[arg(long)]
    pub image: Option<String>,

    /// Number of virtual CPUs (default: 4)
    #[arg(short = 'c', long, default_value_t = 4)]
    pub cpus: u8,

    /// Guest RAM in MiB (default: 8192 MiB / 8 GB)
    #[arg(short = 'm', long, default_value_t = 8192)]
    pub memory: u32,

    /// Enable hardware GPU acceleration (Apple Silicon Metal / Linux DRM Venus)
    #[arg(long, default_value_t = true)]
    pub gpu: bool,

    /// Shared memory vRAM window size for GPU (e.g. 4G, 8G)
    #[arg(long, default_value = "4G")]
    pub gpu_shm_size: String,

    /// VirtioFS DAX window size (e.g. 4G; auto-calculated from model size by default)
    #[arg(long)]
    pub dax: Option<String>,

    /// Maximum model context length (tokens)
    #[arg(long)]
    pub max_model_len: Option<u64>,

    /// Number of tensor parallel ranks
    #[arg(long, default_value_t = 1)]
    pub tensor_parallel_size: u32,

    /// Enforce eager execution mode in vLLM (disables CUDA/Metal graph capture)
    #[arg(long)]
    pub enforce_eager: bool,

    /// API key requirement for the exposed OpenAI endpoints
    #[arg(long)]
    pub api_key: Option<String>,

    /// Public model alias or served model name exposed in the OpenAI-compatible API
    #[arg(long = "served-model-name", visible_alias = "alias")]
    pub served_model_name: Option<String>,

    /// Host plugin directory or file to mount into guest and add to PYTHONPATH (can be repeated)
    #[arg(long = "plugin", action = clap::ArgAction::Append)]
    pub plugins: Vec<PathBuf>,

    /// vLLM plugin module name to register with VLLM_PLUGINS environment variable (can be repeated)
    #[arg(long = "vllm-plugin", action = clap::ArgAction::Append)]
    pub vllm_plugins: Vec<String>,

    /// Allow custom model code execution in vLLM (--trust-remote-code)
    #[arg(long)]
    pub trust_remote_code: bool,

    /// Quantization method override for vLLM (e.g. awq, gptq, fp8, marlin, bitsandbytes)
    #[arg(long)]
    pub quantization: Option<String>,

    /// Path to a custom Jinja chat template file
    #[arg(long)]
    pub chat_template: Option<PathBuf>,

    /// Set custom environment variable inside the microVM (format: KEY=VALUE, can be repeated)
    #[arg(short = 'e', long = "env", action = clap::ArgAction::Append)]
    pub env: Vec<String>,

    /// Run the inference server in the background (detached mode)
    #[arg(short = 'd', long)]
    pub detach: bool,

    /// Wait for server /health probe to become ready before exiting in detached mode
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub wait: bool,

    /// Do not wait for server /health probe before exiting in detached mode
    #[arg(long)]
    pub no_wait: bool,

    /// Automatically capture and freeze a warm-start snapshot when the inference server signals readiness
    #[arg(long)]
    pub snapshot_on_ready: bool,

    /// Restore and instantly resume execution from a pre-warmed snapshot (< 100ms startup)
    #[arg(long)]
    pub warm: bool,

    /// Custom warm-start snapshot tag name override
    #[arg(long)]
    pub warmup_snapshot_tag: Option<String>,

    /// Custom data cache directory
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Additional trailing arguments passed directly to the inference server
    #[arg(last = true)]
    pub extra_args: Vec<String>,

    /// Enable scale-to-zero (auto-pause on idle, sub-2ms SIGCONT wake on HTTP request)
    #[arg(long)]
    pub scale_to_zero: bool,

    /// Inactivity idle timeout before auto-pausing microVM in seconds (default: 60)
    #[arg(long, default_value_t = 60)]
    pub idle_timeout: u64,
}

#[derive(Subcommand, Debug, Clone)]
pub enum VllmCommands {
    /// Serve a ModelPack using the vLLM engine
    Serve(Box<ServeArgs>),
}

#[derive(Subcommand, Debug, Clone)]
pub enum DomainCommands {
    /// Start the local domain ingress reverse router
    Start {
        /// Listening port on host (default: 8080, or 80 if permitted)
        #[arg(short = 'p', long, default_value_t = 8080)]
        port: u16,

        /// Bind address (defaults to 0.0.0.0 on macOS or 127.0.0.1 on Linux)
        #[arg(long)]
        addr: Option<String>,

        /// Run domain router in background as a daemon
        #[arg(short = 'd', long)]
        detach: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,

        /// Embedded Micro-DNS server port for *.cro.local resolution (default: 5354)
        #[arg(long, default_value_t = 5354)]
        dns_port: u16,

        /// Disable the embedded Micro-DNS server
        #[arg(long)]
        no_dns: bool,
    },

    /// Configure macOS /etc/resolver/cro.local for automatic zero-config resolution
    SetupResolver {
        /// Micro-DNS server port (default: 5354)
        #[arg(long, default_value_t = 5354)]
        dns_port: u16,
    },

    /// Test resolution of a .cro.local domain via the embedded Micro-DNS server
    Resolve {
        /// Domain to resolve (e.g. llama.cro.local)
        domain: String,

        /// Micro-DNS server port (default: 5354)
        #[arg(long, default_value_t = 5354)]
        dns_port: u16,
    },

    /// List active domain mappings and target microVMs
    #[command(name = "ls", alias = "list")]
    List {
        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Stop the background domain ingress router
    Stop {
        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum MachineCommands {
    /// Create and provision a new persistent Linux developer machine
    Create {
        /// Name of the machine (letters, digits, dashes)
        name: String,

        /// Linux distribution (ubuntu, debian, alpine, fedora, arch, rocky, opensuse; default: ubuntu)
        #[arg(long, default_value = "ubuntu")]
        distro: String,

        /// Custom container image to use instead of distribution default
        #[arg(long)]
        image: Option<String>,

        /// Number of virtual CPUs
        #[arg(short = 'c', long)]
        cpus: Option<u8>,

        /// RAM in MiB
        #[arg(short = 'm', long)]
        memory: Option<u32>,

        /// Enable Apple Silicon Rosetta 2 x86_64 emulation
        #[arg(long)]
        rosetta: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Open an interactive login shell inside the machine, or run a one-off command
    Shell {
        /// Name of the machine
        name: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,

        /// Command and arguments to execute inside the machine (defaults to login shell)
        #[arg(last = true)]
        cmd: Vec<String>,
    },

    /// List all persistent developer machines and their statuses
    #[command(name = "ls", alias = "list")]
    List {
        /// Format output as JSON
        #[arg(long)]
        json: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Start a stopped machine
    Start {
        /// Name of the machine
        name: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Stop a running machine
    Stop {
        /// Name of the machine
        name: String,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Remove a machine and its persistent storage
    #[command(name = "rm", alias = "delete")]
    Rm {
        /// Name of the machine
        name: String,

        /// Force removal even if running
        #[arg(short = 'f', long)]
        force: bool,

        /// Custom data cache directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}
