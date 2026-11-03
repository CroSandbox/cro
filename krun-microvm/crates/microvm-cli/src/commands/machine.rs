// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CLI handler for `cro machine` (persistent Linux developer workstations).

use anyhow::Result;
use std::path::PathBuf;

use crate::args::MachineCommands;
use crate::utils::default_data_dir;
use microvm_core::{get_host_user_info, ExecRequest, MachineManager, StateManager};

pub async fn handle_machine(command: MachineCommands) -> Result<()> {
    match command {
        MachineCommands::Create {
            name,
            distro,
            image,
            cpus,
            memory,
            rosetta,
            data_dir,
        } => handle_create(name, distro, image, cpus, memory, rosetta, data_dir).await,
        MachineCommands::Shell {
            name,
            data_dir,
            cmd,
        } => handle_shell(name, data_dir, cmd).await,
        MachineCommands::List { json, data_dir } => handle_list(json, data_dir),
        MachineCommands::Start { name, data_dir } => handle_start(name, data_dir).await,
        MachineCommands::Stop { name, data_dir } => handle_stop(name, data_dir),
        MachineCommands::Rm {
            name,
            force,
            data_dir,
        } => handle_rm(name, force, data_dir),
    }
}

async fn handle_create(
    name: String,
    distro: String,
    image: Option<String>,
    cpus: Option<u8>,
    memory: Option<u32>,
    rosetta: bool,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    println!(
        "🛠️  Provisioning persistent developer machine '{}'...",
        name
    );

    let config = MachineManager::create(
        &base,
        &name,
        &distro,
        image.as_deref(),
        cpus,
        memory,
        rosetta,
    )
    .await?;

    println!("✅ Machine '{}' is running!", config.name);
    println!("   Image:    {}", config.image);
    println!("   vCPUs:    {}", config.cpus);
    println!("   RAM:      {} MiB", config.memory_mb);
    println!(
        "   Storage:  {}",
        MachineManager::machine_dir(&base, &config.name).display()
    );
    println!("   User:     Parity with host $(USER)");
    println!("   Mount:    /Users -> /Users (VirtioFS)");
    if config.rosetta {
        println!("   Rosetta:  Enabled (Apple Silicon x86_64)");
    }
    println!();
    println!("💡 Open an interactive shell inside the machine:");
    println!("   cro machine shell {}", config.name);

    Ok(())
}

async fn handle_shell(name: String, data_dir: Option<PathBuf>, cmd: Vec<String>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let mut config = MachineManager::get(&base, &name)?;

    if config.status != "running" {
        println!("Machine '{}' is stopped. Starting...", name);
        config = MachineManager::start(&base, &name).await?;
    }

    let (user, _uid, _gid) = get_host_user_info();

    // Determine current working directory with /Users path parity
    let cwd = std::env::current_dir()
        .ok()
        .filter(|d| d.starts_with("/Users") || d.starts_with("/home"))
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| format!("/Users/{}", user));

    let exec_cmd = if cmd.is_empty() {
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "exec $(command -v bash || command -v zsh || command -v sh) -l".to_string(),
        ]
    } else {
        cmd
    };

    let req = ExecRequest::new(exec_cmd)
        .with_env(vec![
            format!("HOME=/Users/{}", user),
            format!("USER={}", user),
            "TERM=xterm-256color".to_string(),
        ])
        .with_workdir(cwd)
        .with_tty(true);

    let resp = StateManager::exec(&base, &config.instance_id, &req).await?;

    if !resp.stdout.is_empty() {
        print!("{}", resp.stdout);
    }
    if !resp.stderr.is_empty() {
        eprint!("{}", resp.stderr);
    }

    if resp.exit_code != 0 {
        std::process::exit(resp.exit_code);
    }

    Ok(())
}

fn handle_list(json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let machines = MachineManager::list(&base)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&machines)?);
        return Ok(());
    }

    if machines.is_empty() {
        println!("No developer machines found.");
        println!("💡 Tip: Create a machine with: cro machine create dev --distro ubuntu");
        return Ok(());
    }

    println!(
        "{:<16} {:<12} {:<24} {:<6} {:<10} {:<10} STATUS",
        "NAME", "DISTRO", "IMAGE", "CPUS", "RAM (MiB)", "ROSETTA"
    );
    println!("{:-<95}", "");

    for m in machines {
        let status_str = if m.status == "running" {
            "● Running".to_string()
        } else {
            "○ Stopped".to_string()
        };

        println!(
            "{:<16} {:<12} {:<24} {:<6} {:<10} {:<10} {}",
            m.name,
            m.distro,
            m.image,
            m.cpus,
            m.memory_mb,
            if m.rosetta { "yes" } else { "no" },
            status_str,
        );
    }

    Ok(())
}

async fn handle_start(name: String, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    println!("Starting machine '{}'...", name);
    let config = MachineManager::start(&base, &name).await?;
    println!(
        "✅ Machine '{}' is running (ID: {}).",
        name, config.instance_id
    );
    Ok(())
}

fn handle_stop(name: String, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    println!("Stopping machine '{}'...", name);
    MachineManager::stop(&base, &name)?;
    println!("🛑 Machine '{}' stopped.", name);
    Ok(())
}

fn handle_rm(name: String, force: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    println!("Removing machine '{}'...", name);
    MachineManager::delete(&base, &name, force)?;
    println!("🗑️  Machine '{}' removed successfully.", name);
    Ok(())
}
