# CNCF ModelPack Specification Adoption Design

**Topic:** Adopting the CNCF ModelPack Specification (`modelpack/model-spec`) in `cro`  
**Date:** 2026-11-03  
**Status:** Proposed (Under Review)  
**Author:** Pair Programming Agent & User  

---

## 1. Intent & The Why

Container images (`application/vnd.oci.image.manifest.v1+json`) are designed for executable root filesystems, but modern cloud-native AI workloads require distributing large, multi-gigabyte model weights (GGUF, Safetensors, ONNX) along with tokenizers, architecture descriptors, and quantization metadata.

The **[CNCF ModelPack Specification](https://github.com/modelpack/model-spec)** provides an open, vendor-neutral standard for packaging and distributing AI models as OCI artifacts using standard container registries (GHCR, Docker Hub, Quay, Harbor). 

By adopting ModelPack as a first-class citizen in `cro`, we enable developers and AI platforms to:
1. Pull and distribute model weights through standard OCI registries without custom storage silos.
2. Directly attach ModelPack models to hardware-isolated microVMs (`cro run --model <ref>`).
3. Leverage Cro's hypervisor capabilities—such as **VirtioFS DAX zero-copy shared memory windows** and **Apple Silicon Metal GPU acceleration**—to run inference directly against the mapped ModelPack weights with zero duplication and sub-100ms startup.
4. Package local fine-tunes and converted GGUF models into standard OCI ModelPack artifacts (`cro model pack`).

---

## 2. Goals, Non-Goals, and Anti-Goals

### Goals
- **Full CNCF ModelPack Spec Compliance**:
  - Manifest `artifactType`: `application/vnd.cncf.model.manifest.v1+json` (and support for `application/vnd.modelpack` variants).
  - Config schema `mediaType`: `application/vnd.cncf.model.config.v1+json` with fields for `name`, `version`, `architecture`, `format` (gguf, safetensors, onnx), `precision` (q4_k_m, fp16, int8), `context_length`, and file descriptors.
  - Standard layer media types: `application/vnd.cncf.model.weight.v1.tar`, `application/vnd.cncf.model.weight.config.v1.tar`, `application/vnd.cncf.model.tokenizer.v1.tar`, `application/vnd.cncf.model.doc.v1.tar`.
- **First-Class CLI Subcommand (`cro model`)**:
  - `cro model pull <model_ref>`: Pulls and unpacks ModelPack artifacts into `~/.cache/krun-microvm/models/<digest>`.
  - `cro model list`: Lists all cached models with format, architecture, precision, and size.
  - `cro model info <model_ref_or_dir>`: Inspects model metadata, layers, and context length.
  - `cro model pack <dir> -t <tag> --name <name> --format <format>`: Bundles local weights and configs into a spec-compliant ModelPack OCI artifact.
- **MicroVM Integration (`cro run --model <ref>[:mount_path]`)**:
  - Automatically mounts model files via VirtioFS into `/models/<name>` (or specified path).
  - Automatically injects guest environment variables (`MODEL_NAME`, `MODEL_PATH`, `MODEL_FORMAT`, `MODEL_ARCHITECTURE`).
  - Automatically recommends and configures VirtioFS DAX window (`--dax`) for zero-copy `mmap` when GGUF/Safetensors weights are detected.
- **Pure-Rust & Zero CGO**: Native implementation in `microvm-core` with no external python or daemon dependencies.

### Non-Goals
- Building an in-process LLM inference engine inside `cro` itself (Cro provides the hardware isolation, VirtioFS DAX memory mapping, and Metal GPU passthrough to run engines like `llama.cpp`, `vLLM`, or custom inference binaries).
- Training models from scratch.

### Anti-Goals
- Breaking backwards compatibility with generic OCI artifacts (`cro artifact`).
- Requiring root/sudo or a running Docker daemon to pull, unpack, or pack ModelPack artifacts.

---

## 3. Approaches Considered

| Approach | Description | Trade-offs | Decision |
|---|---|---|---|
| **A: First-Class `cro model` Subsystem (Recommended)** | Dedicated `cro model` command suite, `ModelConfig` parser, model packaging, and `cro run --model` flag with auto-DAX optimization. | Cleanest UX, domain-specific metadata (precision, context window, format), aligns with Cro's AI sandbox mission. | **Selected (User picked)** |
| **B: Spec Extension to `cro artifact`** | Treat ModelPack as a sub-type of `cro artifact` without a top-level `cro model` command. | Simpler CLI surface, but buries AI-specific model semantics under generic blob commands. | Rejected |
| **C: Consumer-Only Quick Integration** | Only support `pull` and `run --model`, skipping local `pack` capability. | Faster to implement, but leaves users unable to package their own models as ModelPack artifacts. | Rejected |

---

## 4. Technical Architecture

### 4.1 Spec Media Types & Constants
```rust
pub const MODELPACK_MANIFEST_ARTIFACT_TYPE: &str = "application/vnd.cncf.model.manifest.v1+json";
pub const MODELPACK_CONFIG_MEDIA_TYPE: &str = "application/vnd.cncf.model.config.v1+json";
pub const MODELPACK_WEIGHT_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.model.weight.v1.tar";
pub const MODELPACK_CONFIG_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.model.weight.config.v1.tar";
pub const MODELPACK_TOKENIZER_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.model.tokenizer.v1.tar";
pub const MODELPACK_DOC_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.model.doc.v1.tar";
```

### 4.2 Data Models (`crates/microvm-core/src/modelpack/`)
```rust
/// CNCF ModelPack Configuration Specification (`application/vnd.cncf.model.config.v1+json`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    pub format: ModelFormat, // GGUF, Safetensors, ONNX, PyTorch, MLX, Other
    #[serde(default)]
    pub architecture: Option<String>, // e.g. "llama", "mistral", "qwen"
    #[serde(default)]
    pub precision: Option<String>,    // e.g. "q4_k_m", "fp16", "bf16"
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub files: Vec<ModelFileEntry>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFileEntry {
    pub path: String,
    pub role: ModelFileRole, // Weight, Config, Tokenizer, Doc
    pub size: u64,
    #[serde(default)]
    pub digest: Option<String>,
}
```

### 4.3 Engine Operations (`crates/microvm-core/src/modelpack/engine.rs`)
- **`ModelEngine::pull_model(client, reference, cache_base)`**:
  1. Fetches OCI manifest via `client.fetch_manifest(reference)`.
  2. Validates `artifactType == MODELPACK_MANIFEST_ARTIFACT_TYPE` (or checks ModelPack media types).
  3. Downloads and parses `ModelConfig` blob.
  4. Downloads and extracts layers (`weight`, `config`, `tokenizer`, `doc`) into `cache_base/models/<safe_digest>/`.
  5. Writes `.modelpack.json` cache metadata descriptor.
- **`ModelEngine::pack_model(source_dir, output_dir, config)`**:
  1. Scans `source_dir` for weights (e.g. `.gguf`, `.safetensors`), configs (`config.json`), tokenizers (`tokenizer.json`).
  2. Generates tar archives for weights and auxiliary files with SHA-256 digests.
  3. Generates `ModelConfig` JSON.
  4. Generates OCI Image Manifest with correct CNCF ModelPack `mediaType` and `artifactType`.
  5. Packages into an OCI image layout ready for pushing or local execution.
- **`ModelEngine::list_cached(cache_base)`**:
  Returns all locally cached ModelPack instances.

### 4.4 MicroVM Execution & Zero-Copy DAX Integration
When `--model <ref>[:mount_tag]` is provided to `cro run`:
1. `cro` checks local cache (or pulls the ModelPack artifact).
2. Reads `ModelConfig`.
3. Sets up VirtioFS mount to `<guest_mount_point>` (default `/models/<model_name>`).
4. If format is `gguf` or `safetensors`, automatically sizes and activates the VirtioFS DAX window (`--dax`), allowing the guest kernel to map weight files directly into host physical memory without copying.
5. Injects guest environment variables:
   - `MODEL_NAME=<name>`
   - `MODEL_PATH=/models/<name>`
   - `MODEL_FORMAT=<gguf|safetensors>`
   - `MODEL_ARCHITECTURE=<llama|mistral|...>`

---

## 5. Review & Approval Gate

Please review this design specification. Once approved, we will invoke `superpowers:writing-plans` to create the implementation tasks and proceed with execution.
