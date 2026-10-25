// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for Compose and Kubernetes declarative specifications.

use microvm_core::compose::{ComposeProject, Manifest};
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn test_integration_load_compose_fixture() {
    let path = fixture_path("compose.yaml");
    assert!(path.exists(), "compose.yaml fixture must exist");

    let temp_data = tempfile::tempdir().unwrap();
    let project = ComposeProject::load(&path, Some(temp_data.path().to_path_buf()))
        .expect("Failed to load compose fixture");

    assert_eq!(project.name, "integration-test-suite");
    assert_eq!(project.spec.services.len(), 3);

    // Verify topological dependency resolution
    let launch_order = project
        .resolve_launch_order()
        .expect("Topological sort failed");
    assert_eq!(launch_order.len(), 3);

    // 'api' depends on 'db' and 'redis', so 'api' must be launched last
    let api_pos = launch_order.iter().position(|s| s == "api").unwrap();
    let db_pos = launch_order.iter().position(|s| s == "db").unwrap();
    let redis_pos = launch_order.iter().position(|s| s == "redis").unwrap();

    assert!(db_pos < api_pos);
    assert!(redis_pos < api_pos);

    // Verify IP assignments on the gvproxy subnet (192.168.127.X)
    let api_ip = project.service_ip("api");
    let db_ip = project.service_ip("db");
    let redis_ip = project.service_ip("redis");

    assert!(api_ip.starts_with("192.168.127."));
    assert!(db_ip.starts_with("192.168.127."));
    assert!(redis_ip.starts_with("192.168.127."));
    assert_ne!(api_ip, db_ip);
    assert_ne!(db_ip, redis_ip);

    // Verify VM builder can be generated for each service
    let api_builder = project.build_service_vm("api");
    assert!(api_builder.is_ok());

    let db_builder = project.build_service_vm("db");
    assert!(db_builder.is_ok());

    let redis_builder = project.build_service_vm("redis");
    assert!(redis_builder.is_ok());
}

#[test]
fn test_integration_load_k8s_pod_fixture() {
    let path = fixture_path("pod.yaml");
    assert!(path.exists(), "pod.yaml fixture must exist");

    let manifest = Manifest::load_file(&path).expect("Failed to load Kubernetes Pod fixture");
    let compose = manifest.into_compose_spec();

    assert_eq!(compose.name.as_deref(), Some("integration-pod"));
    assert!(compose.services.contains_key("web"));

    let web = &compose.services["web"];
    assert_eq!(web.image, "nginx:alpine");
    assert_eq!(web.cpus, Some(1));
    assert_eq!(web.memory.as_deref(), Some("256Mi"));
    assert_eq!(web.ports, vec!["8080:80"]);
}

#[test]
fn test_integration_load_k8s_microvm_crd_fixture() {
    let path = fixture_path("microvm-crd.yaml");
    assert!(path.exists(), "microvm-crd.yaml fixture must exist");

    let manifest = Manifest::load_file(&path).expect("Failed to load MicroVm CRD fixture");
    let compose = manifest.into_compose_spec();

    assert_eq!(compose.name.as_deref(), Some("isolated-worker"));
    assert!(compose.services.contains_key("isolated-worker"));

    let svc = &compose.services["isolated-worker"];
    assert_eq!(svc.image, "alpine:latest");
    assert_eq!(svc.cpus, Some(2));
    assert_eq!(svc.memory.as_deref(), Some("512M"));
    assert_eq!(svc.ports, vec!["9000:9000"]);
    assert_eq!(
        svc.cmd,
        Some(vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo ready".to_string()
        ])
    );
}

#[test]
fn test_integration_manifest_not_found() {
    let missing_path = fixture_path("non_existent_manifest.yaml");
    let result = Manifest::load_file(&missing_path);
    assert!(result.is_err());
}
