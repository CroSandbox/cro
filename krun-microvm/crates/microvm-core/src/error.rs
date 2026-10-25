// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Strongly-typed domain error models for microVM orchestration, compose, networking, and state.

use std::path::PathBuf;
use thiserror::Error;

/// Root error type encompassing all domain-specific errors in `microvm-core`.
#[derive(Debug, Error)]
pub enum MicroVmError {
    #[error("VM error: {0}")]
    Vm(#[from] VmError),

    #[error("Compose error: {0}")]
    Compose(#[from] ComposeError),

    #[error("Network error: {0}")]
    Net(#[from] NetError),

    #[error("State error: {0}")]
    State(#[from] StateError),

    #[error("Security violation: {0}")]
    Security(#[from] SecurityError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

/// Errors related to microVM lifecycle, configuration, and virtualization runner operations.
#[derive(Debug, Error)]
pub enum VmError {
    #[error("Missing boot source: please specify an OCI image reference, '--kernel', '--firmware', or '--bundle'")]
    MissingBootSource,

    #[error("Preflight checks failed: {0}")]
    PreflightFailed(String),

    #[error("Rootfs preparation failed for image '{image}': {reason}")]
    RootfsPreparationFailed { image: String, reason: String },

    #[error("VirtioFS mount preparation failed for tag '{tag}': {reason}")]
    VirtioFsFailed { tag: String, reason: String },

    #[error("Runner process spawn failed: {0}")]
    RunnerSpawnFailed(String),

    #[error("Supervisor socket connection failed at '{path}': {reason}")]
    SupervisorConnectionFailed { path: PathBuf, reason: String },

    #[error("Invalid resource limit: {0}")]
    InvalidResource(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors related to Docker Compose and Kubernetes manifest parsing and orchestration.
#[derive(Debug, Error)]
pub enum ComposeError {
    #[error("No compose manifest found in '{path}'")]
    ManifestNotFound { path: PathBuf },

    #[error("Service '{name}' not found in compose manifest")]
    ServiceNotFound { name: String },

    #[error("Service '{service}' depends on undefined service '{depends_on}'")]
    UndefinedDependency { service: String, depends_on: String },

    #[error("Circular dependency detected in compose services depends_on graph")]
    CircularDependency,

    #[error("Invalid port forward specification: '{0}', expected host:guest or single port")]
    InvalidPortForward(String),

    #[error("Empty volume specification string")]
    EmptyVolumeSpec,

    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Errors related to virtual networking, egress filtering, and traffic inspection.
#[derive(Debug, Error)]
pub enum NetError {
    #[error("Invalid port number: {0}")]
    InvalidPort(String),

    #[error("Invalid network CIDR or IP address: '{0}'")]
    InvalidAddress(String),

    #[error("Invalid subnet format: '{0}'")]
    InvalidSubnet(String),

    #[error("Network proxy initialization failed: {0}")]
    ProxyInitFailed(String),

    #[error("Egress connection to target '{0}' denied by network policy")]
    EgressDenied(String),

    #[error("LLM token budget exceeded ({consumed} > {limit})")]
    TokenBudgetExceeded { consumed: u64, limit: u64 },

    #[error("Virtual network device error: {0}")]
    DeviceError(String),
}

/// Errors related to instance state persistence, lookup, and lifecycle transitions.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("MicroVM with ID or PID '{0}' not found in state store")]
    NotFound(String),

    #[error("MicroVM '{0}' is not running")]
    NotRunning(String),

    #[error("MicroVM '{0}' is already running (PID {1})")]
    AlreadyRunning(String, u32),

    #[error("Invalid process ID: {0}")]
    InvalidPid(u32),

    #[error("Resize error: {0}")]
    ResizeError(String),

    #[error("Snapshot not found at path '{0}'")]
    SnapshotNotFound(PathBuf),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors related to containment security, symlink escaping, and path traversal prevention.
#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("Symlink target resolves outside container rootfs boundary: {0}")]
    SymlinkEscapesRootfs(PathBuf),

    #[error("Path traversal detected outside root directory: {0}")]
    PathTraversal(PathBuf),

    #[error("Restricted host access: destination '{0}' is not permitted")]
    HostAccessDenied(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_formatting() {
        let err = ComposeError::UndefinedDependency {
            service: "web".to_string(),
            depends_on: "db".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Service 'web' depends on undefined service 'db'"
        );

        let vm_err = VmError::MissingBootSource;
        assert!(vm_err.to_string().contains("Missing boot source"));

        let net_err = NetError::TokenBudgetExceeded {
            consumed: 5000,
            limit: 4000,
        };
        assert_eq!(
            net_err.to_string(),
            "LLM token budget exceeded (5000 > 4000)"
        );

        let sec_err = SecurityError::SymlinkEscapesRootfs(PathBuf::from("/etc/passwd"));
        assert!(sec_err
            .to_string()
            .contains("Symlink target resolves outside"));
    }

    #[test]
    fn test_microvm_error_conversion() {
        let compose_err = ComposeError::CircularDependency;
        let root_err: MicroVmError = compose_err.into();
        assert!(matches!(
            root_err,
            MicroVmError::Compose(ComposeError::CircularDependency)
        ));
    }
}
