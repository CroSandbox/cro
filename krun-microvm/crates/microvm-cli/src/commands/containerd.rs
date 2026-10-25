// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! containerd runtime v2 shim installation and config generation commands.

use crate::args::ContainerdCommands;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn handle_containerd_command(command: ContainerdCommands) -> Result<()> {
    match command {
        ContainerdCommands::GenerateConfig => {
            println!("# containerd runtime configuration snippet");
            println!("# Add the following to /etc/containerd/config.toml\n");
            println!("[plugins.\"io.containerd.grpc.v1.cri\".containerd.runtimes.krun]");
            println!("  runtime_type = \"io.containerd.krun.v2\"");
            println!("  [plugins.\"io.containerd.grpc.v1.cri\".containerd.runtimes.krun.options]");
            println!("    BinaryName = \"containerd-shim-krun-v2\"");
            println!();
            println!("---");
            println!("# Kubernetes RuntimeClass resource\n");
            println!("apiVersion: node.k8s.io/v1");
            println!("kind: RuntimeClass");
            println!("metadata:");
            println!("  name: krun");
            println!("handler: krun");
            println!("overhead:");
            println!("  podFixed:");
            println!("    memory: \"64Mi\"");
            println!("    cpu: \"50m\"");
            println!("scheduling:");
            println!("  nodeSelector:");
            println!("    krun.io/enabled: \"true\"");
        }
        ContainerdCommands::Install { target } => {
            let shim_name = "containerd-shim-krun-v2";
            let current_exe =
                std::env::current_exe().context("Failed to determine current executable path")?;
            let exe_dir = current_exe.parent().unwrap_or_else(|| Path::new("."));
            let shim_src = exe_dir.join(shim_name);

            if !shim_src.exists() {
                // Try target/release as fallback
                let alt = PathBuf::from("target/release").join(shim_name);
                if alt.exists() {
                    let dest = target.join(shim_name);
                    println!("Symlinking {} -> {}", alt.display(), dest.display());
                    if dest.exists() {
                        std::fs::remove_file(&dest)?;
                    }
                    std::os::unix::fs::symlink(std::fs::canonicalize(&alt)?, &dest)?;
                    println!("✓ Installed {} to {}", shim_name, dest.display());
                } else {
                    eprintln!(
                        "Error: {} not found at {} or {}",
                        shim_name,
                        shim_src.display(),
                        alt.display()
                    );
                    eprintln!("Build first with: cargo build --release -p containerd-shim-krun");
                    std::process::exit(1);
                }
            } else {
                let dest = target.join(shim_name);
                println!("Symlinking {} -> {}", shim_src.display(), dest.display());
                if dest.exists() {
                    std::fs::remove_file(&dest)?;
                }
                std::os::unix::fs::symlink(std::fs::canonicalize(&shim_src)?, &dest)?;
                println!("✓ Installed {} to {}", shim_name, dest.display());
            }
        }
        ContainerdCommands::Status => {
            // Check shim binary
            let shim_name = "containerd-shim-krun-v2";
            let shim_in_path = std::process::Command::new("which").arg(shim_name).output();
            match shim_in_path {
                Ok(out) if out.status.success() => {
                    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    println!("✓ Shim binary found: {}", path);
                    // Try --version
                    if let Ok(ver) = std::process::Command::new(&path).arg("--version").output() {
                        println!("  {}", String::from_utf8_lossy(&ver.stdout).trim());
                    }
                }
                _ => {
                    println!("✗ Shim binary '{}' not found in PATH", shim_name);
                    println!("  Run: microvm containerd install");
                }
            }

            // Check containerd daemon
            let ctr_status = std::process::Command::new("ctr").args(["version"]).output();
            match ctr_status {
                Ok(out) if out.status.success() => {
                    println!("✓ containerd daemon reachable");
                    for line in String::from_utf8_lossy(&out.stdout).lines() {
                        println!("  {}", line);
                    }
                }
                _ => {
                    println!("✗ containerd daemon not reachable (is it running?)");
                }
            }
        }
    }
    Ok(())
}
