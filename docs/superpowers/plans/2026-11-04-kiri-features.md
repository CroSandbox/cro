# Local Domains, Apple Silicon Rosetta, and Linux Developer Machines Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement automatic local domain ingress (`*.cro.local`), Apple Silicon Rosetta 2 (`amd64` emulation), and persistent Linux developer machines (`cro machine`) in `cro`.

**Architecture:** 
1. Rosetta is shared via VirtioFS tag `rosetta` and registered via `binfmt_misc` in `krun-init.sh` for transparent x86_64 container execution on Apple Silicon.
2. A lightweight loopback HTTP router in `microvm-core::domain` routes `http://<name>.cro.local` to microVM instances by parsing `Host:` headers and using smart port heuristics.
3. `microvm-core::machine` and `cro machine` provide persistent dev environments with host macOS UID/GID mapping, passwordless sudo, and `/Users` mount parity.

**Tech Stack:** Rust 2021, `hyper` / `tokio`, VirtioFS, `libkrun`, `clap`, `serde`.

**Spec:** [docs/superpowers/specs/2026-11-04-kiri-features-design.md](../../specs/2026-11-04-kiri-features-design.md)

## Global Constraints

- Must maintain 100% test pass rate across the workspace (`cargo test --workspace`).
- Clean clippy with `cargo clippy --workspace --all-targets -- -D warnings`.
- Code formatted with `cargo fmt --all`.
- Non-Apple Silicon hosts must degrade gracefully without panicking when Rosetta is requested.

## Review Focus

1. Host missing Rosetta runtime directory: verify helpful diagnostic error instead of microVM crash.
2. Domain router receives unknown `Host` header: verify 404 HTML landing page with active microVM list.
3. MicroVM has multiple port forwards: verify domain router prioritizes web/API ports (8000, 8080, 80) over database ports (5432, 6379).
4. Machine creation with non-existent distro: verify validation error with supported distro list.
5. Machine shell execution: verify user UID and GID match macOS host user in container.

---

### Task 1: Rosetta 2 Emulation on Apple Silicon

**Files:**
- Modify: `krun-microvm/crates/microvm-core/src/vm.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/args.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/commands/run.rs`
- Test: `krun-microvm/crates/microvm-core/tests/rosetta_test.rs`

**Interfaces:**
- Produces: `MicroVmBuilder::rosetta(mut self, enabled: bool) -> Self`
- Produces: `MicroVmBuilder::platform(mut self, platform: impl Into<String>) -> Self`
- Produces: `find_rosetta_path() -> Option<PathBuf>`

- [ ] **Step 1: Write failing unit test for Rosetta configuration in `vm.rs`**
- [ ] **Step 2: Verify test fails**
- [ ] **Step 3: Implement `rosetta` and `platform` methods on `MicroVmBuilder` and inject `binfmt_misc` registration in `prepare_virtiofs_mounts`**
- [ ] **Step 4: Add `--rosetta` and `--platform` CLI flags to `RunArgs` in `args.rs` and wire into `commands/run.rs`**
- [ ] **Step 5: Run tests and verify PASS**
- [ ] **Step 6: Commit changes**

---

### Task 2: Core Domain Router Engine

**Files:**
- Create: `krun-microvm/crates/microvm-core/src/domain/mod.rs`
- Create: `krun-microvm/crates/microvm-core/src/domain/router.rs`
- Create: `krun-microvm/crates/microvm-core/src/domain/heuristics.rs`
- Modify: `krun-microvm/crates/microvm-core/src/lib.rs`
- Test: `krun-microvm/crates/microvm-core/src/domain/heuristics.rs` (unit tests)

**Interfaces:**
- Produces: `pub struct DomainRouter`
- Produces: `DomainRouter::new(port: u16, data_dir: PathBuf) -> Self`
- Produces: `DomainRouter::resolve_target(&self, host: &str) -> Option<(u16, String)>`
- Produces: `heuristics::select_web_port(ports: &[(u16, u16)]) -> Option<u16>`

- [ ] **Step 1: Write failing unit tests for `heuristics::select_web_port` and domain resolution**
- [ ] **Step 2: Verify test fails**
- [ ] **Step 3: Implement port heuristic ranking (`WEB_PORTS` vs `NOT_WEB`) and `DomainRouter` request dispatcher**
- [ ] **Step 4: Implement TCP proxying and HTML 404 landing page generator in `router.rs`**
- [ ] **Step 5: Re-export `domain` module in `microvm-core::lib`**
- [ ] **Step 6: Run tests and verify PASS**
- [ ] **Step 7: Commit changes**

---

### Task 3: CLI Subcommand `cro domain`

**Files:**
- Modify: `krun-microvm/crates/microvm-cli/src/args.rs`
- Create: `krun-microvm/crates/microvm-cli/src/commands/domain.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/commands/mod.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/main.rs`
- Test: `krun-microvm/crates/microvm-cli/src/tests.rs`

**Interfaces:**
- Consumes: `DomainRouter` from `microvm-core::domain`
- Produces: `Commands::Domain(DomainArgs)`
- Produces: `handle_domain(args: DomainArgs) -> Result<()>`

- [ ] **Step 1: Write failing CLI parsing test for `cro domain start --port 8080` in `tests.rs`**
- [ ] **Step 2: Verify test fails**
- [ ] **Step 3: Define `DomainArgs` and `DomainCommands` (`Start`, `List`, `Stop`) in `args.rs`**
- [ ] **Step 4: Implement `commands/domain.rs` handler**
- [ ] **Step 5: Wire `Commands::Domain` in `commands/mod.rs` and `main.rs`**
- [ ] **Step 6: Run tests and verify PASS**
- [ ] **Step 7: Commit changes**

---

### Task 4: Core Machine Management Subsystem

**Files:**
- Create: `krun-microvm/crates/microvm-core/src/machine/mod.rs`
- Create: `krun-microvm/crates/microvm-core/src/machine/spec.rs`
- Create: `krun-microvm/crates/microvm-core/src/machine/manager.rs`
- Modify: `krun-microvm/crates/microvm-core/src/lib.rs`
- Test: `krun-microvm/crates/microvm-core/src/machine/spec.rs` (unit tests)

**Interfaces:**
- Produces: `pub struct MachineConfig`
- Produces: `pub struct MachineManager`
- Produces: `MachineManager::create(name: &str, distro: &str, systemd: bool, data_dir: &Path) -> Result<MachineConfig>`
- Produces: `MachineManager::list(data_dir: &Path) -> Result<Vec<MachineInfo>>`
- Produces: `MachineManager::to_builder(config: &MachineConfig) -> Result<MicroVmBuilder>`

- [ ] **Step 1: Write failing tests for `MachineConfig` serialization, distro resolution, and host user detection**
- [ ] **Step 2: Verify test fails**
- [ ] **Step 3: Implement distro resolution (`ubuntu:24.04`, `debian:trixie`, `alpine:3.22`, `fedora:42`, `archlinux:latest`) and user mapping**
- [ ] **Step 4: Implement VirtioFS `/Users` mounting, sudoer injection, and `MachineManager` operations**
- [ ] **Step 5: Re-export `machine` module in `microvm-core::lib`**
- [ ] **Step 6: Run tests and verify PASS**
- [ ] **Step 7: Commit changes**

---

### Task 5: CLI Subcommand `cro machine`

**Files:**
- Modify: `krun-microvm/crates/microvm-cli/src/args.rs`
- Create: `krun-microvm/crates/microvm-cli/src/commands/machine.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/commands/mod.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/main.rs`
- Test: `krun-microvm/crates/microvm-cli/src/tests.rs`

**Interfaces:**
- Consumes: `MachineManager`, `MachineConfig` from `microvm-core::machine`
- Produces: `Commands::Machine(MachineArgs)`
- Produces: `handle_machine(args: MachineArgs) -> Result<()>`

- [ ] **Step 1: Write failing CLI parsing test for `cro machine create dev --distro ubuntu` and `cro machine shell dev`**
- [ ] **Step 2: Verify test fails**
- [ ] **Step 3: Define `MachineArgs` and `MachineCommands` in `args.rs`**
- [ ] **Step 4: Implement `commands/machine.rs` with handlers for `create`, `shell`, `list`, `start`, `stop`, `rm`**
- [ ] **Step 5: Wire `Commands::Machine` in `commands/mod.rs` and `main.rs`**
- [ ] **Step 6: Run tests and verify PASS**
- [ ] **Step 7: Commit changes**

---

### Task 6: Workspace Validation, Formatting, Documentation & Live Verification

**Files:**
- Modify: `README.md`
- Modify: `krun-microvm/README.md`

- [ ] **Step 1: Run `cargo fmt --all --check`**
- [ ] **Step 2: Run `cargo clippy --workspace --all-targets -- -D warnings`**
- [ ] **Step 3: Run `cargo test --workspace`**
- [ ] **Step 4: Update `README.md` and `krun-microvm/README.md` with documentation for `cro domain`, `cro run --rosetta`, and `cro machine`**
- [ ] **Step 5: Build release binaries and update `~/.local/bin/cro` and `~/.local/bin/microvm`**
- [ ] **Step 6: Verify live `cro domain --help` and `cro machine --help`**
- [ ] **Step 7: Final git commit and push**
