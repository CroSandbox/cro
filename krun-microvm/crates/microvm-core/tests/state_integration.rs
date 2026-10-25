// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for StateManager and instance state persistence.

use microvm_core::state::{StateManager, VmState, VmStatus};
use tempfile::tempdir;

#[test]
fn test_integration_state_manager_lifecycle() {
    let dir = tempdir().expect("Failed to create temporary directory");
    let manager = StateManager::new(dir.path());

    assert_eq!(manager.data_dir(), dir.path());
    assert_eq!(manager.state_dir_path(), dir.path().join("state"));

    // 1. Initial state list is empty
    let initial_states = manager.list_states().expect("Failed to list states");
    assert!(initial_states.is_empty());

    // 2. Persist two microVM instance states
    let vm1 = VmState {
        id: "vm-alpha-1".to_string(),
        pid: std::process::id(),
        image: "alpine:3.19".to_string(),
        created_at: 1_700_000_000,
        port_forwards: vec![],
        instance_dir: dir.path().join("instances/vm-alpha-1"),
        status: VmStatus::Running,
        vcpus: Some(2),
        memory_mib: Some(512),
    };

    let vm2 = VmState {
        id: "vm-beta-2".to_string(),
        pid: 0,
        image: "nginx:alpine".to_string(),
        created_at: 1_700_000_100,
        port_forwards: vec![],
        instance_dir: dir.path().join("instances/vm-beta-2"),
        status: VmStatus::Stopped,
        vcpus: Some(1),
        memory_mib: Some(256),
    };

    manager.save_state(&vm1).expect("Failed to save vm1 state");
    manager.save_state(&vm2).expect("Failed to save vm2 state");

    // 3. List all states (should be ordered newest first)
    let states = manager.list_states().expect("Failed to list states");
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].id, "vm-beta-2");
    assert_eq!(states[1].id, "vm-alpha-1");

    // 4. Find by exact ID
    let found_alpha = manager
        .find_state("vm-alpha-1")
        .expect("Search failed")
        .expect("vm1 not found");
    assert_eq!(found_alpha.image, "alpine:3.19");

    // 5. Find by ID prefix
    let found_beta_prefix = manager
        .find_state("vm-beta")
        .expect("Prefix search failed")
        .expect("vm2 not found by prefix");
    assert_eq!(found_beta_prefix.id, "vm-beta-2");

    // 6. Remove one state
    manager
        .remove_state("vm-alpha-1")
        .expect("Failed to remove state");
    let remaining = manager.list_states().expect("Failed to list states");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "vm-beta-2");

    // 7. Remove remaining state
    manager
        .remove_state("vm-beta-2")
        .expect("Failed to remove state");
    let empty = manager.list_states().expect("Failed to list states");
    assert!(empty.is_empty());
}
