// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CLI handler for `cro domain` (automatic local domain ingress router).

use anyhow::{Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::args::DomainCommands;
use crate::utils::default_data_dir;
use microvm_core::{
    list_active_domains, query_micro_dns, run_domain_router, setup_macos_resolver,
    DomainRouterConfig,
};

#[derive(Debug, Serialize, Deserialize)]
struct DomainRouterState {
    pid: u32,
    port: u16,
    bind_addr: String,
    dns_port: u16,
    started_at: u64,
}

fn router_pid_path(data_dir: &Path) -> PathBuf {
    data_dir.join("domain-router.pid")
}

fn router_state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("domain-router.json")
}

fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

pub async fn handle_domain(command: DomainCommands) -> Result<()> {
    match command {
        DomainCommands::Start {
            port,
            addr,
            detach,
            data_dir,
            dns_port,
            no_dns,
        } => handle_start(port, addr, detach, data_dir, dns_port, no_dns).await,
        DomainCommands::SetupResolver { dns_port } => handle_setup_resolver(dns_port),
        DomainCommands::Resolve { domain, dns_port } => handle_resolve(&domain, dns_port).await,
        DomainCommands::List { json, data_dir } => handle_list(json, data_dir),
        DomainCommands::Stop { data_dir } => handle_stop(data_dir),
    }
}

fn handle_setup_resolver(dns_port: u16) -> Result<()> {
    let msg = setup_macos_resolver(dns_port)?;
    println!("{}", msg);
    Ok(())
}

async fn handle_resolve(domain: &str, dns_port: u16) -> Result<()> {
    println!(
        "🔍 Querying embedded Micro-DNS server at 127.0.0.1:{} for '{}'...",
        dns_port, domain
    );
    match query_micro_dns("127.0.0.1", dns_port, domain).await {
        Ok(ip) => {
            println!("✅ Resolved '{}' -> {}", domain, ip);
            Ok(())
        }
        Err(e) => {
            eprintln!("❌ Failed to resolve '{}': {}", domain, e);
            Err(e)
        }
    }
}

async fn handle_start(
    port: u16,
    addr: Option<String>,
    detach: bool,
    data_dir: Option<PathBuf>,
    dns_port: u16,
    no_dns: bool,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    fs::create_dir_all(&base)?;

    let pid_file = router_pid_path(&base);
    if pid_file.exists() {
        if let Ok(pid_str) = fs::read_to_string(&pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                if pid != std::process::id() && is_pid_alive(pid) {
                    println!(
                        "ℹ️  Local domain ingress router is already running on PID {} (port {}).",
                        pid, port
                    );
                    return Ok(());
                }
            }
        }
    }

    let bind_addr = addr.clone().unwrap_or_else(|| {
        if cfg!(target_os = "macos") {
            "0.0.0.0".to_string()
        } else {
            "127.0.0.1".to_string()
        }
    });

    if detach {
        let exe = std::env::current_exe().context("Failed to get current executable path")?;
        let mut cmd = Command::new(exe);
        cmd.arg("domain")
            .arg("start")
            .arg("--port")
            .arg(port.to_string())
            .arg("--addr")
            .arg(&bind_addr)
            .arg("--dns-port")
            .arg(dns_port.to_string())
            .arg("--data-dir")
            .arg(&base);

        if no_dns {
            cmd.arg("--no-dns");
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = cmd
            .spawn()
            .context("Failed to spawn background domain router")?;
        let pid = child.id();

        fs::write(&pid_file, pid.to_string())?;

        let state = DomainRouterState {
            pid,
            port,
            bind_addr: bind_addr.clone(),
            dns_port,
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        fs::write(
            router_state_path(&base),
            serde_json::to_string_pretty(&state)?,
        )?;

        println!(
            "🚀 Local domain ingress router started in background (PID: {}, Port: {})",
            pid, port
        );
        if !no_dns {
            println!(
                "🌐 Micro-DNS Server: 127.0.0.1:{} (*.cro.local -> 127.0.0.1)",
                dns_port
            );
        }
        println!("🌐 Resolving: *.cro.local -> running microVMs");
        println!("👉 Inspect mappings with: cro domain list");
        return Ok(());
    }

    // Foreground execution
    let current_pid = std::process::id();
    fs::write(&pid_file, current_pid.to_string())?;

    let state = DomainRouterState {
        pid: current_pid,
        port,
        bind_addr: bind_addr.clone(),
        dns_port,
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let _ = fs::write(
        router_state_path(&base),
        serde_json::to_string_pretty(&state)?,
    );

    println!(
        "🌐 Starting local domain ingress router on {}:{}...",
        bind_addr, port
    );
    if !no_dns {
        println!(
            "🌐 Micro-DNS Server active on 127.0.0.1:{} (*.cro.local -> 127.0.0.1)",
            dns_port
        );
    }
    println!(
        "👉 Point your browser or curl to: http://<service>.cro.local:{}/",
        port
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let shutdown_tx_clone = shutdown_tx.clone();

    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!("\nReceived shutdown signal, terminating domain router...");
        let _ = shutdown_tx_clone.send(());
    });

    let config = DomainRouterConfig {
        bind_port: port,
        bind_addr: Some(bind_addr),
        data_dir: base.clone(),
        dns_port,
        enable_dns: !no_dns,
    };

    let run_res = run_domain_router(config, shutdown_rx).await;

    // Cleanup PID file on clean shutdown
    let _ = fs::remove_file(&pid_file);
    let _ = fs::remove_file(router_state_path(&base));

    run_res
}

fn handle_list(json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let mappings = list_active_domains(&base);

    let pid_file = router_pid_path(&base);
    let router_status = if pid_file.exists() {
        if let Ok(pid_str) = fs::read_to_string(&pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                if is_pid_alive(pid) {
                    format!("● Running (PID {})", pid)
                } else {
                    "○ Stopped (stale PID)".to_string()
                }
            } else {
                "○ Stopped".to_string()
            }
        } else {
            "○ Stopped".to_string()
        }
    } else {
        "○ Stopped (start with: cro domain start)".to_string()
    };

    if json {
        let out = serde_json::json!({
            "router_status": router_status,
            "domains": mappings,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("Domain Router Status: {}", router_status);
    println!();

    if mappings.is_empty() {
        println!("No active local domain mappings found.");
        println!("💡 Tip: Launch a container with exposed ports to route automatically: cro run -p 8080:80 nginx:alpine");
        return Ok(());
    }

    println!(
        "{:<30} {:<20} {:<16} {:<24} STATUS",
        "DOMAIN", "HOST TARGET", "INSTANCE ID", "SERVICE / IMAGE"
    );
    println!("{:-<105}", "");

    for m in mappings {
        let short_id = if m.instance_id.len() > 12 {
            &m.instance_id[..12]
        } else {
            &m.instance_id
        };

        println!(
            "{:<30} {:<20} {:<16} {:<24} {}",
            m.domain,
            format!("127.0.0.1:{}", m.target_host_port),
            short_id,
            m.image_or_service,
            m.status,
        );
    }

    Ok(())
}

fn handle_stop(data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let pid_file = router_pid_path(&base);

    if !pid_file.exists() {
        println!("ℹ️  Local domain router is not running.");
        return Ok(());
    }

    let pid_str = fs::read_to_string(&pid_file)?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .context("Invalid PID format in domain router pid file")?;

    if is_pid_alive(pid) {
        kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
            .context("Failed to send SIGTERM to domain router process")?;
        println!("🛑 Stopped local domain router (PID: {}).", pid);
    } else {
        println!(
            "ℹ️  Domain router process (PID: {}) is no longer alive.",
            pid
        );
    }

    let _ = fs::remove_file(&pid_file);
    let _ = fs::remove_file(router_state_path(&base));

    Ok(())
}
