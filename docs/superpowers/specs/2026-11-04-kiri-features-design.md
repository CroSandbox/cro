# Design Specification: Local Domains, Apple Silicon Rosetta, and Linux Developer Machines

**Author:** DeepMind Pair Programmer & User  
**Date:** 2026-11-04  
**Target:** `cro` (libkrun-based microVM runtime)  
**Status:** Approved  

---

## 1. Intent & High-Level Architecture

This specification incorporates the three most impactful architectural capabilities inspired by [libracoder/kiri](https://github.com/libracoder/kiri) into `cro`:

1. **Automatic Local Domain Ingress (`*.cro.local`)**: An unprivileged loopback reverse router that transparently routes `http://<name>.cro.local` to microVM instances, AI model servers, and Compose services using HTTP `Host` headers and port heuristics.
2. **Rosetta for Linux on Apple Silicon (`cro run --rosetta` / `--platform linux/amd64`)**: Seamless execution of x86_64/amd64 container images on Apple Silicon Macs by mounting Apple's Rosetta directory over VirtioFS and registering it with `binfmt_misc` in the guest kernel.
3. **Persistent Linux Developer Workstations (`cro machine`)**: Instant, isolated Linux development environments (Ubuntu, Debian, Fedora, Alpine, Arch) matching the macOS user's username, UID, GID, passwordless sudo, and mounting `/Users` at identical paths.

```
                           +-------------------------------------------------------+
                           |                    Host macOS                         |
                           |                                                       |
  Browser / curl --------> |  cro domain (0.0.0.0:8080 or :80)                     |
  http://llama.cro.local   |    ├── parses Host: header                            |
                           |    └── relays to 127.0.0.1:8000 (Model Serving VM)    |
                           |                                                       |
  cro run --platform       |  Rosetta VirtioFS Mount                               |
    linux/amd64 ---------> |    └── /Library/Apple/usr/libexec/oah/RosettaLinux   |
                           |         └── guest binfmt_misc -> executes x86_64 ELF  |
                           |                                                       |
  cro machine shell dev -> |  Persistent MicroVM                                   |
                           |    ├── User: $(USER) [UID/GID matching macOS]         |
                           |    ├── /etc/sudoers.d/cro (NOPASSWD: ALL)             |
                           |    └── /Users mounted at /Users (path parity)         |
                           +-------------------------------------------------------+
```

---

## 2. Feature 1: Automatic Local Domain Ingress (`*.cro.local`)

### 2.1 Goals & Behavior
- **Zero-Friction Access**: Users can access running microVMs without memorizing random host port numbers (e.g. `http://llama.cro.local` instead of `http://127.0.0.1:8000`).
- **Dynamic Routing**:
  - Resolves `<name>.cro.local` (e.g. `llama.cro.local` or `<instance-id>.cro.local`).
  - Automatically queries active microVM instances from `StateManager`.
  - Discovers the HTTP service port by inspecting instance port mappings:
    - Prioritizes standard web/API ports: `80`, `8000`, `8080`, `3000`, `5000`, `5173`.
    - Deprioritizes or filters database/cache ports: `5432`, `3306`, `6379`, `27017`.
- **Proxy Engine**:
  - Built using `hyper` / `tokio::net` in `microvm-core::domain`.
  - Operates unprivileged on port `8080` by default (or port `80` if permitted, configurable via `--http-port`).
  - Supports `--daemon` / background execution.
  - Generates helpful HTML 404 landing page listing all currently running microVMs with clickable links if a domain is not found.

### 2.2 CLI Subcommand Interface
```bash
# Start the local domain ingress router
cro domain start [--port <port>] [-d]

# List active domain mappings
cro domain list

# Stop domain router
cro domain stop
```

---

## 3. Feature 2: Rosetta for Linux on Apple Silicon (`--rosetta` / `--platform`)

### 3.1 Goals & Behavior
- Enables developers on macOS Apple Silicon (`aarch64-apple-darwin`) to seamlessly execute `x86_64` / `amd64` container images.
- Automatically locates the host Rosetta runtime at `/Library/Apple/usr/libexec/oah/RosettaLinux` (or standard macOS location).
- Automatically mounts Rosetta via VirtioFS with tag `rosetta` to `/rosetta` in the guest rootfs.
- Injects `binfmt_misc` registration commands into `krun-init.sh`:
  ```bash
  mount -t binfmt_misc binfmt_misc /proc/sys/fs/binfmt_misc 2>/dev/null || true
  if [ -f /rosetta/rosetta ] && [ -d /proc/sys/fs/binfmt_misc ]; then
    echo ':rosetta:M::\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x3e\x00:\xff\xff\xff\xff\xff\xfe\xfe\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff:/rosetta/rosetta:CF' > /proc/sys/fs/binfmt_misc/register 2>/dev/null || true
  fi
  ```
- **CLI Options**:
  - `cro run --rosetta <image>` or `cro run --platform linux/amd64 <image>`
  - Auto-detection: If the pulled image architecture is `amd64` on Apple Silicon, `--rosetta` is enabled automatically.

---

## 4. Feature 3: Persistent Linux Developer Workstations (`cro machine`)

### 4.1 Goals & Behavior
- Provides instant, persistent Linux development environments powered by microVMs.
- Supports popular developer distributions: `ubuntu` (default: 24.04), `debian`, `alpine`, `fedora`, `arch`, `rocky`.
- **Host User Parity**:
  - Obtains host `USER`, `UID`, and `GID`.
  - Automatically provisions the user inside Linux with home directory `/Users/<user>`.
  - Configures passwordless `sudo` in `/etc/sudoers.d/cro`.
- **Path Parity**:
  - Mounts macOS `/Users` directly to `/Users` inside the microVM via VirtioFS.
  - Developers can run commands with identical file paths across macOS and Linux.
- **Persistence**:
  - Machine state directory preserved under `~/.cache/krun-microvm/machines/<name>`.

### 4.2 CLI Subcommand Interface
```bash
# Create a machine from standard distro
cro machine create dev --distro ubuntu

# Open an interactive login shell inside the machine
cro machine shell dev

# Execute a one-off command
cro machine shell dev -- uname -m

# Manage machines
cro machine list
cro machine stop dev
cro machine start dev
cro machine rm dev
```

---

## 5. Non-Goals & Constraints

- **Non-Goal**: Replacing system-wide Docker daemon on Linux; `cro` remains focused on hardware-isolated microVMs and secure agentic computing.
- **Constraints**:
  - 100% test pass rate across the workspace must be preserved.
  - Zero clippy warnings with `-D warnings`.
  - All features must compile and function cleanly on macOS and Linux.
