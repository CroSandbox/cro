// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Kubernetes Pod manifest integration commands.

use crate::args::KubeCommands;
use crate::commands::compose::{handle_compose_down, handle_compose_up};
use crate::utils::default_data_dir;
use anyhow::{bail, Result};
use microvm_core::compose::ComposeProject;

pub async fn handle_kube_command(command: KubeCommands) -> Result<()> {
    match command {
        KubeCommands::Play {
            file,
            detach,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            if !file.exists() {
                bail!("Kubernetes Pod file '{}' does not exist", file.display());
            }
            println!("🚀 Launching Kubernetes Pod from {}", file.display());
            let project = ComposeProject::load(&file, Some(base_data_dir.clone()))?;
            handle_compose_up(&project, &file, &base_data_dir, detach, &[]).await?;
        }
        KubeCommands::Down { file, data_dir } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            if !file.exists() {
                bail!("Kubernetes Pod file '{}' does not exist", file.display());
            }
            println!("🛑 Tearing down Kubernetes Pod from {}", file.display());
            let project = ComposeProject::load(&file, Some(base_data_dir.clone()))?;
            handle_compose_down(&project, &base_data_dir, false)?;
            println!("✅ Pod containers successfully terminated.");
        }
    }
    Ok(())
}
