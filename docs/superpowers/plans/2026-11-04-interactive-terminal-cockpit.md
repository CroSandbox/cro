# Interactive Multi-Tab Terminal Cockpit (`cro ui`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a native, zero-dependency, interactive multi-tab terminal cockpit (`cro ui`, `cro cockpit`, `cro top -i`) using `ratatui` and `crossterm` to monitor microVMs, stream live console logs, inspect AI model serving engines, and interactively approve/reject agent HITL requests.

**Architecture:** A non-blocking async terminal event loop (`ratatui` + `crossterm`) coupled with a centralized `App` state struct. The dashboard renders 4 distinct tab views (`Instances`, `Live Logs`, `AI Serving`, `HITL Approvals`) alongside a host header bar and contextual hotkey footer, polling `microvm-core` APIs (`StateManager`, `collect_process_stats`, `ApprovalStore`) at 500ms intervals without blocking terminal input responsiveness.

**Tech Stack:** Rust (2021 Edition), `ratatui = "0.30"`, `crossterm = "0.29"`, `tokio`, `microvm-core`

**Spec:** [`docs/superpowers/specs/2026-11-04-interactive-terminal-cockpit-design.md`](file:///Users/apple/cro/docs/superpowers/specs/2026-11-04-interactive-terminal-cockpit-design.md)

## Global Constraints
- Zero vulnerability guarantee: all added dependencies must be 100% clean in `cargo audit` (0 CVEs, 0 advisories).
- Strict compiler hygiene: `cargo clippy --all-targets -- -D warnings` must produce 0 warnings.
- Strict code style: `cargo fmt --all -- --check` must produce 0 diffs.
- 100% test pass rate across the workspace.
- Terminal safety: panic hooks and signal handlers must always restore raw mode and alternate screen on exit.

## Review Focus
- Small terminal dimensions: gracefully render a clean resize alert if dimensions are below 80x24.
- Empty state rendering: render informative empty placeholder panels when no microVMs or approvals exist.
- Non-blocking I/O: background log file tailing and process stats polling must never freeze keyboard navigation.
- Safe process lifecycle: pressing `s` sends `SIGTERM` without deleting instance logs; `d` cleans up stopped instances.
- Fast HITL action: pressing `y`/`n` in Tab 4 immediately updates `ApprovalStore` and shows a confirmation toast.

---

### Task 1: Add Dependencies & Verify Zero Vulnerability

**Files:**
- Modify: `crates/microvm-cli/Cargo.toml`
- Test: `cargo audit --file Cargo.lock`

- [ ] Add `ratatui = "0.30"` and `crossterm = "0.29"` to `crates/microvm-cli/Cargo.toml`.
- [ ] Run `cargo check --manifest-path krun-microvm/Cargo.toml` to lock dependencies.
- [ ] Run `cargo audit --file krun-microvm/Cargo.lock` to verify 0 vulnerabilities.
- [ ] Commit: `build: add ratatui and crossterm dependencies for terminal cockpit`

---

### Task 2: Core TUI State Model & Unit Tests

**Files:**
- Create: `crates/microvm-cli/src/ui/app.rs`
- Create: `crates/microvm-cli/src/ui/tests.rs`
- Create: `crates/microvm-cli/src/ui/mod.rs`

- [ ] Write failing unit tests in `crates/microvm-cli/src/ui/tests.rs` verifying:
  - Tab cycling (`next_tab`, `prev_tab`, `select_tab`).
  - Instance selection clamping and cursor wrapping.
  - CPU sparkline history buffer bounds (capped at 60 data points).
  - Toast notification expiration.
- [ ] Implement `App` and `Tab` in `crates/microvm-cli/src/ui/app.rs`.
- [ ] Run `cargo test --manifest-path krun-microvm/Cargo.toml -p microvm-cli` to confirm tests pass.
- [ ] Commit: `feat(ui): implement App state model, tab navigation, and telemetry buffers`

---

### Task 3: Terminal Setup, Panic Hook, and Event Loop

**Files:**
- Create: `crates/microvm-cli/src/ui/events.rs`
- Modify: `crates/microvm-cli/src/ui/mod.rs`

- [ ] Implement `events.rs` with `EventStream` handling `Event::Key`, `Event::Resize`, and `Event::Tick(250ms)`.
- [ ] Implement `setup_terminal()` and `restore_terminal()` in `ui/mod.rs` with custom `std::panic::set_hook` to guarantee raw mode restoration.
- [ ] Implement background async data fetcher in `App::tick` (refreshing `StateManager::list`, CPU/MEM stats, `ApprovalStore::list_all`).
- [ ] Commit: `feat(ui): implement terminal initialization, panic hook, and async event loop`

---

### Task 4: Multi-Tab View Components

**Files:**
- Create: `crates/microvm-cli/src/ui/views/mod.rs`
- Create: `crates/microvm-cli/src/ui/views/header.rs`
- Create: `crates/microvm-cli/src/ui/views/footer.rs`
- Create: `crates/microvm-cli/src/ui/views/instances.rs`
- Create: `crates/microvm-cli/src/ui/views/logs.rs`
- Create: `crates/microvm-cli/src/ui/views/serve.rs`
- Create: `crates/microvm-cli/src/ui/views/approvals.rs`

- [ ] Implement `header.rs`: displays host CPU cores, memory gauge, hypervisor mode (`HVF`/`KVM`), and instance count badges (`Running`, `Paused`, `Stopped`).
- [ ] Implement `instances.rs`: split-pane layout with microVM table on the left and selected VM sparklines, memory gauge, and inspect metadata on the right.
- [ ] Implement `logs.rs`: monospace console log reader with auto-tailing, scroll offset, and line count.
- [ ] Implement `serve.rs`: inference engine table showing active models, engine types (`vllm`, `llamacpp`, `uzu`), URLs, and memory.
- [ ] Implement `approvals.rs`: HITL network interception requests queue with destination host/port and action hints.
- [ ] Implement `footer.rs`: contextual keyboard shortcuts bar.
- [ ] Run `cargo check --manifest-path krun-microvm/Cargo.toml` to verify rendering logic compiles cleanly.
- [ ] Commit: `feat(ui): implement header, instances, logs, serve, approvals, and footer views`

---

### Task 5: Interactive Lifecycle Controls & CLI Integration

**Files:**
- Modify: `crates/microvm-cli/src/args.rs`
- Modify: `crates/microvm-cli/src/commands/mod.rs`
- Create: `crates/microvm-cli/src/commands/ui.rs`
- Modify: `crates/microvm-cli/src/commands/system.rs` (connect `cro top -i`)

- [ ] Add `Ui` subcommand to `Commands` in `args.rs` (with aliases `cockpit`, and `--interactive` / `-i` flag on `Top`).
- [ ] Connect keybindings in `App::handle_key`:
  - `p`: Pause selected VM (`StateManager::pause`).
  - `r`: Resume selected VM (`StateManager::resume`).
  - `s`: Graceful stop via `SIGTERM`.
  - `x`: Force kill via `SIGKILL`.
  - `d`: Delete stopped VM directory via `StateManager::remove`.
  - `y`: Approve pending HITL request (`ApprovalStore::resolve_request`).
  - `n`: Reject pending HITL request.
  - `Space`: Toggle log follow mode.
- [ ] Wire `handle_ui` in `commands/ui.rs` and dispatch from `commands/mod.rs`.
- [ ] Run `cargo check --manifest-path krun-microvm/Cargo.toml` and verify CLI flags parse.
- [ ] Commit: `feat(cli): bind cro ui and cro top -i with full lifecycle hotkey controls`

---

### Task 6: Comprehensive Verification & Release Installation

**Files:**
- Full workspace verification

- [ ] Run `cargo clippy --manifest-path krun-microvm/Cargo.toml --all-targets -- -D warnings` (must be 0 warnings).
- [ ] Run `cargo fmt --manifest-path krun-microvm/Cargo.toml --all -- --check` (must be 0 diffs).
- [ ] Run full workspace test suite `cargo test --manifest-path krun-microvm/Cargo.toml --workspace` (all 256+ tests must pass).
- [ ] Run `cargo audit --file krun-microvm/Cargo.lock` to verify 0 vulnerabilities.
- [ ] Compile release binaries with `cargo build --manifest-path krun-microvm/Cargo.toml --release`.
- [ ] Install updated release binaries to `~/.local/bin/microvm` (symlinked as `~/.local/bin/cro`).
- [ ] Test `cro ui --help` to verify CLI command registration.
- [ ] Commit & push all changes to `origin/main`.
