# microvm-core

## Overview

`microvm-core` is the foundational virtualization and microVM orchestration library powering the `cro` ecosystem. It encapsulates the core business logic, lifecycle management, OCI container unpacking, declarative compose parsing, in-process image building, hardware-fit auto-sizing, zero-trust network policy enforcement, and warm-start snapshot management.

## Interface

### Public Types & Symbols
- `microvm_core::vm::MicroVmBuilder`: Fluent builder pattern for configuring and launching hardware-isolated microVMs with vCPUs, RAM, VirtioFS mounts, GPU acceleration, and network proxies.
- `microvm_core::vm::MicroVm`: Live microVM process handle supporting awaitable lifecycle management, interactive TTY guards, and proxy telemetry.
- `microvm_core::state::StateManager`: Instance persistence, snapshot serialization (tar/directory CoW), instant restoration, hot-resizing, and process lifecycle operations.
- `microvm_core::serve::manager::ServeManager`: Inference serving lifecycle manager, `/health` readiness prober, and warm-start snapshot capture/restore orchestrator.
- `microvm_core::serve::config::ServeConfig`: Multi-engine serving configuration supporting auto-dispatch, vLLM, llama.cpp, Uzu, and deterministic cache keying (`compute_warm_snapshot_key`).
- `microvm_core::modelpack::discover::ModelDiscovery`: Cross-host AI model scanner crawling Hugging Face hub, LM Studio, Ollama blobs, and ModelPack caches.
- `microvm_core::modelpack::fit::HardwareFit`: Pre-flight hardware sizing computing RAM requirements, KV cache budgets, and Virtio-GPU shared memory limits.
- `microvm_core::net::egress::EgressProxyServer`: In-guest HTTP/HTTPS filtering proxy implementing domain allowlisting, default-deny, credential masking, and LLM token budgets.
- `microvm_core::net::approval::ApprovalStore`: Human-in-the-Loop (HITL) approval gateway with SHA-256 mutation tamper verification.
- `microvm_core::builder::executor::BuildEngine`: In-process Dockerfile parser and OCI layer synthesizer requiring zero external container daemons.

### Related Contracts
- Runner Subsystem: [`../microvm-runner/README.md`](../microvm-runner/README.md)
- CLI Front-End: [`../microvm-cli/README.md`](../microvm-cli/README.md)
- Low-Level Hypervisor Bindings: [`../krun-sys/README.md`](../krun-sys/README.md)
- Root Blueprint: [`../../README.md`](../../README.md)

## Invariants

- **Rootfs Isolation & Path Traversal Prevention**: All layer unpacks, file mounts, and snapshot archives must strictly pass through `sanitize_tar_path` and `ensure_no_symlink_parents` to prevent Zip Slip / Tar Slip directory traversal attacks.
- **Atomic State Persistence**: Writes to instance metadata (`~/.cache/krun-microvm/state/*.json`) and snapshots (`~/.cache/krun-microvm/snapshots/`) must be atomic using temporary staging files/directories and atomic renames to prevent partial corruption.
- **Fail-Safe Pause Guards**: Any operation that pauses a live microVM (such as warm snapshot capture) must guarantee process resumption on both success and error paths.
- **Deterministic Keying**: Model serving warm-start cache keys must deterministically incorporate model digest, engine, vCPU count, memory allocation, GPU configuration, and runtime arguments to prevent invalid cache hits across incompatible setups.

## Side Effects

- **Filesystem Mutations**: Creates and manages instance runtimes under `~/.cache/krun-microvm/instances/<id>/`, layers under `layers/`, and snapshots under `snapshots/serve/`.
- **Process Spawning**: Spawns and supervises `microvm-runner` child processes and communication sockets (`supervisor.sock`, `exec.sock`).
- **Network Proxying**: Binds loopback TCP ports for egress HTTP/HTTPS inspection and reverse proxy routing.

## Verification

- Unit Tests: `cargo test -p microvm-core --lib`
- Integration Tests: `cargo test -p microvm-core --test serve_integration --test state_integration --test modelpack_integration`
- Linting: `cargo clippy -p microvm-core --all-targets -- -D warnings`
