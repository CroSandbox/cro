# containerd-shim-krun

## Overview

`containerd-shim-krun` is a containerd v2 runtime shim implementing the Container Runtime Interface (CRI) specification. It allows Kubernetes (via containerd or CRI-O) to schedule and execute standard Pod containers inside hardware-isolated microVMs with sub-100ms startup latency and native GPU pass-through.

## Interface

### CRI v2 Services
- `TaskService`: Handles standard CRI container operations: `Create`, `Start`, `Delete`, `Pids`, `Pause`, `Resume`, `Kill`, `Exec`, `ResizePty`, `Update`, `Wait`, `Stats`.
- `read_cri_annotations`: Extracts microVM hardware constraints (vCPUs, RAM, GPU shm size, network mode) from Kubernetes Pod annotations (`krun.io/vcpus`, `krun.io/memory`, `krun.io/gpu`).

### Related Contracts
- Core Engine: [`../microvm-core/README.md`](../microvm-core/README.md)
- MicroVM Supervisor: [`../microvm-runner/README.md`](../microvm-runner/README.md)
- Kubernetes Operator: [`../krun-operator/README.md`](../krun-operator/README.md)
- Root Blueprint: [`../../README.md`](../../README.md)

## Invariants

- **Sandbox Preservation on Container Deletion**: Deleting an individual CRI container inside a Pod must never inadvertently terminate or purge the enclosing Pod sandbox microVM.
- **Graceful Lifecycle Handshakes**: All task state transitions must emit valid containerd lifecycle events over the TTRPC event bus.
- **Clean Socket Teardown**: Upon shim exit, all transient TTRPC and supervisor sockets must be purged to avoid socket collision on Pod restarts.

## Side Effects

- **TTRPC IPC Sockets**: Listens on containerd-assigned Unix domain sockets for RPC invocations.
- **MicroVM Process Supervision**: Spawns and manages underlying `microvm-runner` processes for each active Pod sandbox.

## Verification

- CRI Service & Annotation Tests: `cargo test -p containerd-shim-krun --lib`
- Linting: `cargo clippy -p containerd-shim-krun --all-targets -- -D warnings`
