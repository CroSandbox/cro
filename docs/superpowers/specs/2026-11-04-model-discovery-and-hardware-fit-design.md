# Design Document: Local Model Discovery, Pre-flight Hardware Fit, and One-Click Serving

**Status**: Draft  
**Author**: Antigravity  
**Date**: 2026-11-04  
**Target Component**: `krun-microvm` (`microvm-core::modelpack`, `microvm-core::serve`, `microvm-cli`)  

---

## 1. Intent & Context

Serving LLMs in microVMs currently requires users to either know the exact filesystem path to model files or manually pull models through the CNCF ModelPack registry. Users frequently have existing models stored across tools such as Hugging Face Hub, LM Studio, Ollama, and local downloads. Furthermore, determining whether a model will fit inside host memory without causing an Out-Of-Memory (OOM) panic requires mental calculation of weight sizes, KV cache allocation, and hypervisor overhead.

Inspired by patterns from `AlexsJones/llmserve` and `llmfit`, this feature introduces:
1. **Universal Local Model Discovery**: Automatically crawls standard host model directories (`~/.cache/huggingface`, `~/.lmstudio`, `~/.ollama`, `~/models`) to catalog available GGUF, Safetensors, and ModelPack weights.
2. **Pre-flight Hardware Fit Estimation**: Calculates exact guest memory requirements ($\text{Weights} + \text{KV Cache} + \text{Hypervisor Overhead}$) and compares against host capacity, warning if a model will OOM before launching.
3. **One-Click Serve Integration in `cro ui`**: Displays discovered models in Tab 3 of the terminal cockpit, allowing developers to start an isolated microVM with auto-calculated memory and auto-dispatched engines with a single keystroke.

---

## 2. Goals, Non-Goals, and Anti-Goals

### Goals
- **Multi-Directory Discovery Engine**: Recursively scan common model repositories on macOS and Linux with file deduplication and symlink resolution.
- **Hardware Fit Analyzer**: Implement `ModelEngine::calculate_hardware_fit(&model, context_length)` calculating:
  - Model weights size on disk.
  - Estimated KV cache memory for a given context length.
  - Hypervisor and Linux guest runtime overhead.
  - Recommended guest RAM (`--memory`) and Virtio-GPU shared memory (`--gpu-shm-size`).
  - Fit verdict: `FitVerdict::Comfortable`, `FitVerdict::Tight`, `FitVerdict::ExceedsMemory`.
- **CLI Commands**:
  - `cro model discover`: Scan and list all discovered host models with format, size, and origin ecosystem.
  - `cro model fit <model>`: Run pre-flight memory profiling against current host hardware.
  - Auto-sizing in `cro serve`: If `--memory` is omitted, auto-size based on the fit calculation.
- **Cockpit Integration**: Enhance Tab 3 in `cro ui` to list discovered models and allow starting an inference microVM via `[s]`.

### Non-Goals
- Modifying or tampering with external files in `~/.ollama` or `~/.lmstudio` (read-only discovery).
- Downloading remote models automatically (discovery is strictly offline/local).

### Anti-Goals
- **Crashing or OOMing the host**: Never default to a memory allocation that exceeds available host physical RAM.

---

## 3. Constraints

- **Zero Security Vulnerabilities**: Strict path validation; prevent symlink recursion outside user directories.
- **Clean Fallback**: If standard directories don't exist on the host, fail gracefully with zero errors.
- **Performance**: Directory scanning must complete in < 250ms by checking specific candidate prefixes and pruning non-model subtrees.

---

## 4. Approaches

### Approach 1: Integrated Discovery & Fit Subsystem in `microvm-core` (Selected)
- Implement `ModelDiscovery` and `HardwareFit` modules directly inside `crates/microvm-core/src/modelpack/`.
- Wire into `cro model discover`, `cro model fit`, `cro serve` auto-sizing, and `cro ui` Tab 3.
- **Why Selected**: Seamlessly unifies CLI commands, hypervisor engine dispatch, and interactive TUI cockpit without code duplication.

---

## 5. Architectural Specification

### 5.1. Model Discovery (`crates/microvm-core/src/modelpack/discover.rs`)
Standard host scan paths:
- `$HOME/.cache/huggingface/hub/**` (snapshots with `*.gguf`, `*.safetensors`)
- `$HOME/.lmstudio/models/**` and `$HOME/.cache/lm-studio/models/**`
- `$HOME/.ollama/models/manifests/**`
- `$HOME/.cache/krun-microvm/models/**`
- `$HOME/models/**`
- Custom paths configured by user.

Discovered model struct:
```rust
pub struct DiscoveredModel {
    pub name: String,
    pub path: PathBuf,
    pub format: ModelFormat,
    pub size_bytes: u64,
    pub source: ModelSource,
}
```

### 5.2. Hardware Fit Sizing (`crates/microvm-core/src/modelpack/fit.rs`)
```rust
pub struct HardwareFit {
    pub model_size_bytes: u64,
    pub estimated_kv_cache_bytes: u64,
    pub runtime_overhead_bytes: u64,
    pub total_required_bytes: u64,
    pub host_total_bytes: u64,
    pub host_available_bytes: u64,
    pub recommended_guest_ram_mib: u32,
    pub recommended_gpu_shm_gib: u32,
    pub verdict: FitVerdict,
}

pub enum FitVerdict {
    Comfortable, // < 70% available host RAM
    Tight,       // 70% - 95% available host RAM
    ExceedsMemory, // > 95% available host RAM
}
```

---

## 6. What Is Left to the Builder

- Exact KV cache estimation heuristics per model architecture.
- Styling of `cro model fit` terminal output card.
- Unit and integration tests.
