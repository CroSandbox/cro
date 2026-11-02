# vLLM Integration & Turn-Key Model Serving Design

**Topic:** High-Throughput Model Serving Subsystem (`cro serve` & `cro vllm`) with Smart Engine Auto-Dispatch  
**Date:** 2026-11-03  
**Status:** Approved (Ready for Implementation Planning)  
**Author:** Pair Programming Agent & User  

---

## 1. Intent & The Why

Deploying large language models (LLMs) requires high-throughput inference serving with optimized memory usage (PagedAttention, continuous batching) while maintaining strict tenant isolation, hardware GPU acceleration, and token expenditure control.

With `cro`'s adoption of the **CNCF ModelPack Specification**, developers can package, distribute, and attach AI models (GGUF, Safetensors, ONNX) with sub-100ms microVM cold boots and zero-copy VirtioFS DAX memory mapping.

Integrating **vLLM** and multi-engine serving (`llama.cpp`, `uzu`) as premier first-class capabilities in `cro` completes the end-to-end AI workflow:
1. **Packaging**: `cro model pack` bundles local weights and metadata into standard OCI ModelPacks.
2. **Smart Auto-Dispatch**: `cro serve` (and alias `cro vllm`) inspects ModelPack metadata (`meta.format`) and host hardware to automatically select the optimal engine:
   - **`GGUF`** $\rightarrow$ `llama.cpp` (`ghcr.io/ggerganov/llama.cpp:server`) utilizing Vulkan/Metal hardware acceleration on Apple Silicon via `cro`'s virtio-gpu.
   - **`Safetensors`** $\rightarrow$ `vLLM` (`vllm/vllm-openai:latest`) with continuous batching and PagedAttention on Linux/CUDA or CPU.
   - **`Uzu`** $\rightarrow$ `trymirai/uzu` utilizing Apple Silicon unified memory and speculative decoding.
3. **OpenAI-Compatible API**: Instantly exposes `/v1/chat/completions`, `/v1/completions`, and `/v1/models` ready for LangChain, LlamaIndex, OpenAI Python SDKs, and local AI agent sandboxes (`cro sandbox claude`).
4. **Governance & Budgeting**: Sub-millisecond direct port forwarding (`-p 8000:8000`) with native `--api-key` auth for local serving, paired with `cro`'s Cloudflare Pingora gateway for in-flight cumulative token ceiling enforcement (`max_tokens`) in declarative Compose deployments.
5. **Declarative Multi-Service Stacks**: Provides production-ready Docker Compose and Kubernetes manifests running vLLM side-by-side with zero-trust gateways.

---

## 2. Goals, Non-Goals, and Anti-Goals

### Goals
- **Turn-Key CLI Serving (`cro serve` / `cro vllm`)**:
  - `cro serve <model_ref_or_dir> [options]`: Automatically inspects model metadata via `ModelEngine`, selects the best engine, provisions Virtio-GPU acceleration, sets up the VirtioFS DAX window, attaches model weights to `/models/<name>`, and launches the OpenAI API server.
  - Dedicated alias `cro vllm serve` with identical semantics and vLLM-specific flags.
  - Live readiness probing against `/health` with exponential backoff and formatted endpoint output.
- **Smart Auto-Dispatching**:
  - Automatically routes based on `meta.format` (`Gguf` to `llama.cpp`, `Safetensors` to `vLLM`, `Uzu` to `uzu`).
  - Supports manual override via `--engine <auto|vllm|llamacpp|uzu>`.
- **Sane Resource Allocation**:
  - Sets default guest RAM to `8192` MiB (8 GB) and 4 vCPUs to comfortably accommodate Python runtimes, PyTorch, and KV caches beyond the VirtioFS DAX weight mapping.
- **Detached Mode Ergonomics**:
  - `--wait` (default: `true`) so `cro serve -d` waits for `/health` to return 200 OK before detaching and printing the endpoint URL.
- **Declarative Orchestration**:
  - Ready-to-run Docker Compose spec (`examples/vllm-compose.yaml`) running vLLM + Pingora gateway.
  - Kubernetes Pod manifest (`examples/vllm-k8s.yaml`) running vLLM with `runtimeClassName: krun`.
- **Pure-Rust & Zero CGO**: Implemented natively in `microvm-core` and `microvm-cli`.

### Non-Goals
- Re-compiling or embedding C++/Python vLLM code inside the `cro` host binary itself (vLLM executes inside the microVM guest rootfs/container, while `cro` provides the hardware isolation, VirtioFS DAX mapping, and GPU virtualization).
- Training or fine-tuning models in this subsystem (inference and serving focus).

### Anti-Goals
- Locking `cro serve` exclusively to vLLM in a way that crashes on Apple Silicon or prevents serving GGUF models.
- Requiring root/sudo or a host Docker daemon to serve models.

---

## 3. Approaches Considered

| Approach | Description | Trade-offs | Decision |
|---|---|---|---|
| **1: Extensible `cro serve` with Smart Auto-Dispatch & vLLM Engine (Selected)** | Top-level `cro serve [MODEL]` with smart auto-dispatch (`vllm`, `llama.cpp`, `uzu`), auto-DAX, GPU passthrough, health monitoring, and declarative Compose/K8s specs. Alias `cro vllm` provided. | Cleanest UX, domain-specific serving semantics, prevents Apple Silicon GPU crashes by smartly routing GGUF to Vulkan/llama.cpp and Safetensors to vLLM. | **Selected (User picked)** |
| **2: Standalone `cro vllm` Only** | Separate CLI binary/subcommand strictly named `cro vllm`. | Focused on vLLM, but fragments the CLI surface and fails on macOS when vLLM expects CUDA. | Rejected |
| **3: Generic Flag on `cro run`** | Only add `--vllm` to `cro run` with no dedicated serving abstraction. | Minimal code diff, but poor UX: lacks endpoint output, health monitoring, and dedicated serving parameters. | Rejected |

---

## 4. Technical Architecture

### 4.1 CLI Interface (`crates/microvm-cli/src/args.rs`)

```rust
pub enum Commands {
    // Existing commands...

    /// Serve an AI model using a high-throughput inference engine in an isolated microVM
    Serve(Box<ServeArgs>),

    /// Manage and run vLLM inference microVMs (alias to cro serve --engine vllm)
    Vllm(Box<VllmArgs>),
}

#[derive(Args, Debug, Clone)]
pub struct ServeArgs {
    /// Model reference (OCI ModelPack or local directory)
    pub model: String,

    /// Inference engine (auto, vllm, llamacpp, uzu; default: auto)
    #[arg(long, default_value = "auto")]
    pub engine: String,

    /// Host port to expose the OpenAI-compatible API on (default: 8000)
    #[arg(short = 'p', long, default_value_t = 8000)]
    pub port: u16,

    /// Host interface to bind the API to (default: 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Base container image for the inference engine
    #[arg(long)]
    pub image: Option<String>,

    /// Number of virtual CPUs (default: 4)
    #[arg(short = 'c', long, default_value_t = 4)]
    pub cpus: u8,

    /// Guest RAM in MiB (default: 8192 MiB / 8 GB)
    #[arg(short = 'm', long, default_value_t = 8192)]
    pub memory: u32,

    /// Enable hardware GPU acceleration (Apple Silicon Metal / Linux DRM Venus)
    #[arg(long, default_value_t = true)]
    pub gpu: bool,

    /// Shared memory vRAM window size for GPU (e.g. 4G, 8G)
    #[arg(long, default_value = "4G")]
    pub gpu_shm_size: String,

    /// VirtioFS DAX window size (e.g. 4G; auto-calculated from model size by default)
    #[arg(long)]
    pub dax: Option<String>,

    /// Maximum model context length (tokens)
    #[arg(long)]
    pub max_model_len: Option<u64>,

    /// Number of tensor parallel ranks
    #[arg(long, default_value_t = 1)]
    pub tensor_parallel_size: u32,

    /// Enforce eager execution mode in vLLM (disables CUDA/Metal graph capture)
    #[arg(long)]
    pub enforce_eager: bool,

    /// API key requirement for the exposed OpenAI endpoints
    #[arg(long)]
    pub api_key: Option<String>,

    /// Run the inference server in the background (detached mode)
    #[arg(short = 'd', long)]
    pub detach: bool,

    /// Wait for server /health probe to become ready before exiting in detached mode
    #[arg(long, default_value_t = true)]
    pub wait: bool,

    /// Custom data cache directory
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Additional trailing arguments passed directly to the inference server
    #[arg(last = true)]
    pub extra_args: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct VllmArgs {
    #[command(subcommand)]
    pub command: Option<VllmCommands>,

    #[command(flatten)]
    pub serve: ServeArgs,
}

#[derive(Subcommand, Debug, Clone)]
pub enum VllmCommands {
    /// Serve a ModelPack using the vLLM engine
    Serve(Box<ServeArgs>),
}
```

### 4.2 Core Serving Engine (`crates/microvm-core/src/serve/`)

1. **`ServeEngine` Dispatcher**:
   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum ServeEngine {
       Auto,
       Vllm,
       LlamaCpp,
       Uzu,
   }
   ```
   - If `Auto`, inspects `meta.format`:
     - `ModelFormat::Gguf` $\rightarrow$ `ServeEngine::LlamaCpp`
     - `ModelFormat::Safetensors` / `Onnx` / `PyTorch` $\rightarrow$ `ServeEngine::Vllm`
     - Custom Uzu checkpoint $\rightarrow$ `ServeEngine::Uzu`

2. **`ServeConfig` Builder**:
   - Inspects `model` reference via `ModelEngine::inspect_model`.
   - Resolves model path and files.
   - Determines the target container image and startup arguments:
     - **vLLM**:
       - Default image: `vllm/vllm-openai:latest`
       - Command: `vllm serve /models/<name> --host 0.0.0.0 --port 8000` (plus `--max-model-len`, `--tensor-parallel-size`, `--api-key`, `--enforce-eager`).
     - **llama.cpp**:
       - Default image: `ghcr.io/ggerganov/llama.cpp:server`
       - Locates `.gguf` weight file in `/models/<name>`
       - Command: `/server -m /models/<name>/<weight_file> --host 0.0.0.0 --port 8000` (plus `-c <context_length>`, `--api-key`).
     - **uzu**:
       - Command: `uzu server --model /models/<name> --host 0.0.0.0 --port 8000`.
   - Configures `MicroVmBuilder`:
     - `builder = builder.cpus(serve_args.cpus).memory_mb(serve_args.memory)`
     - Attaches model: `builder.model(model_path, Some(format!("/models/{}", model_name)))`.
     - Port forwarding: `builder.port_forward(host_port, 8000)`.
     - GPU configuration: `builder.gpu(gpu).gpu_shm_size(gpu_bytes)`.
     - DAX shared memory window: auto-calculated to fit the weights with zero-copy.

3. **`ServeManager` & Health Probe**:
   - Spawns the microVM.
   - Polls `http://<host>:<port>/health` or `http://<host>:<port>/v1/models` with exponential backoff until ready (or timeout after 120s).
   - Once ready, prints endpoint summary card:
     ```
     🚀 Inference server is live!
        Base URL:          http://127.0.0.1:8000/v1
        Chat Completions:  http://127.0.0.1:8000/v1/chat/completions
        Models Endpoint:   http://127.0.0.1:8000/v1/models
        Active Engine:     vLLM (OpenAI-compatible)
        Model:             llama-3.2-1b-instruct (Format: safetensors, DAX: 2048 MB)
        Guest Resources:   4 vCPUs, 8192 MB RAM, Virtio-GPU (4096 MB vRAM)
     ```
   - In interactive mode: stays attached and streams stdout/stderr until Ctrl+C.
   - In detached mode (`-d`): exits cleanly with status 0 once `/health` is confirmed.

### 4.3 Declarative Compose & Kubernetes Specs

1. **`examples/vllm-compose.yaml`**:
   - Multi-container compose project:
     - `vllm`: Backend serving container mounted via VirtioFS with DAX.
     - `gateway`: Pingora reverse proxy gateway routing `/v1` to `vllm:8000`, enforcing cumulative token ceilings (`max_tokens: 100000`), upstream secret replacement, and zero-trust domain allowlisting.

2. **`examples/vllm-k8s.yaml`**:
   - Kubernetes Pod specification using `runtimeClassName: krun`, mounting ModelPack weights and exposing container port 8000.

---

## 5. Verification Plan

1. **Unit Tests**:
   - `ServeConfig` resolution and argument generation for `vLLM`, `llama.cpp`, and `uzu`.
   - Engine auto-dispatch logic verifying `GGUF` $\rightarrow$ `llama.cpp` and `Safetensors` $\rightarrow$ `vLLM`.
   - CLI argument parsing for `cro serve` and `cro vllm serve`.
2. **Integration Test**:
   - `krun-microvm/crates/microvm-core/tests/serve_integration.rs`:
     - Creates mock ModelPack (GGUF and Safetensors).
     - Verifies `ServeConfig` properly configures `MicroVmBuilder`, auto-dispatch, VirtioFS DAX, GPU settings, memory sizing (8192 MB), and port forwards.
3. **Workspace Verification**:
   - Full test suite: `make test`
   - Strict linting: `make clippy` (`-D warnings`)
   - Formatting: `make fmt`
   - Startup benchmarks: `make bench`
