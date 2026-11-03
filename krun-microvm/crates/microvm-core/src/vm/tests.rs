// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use super::builder::MicroVmBuilder;
use super::kernel::{detect_kernel_format, parse_kernel_format};
use super::lifecycle::INSTANCE_COUNTER;
use super::runtime::MicroVm;
use nix::sys::signal;
use nix::unistd::Pid;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_microvm_builder_from_bundle() {
    let dir = tempdir().unwrap();
    let rootfs = dir.path().join("rootfs");
    std::fs::create_dir_all(&rootfs).unwrap();

    let host_file = dir.path().join("app.yaml");
    std::fs::write(&host_file, "env: production\n").unwrap();

    let config_json = format!(
        r#"{{
        "ociVersion": "1.0.2",
        "root": {{
            "path": "rootfs"
        }},
        "process": {{
            "user": {{
                "uid": 0,
                "gid": 0
            }},
            "cwd": "/",
            "args": ["/bin/sh", "-c", "echo ready"],
            "env": ["PORT=8080"]
        }},
        "linux": {{
            "resources": {{
                "memory": {{
                    "limit": 1073741824
                }},
                "cpu": {{
                    "quota": 200000,
                    "period": 100000
                }}
            }}
        }},
        "mounts": [
            {{
                "source": "{}",
                "destination": "/etc/config/app.yaml"
            }}
        ]
    }}"#,
        host_file.display()
    );

    std::fs::write(dir.path().join("config.json"), config_json).unwrap();

    let builder = MicroVmBuilder::from_bundle(dir.path()).unwrap();
    assert_eq!(builder.get_vcpus(), 2);
    assert_eq!(builder.get_ram_mib(), 1024);
    assert_eq!(builder.get_file_mounts().len(), 1);
    assert_eq!(
        builder.get_file_mounts()[0],
        (host_file, PathBuf::from("/etc/config/app.yaml"))
    );
}

#[test]
fn test_builder_image_acceleration() {
    let builder = MicroVmBuilder::new("alpine:latest")
        .lazy_load(true)
        .chunk_size_str("64M")
        .unwrap()
        .chunk_cache_dir("/var/cache/nydus");

    let acc = builder
        .get_image_acceleration()
        .expect("Expected image acceleration");
    assert!(acc.lazy_load);
    assert_eq!(acc.chunk_size_bytes, Some(64 * 1024 * 1024));
    assert_eq!(acc.chunk_cache_dir, Some(PathBuf::from("/var/cache/nydus")));
}

#[test]
fn test_builder_lazy_load_with_bootstrap() {
    let bootstrap_path = PathBuf::from("/tmp/nydus-bootstrap.rafs");
    let builder = MicroVmBuilder::new("my-image:latest")
        .nydus_bootstrap(&bootstrap_path)
        .dax_window_size_str("4G")
        .unwrap();

    let acc = builder.get_image_acceleration().unwrap();
    assert!(acc.lazy_load);
    assert_eq!(acc.bootstrap_path, Some(bootstrap_path));
    assert_eq!(builder.dax_window_size, Some(4 * 1024 * 1024 * 1024));
}

#[test]
fn test_builder_gpu_config() {
    let builder = MicroVmBuilder::new("pytorch/pytorch:latest")
        .gpu(true)
        .gpu_shm_size(8 * 1024 * 1024 * 1024)
        .gpu_flags(0x40);

    assert!(builder.get_gpu());
    assert_eq!(builder.get_gpu_shm_size(), Some(8 * 1024 * 1024 * 1024));
    assert_eq!(builder.gpu_flags, Some(0x40));
}

#[test]
fn test_builder_egress_and_secret_config() {
    let builder = MicroVmBuilder::new("alpine:latest")
        .allow_host("api.openai.com:443")
        .allow_host("pypi.org:443")
        .secret("OPENAI_KEY", "sk-proj-test123")
        .max_tokens(500_000);

    assert_eq!(
        builder.allow_hosts,
        vec!["api.openai.com:443", "pypi.org:443"]
    );
    assert_eq!(
        builder.secrets,
        vec![("OPENAI_KEY".to_string(), "sk-proj-test123".to_string())]
    );
    assert_eq!(builder.max_tokens, Some(500_000));
}

#[test]
fn test_builder_multi_boot_config() {
    let kernel_vm = MicroVmBuilder::new("")
        .kernel(
            "/boot/vmlinuz",
            Some(PathBuf::from("/boot/initrd")),
            Some("console=ttyS0".to_string()),
        )
        .disk("disk0", "/dev/vda.raw", false);

    match kernel_vm.boot_payload.as_ref().unwrap() {
        crate::types::BootPayload::Kernel(k) => {
            assert_eq!(k.kernel_path, PathBuf::from("/boot/vmlinuz"));
            assert_eq!(k.initramfs, Some(PathBuf::from("/boot/initrd")));
            assert_eq!(k.cmdline, Some("console=ttyS0".to_string()));
        }
        _ => panic!("Expected BootPayload::Kernel"),
    }
    assert_eq!(kernel_vm.disks.len(), 1);
    assert_eq!(kernel_vm.disks[0].id, "disk0");

    let fw_vm = MicroVmBuilder::new("").firmware("/opt/homebrew/share/libkrun/KRUN_EFI.fd");
    match fw_vm.boot_payload.as_ref().unwrap() {
        crate::types::BootPayload::Firmware(f) => {
            assert_eq!(
                f.firmware_path,
                PathBuf::from("/opt/homebrew/share/libkrun/KRUN_EFI.fd")
            );
        }
        _ => panic!("Expected BootPayload::Firmware"),
    }

    let unikernel_vm =
        MicroVmBuilder::new("").unikernel("app.unikraft", Some("ip=192.168.1.5".to_string()));
    match unikernel_vm.boot_payload.as_ref().unwrap() {
        crate::types::BootPayload::Unikernel(u) => {
            assert_eq!(u.kernel_path, PathBuf::from("app.unikraft"));
            assert_eq!(u.cmdline, Some("ip=192.168.1.5".to_string()));
        }
        _ => panic!("Expected BootPayload::Unikernel"),
    }
}

#[test]
fn test_concurrent_instance_id_uniqueness() {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::thread;

    let ids = Arc::new(Mutex::new(HashSet::new()));
    let mut handles = Vec::new();

    for _ in 0..50 {
        let ids_clone = Arc::clone(&ids);
        handles.push(thread::spawn(move || {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            let millis = now.as_millis();
            let nanos = now.subsec_nanos();
            let counter = INSTANCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let id = format!("vm-{millis}-{nanos:06}-{counter:03}");
            let mut set = ids_clone.lock().unwrap();
            assert!(!set.contains(&id), "Duplicate instance ID generated: {id}");
            set.insert(id);
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(ids.lock().unwrap().len(), 50);
}

#[test]
fn test_parse_kernel_format() {
    use krun_sys::kernel_formats::*;
    #[cfg(target_arch = "aarch64")]
    {
        assert_eq!(parse_kernel_format("raw"), Some(KRUN_KERNEL_FORMAT_PE_GZ));
        assert_eq!(parse_kernel_format("m1n1"), Some(KRUN_KERNEL_FORMAT_PE_GZ));
        assert_eq!(parse_kernel_format("bin"), Some(KRUN_KERNEL_FORMAT_PE_GZ));
        assert_eq!(parse_kernel_format("gz"), Some(KRUN_KERNEL_FORMAT_PE_GZ));
        assert_eq!(
            parse_kernel_format("image.gz"),
            Some(KRUN_KERNEL_FORMAT_PE_GZ)
        );
        assert_eq!(
            parse_kernel_format("vmlinuz"),
            Some(KRUN_KERNEL_FORMAT_PE_GZ)
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert_eq!(parse_kernel_format("raw"), Some(KRUN_KERNEL_FORMAT_RAW));
        assert_eq!(parse_kernel_format("m1n1"), Some(KRUN_KERNEL_FORMAT_RAW));
        assert_eq!(parse_kernel_format("bin"), Some(KRUN_KERNEL_FORMAT_RAW));
        assert_eq!(parse_kernel_format("gz"), Some(KRUN_KERNEL_FORMAT_IMAGE_GZ));
        assert_eq!(
            parse_kernel_format("image.gz"),
            Some(KRUN_KERNEL_FORMAT_IMAGE_GZ)
        );
        assert_eq!(
            parse_kernel_format("vmlinuz"),
            Some(KRUN_KERNEL_FORMAT_IMAGE_GZ)
        );
    }
    assert_eq!(parse_kernel_format("elf"), Some(KRUN_KERNEL_FORMAT_ELF));
    assert_eq!(parse_kernel_format("vmlinux"), Some(KRUN_KERNEL_FORMAT_ELF));
    assert_eq!(
        parse_kernel_format("bz2"),
        Some(KRUN_KERNEL_FORMAT_IMAGE_BZ2)
    );
    assert_eq!(
        parse_kernel_format("zstd"),
        Some(KRUN_KERNEL_FORMAT_IMAGE_ZSTD)
    );
    assert_eq!(parse_kernel_format("pe"), Some(KRUN_KERNEL_FORMAT_PE_GZ));
    assert_eq!(parse_kernel_format("unknown_format"), None);
}

#[test]
fn test_detect_kernel_format_from_magic_and_extension() {
    use krun_sys::kernel_formats::*;
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();

    // 1. Gzip magic (Asahi Linux Image.gz / vmlinuz-asahi)
    let gz_path = dir.path().join("kernel-asahi");
    let mut gz_file = std::fs::File::create(&gz_path).unwrap();
    gz_file.write_all(&[0x1f, 0x8b, 0x08, 0x00]).unwrap();
    drop(gz_file);
    #[cfg(target_arch = "aarch64")]
    assert_eq!(detect_kernel_format(&gz_path), KRUN_KERNEL_FORMAT_PE_GZ);
    #[cfg(not(target_arch = "aarch64"))]
    assert_eq!(detect_kernel_format(&gz_path), KRUN_KERNEL_FORMAT_IMAGE_GZ);

    // 2. ELF magic
    let elf_path = dir.path().join("kernel-elf");
    let mut elf_file = std::fs::File::create(&elf_path).unwrap();
    elf_file.write_all(&[0x7f, b'E', b'L', b'F']).unwrap();
    drop(elf_file);
    assert_eq!(detect_kernel_format(&elf_path), KRUN_KERNEL_FORMAT_ELF);

    // 3. Zstd magic
    let zstd_path = dir.path().join("kernel-zstd");
    let mut zstd_file = std::fs::File::create(&zstd_path).unwrap();
    zstd_file.write_all(&[0x28, 0xb5, 0x2f, 0xfd]).unwrap();
    drop(zstd_file);
    assert_eq!(
        detect_kernel_format(&zstd_path),
        KRUN_KERNEL_FORMAT_IMAGE_ZSTD
    );

    // 4. Filename extension fallback for m1n1.bin
    let m1n1_path = dir.path().join("m1n1.bin");
    std::fs::File::create(&m1n1_path).unwrap();
    #[cfg(target_arch = "aarch64")]
    assert_eq!(detect_kernel_format(&m1n1_path), KRUN_KERNEL_FORMAT_PE_GZ);
    #[cfg(not(target_arch = "aarch64"))]
    assert_eq!(detect_kernel_format(&m1n1_path), KRUN_KERNEL_FORMAT_RAW);

    // 5. Filename extension fallback for vmlinuz
    let vmlinuz_path = dir.path().join("vmlinuz-6.8.0-asahi");
    std::fs::File::create(&vmlinuz_path).unwrap();
    #[cfg(target_arch = "aarch64")]
    assert_eq!(
        detect_kernel_format(&vmlinuz_path),
        KRUN_KERNEL_FORMAT_PE_GZ
    );
    #[cfg(not(target_arch = "aarch64"))]
    assert_eq!(
        detect_kernel_format(&vmlinuz_path),
        KRUN_KERNEL_FORMAT_IMAGE_GZ
    );
}

#[tokio::test]
async fn test_microvm_drop_cleanup_non_detached() {
    let dir = tempfile::tempdir().unwrap();
    let instance_dir = dir.path().join("instances/test-vm-drop");
    std::fs::create_dir_all(&instance_dir).unwrap();
    let test_file = instance_dir.join("marker.txt");
    std::fs::write(&test_file, "data").unwrap();

    let child = tokio::process::Command::new("sleep")
        .arg("10")
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();

    let vm = MicroVm {
        id: "test-vm-drop".to_string(),
        child,
        instance_dir: instance_dir.clone(),
        data_dir: dir.path().to_path_buf(),
        is_detached: false,
        supervisor_sock: instance_dir.join("supervisor.sock"),
        _raw_guard: None,
        _proxy: None,
        _gvproxy: None,
    };

    // When vm is dropped, it should terminate the process and cleanup instance_dir
    drop(vm);

    // Instance directory should be removed
    assert!(!instance_dir.exists());

    // Process should be terminated
    tokio::time::sleep(Duration::from_millis(50)).await;
    let nix_pid = Pid::from_raw(pid as i32);
    let alive = signal::kill(nix_pid, None).is_ok();
    assert!(!alive);
}

#[test]
fn test_builder_shm_size_config() {
    let builder = MicroVmBuilder::new("alpine:latest").shm_size("2g");
    assert_eq!(builder.shm_size.as_deref(), Some("2g"));
}

#[tokio::test]
async fn test_builder_model_attachment_and_auto_dax() {
    let temp = tempfile::tempdir().unwrap();
    let model_dir = temp.path().join("llama-model");
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(
        model_dir.join("tiny.Q4_K_M.gguf"),
        b"MOCK_GGUF_WEIGHTS_4096",
    )
    .unwrap();
    std::fs::write(
        model_dir.join("config.json"),
        b"{\"architectures\":[\"LlamaForCausalLM\"]}",
    )
    .unwrap();

    let builder = MicroVmBuilder::new("alpine:latest")
        .model(model_dir.to_str().unwrap(), Some("/custom/llm".to_string()));

    assert_eq!(builder.models.len(), 1);
    assert_eq!(builder.models[0].mount_path.as_deref(), Some("/custom/llm"));

    let instance_dir = temp.path().join("instance");
    let instance_rootfs = temp.path().join("rootfs");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&instance_dir).unwrap();
    std::fs::create_dir_all(&instance_rootfs).unwrap();

    // Write initial krun config
    let initial_cfg = crate::config::KrunConfig::new(vec!["/bin/sh".to_string()], vec![], None);
    initial_cfg.write_to(&instance_rootfs).unwrap();

    let virtio_mounts = builder
        .prepare_virtiofs_mounts(&instance_dir, &instance_rootfs, &data_dir)
        .await
        .unwrap();

    assert_eq!(virtio_mounts.len(), 1);
    // VirtioFS DAX window should be automatically configured for GGUF weights
    assert!(virtio_mounts[0].dax_window_size_bytes.unwrap_or(0) >= 512 * 1024 * 1024);

    // Environment variables should be injected into krun config
    let krun_cfg_path = instance_rootfs.join(crate::config::KrunConfig::FILE_NAME);
    let content = std::fs::read_to_string(&krun_cfg_path).unwrap();
    assert!(content.contains("MODEL_NAME=llama-model"));
    assert!(content.contains("MODEL_PATH=/custom/llm"));
    assert!(content.contains("MODEL_FORMAT=gguf"));
}

#[tokio::test]
async fn test_builder_rosetta_and_platform() {
    let temp = tempdir().unwrap();
    let rosetta_dir = temp.path().join("mock_rosetta");
    std::fs::create_dir_all(&rosetta_dir).unwrap();
    std::fs::write(rosetta_dir.join("rosetta"), b"mock-rosetta-bin").unwrap();

    std::env::set_var("ROSETTA_PATH", &rosetta_dir);

    let builder = MicroVmBuilder::new("ubuntu:latest")
        .rosetta(true)
        .platform("linux/amd64");

    assert!(builder.get_rosetta());
    assert_eq!(builder.get_platform(), Some("linux/amd64"));

    let instance_dir = temp.path().join("instance");
    let instance_rootfs = temp.path().join("rootfs");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&instance_dir).unwrap();
    std::fs::create_dir_all(&instance_rootfs).unwrap();

    let initial_cfg = crate::config::KrunConfig::new(vec!["/bin/sh".to_string()], vec![], None);
    initial_cfg.write_to(&instance_rootfs).unwrap();

    let init_script = instance_rootfs.join("krun-init.sh");
    std::fs::write(&init_script, "#!/bin/sh\nexec \"$@\"\n").unwrap();

    let virtio_mounts = builder
        .prepare_virtiofs_mounts(&instance_dir, &instance_rootfs, &data_dir)
        .await
        .unwrap();

    assert!(virtio_mounts.iter().any(|m| m.tag == "rosetta"));

    let script_content = std::fs::read_to_string(&init_script).unwrap();
    assert!(script_content.contains("binfmt_misc"));
    assert!(script_content.contains(":rosetta:M:"));

    std::env::remove_var("ROSETTA_PATH");
}
