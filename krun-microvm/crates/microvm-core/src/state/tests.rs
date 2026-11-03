// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::PortForward;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_state_manager_save_list_remove() {
    let dir = tempdir().unwrap();
    let vm = VmState {
        id: "vm-test-1".to_string(),
        pid: std::process::id(), // alive current process
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir: dir.path().join("instances/vm-test-1"),
        status: VmStatus::Running,
        vcpus: Some(2),
        memory_mib: Some(512),
    };

    StateManager::save(dir.path(), &vm).unwrap();

    let list = StateManager::list(dir.path()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "vm-test-1");
    assert_eq!(list[0].status, VmStatus::Running);
    assert_eq!(list[0].vcpus, Some(2));
    assert_eq!(list[0].memory_mib, Some(512));

    StateManager::remove(dir.path(), "vm-test-1").unwrap();
    let list = StateManager::list(dir.path()).unwrap();
    assert_eq!(list.len(), 0);
}

#[test]
fn test_copy_into_and_from() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-copy-test");
    fs::create_dir_all(instance_dir.join("rootfs/etc")).unwrap();

    let vm = VmState {
        id: "vm-copy-test".to_string(),
        pid: 0,
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir: instance_dir.clone(),
        status: VmStatus::Stopped,
        vcpus: None,
        memory_mib: None,
    };
    StateManager::save(dir.path(), &vm).unwrap();

    let host_src = dir.path().join("host_app.conf");
    fs::write(&host_src, "config_version=2").unwrap();

    StateManager::copy_into(dir.path(), "vm-copy-test", &host_src, "/etc/app.conf").unwrap();

    let guest_conf = instance_dir.join("rootfs/etc/app.conf");
    assert!(guest_conf.exists());
    assert_eq!(fs::read_to_string(&guest_conf).unwrap(), "config_version=2");

    let host_dst = dir.path().join("copied_back.conf");
    StateManager::copy_from(dir.path(), "vm-copy-test", "/etc/app.conf", &host_dst).unwrap();
    assert!(host_dst.exists());
    assert_eq!(fs::read_to_string(&host_dst).unwrap(), "config_version=2");
}

#[test]
fn test_state_manager_find_and_delete() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-find-del");
    fs::create_dir_all(&instance_dir).unwrap();

    let vm = VmState {
        id: "vm-find-del".to_string(),
        pid: 0, // dead process
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir: instance_dir.clone(),
        status: VmStatus::Stopped,
        vcpus: None,
        memory_mib: None,
    };
    StateManager::save(dir.path(), &vm).unwrap();

    let found = StateManager::find(dir.path(), "vm-find-del").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, "vm-find-del");

    let deleted = StateManager::delete(dir.path(), "vm-find-del", false).unwrap();
    assert_eq!(deleted.id, "vm-find-del");
    assert!(!instance_dir.exists());

    let not_found = StateManager::find(dir.path(), "vm-find-del").unwrap();
    assert!(not_found.is_none());
}

#[test]
fn test_state_manager_pause_and_resume_validation() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-pause-test");
    fs::create_dir_all(&instance_dir).unwrap();

    let vm = VmState {
        id: "vm-pause-test".to_string(),
        pid: 0, // dead
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir,
        status: VmStatus::Stopped,
        vcpus: None,
        memory_mib: None,
    };
    StateManager::save(dir.path(), &vm).unwrap();

    let pause_res = StateManager::pause(dir.path(), "vm-pause-test");
    assert!(pause_res.is_err());
    assert!(pause_res
        .unwrap_err()
        .to_string()
        .contains("process is not running"));

    let resume_res = StateManager::resume(dir.path(), "vm-pause-test");
    assert!(resume_res.is_err());
    assert!(resume_res
        .unwrap_err()
        .to_string()
        .contains("process is not running"));
}

#[test]
fn test_state_manager_resize_success() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-resize-test");
    fs::create_dir_all(&instance_dir).unwrap();

    let runner_cfg = serde_json::json!({
        "root_path": instance_dir.join("rootfs"),
        "num_vcpus": 2,
        "ram_mib": 512,
        "port_forwards": [],
        "virtiofs_mounts": [],
        "vsock_ports": []
    });
    fs::write(
        instance_dir.join("runner_config.json"),
        serde_json::to_string_pretty(&runner_cfg).unwrap(),
    )
    .unwrap();

    let vm = VmState {
        id: "vm-resize-test".to_string(),
        pid: std::process::id(), // alive process so resize proceeds
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir: instance_dir.clone(),
        status: VmStatus::Running,
        vcpus: Some(2),
        memory_mib: Some(512),
    };
    StateManager::save(dir.path(), &vm).unwrap();

    let updated = StateManager::resize(dir.path(), "vm-resize-test", Some(1024), Some(4)).unwrap();
    assert_eq!(updated.memory_mib, Some(1024));
    assert_eq!(updated.vcpus, Some(4));

    let found = StateManager::find(dir.path(), "vm-resize-test")
        .unwrap()
        .unwrap();
    assert_eq!(found.memory_mib, Some(1024));
    assert_eq!(found.vcpus, Some(4));

    let cfg_content = fs::read_to_string(instance_dir.join("runner_config.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cfg_content).unwrap();
    assert_eq!(parsed["ram_mib"], 1024);
    assert_eq!(parsed["num_vcpus"], 4);
}

#[test]
fn test_state_manager_resize_validation_errors() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-resize-val");
    fs::create_dir_all(&instance_dir).unwrap();

    let vm = VmState {
        id: "vm-resize-val".to_string(),
        pid: std::process::id(),
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir,
        status: VmStatus::Running,
        vcpus: Some(2),
        memory_mib: Some(512),
    };
    StateManager::save(dir.path(), &vm).unwrap();

    assert!(StateManager::resize(dir.path(), "vm-resize-val", None, None).is_err());
    assert!(StateManager::resize(dir.path(), "vm-resize-val", Some(0), None).is_err());
    assert!(StateManager::resize(dir.path(), "vm-resize-val", None, Some(0)).is_err());
    assert!(StateManager::resize(dir.path(), "vm-nonexistent", Some(512), None).is_err());
}

#[test]
fn test_state_manager_resize_dead_process() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-resize-dead");
    fs::create_dir_all(&instance_dir).unwrap();

    let vm = VmState {
        id: "vm-resize-dead".to_string(),
        pid: 0,
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir,
        status: VmStatus::Stopped,
        vcpus: Some(2),
        memory_mib: Some(512),
    };
    StateManager::save(dir.path(), &vm).unwrap();

    let res = StateManager::resize(dir.path(), "vm-resize-dead", Some(1024), None);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("not running"));
}

#[test]
fn test_pid_reuse_protection() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-pid-reuse");
    fs::create_dir_all(&instance_dir).unwrap();

    let vm = VmState {
        id: "vm-pid-reuse".to_string(),
        pid: 1, // PID 1 is init/launchd, definitely not microvm-runner
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir,
        status: VmStatus::Running,
        vcpus: Some(1),
        memory_mib: Some(512),
    };
    StateManager::save(dir.path(), &vm).unwrap();

    let list = StateManager::list(dir.path()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].status,
        VmStatus::Stopped,
        "PID 1 is not microvm-runner; must be treated as Stopped"
    );
}

#[test]
fn test_snapshot_and_restore_directory() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-snap-1");
    let rootfs = instance_dir.join("rootfs");
    fs::create_dir_all(rootfs.join("etc")).unwrap();
    fs::write(rootfs.join("etc/version.txt"), "1.0.0-snapshot").unwrap();

    let runner_cfg_json = serde_json::json!({
        "root_path": rootfs,
        "num_vcpus": 4,
        "ram_mib": 4096,
        "port_forwards": [],
        "virtiofs_mounts": [],
        "vsock_ports": []
    });
    fs::write(
        instance_dir.join("runner_config.json"),
        serde_json::to_string_pretty(&runner_cfg_json).unwrap(),
    )
    .unwrap();

    let vm = VmState {
        id: "vm-snap-1".to_string(),
        pid: 0,
        image: "ubuntu:22.04".to_string(),
        created_at: 100,
        port_forwards: vec![PortForward {
            host: 8080,
            guest: 80,
        }],
        instance_dir,
        status: VmStatus::Stopped,
        vcpus: None,
        memory_mib: None,
    };
    StateManager::save(dir.path(), &vm).unwrap();

    let snap_dir = dir.path().join("snapshots/snap-1");
    let manifest = StateManager::snapshot(dir.path(), "vm-snap-1", &snap_dir).unwrap();
    assert_eq!(manifest.original_id, "vm-snap-1");
    assert_eq!(manifest.image, "ubuntu:22.04");

    // Restore into new instance
    let restored = StateManager::restore(dir.path(), &snap_dir, Some("vm-restored-1")).unwrap();
    assert_eq!(restored.id, "vm-restored-1");
    assert_eq!(restored.image, "ubuntu:22.04");
    assert_eq!(restored.port_forwards.len(), 1);

    let restored_ver =
        fs::read_to_string(restored.instance_dir.join("rootfs/etc/version.txt")).unwrap();
    assert_eq!(restored_ver, "1.0.0-snapshot");
    assert!(restored.instance_dir.join("runner_config.json").exists());
    assert!(restored.instance_dir.join(".krun_config.json").exists());
    assert_eq!(restored.vcpus, Some(4));
    assert_eq!(restored.memory_mib, Some(4096));
}

#[test]
fn test_snapshot_and_restore_tar() {
    let dir = tempdir().unwrap();
    let instance_dir = dir.path().join("instances/vm-snap-tar");
    let rootfs = instance_dir.join("rootfs");
    fs::create_dir_all(rootfs.join("app")).unwrap();
    fs::write(rootfs.join("app/data.json"), r#"{"model":"mistral"}"#).unwrap();

    let vm = VmState {
        id: "vm-snap-tar".to_string(),
        pid: 0,
        image: "mistral:latest".to_string(),
        created_at: 200,
        port_forwards: vec![],
        instance_dir,
        status: VmStatus::Stopped,
        vcpus: None,
        memory_mib: None,
    };
    StateManager::save(dir.path(), &vm).unwrap();

    let tar_path = dir.path().join("snapshots/vm-snap-tar.tar");
    fs::create_dir_all(tar_path.parent().unwrap()).unwrap();
    let manifest = StateManager::snapshot(dir.path(), "vm-snap-tar", &tar_path).unwrap();
    assert_eq!(manifest.original_id, "vm-snap-tar");

    // Restore from tar
    let restored = StateManager::restore(dir.path(), &tar_path, Some("vm-tar-restored")).unwrap();
    assert_eq!(restored.id, "vm-tar-restored");
    let data = fs::read_to_string(restored.instance_dir.join("rootfs/app/data.json")).unwrap();
    assert_eq!(data, r#"{"model":"mistral"}"#);
}

#[test]
fn test_atomic_save_concurrent() {
    use std::sync::Arc;
    let dir = tempdir().unwrap();
    let dir_path = Arc::new(dir.path().to_path_buf());

    let mut handles = Vec::new();
    for i in 0..10 {
        let dp = Arc::clone(&dir_path);
        handles.push(std::thread::spawn(move || {
            let vm = VmState {
                id: "vm-concurrent".to_string(),
                pid: 0,
                image: format!("alpine:3.{}", i),
                created_at: 100 + i as u64,
                port_forwards: vec![],
                instance_dir: PathBuf::from("/tmp/concurrent"),
                status: VmStatus::Stopped,
                vcpus: Some(2),
                memory_mib: Some(512),
            };
            StateManager::save(&dp, &vm).unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let loaded = StateManager::find(dir.path(), "vm-concurrent").unwrap();
    assert!(loaded.is_some());
}

#[test]
fn test_state_manager_instance_methods() {
    let dir = tempdir().unwrap();
    let mgr = StateManager::new(dir.path());
    assert_eq!(mgr.data_dir(), dir.path());
    assert_eq!(mgr.state_dir_path(), dir.path().join("state"));

    let vm = VmState {
        id: "vm-inst-1".to_string(),
        pid: std::process::id(),
        image: "alpine:latest".to_string(),
        created_at: 1000,
        port_forwards: vec![],
        instance_dir: dir.path().join("instances/vm-inst-1"),
        status: VmStatus::Running,
        vcpus: Some(2),
        memory_mib: Some(512),
    };

    mgr.save_state(&vm).unwrap();
    let list = mgr.list_states().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "vm-inst-1");

    let found = mgr.find_state("vm-inst-1").unwrap();
    assert!(found.is_some());

    mgr.remove_state("vm-inst-1").unwrap();
    let list_after = mgr.list_states().unwrap();
    assert!(list_after.is_empty());
}
