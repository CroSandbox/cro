// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Multi-service declarative composition, Kubernetes manifest compatibility, and orchestration.
//!
//! Submodules:
//! - [`project`]: Orchestration, lifecycle management, dependency ordering, and networking.
//! - [`spec`]: Schema definitions and parsers for Docker Compose, Kubernetes Pod, and MicroVm CRD manifests.
//! - [`state`]: State serialization structures for tracking active compose projects and services on disk.

pub mod project;
pub mod spec;
pub mod state;

// Re-exports for clean API ergonomics and backwards compatibility
pub use project::ComposeProject;
pub use spec::{
    ComposeNetworkSpec, ComposeSpec, ComposeVolumeSpec, K8sContainerPort, K8sContainerSpec,
    K8sEmptyDirVolumeSource, K8sEnvVar, K8sHostPathVolumeSource, K8sMetadata, K8sMicroVmManifest,
    K8sMicroVmSpec, K8sPodManifest, K8sPodSpec, K8sPodVolume, K8sPortMapping, K8sResourceLimits,
    K8sResourceRequirements, K8sVolumeMount, Manifest, ServiceSpec,
};
pub use state::{ComposeProjectState, ComposeServiceState};
