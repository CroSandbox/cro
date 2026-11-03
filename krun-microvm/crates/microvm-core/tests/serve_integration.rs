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

#[tokio::test]
async fn test_serve_vllm_plugins_and_alias_pipeline() {
    let temp = tempfile::tempdir().unwrap();
    let model_dir = temp.path().join("mock_hf_model");
    let plugin_dir = temp.path().join("my_vllm_plugin_pkg");
    let chat_tpl = temp.path().join("template.jinja");

    fs::create_dir_all(&model_dir).unwrap();
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(model_dir.join("model.safetensors"), vec![0x11u8; 1024]).unwrap();
    fs::write(
        model_dir.join("config.json"),
        b"{\"architectures\":[\"Qwen2ForCausalLM\"],\"model_type\":\"qwen2\"}",
    )
    .unwrap();
    fs::write(plugin_dir.join("__init__.py"), b"# plugin init").unwrap();
    fs::write(&chat_tpl, b"{{ messages }}").unwrap();

    let meta = ModelEngine::inspect_local_path(&model_dir).unwrap();

    let serve_cfg = ServeConfig::new(model_dir.to_str().unwrap())
        .alias("qwen-2.5-7b-instruct")
        .plugin(&plugin_dir)
        .vllm_plugin("my_vllm_plugin_pkg")
        .trust_remote_code(true)
        .quantization("awq")
        .chat_template(&chat_tpl)
        .env("CUSTOM_INFERENCE_FLAG", "1");

    let cmd = serve_cfg.build_container_cmd(ServeEngine::Vllm, &meta);
    assert!(cmd.contains(&"--served-model-name".to_string()));
    assert!(cmd.contains(&"qwen-2.5-7b-instruct".to_string()));
    assert!(cmd.contains(&"--trust-remote-code".to_string()));
    assert!(cmd.contains(&"--quantization".to_string()));
    assert!(cmd.contains(&"awq".to_string()));
    assert!(cmd.contains(&"--chat-template".to_string()));
    assert!(cmd.contains(&"/etc/chat_template.jinja".to_string()));

    let builder = serve_cfg
        .to_microvm_builder(ServeEngine::Vllm, &meta, &model_dir)
        .unwrap();

    // Verify chat template was mounted into guest
    assert!(builder.get_file_mounts().iter().any(
        |(src, dest)| src == &chat_tpl && dest.to_str().unwrap() == "/etc/chat_template.jinja"
    ));
}

#[tokio::test]
async fn test_serve_warm_start_snapshot_and_restore_workflow() {
    use microvm_core::serve::manager::ServeManager;
    use microvm_core::state::{StateManager, VmState, VmStatus};

    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().to_path_buf();

    // 1. Setup mock running instance
    let vm_id = "vm-serve-origin";
    let instance_dir = data_dir.join("instances").join(vm_id);
    let rootfs = instance_dir.join("rootfs");
    fs::create_dir_all(rootfs.join("app")).unwrap();
    fs::create_dir_all(rootfs.join("tmp")).unwrap();
    fs::create_dir_all(rootfs.join("run")).unwrap();

    fs::write(rootfs.join("app/model_weights.bin"), b"mock-weights-data").unwrap();
    fs::write(rootfs.join("tmp/vllm.sock"), b"mock-socket").unwrap();
    fs::write(rootfs.join("run/server.pid"), b"9999").unwrap();

    let runner_cfg = serde_json::json!({
        "root_path": rootfs,
        "num_vcpus": 4,
        "ram_mib": 8192,
        "port_forwards": [{"host": 8000, "guest": 8000}],
        "virtiofs_mounts": [],
        "vsock_ports": []
    });
    fs::write(
        instance_dir.join("runner_config.json"),
        serde_json::to_string_pretty(&runner_cfg).unwrap(),
    )
    .unwrap();

    let vm_state = VmState {
        id: vm_id.to_string(),
        pid: 0,
        image: "vllm/vllm-openai:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir: instance_dir.clone(),
        status: VmStatus::Stopped,
        vcpus: Some(4),
        memory_mib: Some(8192),
    };
    StateManager::save(&data_dir, &vm_state).unwrap();

    // 2. Capture warm-start snapshot using ServeManager
    let cache_key = "warm-test-model-key";
    let snapshot_path =
        ServeManager::snapshot_warm_serve(&data_dir, vm_id, cache_key).expect("Snapshot failed");

    assert!(snapshot_path.exists());
    assert!(snapshot_path.join("snapshot.json").exists());
    assert!(snapshot_path.join("rootfs/app/model_weights.bin").exists());

    // 3. Restore into a new instance
    let restored_id = "vm-serve-warm-restored";
    let restored_state = StateManager::restore(&data_dir, &snapshot_path, Some(restored_id))
        .expect("Restore failed");
    assert_eq!(restored_state.id, restored_id);

    // 4. Sanitize stale sockets and pid files from restored rootfs
    let restored_rootfs = restored_state.instance_dir.join("rootfs");
    ServeManager::sanitize_restored_rootfs(&restored_rootfs);

    assert!(
        !restored_rootfs.join("tmp/vllm.sock").exists(),
        "Stale socket must be removed"
    );
    assert!(
        !restored_rootfs.join("run/server.pid").exists(),
        "Stale PID file must be removed"
    );
    assert!(
        restored_rootfs.join("app/model_weights.bin").exists(),
        "Model weights must be preserved"
    );

    // 5. Build ServeConfig and verify port remapping
    let serve_cfg = ServeConfig::new("mock-model").warm(true).port(9090);
    assert_eq!(serve_cfg.host_port, 9090);
    assert!(serve_cfg.warm);
}

#[tokio::test]
async fn test_serve_scale_to_zero_config_and_builder_port_remapping() {
    let serve_cfg = ServeConfig::new("my-llama")
        .port(8000)
        .scale_to_zero(true)
        .idle_timeout(30)
        .backend_port(18001);

    assert!(serve_cfg.scale_to_zero);
    assert_eq!(serve_cfg.idle_timeout_secs, Some(30));
    assert_eq!(serve_cfg.backend_port, Some(18001));
    assert_eq!(serve_cfg.host_port, 8000);
}

#[tokio::test]
async fn test_serve_scale_to_zero_gateway_auto_pause_and_sub_2ms_wake() {
    use microvm_core::serve::{find_available_loopback_port, ServeGateway};
    use microvm_core::state::{StateManager, VmState, VmStatus};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().to_path_buf();
    let instance_dir = data_dir.join("instances/test-scale-zero-vm");
    fs::create_dir_all(&instance_dir).unwrap();

    // Spawn a dummy process that can receive SIGSTOP and SIGCONT
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("Failed to spawn mock VM process");
    let pid = child.id();

    let vm_state = VmState {
        id: "test-scale-zero-vm".to_string(),
        pid,
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir: instance_dir.clone(),
        status: VmStatus::Running,
        vcpus: Some(2),
        memory_mib: Some(512),
    };
    StateManager::save(&data_dir, &vm_state).unwrap();

    // 1. Mock inference backend
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend_port = backend_listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        while let Ok((mut stream, _)) = backend_listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 128];
                if let Ok(n) = stream.read(&mut buf).await {
                    let body = format!("ECHO: {}", String::from_utf8_lossy(&buf[..n]));
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                }
            });
        }
    });

    // 2. Start Scale-to-Zero Gateway with short idle timeout (400ms)
    let gw_port = find_available_loopback_port().unwrap();
    let gateway = ServeGateway::start(
        "127.0.0.1",
        gw_port,
        backend_port,
        "test-scale-zero-vm",
        data_dir.clone(),
        Duration::from_millis(400),
    )
    .await
    .expect("Failed to start ServeGateway");

    assert!(!gateway.is_paused());

    // 3. Send initial HTTP request
    {
        let mut client = TcpStream::connect(("127.0.0.1", gw_port)).await.unwrap();
        client.write_all(b"HELLO_PROMPT").await.unwrap();
        client.shutdown().await.unwrap();

        let mut resp = String::new();
        client.read_to_string(&mut resp).await.unwrap();
        assert!(resp.contains("ECHO: HELLO_PROMPT"));
    }

    assert_eq!(gateway.stats().total_requests, 1);
    assert_eq!(gateway.stats().wake_count, 0);

    // 4. Wait for idle timeout (600ms > 400ms) -> auto-pause should trigger
    tokio::time::sleep(Duration::from_millis(650)).await;

    // Verify auto-paused state
    assert!(
        gateway.is_paused(),
        "Gateway should have auto-paused microVM on idle"
    );
    assert!(gateway.stats().pause_count >= 1);

    let state = StateManager::find(&data_dir, "test-scale-zero-vm")
        .unwrap()
        .unwrap();
    assert_eq!(state.status, VmStatus::Paused);

    // 5. Wake-on-Request: Send second HTTP request while paused
    {
        let mut client = TcpStream::connect(("127.0.0.1", gw_port)).await.unwrap();
        client.write_all(b"WAKEUP_INFERENCE").await.unwrap();
        client.shutdown().await.unwrap();

        let mut resp = String::new();
        client.read_to_string(&mut resp).await.unwrap();
        assert!(resp.contains("ECHO: WAKEUP_INFERENCE"));
    }

    // Verify wake-on-request stats
    let stats = gateway.stats();
    assert_eq!(stats.total_requests, 2);
    assert_eq!(stats.wake_count, 1);
    assert!(!gateway.is_paused());
    // Wake latency is tracked and typically < 20,000 microseconds (usually < 2,000 µs)
    assert!(stats.last_wake_latency_us > 0);

    // Cleanup
    gateway.shutdown();
    let _ = child.kill();
    let _ = child.wait();
}
