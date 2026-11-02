# CNCF ModelPack Specification Adoption Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Adopt the CNCF ModelPack Specification (`modelpack/model-spec`) in `cro`, providing first-class AI model packaging (`cro model pack`), OCI distribution (`cro model pull`, `cro model list`, `cro model info`), and microVM execution with automatic VirtioFS DAX zero-copy shared memory mapping (`cro run --model <ref>[:mount_path]`).

**Architecture:** 
1. `microvm-core::modelpack::spec` defines CNCF ModelPack schema types (`ModelConfig`, `ModelManifest`, media types `application/vnd.cncf.model.*`).
2. `microvm-core::modelpack::pack` packages local model weights (GGUF, Safetensors, ONNX) into spec-compliant OCI layers with SHA-256 digests.
3. `microvm-core::modelpack::engine` manages pulling, caching, and listing ModelPack artifacts from standard OCI registries.
4. `microvm-core::vm::MicroVmBuilder` supports `.model(...)`, mounting weights via VirtioFS, auto-injecting `MODEL_*` env vars, and provisioning the VirtioFS DAX zero-copy window.
5. `microvm-cli` exposes `cro model` (`pull`, `list`, `info`, `pack`) and `cro run --model`.

**Tech Stack:** Rust, OCI Distribution Spec, CNCF ModelPack Spec, Tokio, Serde JSON/YAML, Tar, SHA-256, VirtioFS DAX.

**Spec:** [`docs/superpowers/specs/2026-11-03-modelpack-spec-design.md`](file:///Users/apple/cro/docs/superpowers/specs/2026-11-03-modelpack-spec-design.md)

---

## Global Constraints

- Full compliance with CNCF ModelPack specification media types:
  - `application/vnd.cncf.model.manifest.v1+json`
  - `application/vnd.cncf.model.config.v1+json`
  - `application/vnd.cncf.model.weight.v1.tar`
  - `application/vnd.cncf.model.weight.config.v1.tar`
  - `application/vnd.cncf.model.tokenizer.v1.tar`
  - `application/vnd.cncf.model.doc.v1.tar`
- Zero regression across existing workspace tests (`make test`).
- Strict clippy clean (`-D warnings`) and Rust formatting (`cargo fmt`).
- Pure-Rust, zero CGO, rootless execution.

## Review Focus

1. Correct validation of ModelPack OCI manifest `artifactType` vs generic container manifests.
2. Layer SHA-256 digests must match actual blob bytes when packing or pulling.
3. Path containment: unpacking model layers must never escape the target directory.
4. Safe fallback if a model does not declare an architecture or precision.
5. Automatic VirtioFS DAX window calculation when weights are in GGUF or Safetensors format.

---

### Task 1: ModelPack Spec Schema & Types

**Files:**
- Create: `krun-microvm/crates/microvm-core/src/modelpack/mod.rs`
- Create: `krun-microvm/crates/microvm-core/src/modelpack/spec.rs`
- Modify: `krun-microvm/crates/microvm-core/src/lib.rs`
- Test: `krun-microvm/crates/microvm-core/src/modelpack/spec.rs`

**Steps:**
- Define media type constants for CNCF ModelPack.
- Implement `ModelFormat` enum: `Gguf`, `Safetensors`, `Onnx`, `PyTorch`, `Mlx`, `Other(String)`.
- Implement `ModelFileRole` enum: `Weight`, `Config`, `Tokenizer`, `Doc`.
- Implement `ModelConfig` struct matching CNCF `config.v1+json`.
- Implement `ModelManifest` parser and builder for `manifest.v1+json`.
- Write unit tests for JSON serialization/deserialization and media type validation.

### Task 2: ModelPack Packaging Engine (`pack_model`)

**Files:**
- Create: `krun-microvm/crates/microvm-core/src/modelpack/pack.rs`
- Modify: `krun-microvm/crates/microvm-core/src/modelpack/mod.rs`
- Test: `krun-microvm/crates/microvm-core/src/modelpack/pack.rs`

**Steps:**
- Implement `ModelPacker::pack(source_dir, output_dir, options)`:
  - Scans `source_dir` for weight files (`*.gguf`, `*.safetensors`, `*.onnx`, `*.bin`), config files (`config.json`), tokenizers (`tokenizer.json`).
  - Creates tar archives for weights, config, and tokenizer layers.
  - Computes exact SHA-256 digests and byte sizes for each layer.
  - Generates `ModelConfig` JSON and computes its digest.
  - Generates OCI Image Manifest with `artifactType: application/vnd.cncf.model.manifest.v1+json`.
  - Outputs standard OCI Image Layout in `output_dir`.
- Write unit tests for packaging mock GGUF and Safetensors models.

### Task 3: ModelPack Client & Pull/List Engine

**Files:**
- Create: `krun-microvm/crates/microvm-core/src/modelpack/engine.rs`
- Modify: `krun-microvm/crates/microvm-core/src/modelpack/mod.rs`
- Test: `krun-microvm/crates/microvm-core/src/modelpack/engine.rs`

**Steps:**
- Implement `ModelEngine::pull_model(client, reference, cache_base)`:
  - Fetches manifest and validates ModelPack `artifactType` or media types.
  - Downloads and parses `ModelConfig`.
  - Unpacks layers into `cache_base/models/<safe_digest>/`.
  - Stores `.modelpack.json` cache descriptor.
- Implement `ModelEngine::list_cached(cache_base)`:
  - Reads cached models and returns metadata (name, format, architecture, precision, files, size).
- Implement `ModelEngine::inspect_model(path_or_ref, cache_base)`:
  - Inspects local directory or cached model.
- Write unit tests for pulling, caching, and listing.

### Task 4: MicroVM Integration & Auto-DAX Window Provisioning

**Files:**
- Modify: `krun-microvm/crates/microvm-core/src/vm.rs`
- Modify: `krun-microvm/crates/microvm-core/src/types.rs`
- Test: `krun-microvm/crates/microvm-core/src/vm.rs`

**Steps:**
- Add `model: Option<ModelAttachment>` to `MicroVmBuilder`.
- Implement `builder.model(reference_or_path, mount_path)`.
- When resolving microVM configuration:
  - If a model is attached, mount it via VirtioFS to `/models/<name>` (or custom path).
  - Automatically detect model format (GGUF, Safetensors) and calculate appropriate DAX window size if not explicitly set.
  - Inject guest environment variables: `MODEL_NAME`, `MODEL_PATH`, `MODEL_FORMAT`, `MODEL_ARCHITECTURE`.
- Write unit tests verifying model attachment, environment injection, and DAX setup.

### Task 5: CLI Subcommand `cro model` & `cro run --model`

**Files:**
- Create: `krun-microvm/crates/microvm-cli/src/commands/model.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/args.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/commands/mod.rs`
- Modify: `krun-microvm/crates/microvm-cli/src/commands/run.rs`
- Test: `krun-microvm/crates/microvm-cli/src/tests.rs`

**Steps:**
- Define `ModelCommands` in `args.rs`:
  - `Pull { model: String, data_dir: Option<PathBuf> }`
  - `List { data_dir: Option<PathBuf> }`
  - `Info { model: String, data_dir: Option<PathBuf> }`
  - `Pack { source: PathBuf, output: PathBuf, name: String, format: String, architecture: Option<String>, precision: Option<String> }`
- Add `--model <ref>[:mount_path]` flag to `cro run`.
- Implement command handlers in `commands/model.rs`.
- Wire into CLI dispatcher in `commands/mod.rs` and `commands/run.rs`.
- Write unit tests for CLI argument parsing.

### Task 6: End-to-End Integration Tests & Verification

**Files:**
- Create: `krun-microvm/crates/microvm-core/tests/modelpack_integration.rs`
- Modify: `README.md`
- Test: Full test suite (`make test`), `make check`, `make clippy`, `make fmt`, `make bench`.

**Steps:**
- Write end-to-end integration test:
  - Pack mock model into ModelPack OCI layout.
  - Pull and cache the ModelPack.
  - Verify inspect/info reports correct architecture, format, and layers.
  - Attach to MicroVM builder and verify VirtioFS mount, DAX config, and env injection.
- Run complete workspace test suite, clippy, fmt, and benchmark harness.
- Update `README.md` with ModelPack usage guide.
