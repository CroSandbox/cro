// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CLI handlers for system operations: pull, preflight, info, stats, build, snapshot, restore, exec.

use anyhow::{bail, Result};
use microvm_core::{
    collect_process_stats, BuildEngine, BuildOptions, ExecRequest, ImageReference, OciClient,
    OciLayout, Preflight, StateManager,
};
use serde_json::json;
use std::path::PathBuf;

use crate::utils::{default_data_dir, dir_size, format_bytes, host_memory_bytes};

pub async fn handle_pull(image: String, data_dir: Option<PathBuf>) -> Result<()> {
    let cache_base = data_dir.unwrap_or_else(default_data_dir);

    println!("📦 Pulling image '{}'...", image);
    let reference = ImageReference::parse(&image)?;
    let (rootfs, config) = if reference.is_local_layout {
        let layout_path = reference.layout_path.as_ref().unwrap();
        println!(
            "📂 Loading local OCI image layout from '{}'...",
            layout_path.display()
        );
        let (rootfs, config) = OciLayout::load(layout_path, Some(&reference.tag), &cache_base)?;
        println!("✅ Offline OCI layout loaded and unpacked!");
        (rootfs, config)
    } else {
        let client = OciClient::new();
        let (rootfs, config) = client.pull_and_unpack(&reference, &cache_base).await?;
        println!("✅ Image successfully pulled and cached!");
        (rootfs, config)
    };

    println!("   Rootfs path: {}", rootfs.display());
    println!("   Entrypoint: {:?}", config.entrypoint);
    println!("   Cmd: {:?}", config.cmd);
    println!("   Env count: {}", config.env.len());
    Ok(())
}

pub fn handle_preflight(port: Vec<u16>) -> Result<()> {
    println!("🔍 Running MicroVM preflight checks...\n");
    let workdir = default_data_dir();
    let results = Preflight::run_all(&port, &workdir);

    let mut all_ok = true;
    for r in results {
        let status_icon = if r.passed { " [PASS]" } else { "❌ [FAIL]" };
        println!("{} {}: {}", status_icon, r.name, r.message);
        if !r.passed {
            all_ok = false;
        }
    }

    if all_ok {
        println!("\n🎉 System is fully ready to run hardware-isolated microVMs!");
    } else {
        println!("\n⚠️ Some preflight checks failed. Please address them before running VMs.");
    }
    Ok(())
}

pub fn handle_info() -> Result<()> {
    let base = default_data_dir();
    let vms = StateManager::list(&base).unwrap_or_default();
    let running_count = vms.iter().filter(|v| v.is_process_alive()).count();
    let stopped_count = vms.len().saturating_sub(running_count);

    let layers_size = dir_size(&base.join("layers"));
    let artifacts_size = dir_size(&base.join("artifacts"));
    let instances_size = dir_size(&base.join("instances"));
    let total_cache = layers_size + artifacts_size + instances_size;

    let cpus = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    let mem_str = host_memory_bytes()
        .map(format_bytes)
        .unwrap_or_else(|| "Unknown".to_string());

    println!("libkrun-microvm SDK (Rust Edition)");
    println!("--------------------------------------------------");
    println!("OS:                      {}", std::env::consts::OS);
    println!("Architecture:            {}", std::env::consts::ARCH);
    println!(
        "Hypervisor:              {}",
        if cfg!(target_os = "macos") {
            "Apple Silicon Hypervisor.framework"
        } else {
            "Linux KVM"
        }
    );
    println!("Host CPUs:               {}", cpus);
    println!("Host Memory:             {}", mem_str);
    println!("Data Cache Root:         {}", base.display());
    println!("--------------------------------------------------");
    println!(
        "Active MicroVMs:         {} running ({} total)",
        running_count,
        vms.len()
    );
    println!("  - Running:             {}", running_count);
    println!("  - Stopped:             {}", stopped_count);
    println!("Disk Utilization:");
    println!("  - OCI Layer Cache:     {}", format_bytes(layers_size));
    println!("  - OCI Artifacts:       {}", format_bytes(artifacts_size));
    println!("  - MicroVM Instances:   {}", format_bytes(instances_size));
    println!("  - Total Disk Used:     {}", format_bytes(total_cache));
    println!("libkrun Library:         Dynamic linkage (/opt/homebrew/lib or system lib)");
    Ok(())
}

pub async fn handle_stats(
    id: Option<String>,
    no_stream: bool,
    json: bool,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);

    if json {
        let vms = StateManager::list(&base)?;
        let targets: Vec<_> = match id {
            Some(ref target_id) => {
                let vm = StateManager::find(&base, target_id)?
                    .ok_or_else(|| anyhow::anyhow!("MicroVM '{}' not found", target_id))?;
                vec![vm]
            }
            None => vms.into_iter().filter(|v| v.is_process_alive()).collect(),
        };

        let mut list = Vec::new();
        for vm in targets {
            let stats = if vm.is_process_alive() {
                collect_process_stats(vm.pid)
            } else {
                None
            };
            list.push(json!({
                "id": vm.id,
                "pid": vm.pid,
                "image": vm.image,
                "status": if vm.is_process_alive() { "Running" } else { "Stopped" },
                "stats": stats,
            }));
        }
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }

    let sample_interval = std::time::Duration::from_millis(if no_stream { 250 } else { 1000 });

    loop {
        let vms = StateManager::list(&base)?;
        let active_vms: Vec<_> = match id {
            Some(ref target_id) => {
                let vm = StateManager::find(&base, target_id)?
                    .ok_or_else(|| anyhow::anyhow!("MicroVM '{}' not found", target_id))?;
                if !vm.is_process_alive() {
                    bail!("MicroVM '{}' is not running (PID {})", vm.id, vm.pid);
                }
                vec![vm]
            }
            None => vms.into_iter().filter(|v| v.is_process_alive()).collect(),
        };

        if active_vms.is_empty() {
            println!("No running microVMs found.");
            return Ok(());
        }

        // Sample 1
        let mut samples1 = Vec::new();
        for vm in &active_vms {
            if let Some(s) = collect_process_stats(vm.pid) {
                samples1.push((vm.clone(), std::time::Instant::now(), s));
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(sample_interval) => {}
            _ = tokio::signal::ctrl_c() => {
                if !no_stream {
                    println!();
                }
                return Ok(());
            }
        }

        // Sample 2
        let mut rows = Vec::new();
        for (vm, t1, s1) in samples1 {
            if let Some(s2) = collect_process_stats(vm.pid) {
                let t2 = std::time::Instant::now();
                let delta_wall = t2.duration_since(t1).as_nanos() as f64;
                let delta_cpu = s2.total_cpu_ns.saturating_sub(s1.total_cpu_ns) as f64;
                let cpu_pct = if delta_wall > 0.0 {
                    (delta_cpu / delta_wall) * 100.0
                } else {
                    0.0
                };
                rows.push((vm, cpu_pct, s2));
            }
        }

        if !no_stream {
            print!("\x1B[2J\x1B[1;1H");
        }

        println!(
            "{:<14} {:<24} {:<10} {:<14} {:<14} {:<8} {:<12}",
            "CONTAINER ID", "IMAGE", "CPU %", "MEM USAGE", "VIRTUAL MEM", "PIDS", "PAGE FAULTS"
        );
        println!("{:-<100}", "");

        for (vm, cpu_pct, stats) in rows {
            let short_id = if vm.id.len() > 12 {
                &vm.id[..12]
            } else {
                &vm.id
            };
            let short_img = if vm.image.len() > 23 {
                format!("{}...", &vm.image[..20])
            } else {
                vm.image
            };
            println!(
                "{:<14} {:<24} {:<10} {:<14} {:<14} {:<8} {:<12}",
                short_id,
                short_img,
                format!("{:.2}%", cpu_pct),
                format_bytes(stats.memory_rss_bytes),
                format_bytes(stats.memory_vsize_bytes),
                stats.threads,
                stats.page_faults,
            );
        }

        if no_stream {
            break;
        }
    }

    Ok(())
}

pub async fn handle_build(
    tag: Option<String>,
    file: Option<PathBuf>,
    no_cache: bool,
    context: PathBuf,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
    let context_abs = std::fs::canonicalize(&context).unwrap_or(context);
    if !context_abs.exists() {
        bail!(
            "Build context path '{}' does not exist",
            context_abs.display()
        );
    }

    let mut opts = BuildOptions::new(context_abs, base_data_dir);
    opts.no_cache = no_cache;
    if let Some(t) = tag {
        opts.tag = Some(t);
    }
    if let Some(f) = file {
        opts.dockerfile_path = Some(f);
    }

    let engine = BuildEngine::new(opts);
    println!("🔨 Building image with native in-process engine...");
    let result = engine.build().await?;
    println!("✅ Successfully built {}", result.digest);
    if let Some(ref t) = result.tag {
        println!("🏷️  Successfully tagged {}", t);
    }
    Ok(())
}

pub fn handle_snapshot(
    id: String,
    output: Option<PathBuf>,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let out_path = output.unwrap_or_else(|| base.join("snapshots").join(format!("{}.tar", id)));
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manifest = StateManager::snapshot(&base, &id, &out_path)?;
    println!("📸 Snapshot created successfully!");
    println!("   Target: {}", out_path.display());
    println!(
        "   Origin: {} (Image: {})",
        manifest.original_id, manifest.image
    );
    Ok(())
}

pub fn handle_restore(
    snapshot: PathBuf,
    name: Option<String>,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let restored = StateManager::restore(&base, &snapshot, name.as_deref())?;
    println!("⚡ MicroVM restored successfully from snapshot!");
    println!("   ID:     {}", restored.id);
    println!("   Image:  {}", restored.image);
    println!("   Rootfs: {}", restored.rootfs_path().display());
    println!(
        "💡 Run 'microvm run --bundle {}' or use with runner to boot.",
        restored.instance_dir.display()
    );
    Ok(())
}

pub async fn handle_exec(
    id: String,
    env: Vec<String>,
    workdir: Option<String>,
    tty: bool,
    data_dir: Option<PathBuf>,
    cmd: Vec<String>,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let mut req = ExecRequest::new(cmd).with_env(env).with_tty(tty);
    if let Some(wd) = workdir {
        req = req.with_workdir(wd);
    }
    let resp = StateManager::exec(&base, &id, &req).await?;
    if !resp.stdout.is_empty() {
        print!("{}", resp.stdout);
    }
    if !resp.stderr.is_empty() {
        eprint!("{}", resp.stderr);
    }
    if let Some(err) = resp.error {
        eprintln!("Exec error: {}", err);
    }
    if resp.exit_code != 0 {
        std::process::exit(resp.exit_code);
    }
    Ok(())
}
