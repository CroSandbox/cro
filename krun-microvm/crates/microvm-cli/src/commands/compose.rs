// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Multi-microVM orchestration commands using Compose specifications.

use crate::args::ComposeCommands;
use crate::utils::default_data_dir;
use anyhow::{bail, Context, Result};
use microvm_core::compose::{ComposeProject, ComposeProjectState, ComposeServiceState};
use microvm_core::StateManager;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub fn resolve_compose_file(file: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(f) = file {
        if !f.exists() {
            bail!("Compose file '{}' does not exist", f.display());
        }
        Ok(f)
    } else {
        let cur = std::env::current_dir().context("Failed to get current working directory")?;
        ComposeProject::discover_compose_file(&cur)
    }
}

pub async fn handle_compose_command(command: ComposeCommands) -> Result<()> {
    match command {
        ComposeCommands::Up {
            file,
            detach,
            services,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let compose_file = resolve_compose_file(file)?;
            let project = ComposeProject::load(&compose_file, Some(base_data_dir.clone()))?;
            handle_compose_up(&project, &compose_file, &base_data_dir, detach, &services).await?;
        }
        ComposeCommands::Down {
            file,
            volumes,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let compose_file = resolve_compose_file(file)?;
            let project = ComposeProject::load(&compose_file, Some(base_data_dir.clone()))?;
            handle_compose_down(&project, &base_data_dir, volumes)?;
        }
        ComposeCommands::Ps { file, data_dir } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let compose_file = resolve_compose_file(file)?;
            let project = ComposeProject::load(&compose_file, Some(base_data_dir.clone()))?;
            handle_compose_ps(&project, &base_data_dir)?;
        }
        ComposeCommands::Logs {
            file,
            service,
            follow,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let compose_file = resolve_compose_file(file)?;
            let project = ComposeProject::load(&compose_file, Some(base_data_dir.clone()))?;
            handle_compose_logs(&project, &base_data_dir, service, follow).await?;
        }
        ComposeCommands::Stop {
            file,
            services,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let compose_file = resolve_compose_file(file)?;
            let project = ComposeProject::load(&compose_file, Some(base_data_dir.clone()))?;
            handle_compose_stop(&project, &base_data_dir, &services)?;
        }
        ComposeCommands::Start {
            file,
            services,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let compose_file = resolve_compose_file(file)?;
            let project = ComposeProject::load(&compose_file, Some(base_data_dir.clone()))?;
            handle_compose_start(&project, &base_data_dir, &services).await?;
        }
        ComposeCommands::Restart {
            file,
            services,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let compose_file = resolve_compose_file(file)?;
            let project = ComposeProject::load(&compose_file, Some(base_data_dir.clone()))?;
            handle_compose_restart(&project, &base_data_dir, &services).await?;
        }
        ComposeCommands::Config { file, data_dir } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let compose_file = resolve_compose_file(file)?;
            let project = ComposeProject::load(&compose_file, Some(base_data_dir))?;
            handle_compose_config(&project)?;
        }
    }
    Ok(())
}

pub async fn handle_compose_up(
    project: &ComposeProject,
    manifest_path: &Path,
    data_dir: &Path,
    detach: bool,
    services_filter: &[String],
) -> Result<()> {
    use std::io::Write;

    let all_order = project.resolve_launch_order()?;
    let to_launch: Vec<String> = if services_filter.is_empty() {
        all_order
    } else {
        let set: std::collections::HashSet<&str> =
            services_filter.iter().map(|s| s.as_str()).collect();
        for s in services_filter {
            if !project.spec.services.contains_key(s) {
                bail!("Service '{}' not found in compose manifest", s);
            }
        }
        all_order
            .into_iter()
            .filter(|s| set.contains(s.as_str()))
            .collect()
    };

    println!(
        "[+] Running {} microVM service(s) for project '{}':",
        to_launch.len(),
        project.name
    );

    let mut state = project
        .load_state()?
        .unwrap_or_else(|| ComposeProjectState {
            name: project.name.clone(),
            compose_file: manifest_path.to_path_buf(),
            working_dir: project.base_dir.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            services: BTreeMap::new(),
        });

    let mut active_vms: Vec<microvm_core::MicroVm> = Vec::new();

    for service_name in &to_launch {
        let svc_spec = &project.spec.services[service_name];

        if let Some(prev) = state.services.get(service_name) {
            if let Ok(Some(existing_vm)) = StateManager::find(data_dir, &prev.instance_id) {
                if existing_vm.is_process_alive() {
                    println!(
                        " ✔ Service '{}' is already running (ID: {}, PID: {})",
                        service_name,
                        prev.instance_id,
                        prev.pid.unwrap_or(0)
                    );
                    continue;
                }
            }
        }

        print!(
            " ⏳ Starting service '{}' ({}) ...",
            service_name, svc_spec.image
        );
        let _ = std::io::stdout().flush();

        let builder = project.build_service_vm(service_name)?;
        let vm = builder
            .run()
            .await
            .with_context(|| format!("Failed to launch service '{service_name}'"))?;

        println!(
            "\r ✔ Service '{}' started  (ID: {}, PID: {:?})",
            service_name,
            vm.id(),
            vm.pid()
        );

        state.services.insert(
            service_name.clone(),
            ComposeServiceState {
                service_name: service_name.clone(),
                instance_id: vm.id().to_string(),
                pid: vm.pid(),
                image: svc_spec.image.clone(),
                status: "running".to_string(),
                ports: svc_spec.ports.clone(),
            },
        );
        project.save_state(&state)?;

        if !detach {
            active_vms.push(vm);
        }
    }

    if detach {
        println!(
            "\n✨ Compose project '{}' started in detached mode.",
            project.name
        );
        print_compose_ps_table(&state, data_dir);
        return Ok(());
    }

    println!("\nAttaching to compose project console output (Press Ctrl+C to stop)...");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\n⚠️ Received interrupt (Ctrl+C). Gracefully stopping compose project '{}'...", project.name);
            for mut vm in active_vms {
                let _ = vm.stop().await;
            }
            for svc_state in state.services.values_mut() {
                svc_state.status = "stopped".to_string();
            }
            let _ = project.save_state(&state);
            println!("🛑 All microVM services stopped.");
        }
    }

    Ok(())
}

pub fn handle_compose_down(
    project: &ComposeProject,
    data_dir: &Path,
    remove_volumes: bool,
) -> Result<()> {
    use std::io::Write;

    println!(
        "[+] Stopping and removing microVM services for project '{}':",
        project.name
    );
    if let Some(state) = project.load_state()? {
        let mut rev_services: Vec<_> = state.services.values().cloned().collect();
        rev_services.reverse();
        for svc in rev_services {
            print!(
                " ⏳ Stopping service '{}' (ID: {})...",
                svc.service_name, svc.instance_id
            );
            let _ = std::io::stdout().flush();
            let _ = StateManager::stop(data_dir, &svc.instance_id);
            let _ = StateManager::delete(data_dir, &svc.instance_id, true);
            println!("\r ✔ Service '{}' stopped and removed", svc.service_name);
        }
        let _ = project.remove_state();
    } else {
        println!("No active state found for project '{}'.", project.name);
    }

    if remove_volumes {
        println!(" ✔ Volumes cleaned up");
    }

    println!("✨ Compose project '{}' is down.", project.name);
    Ok(())
}

pub fn handle_compose_ps(project: &ComposeProject, data_dir: &Path) -> Result<()> {
    let state = project.load_state()?;
    println!("Project: {}", project.name);
    match state {
        Some(ref s) => print_compose_ps_table(s, data_dir),
        None => println!(
            "No active compose state found for project '{}'.",
            project.name
        ),
    }
    Ok(())
}

pub fn print_compose_ps_table(state: &ComposeProjectState, data_dir: &Path) {
    if state.services.is_empty() {
        println!("No services declared or running.");
        return;
    }

    println!(
        "{:<18} {:<18} {:<18} {:<24} {:<20}",
        "SERVICE", "CONTAINER ID", "STATUS", "IMAGE", "PORTS"
    );
    println!("{:-<100}", "");
    for (name, svc) in &state.services {
        let is_alive = if let Ok(Some(vm)) = StateManager::find(data_dir, &svc.instance_id) {
            vm.is_process_alive()
        } else {
            false
        };
        let status_str = if is_alive {
            format!("Up (PID {})", svc.pid.unwrap_or(0))
        } else {
            "Exited".to_string()
        };
        let ports_str = if svc.ports.is_empty() {
            "-".to_string()
        } else {
            svc.ports.join(", ")
        };
        let trunc_id = if svc.instance_id.len() > 14 {
            &svc.instance_id[..14]
        } else {
            &svc.instance_id
        };
        println!(
            "{:<18} {:<18} {:<18} {:<24} {:<20}",
            name, trunc_id, status_str, svc.image, ports_str
        );
    }
}

pub async fn handle_compose_logs(
    project: &ComposeProject,
    data_dir: &Path,
    service_filter: Option<String>,
    follow: bool,
) -> Result<()> {
    use std::io::{Read, Write};

    let state = match project.load_state()? {
        Some(s) => s,
        None => bail!(
            "No state found for compose project '{}'. Is it running?",
            project.name
        ),
    };

    if let Some(target_svc) = service_filter {
        let svc_state = state.services.get(&target_svc).with_context(|| {
            format!(
                "Service '{target_svc}' not found in project '{}'",
                project.name
            )
        })?;

        let vm = StateManager::find(data_dir, &svc_state.instance_id)?.with_context(|| {
            format!(
                "Instance '{}' for service '{target_svc}' not found",
                svc_state.instance_id
            )
        })?;

        let log_file = vm.instance_dir.join("console.log");
        if !log_file.exists() && !follow {
            println!("No log output for service '{}'", target_svc);
            return Ok(());
        }

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

        let mut buf = Vec::new();
        let _ = file.read_to_end(&mut buf);
        print!("{}", String::from_utf8_lossy(&buf));
        let _ = std::io::stdout().flush();

        if follow {
            let mut pos = buf.len() as u64;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                if let Ok(meta) = std::fs::metadata(&log_file) {
                    if meta.len() > pos {
                        use std::io::Seek;
                        if let Ok(mut f) = std::fs::File::open(&log_file) {
                            if f.seek(std::io::SeekFrom::Start(pos)).is_ok() {
                                let mut new_buf = Vec::new();
                                if f.read_to_end(&mut new_buf).is_ok() {
                                    pos = meta.len();
                                    print!("{}", String::from_utf8_lossy(&new_buf));
                                    let _ = std::io::stdout().flush();
                                }
                            }
                        }
                    }
                }
            }
        }
    } else {
        for (name, svc_state) in &state.services {
            if let Ok(Some(vm)) = StateManager::find(data_dir, &svc_state.instance_id) {
                let log_file = vm.instance_dir.join("console.log");
                if let Ok(mut f) = std::fs::File::open(&log_file) {
                    let mut content = String::new();
                    let _ = f.read_to_string(&mut content);
                    for line in content.lines() {
                        println!("{:<12} | {}", name, line);
                    }
                }
            }
        }

        if follow {
            println!("Streaming logs (Press Ctrl+C to exit)...");
            let mut file_positions: HashMap<String, u64> = HashMap::new();
            for (name, svc_state) in &state.services {
                if let Ok(Some(vm)) = StateManager::find(data_dir, &svc_state.instance_id) {
                    let log_file = vm.instance_dir.join("console.log");
                    if let Ok(meta) = std::fs::metadata(&log_file) {
                        file_positions.insert(name.clone(), meta.len());
                    }
                }
            }

            loop {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                for (name, svc_state) in &state.services {
                    if let Ok(Some(vm)) = StateManager::find(data_dir, &svc_state.instance_id) {
                        let log_file = vm.instance_dir.join("console.log");
                        let pos = file_positions.get(name).copied().unwrap_or(0);
                        if let Ok(meta) = std::fs::metadata(&log_file) {
                            if meta.len() > pos {
                                use std::io::Seek;
                                if let Ok(mut f) = std::fs::File::open(&log_file) {
                                    if f.seek(std::io::SeekFrom::Start(pos)).is_ok() {
                                        let mut new_content = String::new();
                                        if f.read_to_string(&mut new_content).is_ok() {
                                            file_positions.insert(name.clone(), meta.len());
                                            for line in new_content.lines() {
                                                println!("{:<12} | {}", name, line);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn handle_compose_stop(
    project: &ComposeProject,
    data_dir: &Path,
    services_filter: &[String],
) -> Result<()> {
    use std::io::Write;

    let mut state = match project.load_state()? {
        Some(s) => s,
        None => {
            println!("No active state found for project '{}'.", project.name);
            return Ok(());
        }
    };

    println!("[+] Stopping services for project '{}':", project.name);
    for (name, svc) in state.services.iter_mut() {
        if !services_filter.is_empty() && !services_filter.contains(name) {
            continue;
        }
        print!(
            " ⏳ Stopping service '{}' (ID: {})...",
            name, svc.instance_id
        );
        let _ = std::io::stdout().flush();
        let _ = StateManager::stop(data_dir, &svc.instance_id);
        svc.status = "stopped".to_string();
        println!("\r ✔ Service '{}' stopped", name);
    }
    project.save_state(&state)?;
    Ok(())
}

pub async fn handle_compose_start(
    project: &ComposeProject,
    data_dir: &Path,
    services_filter: &[String],
) -> Result<()> {
    use std::io::Write;

    let mut state = project
        .load_state()?
        .unwrap_or_else(|| ComposeProjectState {
            name: project.name.clone(),
            compose_file: project.base_dir.join("krun-compose.yaml"),
            working_dir: project.base_dir.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            services: BTreeMap::new(),
        });

    let launch_order = project.resolve_launch_order()?;
    let to_start: Vec<String> = if services_filter.is_empty() {
        launch_order
    } else {
        launch_order
            .into_iter()
            .filter(|s| services_filter.contains(s))
            .collect()
    };

    println!("[+] Starting services for project '{}':", project.name);
    for name in &to_start {
        if let Some(svc) = state.services.get(name) {
            if let Ok(Some(existing_vm)) = StateManager::find(data_dir, &svc.instance_id) {
                if existing_vm.is_process_alive() {
                    println!(" ✔ Service '{}' is already running", name);
                    continue;
                }
            }
        }

        print!(" ⏳ Starting service '{}'...", name);
        let _ = std::io::stdout().flush();
        let builder = project.build_service_vm(name)?;
        let vm = builder
            .run()
            .await
            .with_context(|| format!("Failed to start service '{name}'"))?;
        let svc_spec = &project.spec.services[name];

        state.services.insert(
            name.clone(),
            ComposeServiceState {
                service_name: name.clone(),
                instance_id: vm.id().to_string(),
                pid: vm.pid(),
                image: svc_spec.image.clone(),
                status: "running".to_string(),
                ports: svc_spec.ports.clone(),
            },
        );
        println!(
            "\r ✔ Service '{}' started  (ID: {}, PID: {:?})",
            name,
            vm.id(),
            vm.pid()
        );
    }
    project.save_state(&state)?;
    Ok(())
}

pub async fn handle_compose_restart(
    project: &ComposeProject,
    data_dir: &Path,
    services_filter: &[String],
) -> Result<()> {
    println!("[+] Restarting services for project '{}':", project.name);
    handle_compose_stop(project, data_dir, services_filter)?;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    handle_compose_start(project, data_dir, services_filter).await?;
    Ok(())
}

pub fn handle_compose_config(project: &ComposeProject) -> Result<()> {
    let yaml = serde_yaml::to_string(&project.spec)
        .context("Failed to serialize resolved compose configuration to YAML")?;
    println!("{yaml}");
    Ok(())
}
