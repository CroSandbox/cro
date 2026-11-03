# Model Discovery, Hardware Fit, and One-Click Serving Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement universal host model discovery (`cro model discover`), pre-flight hardware fit estimation (`cro model fit`), and one-click model serving in the `cro ui` cockpit with automatic guest memory allocation.

**Architecture:** A fast filesystem crawler in `microvm-core::modelpack::discover` catalogs host models across Hugging Face, LM Studio, Ollama, and local directories. A memory calculator in `microvm-core::modelpack::fit` estimates model weight footprint, KV cache consumption, and guest runtime overhead to compute recommended vRAM/RAM allocations and detect potential OOMs. The `cro ui` cockpit integrates discovered models into Tab 3 with a hotkey to instantly serve any discovered model in an isolated microVM.

**Tech Stack:** Rust (2021 Edition), `microvm-core`, `microvm-cli`, `ratatui 0.30`

**Spec:** [`docs/superpowers/specs/2026-11-04-model-discovery-and-hardware-fit-design.md`](file:///Users/apple/cro/docs/superpowers/specs/2026-11-04-model-discovery-and-hardware-fit-design.md)

## Global Constraints
- Zero security vulnerabilities (100% clean `cargo audit`).
- Strict compiler hygiene: `cargo clippy --all-targets -- -D warnings` must produce 0 warnings.
- Strict code style: `cargo fmt --all -- --check` must produce 0 diffs.
- 100% test pass rate across the workspace.

## Review Focus
- Empty directory handling: If Hugging Face or LM Studio directories do not exist on the host, gracefully skip them without error.
- Large files: Accurately parse multi-gigabyte file sizes without integer overflow.
- Host memory limits: Ensure recommended RAM never exceeds total host physical memory.

---

### Task 1: Model Discovery Subsystem (`microvm-core`)

**Files:**
- Create: `crates/microvm-core/src/modelpack/discover.rs`
- Modify: `crates/microvm-core/src/modelpack/mod.rs`
- Test: Unit tests in `discover.rs`

- [x] Write failing unit tests in `discover.rs` testing directory scanning, format detection (`.gguf`, `.safetensors`), and deduplication.
- [x] Implement `ModelDiscovery::discover(custom_paths: &[PathBuf]) -> Vec<DiscoveredModel>`.
- [x] Implement candidate path resolvers for Hugging Face (`~/.cache/huggingface/hub`), LM Studio (`~/.lmstudio/models`), Ollama (`~/.ollama/models`), and ModelPack cache.
- [x] Run `cargo test -p microvm-core --lib modelpack::discover` to verify tests pass.
- [x] Commit: `feat(modelpack): implement host model discovery subsystem`

---

### Task 2: Hardware Fit Estimator & Auto-Sizing (`microvm-core`)

**Files:**
- Create: `crates/microvm-core/src/modelpack/fit.rs`
- Modify: `crates/microvm-core/src/modelpack/mod.rs`
- Test: Unit tests in `fit.rs`

- [x] Write failing unit tests for `HardwareFit::calculate(model_size_bytes, context_length, host_total, host_available)`.
- [x] Implement `HardwareFit::profile(model_path: &Path, context_length: Option<u64>) -> Result<HardwareFit>`.
- [x] Implement calculation of model weights, KV cache (per-token allocation), runtime overhead (1024 MiB), and verdict (`Comfortable`, `Tight`, `ExceedsMemory`).
- [x] Run `cargo test -p microvm-core --lib modelpack::fit` to verify tests pass.
- [x] Commit: `feat(modelpack): implement hardware fit estimation and memory auto-sizing`

---

### Task 3: CLI Commands (`cro model discover`, `cro model fit`) & `cro serve` Auto-Sizing

**Files:**
- Modify: `crates/microvm-cli/src/args.rs`
- Modify: `crates/microvm-cli/src/commands/model.rs`
- Modify: `crates/microvm-core/src/serve/config.rs` (auto-size RAM if not explicitly passed)
- Test: CLI tests in `crates/microvm-cli/src/tests.rs`

- [x] Add `Discover` and `Fit` subcommands to `ModelCommands` in `args.rs`.
- [x] Implement handlers in `commands/model.rs` rendering clean formatted tables and hardware fit verdict cards.
- [x] Wire `HardwareFit` into `cro serve` so omitting `--memory` dynamically sets the recommended RAM allocation.
- [x] Test CLI parsing and execution with `cargo test -p microvm-cli`.
- [x] Commit: `feat(cli): add cro model discover, cro model fit, and serve auto-memory sizing`

---

### Task 4: One-Click Discovery & Serve in `cro ui` (Cockpit)

**Files:**
- Modify: `crates/microvm-cli/src/ui/app.rs`
- Modify: `crates/microvm-cli/src/ui/views/serve.rs`
- Modify: `crates/microvm-cli/src/ui/mod.rs`

- [x] Add `discovered_models: Vec<DiscoveredModel>` and `selected_discovered: usize` to `App` state.
- [x] Update `views/serve.rs` to render a split view in Tab 3:
  - Top half: Active Serving MicroVMs.
  - Bottom half: Discovered Local Models with format, size, source, and hardware fit badge.
- [x] In `ui/mod.rs`, connect keybindings:
  - `[d]`: Refresh and re-scan host model directories.
  - `[s]` or `[Enter]` on a discovered model: spawn detached inference microVM with auto-calculated memory and auto-dispatched engine.
- [x] Unit tests for `App` model discovery state in `crates/microvm-cli/src/ui/tests.rs`.
- [x] Commit: `feat(ui): add discovered models panel and one-click serve in cockpit tab 3`

---

### Task 5: Workspace Verification & Release Installation

**Files:**
- Full workspace verification

- [x] Run `cargo clippy --manifest-path krun-microvm/Cargo.toml --all-targets -- -D warnings` (0 warnings).
- [x] Run `cargo fmt --manifest-path krun-microvm/Cargo.toml --all -- --check` (0 diffs).
- [x] Run full workspace test suite `cargo test --manifest-path krun-microvm/Cargo.toml --workspace`.
- [x] Run `cargo audit --file krun-microvm/Cargo.lock` (0 vulnerabilities).
- [x] Compile release binaries: `cargo build --manifest-path krun-microvm/Cargo.toml --release`.
- [x] Install updated release binaries to `~/.local/bin/`.
- [x] Commit & push all changes to `origin/main`.
