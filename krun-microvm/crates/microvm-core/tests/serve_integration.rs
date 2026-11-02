// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end integration tests for multi-engine model serving subsystem (`cro serve` / `cro vllm`).

use microvm_core::config::KrunConfig;
use microvm_core::modelpack::{ModelEngine, ModelFormat};
use microvm_core::serve::{ServeConfig, ServeEngine};
use std::fs;

#[tokio::test]
async fn test_serve_auto_dispatch_gguf_to_llamacpp() {
    let temp = tempfile::tempdir().unwrap();
    let model_dir = temp.path().join("mock_gguf_model");
    let cache_dir = temp.path().join("cro_cache");
    let instance_dir = temp.path().join("vm_instance");
    let instance_rootfs = temp.path().join("vm_rootfs");

    fs::create_dir_all(&model_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();
    fs::create_dir_all(&instance_dir).unwrap();
    fs::create_dir_all(&instance_rootfs).unwrap();

    // 1. Create mock GGUF model files
    let dummy_weights = vec![0x42u8; 1024 * 1024 * 4]; // 4 MB dummy weights
    let gguf_file = model_dir.join("qwen-2.5-coder-7b.Q4_K_M.gguf");
    fs::write(&gguf_file, &dummy_weights).unwrap();
    fs::write(model_dir.join("config.json"), b"{\"model_type\":\"qwen2\"}").unwrap();

    // Inspect local model directory
    let meta =
        ModelEngine::inspect_local_path(&model_dir).expect("Failed to inspect mock GGUF directory");
    assert_eq!(meta.format, ModelFormat::Gguf);

    // 2. Build ServeConfig with Auto engine
    let serve_cfg = ServeConfig::new(model_dir.to_str().unwrap())
        .engine(ServeEngine::Auto)
        .port(8000)
        .host("0.0.0.0")
        .max_model_len(4096)
        .gpu(true);

    // 3. Verify format detection & auto-dispatch
    let resolved_engine = serve_cfg.resolve_engine(&meta);
    assert_eq!(resolved_engine, ServeEngine::LlamaCpp);

    let image = serve_cfg.resolve_image(resolved_engine);
    assert_eq!(image, "ghcr.io/ggml-org/llama.cpp:server");

    let cmd = serve_cfg.build_container_cmd(resolved_engine, &meta);
    assert_eq!(cmd[0], "/app/llama-server");
    assert_eq!(cmd[1], "-m");
    assert!(cmd[2].ends_with(".gguf"));
    assert!(cmd.contains(&"--host".to_string()));
    assert!(cmd.contains(&"0.0.0.0".to_string()));
    assert!(cmd.contains(&"-c".to_string()));
    assert!(cmd.contains(&"4096".to_string()));

    // 4. Build MicroVm and verify configuration
    let builder = serve_cfg
        .to_microvm_builder(resolved_engine, &meta, &model_dir)
        .expect("Failed to construct MicroVmBuilder");

    assert_eq!(builder.get_image(), image);
    assert_eq!(builder.get_ram_mib(), 8192);
    assert_eq!(builder.get_vcpus(), 4);
    assert!(builder.get_gpu());
    assert!(builder
        .get_port_forwards()
        .iter()
        .any(|pf| pf.host == 8000 && pf.guest == 8000));

    // Seed baseline krun config in rootfs
    let baseline_cfg = KrunConfig::new(vec!["/bin/sh".to_string()], vec![], None);
    baseline_cfg.write_to(&instance_rootfs).unwrap();

    let mounts = builder
        .prepare_virtiofs_mounts(&instance_dir, &instance_rootfs, &cache_dir)
        .await
        .expect("Failed to prepare VirtioFS mounts");

    assert_eq!(mounts.len(), 1);
    let mount = &mounts[0];
    assert!(mount.read_only);
    assert!(mount.dax_window_size_bytes.is_some());
    assert!(mount.dax_window_size_bytes.unwrap() >= 512 * 1024 * 1024);

    // Verify injected rootfs config
    let krun_cfg_path = instance_rootfs.join(KrunConfig::FILE_NAME);
    let cfg_content = fs::read_to_string(&krun_cfg_path).unwrap();
    assert!(cfg_content.contains("MODEL_PATH=/models/mock_gguf_model"));
    assert!(cfg_content.contains("MODEL_FORMAT=gguf"));
}

#[tokio::test]
async fn test_serve_auto_dispatch_safetensors_to_vllm() {
    let temp = tempfile::tempdir().unwrap();
    let model_dir = temp.path().join("mock_hf_model");
    let cache_dir = temp.path().join("cro_cache");
    let instance_dir = temp.path().join("vm_instance");
    let instance_rootfs = temp.path().join("vm_rootfs");

    fs::create_dir_all(&model_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();
    fs::create_dir_all(&instance_dir).unwrap();
    fs::create_dir_all(&instance_rootfs).unwrap();

    // 1. Create mock Safetensors model
    fs::write(model_dir.join("model.safetensors"), vec![0x33u8; 1024 * 64]).unwrap();
    fs::write(
        model_dir.join("config.json"),
        b"{\"architectures\":[\"LlamaForCausalLM\"],\"model_type\":\"llama\"}",
    )
    .unwrap();
    fs::write(model_dir.join("tokenizer.json"), b"{\"version\":\"1.0\"}").unwrap();

    let meta = ModelEngine::inspect_local_path(&model_dir)
        .expect("Failed to inspect mock Safetensors directory");
    assert_eq!(meta.format, ModelFormat::Safetensors);

    // 2. Build ServeConfig
    let serve_cfg = ServeConfig::new(model_dir.to_str().unwrap())
        .port(8000)
        .max_model_len(8192);

    let resolved_engine = serve_cfg.resolve_engine(&meta);
    assert_eq!(resolved_engine, ServeEngine::Vllm);

    let image = serve_cfg.resolve_image(resolved_engine);
    assert_eq!(image, "vllm/vllm-openai:latest");

    let cmd = serve_cfg.build_container_cmd(resolved_engine, &meta);
    assert_eq!(cmd[0], "vllm");
    assert_eq!(cmd[1], "serve");
    assert_eq!(cmd[2], "/models/mock_hf_model");
    assert!(cmd.contains(&"--max-model-len".to_string()));
    assert!(cmd.contains(&"8192".to_string()));

    // 3. Build MicroVm and verify mounts
    let builder = serve_cfg
        .to_microvm_builder(resolved_engine, &meta, &model_dir)
        .expect("Failed to construct MicroVmBuilder");

    assert_eq!(builder.get_ram_mib(), 8192);
    assert_eq!(builder.get_vcpus(), 4);

    let baseline_cfg = KrunConfig::new(vec!["/bin/sh".to_string()], vec![], None);
    baseline_cfg.write_to(&instance_rootfs).unwrap();

    let mounts = builder
        .prepare_virtiofs_mounts(&instance_dir, &instance_rootfs, &cache_dir)
        .await
        .expect("Failed to prepare VirtioFS mounts");

    assert_eq!(mounts.len(), 1);
    assert!(mounts[0].read_only);

    let krun_cfg_path = instance_rootfs.join(KrunConfig::FILE_NAME);
    let cfg_content = fs::read_to_string(&krun_cfg_path).unwrap();
    assert!(cfg_content.contains("MODEL_FORMAT=safetensors"));
}

#[tokio::test]
async fn test_serve_auto_dispatch_uzu() {
    let temp = tempfile::tempdir().unwrap();
    let model_dir = temp.path().join("mock_uzu_model");
    fs::create_dir_all(&model_dir).unwrap();

    fs::write(model_dir.join("model.uzu"), vec![0x77u8; 1024 * 32]).unwrap();

    let meta =
        ModelEngine::inspect_local_path(&model_dir).expect("Failed to inspect mock Uzu directory");
    assert_eq!(meta.format, ModelFormat::Other("uzu".to_string()));

    let serve_cfg = ServeConfig::new(model_dir.to_str().unwrap()).port(8080);

    let resolved_engine = serve_cfg.resolve_engine(&meta);
    assert_eq!(resolved_engine, ServeEngine::Uzu);
    assert_eq!(
        serve_cfg.resolve_image(resolved_engine),
        "trymirai/uzu:latest"
    );

    let cmd = serve_cfg.build_container_cmd(resolved_engine, &meta);
    assert_eq!(cmd[0], "uzu");
    assert_eq!(cmd[1], "server");
    assert!(cmd.contains(&"--model".to_string()));
    assert!(cmd.contains(&"/models/mock_uzu_model".to_string()));
}

#[tokio::test]
async fn test_serve_explicit_engine_and_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let model_dir = temp.path().join("custom_model");
    fs::create_dir_all(&model_dir).unwrap();
    fs::write(model_dir.join("custom.bin"), b"binary").unwrap();

    let meta = ModelEngine::inspect_local_path(&model_dir).unwrap();

    // Explicitly choose vLLM with custom image and custom extra args
    let serve_cfg = ServeConfig::new(model_dir.to_str().unwrap())
        .engine(ServeEngine::Vllm)
        .container_image("my-registry/custom-vllm:v0.6.0")
        .memory_mb(16384)
        .cpus(8)
        .host_port(9000)
        .host("127.0.0.1")
        .tensor_parallel_size(2);

    let resolved_engine = serve_cfg.resolve_engine(&meta);
    assert_eq!(resolved_engine, ServeEngine::Vllm);
    assert_eq!(
        serve_cfg.resolve_image(resolved_engine),
        "my-registry/custom-vllm:v0.6.0"
    );

    let cmd = serve_cfg.build_container_cmd(resolved_engine, &meta);
    assert!(cmd.contains(&"--tensor-parallel-size".to_string()));
    assert!(cmd.contains(&"2".to_string()));

    let builder = serve_cfg
        .to_microvm_builder(resolved_engine, &meta, &model_dir)
        .unwrap();
    assert_eq!(builder.get_ram_mib(), 16384);
    assert_eq!(builder.get_vcpus(), 8);
    assert!(builder
        .get_port_forwards()
        .iter()
        .any(|pf| pf.host == 9000 && pf.guest == 8000));
}
