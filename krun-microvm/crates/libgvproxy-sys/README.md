# libgvproxy-sys

## Overview

`libgvproxy-sys` provides Rust FFI bindings to `libgvproxy`, a user-mode network stack derived from `gvisor-tap-vsock`. It enables unprivileged, non-root microVM networking with transparent DNS resolution, host-guest port forwarding, and user-space NAT without requiring host TUN/TAP device drivers or administrative privileges.

## Interface

### Core Bindings & Functions
- `gvproxy_create`: Initializes a user-mode virtual network stack from a JSON configuration string specifying listening endpoints, gateway IP, and forwarding rules.
- `gvproxy_destroy`: Shuts down the virtual network proxy and frees underlying socket resources.
- `gvproxy_version`: Returns the version identifier of the linked gvproxy library.

### Related Contracts
- Core Networking Module: [`../microvm-core/README.md`](../microvm-core/README.md)
- MicroVM Runner: [`../microvm-runner/README.md`](../microvm-runner/README.md)
- Root Blueprint: [`../../README.md`](../../README.md)

## Invariants

- **Valid JSON Configuration**: The configuration passed to `gvproxy_create` must be valid JSON; invalid syntax must immediately return a descriptive error code without panicking or leaking pointers.
- **Clean Destruction**: Every successfully created gvproxy instance must be explicitly released via `gvproxy_destroy` to close open sockets and prevent file descriptor leaks.

## Side Effects

- **Network Socket Binding**: Binds TCP/UDP sockets on the host loopback or specified interfaces for virtual network packet routing.

## Verification

- Lifecycle & JSON Error Tests: `cargo test -p libgvproxy-sys --lib`
- Linting: `cargo clippy -p libgvproxy-sys --all-targets -- -D warnings`
