// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Prometheus metrics exporter HTTP server and metrics command handler.

use anyhow::{Context, Result};
use microvm_core::{StateManager, VmStatus};
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::utils::default_data_dir;

pub async fn handle_metrics_command(
    listen: Option<String>,
    format_json: bool,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    if let Some(ref addr) = listen {
        start_metrics_server(&base, addr, format_json).await?;
    } else {
        let vms = StateManager::list(&base)?;
        if format_json {
            let summary = json!({
                "total": vms.len(),
                "running": vms.iter().filter(|v| v.status == VmStatus::Running).count(),
                "paused": vms.iter().filter(|v| v.status == VmStatus::Paused).count(),
                "stopped": vms.iter().filter(|v| v.status == VmStatus::Stopped).count(),
                "vms": vms,
            });
            println!("{}", serde_json::to_string_pretty(&summary)?);
        } else {
            println!("{}", microvm_core::metrics::export_prometheus_metrics(&vms));
        }
    }
    Ok(())
}

pub async fn start_metrics_server(data_dir: &Path, addr: &str, format_json: bool) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind metrics HTTP server to {}", addr))?;
    println!(
        "📊 MicroVM Prometheus Metrics Exporter listening on http://{}/metrics",
        addr
    );
    println!("   Press Ctrl+C to stop.");

    let data_dir = data_dir.to_path_buf();
    loop {
        tokio::select! {
            accept_res = listener.accept() => {
                let (mut socket, peer) = accept_res?;
                tracing::debug!("Metrics scrape connection from {}", peer);
                let vms = StateManager::list(&data_dir).unwrap_or_default();
                let (content_type, body) = if format_json {
                    let summary = json!({
                        "total": vms.len(),
                        "running": vms.iter().filter(|v| v.status == VmStatus::Running).count(),
                        "paused": vms.iter().filter(|v| v.status == VmStatus::Paused).count(),
                        "stopped": vms.iter().filter(|v| v.status == VmStatus::Stopped).count(),
                        "vms": vms,
                    });
                    ("application/json", serde_json::to_string_pretty(&summary).unwrap_or_default())
                } else {
                    ("text/plain; version=0.0.4; charset=utf-8", microvm_core::metrics::export_prometheus_metrics(&vms))
                };

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    content_type,
                    body.len(),
                    body
                );

                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                });
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down metrics exporter...");
                break;
            }
        }
    }
    Ok(())
}
