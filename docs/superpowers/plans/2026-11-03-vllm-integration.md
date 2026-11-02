# vLLM Integration & Turn-Key Model Serving Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a first-class model serving subsystem (`cro serve` and alias `cro vllm`) with smart engine auto-dispatch (`vllm`, `llama.cpp`, `uzu`), VirtioFS DAX zero-copy mapping, GPU hardware acceleration, live readiness probing, and declarative Compose/Kubernetes templates paired with the Pingora gateway.

**Architecture:** 
1. `microvm-core::serve::config` defines `ServeEngine` and `ServeConfig`, inspecting ModelPack metadata to auto-dispatch `GGUF` to `llama.cpp` (Vulkan/Metal on Apple Silicon) and `Safetensors` to `vLLM` (PagedAttention / CUDA / CPU), while building container commands and configuring guest resources (8GB RAM default, 4 vCPUs, auto-DAX).
2. `microvm-core::serve::manager` manages the microVM serving lifecycle, executes readiness probing against `/health` and `/v1/models`, formats endpoint summary cards, and handles `--wait` in detached mode.
3. `microvm-cli` adds `cro serve` and `cro vllm [serve]`, dispatching to the serving engine.
4. `examples/vllm-compose.yaml` and `examples/vllm-k8s.yaml` provide declarative production deployments pairing vLLM with `cro`'s Pingora token-metering gateway.

**Tech Stack:** Rust, Tokio, Reqwest, Serde JSON, OCI Distribution Spec, CNCF ModelPack Spec, VirtioFS DAX, Virtio-GPU (Metal / Venus).

**Spec:** [`docs/superpowers/specs/2026-11-03-vllm-integration-design.md`](file:///Users/apple/cro/docs/superpowers/specs/2026-11-03-vllm-integration-design.md)

---

## Global Constraints

- Full compliance with OpenAI-compatible API schemas (`/v1/chat/completions`, `/v1/models`, `/health`).
- Sane serving defaults: guest RAM scaled to 8192 MiB (8 GB) and 4 vCPUs to prevent Linux guest OOM when loading PyTorch/vLLM runtimes.
- Smart auto-dispatch must never crash on Apple Silicon: GGUF models automatically route to `llama.cpp` using Vulkan/Metal virtio-gpu acceleration.
- Zero regression across existing 230 workspace tests (`make test`).
- Strict clippy clean (`-D warnings`) and Rust formatting (`cargo fmt`).
- Pure-Rust, zero CGO, rootless execution.

## Review Focus

1. Correct engine auto-dispatch: GGUF models select `llama.cpp` with `-m /models/<name>/<file>.gguf`; Safetensors models select `vLLM` with `vllm serve /models/<name>`.
2. Port forwarding correctly maps host port (default 8000) to container port 8000 without port collisions.
3. Health check polling handles delayed engine initialization with exponential backoff and timeout (120s).
4. Detached mode (`-d`) with `--wait true` blocks until `/health` returns 200 OK before exiting.
5. Injected ModelPack environment variables and VirtioFS DAX windows match the selected model.

---

### Task 1: Core Serving Configuration & Engine Auto-Dispatch

**Files:**
- Create: `krun-microvm/crates/microvm-core/src/serve/mod.rs`
- Create: `krun-microvm/crates/microvm-core/src/serve/config.rs`
- Modify: `krun-microvm/crates/microvm-core/src/lib.rs`
- Test: `krun-microvm/crates/microvm-core/src/serve/config.rs`

**Steps:**
- [ ] Define `ServeEngine` enum: `Auto`, `Vllm`, `LlamaCpp`, `Uzu`.
- [ ] Implement `ServeConfig` struct:
  - Model path resolution via `ModelEngine::inspect_model`.
  - Auto-dispatch logic:
    - If `Auto`: `ModelFormat::Gguf` $\rightarrow$ `LlamaCpp`, `ModelFormat::Safetensors` / `Onnx` / `PyTorch` $\rightarrow$ `Vllm`, `Uzu` $\rightarrow$ `Uzu`.
  - Image resolution:
    - `vllm`: defaults to `vllm/vllm-openai:latest`.
    - `llamacpp`: defaults to `ghcr.io/ggerganov/llama.cpp:server`.
  - Container command generator:
    - `vllm`: `vllm serve /models/<name> --host 0.0.0.0 --port 8000` + flags.
    - `llamacpp`: `/server -m /models/<name>/<weight_file> --host 0.0.0.0 --port 8000` + flags.
    - `uzu`: `uzu server --model /models/<name> --host 0.0.0.0 --port 8000` + flags.
  - Builder configurator:
    - Applies `cpus(4)`, `memory_mb(8192)` default, `port_forward(host_port, 8000)`, `gpu(true)`, auto-DAX window, and model mount.
- [ ] Re-export `serve` module and types in `microvm-core::lib`.
- [ ] Write unit tests for auto-dispatch and container command generation.
- [ ] Verify with `cargo test -p microvm-core serve::config`.

---

### Task 2: Serving Lifecycle Manager & Health Probing

**Files:**
- Create: `krun-microvm/crates/microvm-core/src/serve/manager.rs`
- Modify: `krun-microvm/crates/microvm-core/src/serve/mod.rs`
- Test: `krun-microvm/crates/microvm-core/src/serve/manager.rs`

**Steps:**
- [ ] Implement `ServeManager`:
  - `start(config: &ServeConfig, detach: bool, wait: bool) -> Result<ServeHandle>`
  - MicroVM startup with formatted banner showing active engine, model, and hardware acceleration.
  - Health check prober:
    - Polls `http://<host>:<port>/health` or `http://<host>:<port>/v1/models` every 500ms–2s with exponential backoff up to 120s timeout.
  - Endpoint summary card generator:
    - Prints Base URL, Chat Completions URL, Models URL, Engine, Format, DAX size, and GPU vRAM.
  - Handles interactive streaming vs detached background execution.
- [ ] Write unit tests for health check retry logic and summary card formatting.
- [ ] Verify with `cargo test -p microvm-core serve::manager`.

---

### Task 3: CLI Subcommand `cro serve` and `cro vllm`

**Files:**
- Modify: `krun-microvm/crates/microvm-cli/src/args.rs`
- Create: `krun-microvm/crates/microvm-cli/src/commands/serve.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/commands/mod.rs`
- Test: `krun-microvm/crates/microvm-cli/src/tests.rs`

**Steps:**
- [ ] Define `ServeArgs`, `VllmArgs`, and `VllmCommands` in `args.rs`.
- [ ] Add `Commands::Serve(Box<ServeArgs>)` and `Commands::Vllm(Box<VllmArgs>)` to `Commands` enum in `args.rs`.
- [ ] Implement command handler in `commands/serve.rs`:
  - Maps `ServeArgs` into `ServeConfig`.
  - Invokes `ServeManager::start`.
- [ ] Wire `Commands::Serve` and `Commands::Vllm` in `commands/mod.rs`.
- [ ] Add CLI parsing tests in `tests.rs` for `cro serve` and `cro vllm serve` with all flags.
- [ ] Verify with `cargo test -p microvm-cli test_cli_parse_serve`.

---

### Task 4: Declarative Multi-Service Compose & Kubernetes Templates

**Files:**
- Create: `examples/vllm-compose.yaml`
- Create: `examples/vllm-k8s.yaml`
- Test: Compose config validation via `cro compose config`

**Steps:**
- [ ] Create `examples/vllm-compose.yaml`:
  - `vllm` service: containerized inference with VirtioFS DAX model mount and GPU acceleration.
  - `gateway` service: Cloudflare Pingora L7 proxy routing `/v1` to `vllm:8000`, enforcing cumulative token ceilings (`max_tokens: 100000`), upstream secret masking, and zero-trust egress allowlisting.
- [ ] Create `examples/vllm-k8s.yaml`:
  - Kubernetes Pod manifest utilizing `runtimeClassName: krun`, mounting ModelPack weights into `/models` and exposing container port 8000.
- [ ] Test compose file parsing via unit test or CLI command: `cro compose config -f examples/vllm-compose.yaml`.

---

### Task 5: End-to-End Integration Tests & Verification

**Files:**
- Create: `krun-microvm/crates/microvm-core/tests/serve_integration.rs`
- Modify: `README.md`
- Test: Full test suite (`make test`), `make check`, `make clippy`, `make fmt`, `make bench`.

**Steps:**
- [ ] Implement `tests/serve_integration.rs`:
  - Pack mock GGUF model and Safetensors model.
  - Test `ServeConfig` auto-dispatch:
    - GGUF resolves to `llama.cpp` with exact `.gguf` file argument.
    - Safetensors resolves to `vLLM` with `/models/<name>` directory argument.
  - Verify `MicroVmBuilder` configuration (guest RAM = 8192 MB, vCPUs = 4, VirtioFS DAX window provisioned, port forward 8000).
- [ ] Update `README.md` with Model Serving section, CLI syntax, and examples.
- [ ] Run full workspace validation: `make check`, `make clippy`, `make fmt`, `make test`, `make bench`.
