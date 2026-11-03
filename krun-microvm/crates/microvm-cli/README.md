# microvm-cli

## Overview

`microvm-cli` provides the unified command-line interface (`cro` / `microvm`) and full-screen Ratatui terminal dashboard (Cockpit) for interacting with hardware-isolated microVMs, declarative compose stacks, local AI model serving, and zero-trust developer sandboxes.

## Interface

### Public Commands & Handlers
- `handle_run`: Launches standalone microVMs from OCI images or rootfs bundles with configurable hardware allocation, port forwarding, and GPU virtualization.
- `handle_serve` & `handle_vllm`: Orchestrates local AI model serving with auto-dispatch (vLLM, llama.cpp, Uzu), pre-flight hardware fit auto-sizing, readiness polling, and warm-start snapshot acceleration (`--warm`, `--snapshot-on-ready`).
- `handle_model`: Discovers local models across Hugging Face, LM Studio, Ollama, and evaluates pre-flight hardware suitability (`cro model discover`, `cro model fit`).
- `handle_compose`: Manages multi-microVM declarative application topologies defined in Docker Compose YAML or Kubernetes Pod CRDs.
- `handle_sandbox`: Spawns disposable or persistent developer sandboxes with Copy-on-Write workspace isolation, Git cherry-picking, and Human-in-the-Loop (HITL) approval gateways.
- `handle_ui`: Launches the interactive full-screen TUI cockpit featuring VM monitoring, live console log streaming, active AI serving control, and one-click model launch.

### Related Contracts
- Core Virtualization Engine: [`../microvm-core/README.md`](../microvm-core/README.md)
- MicroVM Supervisor Runner: [`../microvm-runner/README.md`](../microvm-runner/README.md)
- Root Blueprint: [`../../README.md`](../../README.md)

## Invariants

- **Signal Interception & Graceful Teardown**: Interactive foreground microVM sessions must capture `SIGINT` (Ctrl+C) and send graceful termination signals to the runner process before exiting.
- **Snapshot Readiness Precondition**: The `--snapshot-on-ready` serving flag requires readiness verification; passing `--no-wait` with `--snapshot-on-ready` must be rejected with an actionable error message.
- **Dual Binary Compatibility**: Both `cro` and `microvm` binary aliases must provide identical subcommand ergonomics, exit code semantics, and flag behaviors.

## Side Effects

- **Terminal Manipulation**: Acquires raw terminal mode and alternate screen buffer when running interactive TTY microVM sessions or launching the TUI Cockpit.
- **Signal Handling**: Installs global signal handlers for `SIGINT` and `SIGTERM`.
- **Process Orchestration**: Spawns detached or foreground `microvm-runner` processes via `microvm-core`.

## Verification

- Command & Parsing Tests: `cargo test -p microvm-cli --lib`
- Formatting & Hygiene: `cargo fmt -p microvm-cli -- --check`
- Linting: `cargo clippy -p microvm-cli --all-targets -- -D warnings`
