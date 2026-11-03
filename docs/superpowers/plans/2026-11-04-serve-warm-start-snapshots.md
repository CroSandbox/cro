# Warm-Start Snapshots for MicroVM Model Serving Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement instantaneous warm-start for microVM model serving (`cro serve --warm` and `--snapshot-on-ready`), reducing model serving startup latency from 15-45 seconds to < 100ms by snapshotting guest RAM and CoW rootfs layers upon `/health` readiness.

**Architecture:** A SHA-256 fingerprint of the model reference, inference engine, vCPU/RAM sizing, and container image uniquely keys warm snapshots under `~/.cache/krun-microvm/snapshots/serve/<key>/`. When `--snapshot-on-ready` is set, `ServeManager` pauses the guest once `/health` responds, captures a snapshot via `StateManager::snapshot`, and resumes execution. When `--warm` is requested, `ServeManager` restores the pre-warmed snapshot in < 100ms, updates port forwarding, and starts the microVM.

**Tech Stack:** Rust (2021 Edition), `microvm-core`, `microvm-cli`, `sha2`, `krun-sys`

**Spec:** [`docs/superpowers/specs/2026-11-04-serve-warm-start-snapshots-design.md`](file:///Users/apple/cro/docs/superpowers/specs/2026-11-04-serve-warm-start-snapshots-design.md)

## Global Constraints
- Zero security vulnerabilities (100% clean `cargo audit`).
- Zero compiler warnings: `cargo clippy --all-targets -- -D warnings`.
- Code formatting: `cargo fmt --all -- --check`.
- 100% test pass rate across the workspace.

---

### Task 1: CLI Flags & Cache Key Generation

**Files:**
- Modify: `crates/microvm-cli/src/args.rs`
- Modify: `crates/microvm-core/src/serve/config.rs`
- Test: Unit tests in `crates/microvm-core/src/serve/config.rs` and `crates/microvm-cli/src/tests.rs`

- [ ] Add `--snapshot-on-ready`, `--warm`, and `--warmup-snapshot-tag` to `ServeArgs` in `crates/microvm-cli/src/args.rs`.
- [ ] Add `snapshot_on_ready: bool`, `warm: bool`, and `warm_snapshot_tag: Option<String>` to `ServeConfig`.
- [ ] Implement `ServeConfig::compute_warm_snapshot_key(&self, model_id: &str) -> String` using `sha2::Sha256`.
- [ ] Unit tests for deterministic cache keying across identical vs different configurations.
- [ ] Commit: `feat(serve): add warm boot CLI arguments and deterministic snapshot keying`

---

### Task 2: ServeManager Lifecycle Orchestration (Capture & Restore)

**Files:**
- Modify: `crates/microvm-core/src/state.rs` (ensure runner_config.json is saved and loaded in snapshot/restore)
- Modify: `crates/microvm-core/src/serve/manager.rs`
- Modify: `crates/microvm-cli/src/commands/serve.rs`
- Test: Unit and integration tests in `crates/microvm-core/` and `tests/serve_integration.rs`

- [ ] Fix `StateManager::snapshot` and `StateManager::restore` in `crates/microvm-core/src/state.rs` to read/write `runner_config.json` alongside `.krun_config.json`.
- [ ] Implement `ServeManager::snapshot_warm_serve(data_dir, vm_id, cache_key)`:
  - Calls `StateManager::pause(data_dir, vm_id)`.
  - Atomically captures snapshot into `data_dir/snapshots/serve/.tmp-<cache_key>` and renames to `<cache_key>`.
  - Ensures `StateManager::resume(data_dir, vm_id)` is always called even if snapshot fails.
- [ ] Implement warm boot path in `ServeManager::start`:
  - When `config.warm`: checks if `data_dir/snapshots/serve/<cache_key>` exists.
  - If snapshot exists: restores into `instances/<target_id>/rootfs` via `StateManager::restore`.
  - Purges any stale sockets (`/tmp/*.sock`) and PID files from restored rootfs to prevent `EADDRINUSE`.
  - Uses `config.to_microvm_builder(...).instance_id(&target_id)`: reuses pre-warmed rootfs while dynamically applying the new `host_port` forward and supervisor socket.
  - Spawns VM and measures startup duration.
  - If snapshot does not exist: logs informative fallback message and proceeds with cold boot.
- [ ] Implement cold boot snapshot capture:
  - If `config.snapshot_on_ready` is true (or if `--warm` was specified on first run): captures warm snapshot after `/health` signals readiness.
- [ ] Validate CLI constraints: `--snapshot-on-ready` requires readiness wait (reject `--no-wait`).
- [ ] Commit: `feat(serve): implement automated snapshot capture on ready and warm boot restoration`

---

### Task 3: EGM Goldfish Review & Drift Auditing

**Files:**
- Audit: `docs/superpowers/specs/`
- Report: EGM drift report artifact

- [ ] Dispatch Goldfish review subagents (`egm-design`, `egm-implementation-readiness`, `egm-codebase-details`).
- [ ] Run drift detection comparing all specs in `docs/superpowers/specs/` against codebase reality.
- [ ] Reconcile any identified specification gaps or code divergences.
- [ ] Commit: `docs(egm): record goldfish review findings and drift report`

---

### Task 4: Bootstrap EGM Architectural Contracts ("Peanuts & Hay")

**Files:**
- Create: `krun-microvm/crates/microvm-core/README.md`
- Create: `krun-microvm/crates/microvm-cli/README.md`
- Create: `krun-microvm/crates/microvm-runner/README.md`
- Create: `krun-microvm/crates/containerd-shim-krun/README.md`
- Create: `krun-microvm/crates/krun-sys/README.md`
- Create: `krun-microvm/crates/libgvproxy-sys/README.md`
- Update: `krun-microvm/README.md` (Repository Blueprint)

- [ ] Draft Peanuts contracts with `Overview`, `Interface`, `Invariants`, `Side Effects`, and `Verification`.
- [ ] Draft Repository Blueprint rolling up all crates and global invariants.
- [ ] Verify all relative links and test commands.
- [ ] Commit: `docs(egm): bootstrap architectural contract READMEs across workspace`

---

### Task 5: Integration Testing, Verification, & Release Build

**Files:**
- Modify: `tests/serve_integration.rs`

- [ ] Write integration test verifying warm snapshot creation and restoration cycle.
- [ ] Run `cargo clippy --manifest-path krun-microvm/Cargo.toml --all-targets -- -D warnings` (0 warnings).
- [ ] Run `cargo fmt --manifest-path krun-microvm/Cargo.toml --all -- --check` (0 diffs).
- [ ] Run full workspace test suite `cargo test --workspace`.
- [ ] Run `cargo audit` (0 vulnerabilities).
- [ ] Compile release binaries and install to `~/.local/bin/`.
- [ ] Push to `origin/main`.
