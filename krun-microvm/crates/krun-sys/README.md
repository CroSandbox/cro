# krun-sys

## Overview

`krun-sys` provides raw and idiomatic Rust Foreign Function Interface (FFI) bindings to `libkrun`, the lightweight C virtualization engine. It abstracts platform-specific hypervisor interfaces (`Hypervisor.framework` on macOS Apple Silicon and KVM on Linux) into typed Rust wrappers.

## Interface

### Core Bindings & Types
- `krun_create_ctx`: Allocates and initializes a new libkrun virtual machine execution context.
- `krun_set_vm_config`: Sets vCPU count and RAM capacity in megabytes.
- `krun_set_root`: Designates the guest root filesystem path.
- `krun_set_gpu_options`: Configures hardware GPU virtualization (Metal pass-through on macOS, Virgl/DRM Venus on Linux).
- `krun_add_vsock_port`: Binds host-guest virtual socket communication channels.
- `krun_start_enter`: Enters the microVM hypervisor loop and hands execution over to guest PID 1.
- `has_feature`: Probes hypervisor feature flags at runtime (e.g. GPU, TSI, DAX).

### Related Contracts
- MicroVM Runner: [`../microvm-runner/README.md`](../microvm-runner/README.md)
- Core Engine: [`../microvm-core/README.md`](../microvm-core/README.md)
- Root Blueprint: [`../../README.md`](../../README.md)

## Invariants

- **NUL-Terminated Strings**: All path and command strings passed across the C boundary must be valid NUL-terminated `CString` pointers without interior NUL bytes.
- **Context Lifetime & Thread Affinity**: A `krun_ctx` handle is `Send` but not `Sync`; it must be initialized and started within its owning thread context.
- **Single Start Execution**: `krun_start_enter` consumes the execution thread and never returns on successful guest termination (exits via `exit()`).

## Side Effects

- **Direct Hypervisor Syscalls**: Directly invokes OS hypervisor APIs to allocate hardware virtual machine instances, guest page tables, and virtual CPU threads.

## Verification

- FFI & NUL Safety Tests: `cargo test -p krun-sys --lib`
- Linting: `cargo clippy -p krun-sys --all-targets -- -D warnings`
