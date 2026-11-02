// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end integration tests for CNCF ModelPack specification adoption.

use microvm_core::config::KrunConfig;
use microvm_core::modelpack::{
    CachedModelMetadata, ModelEngine, ModelFileRole, ModelFormat, ModelPackOptions, ModelPacker,
};
use microvm_core::MicroVmBuilder;
use std::fs;

#[tokio::test]
async fn test_modelpack_full_lifecycle_and_microvm_dax_injection() {
    let temp = tempfile::tempdir().unwrap();
    let src_dir = temp.path().join("source_model");
    let oci_layout_dir = temp.path().join("packaged_layout");
    let cache_dir = temp.path().join("cro_cache");

    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();

    // 1. Setup mock model files
    let mock_weights = vec![0x42u8; 1024 * 1024 * 8]; // 8 MB mock weights
    fs::write(src_dir.join("llama-3.2-1b.Q4_K_M.gguf"), &mock_weights).unwrap();
    fs::write(
        src_dir.join("config.json"),
        b"{\"architectures\":[\"LlamaForCausalLM\"],\"model_type\":\"llama\"}",
    )
    .unwrap();
    fs::write(
        src_dir.join("tokenizer.json"),
        b"{\"version\":\"1.0\",\"model\":{\"type\":\"BPE\"}}",
    )
    .unwrap();
    fs::write(
        src_dir.join("README.md"),
        b"# Llama 3.2 1B Instruct\nModel card and usage details.",
    )
    .unwrap();

    // 2. Package into CNCF ModelPack OCI layout
    let pack_options = ModelPackOptions {
        source_dir: src_dir.clone(),
        output_dir: oci_layout_dir.clone(),
        name: "llama-3.2-1b-instruct".to_string(),
        format: ModelFormat::Gguf,
        version: Some("1.0.0".to_string()),
        architecture: Some("llama".to_string()),
        precision: Some("q4_k_m".to_string()),
        context_length: Some(131072),
        tag: Some("latest".to_string()),
    };

    let manifest = ModelPacker::pack(&pack_options).expect("Failed to pack ModelPack");
    assert!(manifest.is_modelpack());
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.layers.len(), 4); // weight, config, tokenizer, doc

    // 3. Inspect the packaged OCI layout
    let layout_meta = ModelEngine::inspect_local_path(&oci_layout_dir)
        .expect("Failed to inspect packaged OCI layout");
    assert_eq!(layout_meta.name, "llama-3.2-1b-instruct");
    assert_eq!(layout_meta.format, ModelFormat::Gguf);
    assert_eq!(layout_meta.architecture.as_deref(), Some("llama"));
    assert_eq!(layout_meta.precision.as_deref(), Some("q4_k_m"));
    assert_eq!(layout_meta.context_length, Some(131072));
    assert_eq!(layout_meta.files.len(), 4);
    assert!(layout_meta.total_size > mock_weights.len() as u64);

    // 4. Test caching and discovery
    let safe_digest = manifest.config.digest.replace(':', "_");
    let cached_model_dir = cache_dir.join("models").join(&safe_digest);
    fs::create_dir_all(&cached_model_dir).unwrap();

    let cached_entry = CachedModelMetadata {
        reference: "ghcr.io/meta-llama/llama-3.2-1b-instruct:latest".to_string(),
        digest: manifest.config.digest.clone(),
        name: "llama-3.2-1b-instruct".to_string(),
        format: ModelFormat::Gguf,
        architecture: Some("llama".to_string()),
        precision: Some("q4_k_m".to_string()),
        context_length: Some(131072),
        files: layout_meta.files.clone(),
        total_size: layout_meta.total_size,
        created_at: 1700000000,
        path: cached_model_dir.clone(),
    };

    fs::write(
        cached_model_dir.join(".modelpack.json"),
        serde_json::to_vec_pretty(&cached_entry).unwrap(),
    )
    .unwrap();

    let cached_list = ModelEngine::list_cached(&cache_dir).expect("Failed to list cached models");
    assert_eq!(cached_list.len(), 1);
    assert_eq!(cached_list[0].name, "llama-3.2-1b-instruct");

    let found_by_name = ModelEngine::find_cached("llama-3.2-1b-instruct", &cache_dir)
        .expect("Failed to find cached model");
    assert!(found_by_name.is_some());

    // 5. Attach ModelPack to MicroVM and verify automatic VirtioFS DAX window and rootfs env injection
    let instance_dir = temp.path().join("vm_instance");
    let instance_rootfs = temp.path().join("vm_rootfs");
    fs::create_dir_all(&instance_dir).unwrap();
    fs::create_dir_all(&instance_rootfs).unwrap();

    // Seed baseline rootfs krun config
    let baseline_krun_cfg = KrunConfig::new(vec!["/bin/sh".to_string()], vec![], None);
    baseline_krun_cfg.write_to(&instance_rootfs).unwrap();

    let builder = MicroVmBuilder::new("alpine:latest").model(
        src_dir.to_str().unwrap(),
        Some("/models/llama3".to_string()),
    );

    let mounts = builder
        .prepare_virtiofs_mounts(&instance_dir, &instance_rootfs, &cache_dir)
        .await
        .expect("Failed to prepare VirtioFS mounts");

    assert_eq!(mounts.len(), 1);
    let model_mount = &mounts[0];
    assert!(model_mount.read_only);

    // Verify auto-provisioned DAX window size for GGUF model (should be at least 512 MiB)
    let dax_size = model_mount
        .dax_window_size_bytes
        .expect("DAX window size was not provisioned for GGUF model");
    assert!(
        dax_size >= 512 * 1024 * 1024,
        "DAX window should be at least 512 MiB, got: {} bytes",
        dax_size
    );

    // Verify guest rootfs configuration has MODEL_* environment variables injected
    let krun_cfg_path = instance_rootfs.join(KrunConfig::FILE_NAME);
    let cfg_content = fs::read_to_string(&krun_cfg_path).unwrap();
    assert!(
        cfg_content.contains("MODEL_NAME=source_model"),
        "Missing MODEL_NAME in guest config: {}",
        cfg_content
    );
    assert!(
        cfg_content.contains("MODEL_PATH=/models/llama3"),
        "Missing MODEL_PATH in guest config: {}",
        cfg_content
    );
    assert!(
        cfg_content.contains("MODEL_FORMAT=gguf"),
        "Missing MODEL_FORMAT in guest config: {}",
        cfg_content
    );
}

#[test]
fn test_modelpack_role_classification() {
    assert_eq!(
        ModelFileRole::Weight.layer_media_type(),
        "application/vnd.cncf.model.weight.v1.tar"
    );
    assert_eq!(
        ModelFileRole::Config.layer_media_type(),
        "application/vnd.cncf.model.weight.config.v1.tar"
    );
    assert_eq!(
        ModelFileRole::Tokenizer.layer_media_type(),
        "application/vnd.cncf.model.tokenizer.v1.tar"
    );
    assert_eq!(
        ModelFileRole::Doc.layer_media_type(),
        "application/vnd.cncf.model.doc.v1.tar"
    );
}
