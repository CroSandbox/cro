// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Multi-service compose project state persistence models.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// State of an active compose project saved to disk for CLI inspection and shutdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeProjectState {
    pub name: String,
    pub compose_file: PathBuf,
    pub working_dir: PathBuf,
    pub created_at: u64,
    pub services: BTreeMap<String, ComposeServiceState>,
}

/// Runtime state of an individual microVM service within an active compose project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeServiceState {
    pub service_name: String,
    pub instance_id: String,
    pub pid: Option<u32>,
    pub image: String,
    pub status: String,
    pub ports: Vec<String>,
}
