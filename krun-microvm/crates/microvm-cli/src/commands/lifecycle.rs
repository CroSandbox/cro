// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CLI handlers for microVM container lifecycle: ps, stop, pause, resume, rm, inspect, logs, top, cp, prune, resize.

use anyhow::{bail, Result};
use microvm_core::{collect_process_stats, StateManager, VmStatus};
use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;

use crate::utils::{default_data_dir, format_bytes, format_duration_since};

pub fn handle_ps(all: bool, no_trunc: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let vms = StateManager::list(&base)?;

    let displayed: Vec<_> = vms
        .into_iter()
        .filter(|v| all || v.status == VmStatus::Running || v.status == VmStatus::Paused)
        .collect();

    if displayed.is_empty() {
        println!("No {} microVMs found.", if all { "" } else { "running" });
        return Ok(());
    }

    println!(
        "{:<16} {:<24} {:<8} {:<18} {:<18}",
        "INSTANCE ID", "IMAGE", "PID", "STATUS", "PORTS"
    );
    println!("{:-<90}", "");

    for vm in displayed {
        let id_display = if no_trunc || vm.id.len() <= 12 {
            vm.id.clone()
        } else {
            vm.id[..12].to_string()
        };

        let time_rel = format_duration_since(vm.created_at);
        let status_str = match vm.status {
            VmStatus::Running => format!("● Up ({})", time_rel),
            VmStatus::Paused => format!("⏸ Paused ({})", time_rel),
            VmStatus::Stopped => format!("○ Exited ({})", time_rel),
        };

        let port_str = if vm.port_forwards.is_empty() {
            "-".to_string()
        } else {
            vm.port_forwards
                .iter()
                .map(|p| format!("{}:{}", p.host, p.guest))
                .collect::<Vec<_>>()
                .join(", ")
        };

        let image_display = if vm.image.len() > 23 && !no_trunc {
            format!("{}...", &vm.image[..20])
        } else {
            vm.image.clone()
        };

        println!(
            "{:<16} {:<24} {:<8} {:<18} {:<18}",
            id_display, image_display, vm.pid, status_str, port_str
        );
    }

    Ok(())
}

pub fn handle_stop(id: String, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    println!("Stopping microVM '{}'...", id);
    StateManager::stop(&base, &id)?;
    println!("✅ MicroVM '{}' stopped and state cleaned up.", id);
    Ok(())
}

pub fn handle_pause(id: String, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    match StateManager::pause(&base, &id) {
        Ok(vm) => {
            println!("⏸️  Paused microVM '{}' (PID {})", vm.id, vm.pid);
            Ok(())
        }
        Err(e) => {
            bail!("Failed to pause microVM '{}': {}", id, e);
        }
    }
}

pub fn handle_resume(id: String, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    match StateManager::resume(&base, &id) {
        Ok(vm) => {
            println!("▶️  Resumed microVM '{}' (PID {})", vm.id, vm.pid);
            Ok(())
        }
        Err(e) => {
            bail!("Failed to resume microVM '{}': {}", id, e);
        }
    }
}

pub fn handle_rm(id: String, force: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    match StateManager::delete(&base, &id, force) {
        Ok(vm) => {
            println!(
                "✅ Removed microVM '{}' (Image: {}, PID: {})",
                vm.id, vm.image, vm.pid
            );
            Ok(())
        }
        Err(e) => {
            bail!("Failed to remove microVM '{}': {}", id, e);
        }
    }
}

pub fn handle_inspect(id: String, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let vm = match StateManager::find(&base, &id)? {
        Some(v) => v,
        None => bail!("MicroVM '{}' not found in state", id),
    };

    let is_alive = vm.is_process_alive();
    let telemetry = if is_alive {
        collect_process_stats(vm.pid)
    } else {
        None
    };

    let config_json_path = vm.instance_dir.join("config.json");
    let oci_config: Option<serde_json::Value> = if config_json_path.exists() {
        std::fs::read_to_string(&config_json_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    let console_log = vm.instance_dir.join("console.log");

    if json {
        let info = json!({
            "id": vm.id,
            "pid": vm.pid,
            "image": vm.image,
            "status": if is_alive { "Running" } else { "Stopped" },
            "created_at": vm.created_at,
            "vcpus": vm.vcpus,
            "memory_mib": vm.memory_mib,
            "port_forwards": vm.port_forwards,
            "instance_dir": vm.instance_dir.display().to_string(),
            "console_log": console_log.display().to_string(),
            "telemetry": telemetry,
            "oci_config": oci_config,
        });
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        println!("📦 MicroVM Inspection: {}", vm.id);
        println!("{:-<60}", "");
        println!(
            "  Status:          {}",
            if is_alive {
                format!("● Running (PID {})", vm.pid)
            } else {
                "○ Stopped".to_string()
            }
        );
        println!("  Image:           {}", vm.image);
        if let Some(c) = vm.vcpus {
            println!("  Configured CPUs: {}", c);
        }
        if let Some(m) = vm.memory_mib {
            println!("  Allocated RAM:   {} MiB", m);
        }
        println!(
            "  Created:         {} ({})",
            format_duration_since(vm.created_at),
            vm.created_at
        );
        println!("  Instance Path:   {}", vm.instance_dir.display());
        println!("  Console Log:     {}", console_log.display());
        if !vm.port_forwards.is_empty() {
            let ports: Vec<String> = vm
                .port_forwards
                .iter()
                .map(|p| format!("{}:{}", p.host, p.guest))
                .collect();
            println!("  Port Mappings:   {}", ports.join(", "));
        }

        if let Some(ref oci) = oci_config {
            if let Some(entrypoint) = oci.get("entrypoint").and_then(|v| v.as_array()) {
                let ep: Vec<&str> = entrypoint.iter().filter_map(|s| s.as_str()).collect();
                println!("  Entrypoint:      {:?}", ep);
            }
            if let Some(cmd) = oci.get("cmd").and_then(|v| v.as_array()) {
                let c: Vec<&str> = cmd.iter().filter_map(|s| s.as_str()).collect();
                println!("  Cmd:             {:?}", c);
            }
            if let Some(workdir) = oci.get("working_dir").and_then(|v| v.as_str()) {
                if !workdir.is_empty() {
                    println!("  Working Dir:     {}", workdir);
                }
            }
        }

        if let Some(ref stats) = telemetry {
            println!("\n  Telemetry (Live):");
            println!(
                "    CPU Time (Total):  {:.2} ms (User: {:.2} ms, Sys: {:.2} ms)",
                stats.total_cpu_ns as f64 / 1_000_000.0,
                stats.user_cpu_ns as f64 / 1_000_000.0,
                stats.kernel_cpu_ns as f64 / 1_000_000.0,
            );
            println!(
                "    Memory (RSS):      {}",
                format_bytes(stats.memory_rss_bytes)
            );
            println!(
                "    Memory (Virtual):  {}",
                format_bytes(stats.memory_vsize_bytes)
            );
            println!("    Active Threads:    {}", stats.threads);
            println!(
                "    Page Faults:       {} (Major: {})",
                stats.page_faults, stats.major_page_faults
            );
        }
    }

    Ok(())
}

pub fn handle_top(id: String, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let vm = match StateManager::find(&base, &id)? {
        Some(v) => v,
        None => bail!("MicroVM '{}' not found in state", id),
    };

    if !vm.is_process_alive() {
        bail!("MicroVM '{}' is not running (PID {})", vm.id, vm.pid);
    }

    let stats = match collect_process_stats(vm.pid) {
        Some(s) => s,
        None => bail!("Failed to collect process stats for PID {}", vm.pid),
    };

    println!("Top - MicroVM {} (PID {})", vm.id, vm.pid);
    println!("{:-<75}", "");
    println!(
        "{:<10} {:<10} {:<16} {:<16} {:<14}",
        "PID", "THREADS", "USER CPU", "SYS CPU", "RSS MEMORY"
    );
    println!(
        "{:<10} {:<10} {:<16} {:<16} {:<14}",
        vm.pid,
        stats.threads,
        format!("{:.2} ms", stats.user_cpu_ns as f64 / 1_000_000.0),
        format!("{:.2} ms", stats.kernel_cpu_ns as f64 / 1_000_000.0),
        format_bytes(stats.memory_rss_bytes)
    );
    Ok(())
}

pub fn handle_cp(src: String, dest: String, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    if let Some((vm_id, guest_path)) = dest.split_once(':') {
        let src_path = PathBuf::from(&src);
        if !src_path.exists() {
            bail!("Source path does not exist: {}", src);
        }
        StateManager::copy_into(&base, vm_id, &src_path, guest_path)?;
        println!(
            "✅ Successfully copied '{}' into '{}:{}'",
            src, vm_id, guest_path
        );
    } else if let Some((vm_id, guest_path)) = src.split_once(':') {
        let dest_path = PathBuf::from(&dest);
        StateManager::copy_from(&base, vm_id, guest_path, &dest_path)?;
        println!(
            "✅ Successfully copied '{}:{}' to '{}'",
            vm_id, guest_path, dest
        );
    } else {
        bail!("Invalid cp syntax. Usage:\n  microvm cp <src_host> <vm_id>:<dest_guest>\n  microvm cp <vm_id>:<src_guest> <dest_host>");
    }
    Ok(())
}

pub async fn handle_logs(id: String, follow: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let vms = StateManager::list(&base)?;
    let target = vms
        .iter()
        .find(|v| v.id == id || v.id.starts_with(&id) || v.pid.to_string() == id);

    let vm = match target {
        Some(v) => v,
        None => bail!("MicroVM '{}' not found in state", id),
    };

    let console_log = vm.instance_dir.join("console.log");
    let trace_log = vm.instance_dir.join("rootfs/init.trace.log");
    let log_file = if console_log.exists() {
        console_log
    } else if trace_log.exists() {
        trace_log
    } else if follow {
        console_log
    } else {
        println!("No log output found for VM '{}'", id);
        return Ok(());
    };

    let mut file = match std::fs::File::open(&log_file) {
        Ok(f) => f,
        Err(_) if follow => {
            let start = std::time::Instant::now();
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if let Ok(f) = std::fs::File::open(&log_file) {
                    break f;
                }
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    bail!(
                        "Log file '{}' was not created after 5 seconds",
                        log_file.display()
                    );
                }
            }
        }
        Err(e) => bail!("Failed to open log file '{}': {}", log_file.display(), e),
    };

    let mut buffer = Vec::new();
    let _ = file.read_to_end(&mut buffer);
    if !buffer.is_empty() {
        print!("{}", String::from_utf8_lossy(&buffer));
        let _ = std::io::stdout().flush();
    }

    if follow {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let mut new_bytes = Vec::new();
            let n = file.read_to_end(&mut new_bytes)?;
            if n > 0 {
                print!("{}", String::from_utf8_lossy(&new_bytes));
                let _ = std::io::stdout().flush();
            } else {
                let vms = StateManager::list(&base)?;
                let is_alive = vms.iter().any(|v| (v.id == vm.id) && v.is_process_alive());
                if !is_alive {
                    let mut final_bytes = Vec::new();
                    let _ = file.read_to_end(&mut final_bytes);
                    if !final_bytes.is_empty() {
                        print!("{}", String::from_utf8_lossy(&final_bytes));
                        let _ = std::io::stdout().flush();
                    }
                    break;
                }
            }
        }
    }

    Ok(())
}

pub fn handle_prune(data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    println!("Pruning stopped microVMs and staging caches...");
    let summary = StateManager::prune(&base)?;
    println!(
        "✅ Pruned {} dead instance directories and {} staging cache directories.",
        summary.pruned_instances, summary.pruned_layers
    );
    Ok(())
}

pub fn handle_resize(
    id: String,
    memory: Option<u32>,
    cpus: Option<u8>,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let updated = StateManager::resize(&base, &id, memory, cpus)?;
    println!("✅ MicroVM '{}' resized successfully:", updated.id);
    if let Some(m) = updated.memory_mib {
        println!("   RAM:   {} MiB", m);
    }
    if let Some(c) = updated.vcpus {
        println!("   vCPUs: {}", c);
    }
    Ok(())
}
