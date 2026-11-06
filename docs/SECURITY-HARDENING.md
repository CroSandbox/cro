# Security and stability hardening — 2026-11-06

This change fixes concrete trust-boundary failures found during source review and
regression testing. It is not a certification that every feature is secure or
production-ready.

## Changes

- Python guest results use bounded, validated JSON instead of host-side pickle
  deserialization. Malicious pickle output is rejected without executing it.
  Captured stdout/stderr share a 16 MiB limit; timeouts terminate and reap the CLI.
- OCI digests must be SHA-256 with 64 lowercase hexadecimal characters before
  they enter paths. Downloaded child manifests, cached layers, and offline image
  and model metadata blobs are verified against their digests. Missing offline
  image tags and config blobs now fail instead of selecting defaults.
- Archive extraction rejects symlink-based directory and hard-link escapes,
  unsafe temporary directories, and whiteouts aimed at parent directories.
  Existing hard-linked files are unlinked before replacement. Raw ModelPack and
  artifact files cannot bypass path validation or overwrite host symlink targets.
- Snapshot archives use the hardened extractor in a private staging directory.
  Invalid instance IDs, symlink manifests, and existing restore destinations are
  rejected.
- The embedded HTTP proxy rejects ambiguous framing, chunked encoding, duplicate
  Content-Length/Host headers, and protocol upgrades. Only the authorized request
  body is forwarded; trailing pipelined requests are discarded. Opaque CONNECT
  tunnels are rejected when secrets or token limits require content inspection.
  Token counters saturate instead of wrapping.
- Normal builds require native libkrun. Stub mode requires an explicit `1` or
  `true`; `0` disables it. Release-profile stub builds are rejected. Native Linux
  library build failures no longer get ignored by release CI.
- The Elixir HTTP service binds to loopback, checks local Host and Origin values,
  requires JSON POSTs, bounds headers/bodies, handles fragmented requests, and
  closes sockets reliably. HTTP document strings cannot implicitly read host
  files; file reading requires an explicit trusted local SDK option.
- Hardware document detonation is not implemented. Its HTTP endpoint returns 501;
  SDK analysis and the web UI explicitly identify simulation. Fabricated SDK VM
  telemetry was removed. The UI limits uploaded demo files to 1 MiB.
- Locked Rust dependencies no longer include the two yanked versions found by
  the audit. The Go bridge requires Go 1.27.3 and updates crypto, network, system,
  and DHCP dependencies. The crypto dependency uses the exact patched commit
  identified by GO-2026-5932 because the latest tagged version remained affected.
- CI adds Python, Go, TypeScript, and Elixir checks, Rust/Go/npm advisory checks,
  and locked native release builds.

## Compatibility changes

Python results must be JSON-compatible; arbitrary objects and legacy pickle
results are rejected. The result limit is 8 MiB after base64 decoding. Existing
Python callable serialization still requires a callable that can be pickled and
resolved inside the guest; arbitrary decorated functions and closures are not
validated by VM end-to-end tests in this pass.

The Go bridge requires Go 1.27.3, and Elixir requires 1.18+ for its JSON module.
Installing Go may download its selected toolchain. Stub builds are restricted to
explicit debug/test use. CONNECT with secret injection or a token ceiling now
fails instead of silently bypassing content inspection. Chunked proxy requests
and existing snapshot restore destinations are rejected.

The document UI and detonation reports are demonstrations. Heuristic text
matching, sanitized output, and a zero-pattern result do not establish that an
untrusted document is safe for an agent.

## Validation

On macOS ARM64:

- Native libkrun Rust workspace: 300 tests passed.
- Strict Clippy: workspace and all targets passed with `-D warnings`.
- Rust formatting and Git whitespace checks passed.
- Python SDK: 17 tests passed, including malicious serialization, bounded capture,
  malformed results, and timeout tests.
- Installer: 7 tests passed.
- Elixir: 21 tests/doctests passed, including fragmented HTTP, oversized requests,
  framing ambiguity, host-file disclosure, origin checks, and fail-closed detonation.
- TypeScript SDK: build and 5 tests passed; npm audit reported zero vulnerabilities.
- Go SDK: tests and vet passed. The patched embedded bridge rebuilt successfully;
  its advisory scan reported zero findings.
- Fresh Rust advisory scan: zero known vulnerability advisories and zero yanked
  locked packages. Seven unmaintained-package warnings remain: derivative,
  pin-utils, rustls-pemfile, unsafe-libyaml, zbus_names, zvariant, zvariant_derive.
- A release-profile check confirmed that an explicit test stub is rejected.

## Validation limits and remaining work

Linux runtime behavior, signed release binaries, real guest boots, long-running
workloads, and deployment load/failure recovery have not been exercised here.
Native linking and passing unit/integration tests do not prove hypervisor
isolation under adversarial workloads. Hosted CI has not been observed running.

The seven unmaintained Rust dependency warnings require upstream upgrades or
reviewed parser/network dependency migrations. Native C libraries, libkrun,
firmware/kernel submodules, and the host hypervisor have not received a complete
vulnerability review in this pass.

Archive filesystem checks assume exclusive control of the extraction root.
Do not extract into a directory concurrently writable by an active guest or
another local process. Snapshots include host-side runner configuration; import
only configurations whose host mounts and device access you trust.

The loopback document server has no multi-user authentication and is intended
for a trusted local workstation. The web UI's approvals are demo data and cannot
be used as an authorization control. Token metering depends on visible provider
usage reports and is not a guaranteed financial spending cap. Broader proxy and
HITL payload-binding behavior still need protocol-level adversarial testing.

## References

- Python serialization trust boundary: https://docs.python.org/3/library/pickle.html
- Go toolchain releases: https://go.dev/dl/
- Rust advisories: https://rustsec.org/
- Go vulnerability database: https://vuln.go.dev/
