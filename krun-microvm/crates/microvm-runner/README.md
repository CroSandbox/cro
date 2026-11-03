# microvm-runner

## Overview

`microvm-runner` is the lightweight, dedicated hypervisor supervisor process spawned by `cro` to manage an individual microVM's lifecycle. Running directly on the host, it translates declarative `RunnerConfig` definitions into low-level `libkrun` hypervisor calls, initializes Virtio devices (VirtioFS, Virtio-GPU, Virtio-Net, Virtio-Console), enforces host sandbox boundaries, and runs the control IPC event loop.

## Interface

### CLI Interface
- Binary: `microvm-runner --config <path-to-runner-config.json>`
- Accepts configuration describing rootfs path, vCPU count, RAM size, port forwarding, DAX window allocations, and VirtioFS shares.

### IPC Control Sockets
- `supervisor.sock`: Unix domain socket listening for protocol control frames (`Pause`, `Resume`, `Resize`, `Ping`, `Stats`).
- `exec.sock`: Unix domain socket for executing guest commands via in-guest `krun-init` agent.

### Related Contracts
- Hypervisor FFI Bindings: [`../krun-sys/README.md`](../krun-sys/README.md)
- Core Orchestrator: [`../microvm-core/README.md`](../microvm-core/README.md)
- Root Blueprint: [`../../README.md`](../../README.md)

## Invariants

- **Single MicroVM Per Runner Process**: Each `microvm-runner` process instance manages exactly one `libkrun` VM context and terminates immediately when the guest powers off or halts.
- **Supervisor Handshake Guarantee**: The runner must bind `supervisor.sock` and signal readiness before entering the hypervisor execution loop (`krun_start_enter`).
- **Strict Host Sandboxing**: When sandboxing is enabled, host file access must be strictly confined to the instance rootfs, mapped VirtioFS directories, and runtime sockets.

## Side Effects

- **Hypervisor Allocations**: Allocates host memory and hypervisor vCPUs via `Hypervisor.framework` (macOS) or KVM (Linux).
- **Socket Creation**: Creates `supervisor.sock` and `exec.sock` inside the instance runtime directory.
- **Console Redirection**: Redirects guest serial console output to `console.log` or host stdio.

## Verification

- Unit & Watchdog Tests: `cargo test -p microvm-runner --lib`
- Linting: `cargo clippy -p microvm-runner --all-targets -- -D warnings`
