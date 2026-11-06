# Isolated Dockerfile RUN execution

Dockerfile `RUN` instructions now execute in dedicated foreground microVMs through
`microvm-runner --build-step`. The builder does not execute commands from the
Dockerfile with the host shell or fall back to host PATH when guest execution is
unavailable.

## Execution boundary

1. The builder creates a private, disposable stage rootfs from the selected image.
   Only files explicitly copied into that stage become part of the build context
   visible to the guest; the host context/cache directories are not mounted.
2. A private host request file carries VM settings and command argv, environment,
   and working directory. The build VM uses one vCPU, 512 MiB RAM, host sandboxing,
   no extra host mounts/devices, no forwarded ports, and foreground execution.
3. The runner rejects the explicit libkrun test stub in build mode. It writes the
   command using the existing guest JSON init protocol, which preserves quoted
   arguments and environment values. The config writer does not follow a planted
   config symlink. The original image init config is preserved outside the guest
   share and restored after the step.
4. A real libkrun-backed VM boots for each RUN instruction. Changes remain in the
   disposable stage for subsequent RUN/COPY instructions. The default native TSI
   networking remains available for build dependencies; this change is not an
   egress firewall or a fully disconnected build mode.
5. The runner's exit status determines success. Startup errors, missing executables,
   and nonzero guest exits abort the build. Each RUN has a default ten-minute
   timeout. Timed-out runners are killed and reaped; runner handles also use
   kill-on-drop for cancellation. Guest stage/request temporary directories have
   scoped cleanup. Abrupt host/CLI termination still requires separate crash
   recovery work.
6. Final cache/tag publication happens only after all instructions succeed. Build
   digests now include configuration plus filesystem paths, file contents,
   permission modes, and symlink targets, rather than configuration alone.
   Files are streamed into the hash; symlink targets are not followed. Different
   RUN outputs therefore cannot overwrite one another under a configuration-only
   cache identity. This is a local build identity, not a canonical OCI layer digest.

The host runner process receives only native loader configuration from the host
environment; build variables are supplied separately in guest init JSON. The
macOS loader fallback includes Homebrew/system library locations so libkrun can
find its firmware library without exposing ambient host application credentials
to the guest.

## Requirements and compatibility

- Build/install matching `microvm` and `microvm-runner` binaries with native
  libkrun and libkrunfw. Explicit debug test stubs can run unit tests but cannot
  claim successful RUN execution. Existing builds without RUN do not require a VM.
- Stub archives now use a distinct `krun_test_stub` name, and the build script
  removes legacy `libkrun.a` stub output before linking. This prevents cached stub
  archives from shadowing a native library after switching build modes. macOS CI
  checks that the native runner actually links libkrun.
- Native virtualization must be available. Missing firmware, unsupported hosts,
  or an outdated runner cause an explicit error, with no host execution fallback.
- RUN executes as guest root. Before RUN, only `USER root`, `0`, `root:root`,
  and `0:0` are supported; other identity forms are explicitly rejected instead
  of being silently ignored. A final USER instruction can still be recorded in
  image metadata.
- The CLI uses the ten-minute per-step timeout. Rust callers can configure it:

```rust
let engine = BuildEngine::new(options)
    .with_run_timeout(std::time::Duration::from_secs(1200));
let result = engine.build().await?;
```

- Intermediate stages are now temporary-directory owned and cleaned up after
  success, failure, or normal cancellation. WORKDIR paths are checked against the
  stage root before creating host-side directories.
- Hashing the completed rootfs adds one read pass over regular-file contents.
  Unsupported special files fail explicitly instead of being read as streams.
- This does not implement every Dockerfile feature, guest non-root execution,
  network enforcement, or cleanup after SIGKILL/power loss. It does not change
  the explicitly host-side `exec_in_guest_rootfs` API for trusted callers; the
  Dockerfile builder no longer uses it.

## Verification

The native macOS ARM64 workspace passed 327 Rust tests and strict Clippy.
Regression coverage includes runner failures, absent runners, timeouts, host-write
prevention, private request cleanup, guest config restoration/symlinks, USER and
WORKDIR validation, content hashing, and build-mode safety validation. A separate
stub-mode test verifies that a non-virtualizing runner cannot accept a build step.

Opt-in real VM tests exercise shell and JSON RUN forms, quote preservation,
working directory, explicit guest environment, absence of a host-only environment
variable, host-file invisibility, persistence across steps, failure without tag
publication, and independent cache identities for different RUN outputs.

```bash
cargo build --manifest-path krun-microvm/Cargo.toml \
  -p microvm-cli -p microvm-runner --locked

# Supply a trusted native-architecture Linux rootfs with /bin/sh and /bin/busybox.
CRO_BIN="$PWD/krun-microvm/target/debug/microvm" \
CRO_BUILD_TEST_ROOTFS=/absolute/path/to/linux-rootfs \
python3 -m unittest discover -s tests -p test_build_vm_integration.py -v
```

Real VM tests are skipped when the explicit binary/rootfs inputs are absent, so
ordinary installer CI does not pretend to exercise hardware virtualization. The
three opt-in native tests passed on macOS ARM64. Linux hardware execution remains
unverified by this run.
