# krun-operator

## Overview

`krun-operator` is a Kubernetes native operator and CRD controller that manages declarative `MicroVm` custom resources (`krun.io/v1alpha1`). It reconciles desired microVM topology, resource limits, and network routing directly against the local node's `cro` hypervisor runtime.

## Interface

### CRD Specs & Symbols
- `crd::MicroVm`: Kubernetes Custom Resource definition specifying image, vCPUs, RAM, GPU acceleration, and network port mappings.
- `controller::reconcile`: Reconciler loop managing create, update, and delete events for scheduled `MicroVm` resources.

### Related Contracts
- CRI Shim: [`../containerd-shim-krun/README.md`](../containerd-shim-krun/README.md)
- Core Engine: [`../microvm-core/README.md`](../microvm-core/README.md)
- Root Blueprint: [`../../README.md`](../../README.md)

## Invariants

- **Idempotent Reconciliation**: Successive reconciliation passes for unchanged `MicroVm` custom resources must not restart running microVMs or disrupt network connections.
- **Resource Cleanup on Deletion**: Deleting a `MicroVm` custom resource must trigger a graceful shutdown sequence and reclaim host memory and instance directories.

## Side Effects

- **Kubernetes API Interaction**: Watches and patches `krun.io/v1alpha1 MicroVm` status subresources via the Kubernetes API server.
- **MicroVM Orchestration**: Spawns and manages microVM instances via `microvm-core`.

## Verification

- Spec Serialization Tests: `cargo test -p krun-operator --lib`
- Linting: `cargo clippy -p krun-operator --all-targets -- -D warnings`
