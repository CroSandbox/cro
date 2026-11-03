# Workspace Crates

## Overview

The `crates/` directory contains the core Rust workspace components powering the `cro` microVM virtualization suite, ranging from low-level C hypervisor FFI wrappers to high-level CLI binaries and Kubernetes integration shims.

## Subdirectories & Boundaries

- [`microvm-core`](./microvm-core/README.md): Primary virtualization library handling lifecycle orchestration, state persistence, image building, AI model serving, and zero-trust proxying.
- [`microvm-cli`](./microvm-cli/README.md): Unified CLI binary (`cro` / `microvm`) and full-screen Ratatui terminal dashboard (Cockpit).
- [`microvm-runner`](./microvm-runner/README.md): Dedicated hypervisor supervisor process managing individual microVM contexts and IPC sockets.
- [`containerd-shim-krun`](./containerd-shim-krun/README.md): Containerd CRI v2 runtime shim for running Kubernetes Pods inside hardware-isolated microVMs.
- [`krun-operator`](./krun-operator/README.md): Kubernetes CRD controller managing declarative `krun.io/v1alpha1 MicroVm` resources.
- [`krun-sys`](./krun-sys/README.md): Low-level Foreign Function Interface (FFI) bindings to `libkrun` C virtualization engine.
- [`libgvproxy-sys`](./libgvproxy-sys/README.md): Low-level Foreign Function Interface (FFI) bindings to user-mode `libgvproxy` virtual networking.
