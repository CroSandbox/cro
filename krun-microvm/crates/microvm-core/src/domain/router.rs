// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Local domain ingress HTTP reverse router.
//!
//! Maps `*.cro.local` (and `*.localhost`) to running microVM instances and Compose services
//! via port heuristics and dynamic instance discovery.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

use super::heuristics::select_best_forward_host_port;
use crate::compose::state::ComposeProjectState;
use crate::state::{StateManager, VmStatus};

/// Largest HTTP request header buffer size before routing (64 KB).
pub const MAX_HEAD_SIZE: usize = 64 * 1024;

/// Active domain-to-microVM mapping descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainMapping {
    /// Domain name (e.g. `llama.cro.local`, `web.cro.local`).
    pub domain: String,
    /// Destination port on host loopback (127.0.0.1).
    pub target_host_port: u16,
    /// Target microVM or compose service instance ID.
    pub instance_id: String,
    /// Name of the container image or compose service.
    pub image_or_service: String,
    /// Current execution status.
    pub status: String,
}

/// Configuration options for the local domain ingress router.
#[derive(Debug, Clone)]
pub struct DomainRouterConfig {
    /// Listening TCP port on host (default: 8080, or 80 if permitted).
    pub bind_port: u16,
    /// Optional custom host bind address (defaults to 127.0.0.1 and ::1, or 0.0.0.0 on macOS).
    pub bind_addr: Option<String>,
    /// Base data directory for state lookup.
    pub data_dir: PathBuf,
    /// Port for the embedded Micro-DNS server (default: 5354).
    pub dns_port: u16,
    /// Whether to launch the embedded Micro-DNS server (default: true).
    pub enable_dns: bool,
}

impl Default for DomainRouterConfig {
    fn default() -> Self {
        Self {
            bind_port: 8080,
            bind_addr: None,
            data_dir: PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
                .join(".cache/krun-microvm"),
            dns_port: super::dns::DEFAULT_DNS_PORT,
            enable_dns: true,
        }
    }
}

/// Helper to sanitize container image or service name into a valid domain label.
pub fn sanitize_domain_label(name: &str) -> String {
    let base = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .split(':')
        .next()
        .unwrap_or(name);

    let mut sanitized = String::new();
    for ch in base.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
        } else if (ch == '-' || ch == '_' || ch == '.')
            && !sanitized.ends_with('-')
            && !sanitized.is_empty()
        {
            sanitized.push('-');
        }
    }
    sanitized.trim_matches('-').to_string()
}

/// Inspects active microVMs and Compose services to produce the list of active domain mappings.
pub fn list_active_domains(data_dir: &Path) -> Vec<DomainMapping> {
    let mut mappings = Vec::new();
    let state_mgr = StateManager::new(data_dir);

    // 1. Query standalone microVM instances
    if let Ok(states) = state_mgr.list_states() {
        for vm in states {
            if vm.status != VmStatus::Running && !vm.is_process_alive() {
                continue;
            }

            if let Some(host_port) = select_best_forward_host_port(&vm.port_forwards) {
                // Generate primary domain from short ID
                let short_id = if vm.id.len() > 12 {
                    &vm.id[..12]
                } else {
                    &vm.id
                };

                mappings.push(DomainMapping {
                    domain: format!("{}.cro.local", short_id),
                    target_host_port: host_port,
                    instance_id: vm.id.clone(),
                    image_or_service: vm.image.clone(),
                    status: "running".to_string(),
                });

                // Generate domain from image name if clean and distinct
                let image_label = sanitize_domain_label(&vm.image);
                if !image_label.is_empty() && image_label != short_id {
                    let image_domain = format!("{}.cro.local", image_label);
                    if !mappings.iter().any(|m| m.domain == image_domain) {
                        mappings.push(DomainMapping {
                            domain: image_domain,
                            target_host_port: host_port,
                            instance_id: vm.id.clone(),
                            image_or_service: vm.image.clone(),
                            status: "running".to_string(),
                        });
                    }
                }
            }
        }
    }

    // 2. Query Compose projects
    let compose_dir = data_dir.join("compose");
    if compose_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&compose_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(proj) = serde_json::from_str::<ComposeProjectState>(&content) {
                            for (svc_name, svc) in &proj.services {
                                if svc.status != "running" {
                                    continue;
                                }

                                // Extract best port from svc.ports e.g. "8080:80" or "8000"
                                let mut best_port: Option<u16> = None;
                                for p_str in &svc.ports {
                                    let parts: Vec<&str> = p_str.split(':').collect();
                                    let host_port_candidate = match parts.len() {
                                        1 => parts[0].parse::<u16>().ok(),
                                        2 => parts[0].parse::<u16>().ok(),
                                        3 => parts[1].parse::<u16>().ok(),
                                        _ => None,
                                    };
                                    if let Some(hp) = host_port_candidate {
                                        best_port = Some(hp);
                                        break;
                                    }
                                }

                                if let Some(hp) = best_port {
                                    // Map <svc>.<proj>.cro.local
                                    mappings.push(DomainMapping {
                                        domain: format!("{}.{}.cro.local", svc_name, proj.name),
                                        target_host_port: hp,
                                        instance_id: svc.instance_id.clone(),
                                        image_or_service: format!("{}/{}", proj.name, svc_name),
                                        status: "running".to_string(),
                                    });

                                    // Map <svc>.cro.local if unique
                                    let short_domain = format!("{}.cro.local", svc_name);
                                    if !mappings.iter().any(|m| m.domain == short_domain) {
                                        mappings.push(DomainMapping {
                                            domain: short_domain,
                                            target_host_port: hp,
                                            instance_id: svc.instance_id.clone(),
                                            image_or_service: format!("{}/{}", proj.name, svc_name),
                                            status: "running".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    mappings
}

/// Strips standard domain suffixes and host port to isolate routing identifiers.
pub fn extract_routing_keys(raw_host: &str) -> Vec<String> {
    // 1. Strip port if present (e.g. `llama.cro.local:8080` -> `llama.cro.local`)
    let host = raw_host.split(':').next().unwrap_or(raw_host).trim();
    let lower = host.to_ascii_lowercase();

    let mut keys = Vec::new();
    keys.push(lower.clone());

    for suffix in &[
        ".cro.local",
        ".kiri.local",
        ".krun.local",
        ".cro.localhost",
        ".localhost",
        ".local",
    ] {
        if let Some(prefix) = lower.strip_suffix(suffix) {
            keys.push(prefix.to_string());
            break;
        }
    }

    keys
}

/// Resolves an incoming HTTP `Host:` header to an active `DomainMapping`.
pub fn resolve_domain(data_dir: &Path, raw_host: &str) -> Option<DomainMapping> {
    let mappings = list_active_domains(data_dir);
    let keys = extract_routing_keys(raw_host);

    for mapping in &mappings {
        // Direct match on full domain (e.g. `llama.cro.local`)
        for key in &keys {
            if mapping.domain.eq_ignore_ascii_case(key) {
                return Some(mapping.clone());
            }

            // Match prefix before `.cro.local`
            if let Some(prefix) = mapping.domain.strip_suffix(".cro.local") {
                if prefix.eq_ignore_ascii_case(key) {
                    return Some(mapping.clone());
                }
            }

            // Match by instance ID or short ID prefix
            if mapping.instance_id.eq_ignore_ascii_case(key)
                || (key.len() >= 6 && mapping.instance_id.starts_with(key))
            {
                return Some(mapping.clone());
            }
        }
    }

    None
}

/// Generates a clean HTML 404 response page detailing active microVMs and domains.
pub fn generate_404_html(host: &str, domains: &[DomainMapping], bind_port: u16) -> String {
    let port_suffix = if bind_port == 80 {
        String::new()
    } else {
        format!(":{}", bind_port)
    };

    let domain_rows = if domains.is_empty() {
        "<tr><td colspan=\"4\" style=\"text-align: center; color: #888; padding: 24px;\">No running microVMs with published web ports found.</td></tr>".to_string()
    } else {
        domains
            .iter()
            .map(|d| {
                format!(
                    "<tr>\
                      <td><a href=\"http://{}{}/\" style=\"color: #0366d6; font-weight: 600; text-decoration: none;\">{}</a></td>\
                      <td><code style=\"background: #f6f8fa; padding: 2px 6px; border-radius: 4px;\">127.0.0.1:{}</code></td>\
                      <td>{}</td>\
                      <td><span style=\"color: #28a745; font-weight: 500;\">● {}</span></td>\
                    </tr>",
                    html_escape(&d.domain),
                    port_suffix,
                    html_escape(&d.domain),
                    d.target_host_port,
                    html_escape(&d.image_or_service),
                    html_escape(&d.status)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>cro: Domain Not Found</title>
  <style>
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background: #fafbfc;
      color: #24292e;
      margin: 0;
      padding: 40px 20px;
      display: flex;
      justify-content: center;
    }}
    .card {{
      max-width: 780px;
      width: 100%;
      background: #ffffff;
      border: 1px solid #e1e4e8;
      border-radius: 8px;
      box-shadow: 0 4px 12px rgba(0,0,0,0.05);
      padding: 32px;
    }}
    h1 {{
      font-size: 22px;
      margin-top: 0;
      color: #cb2431;
    }}
    p {{
      color: #586069;
      line-height: 1.5;
    }}
    table {{
      width: 100%;
      border-collapse: collapse;
      margin-top: 24px;
      margin-bottom: 24px;
    }}
    th, td {{
      padding: 10px 12px;
      text-align: left;
      border-bottom: 1px solid #e1e4e8;
      font-size: 14px;
    }}
    th {{
      background: #f6f8fa;
      color: #24292e;
      font-weight: 600;
    }}
    code {{
      font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace;
      font-size: 13px;
    }}
    .footer {{
      margin-top: 24px;
      font-size: 13px;
      color: #6a737d;
      border-top: 1px solid #eaecef;
      padding-top: 16px;
    }}
  </style>
</head>
<body>
  <div class="card">
    <h1>cro: Domain Not Found</h1>
    <p>No active microVM or Compose service was found matching requested domain: <code>{}</code></p>
    <h3>Active Local Domains</h3>
    <table>
      <thead>
        <tr>
          <th>Domain</th>
          <th>Host Target</th>
          <th>Service / Image</th>
          <th>Status</th>
        </tr>
      </thead>
      <tbody>
        {}
      </tbody>
    </table>
    <div class="footer">
      💡 Tip: Launch a container with <code>cro run -p 8080:80 &lt;image&gt;</code> to automatically route traffic here.
    </div>
  </div>
</body>
</html>"#,
        html_escape(host),
        domain_rows
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Processes an individual client connection: parses `Host:` header and proxies to target microVM.
async fn handle_connection(mut client: TcpStream, data_dir: PathBuf, bind_port: u16) -> Result<()> {
    // Security: Only accept connections originating from loopback
    if let Ok(peer) = client.peer_addr() {
        if !peer.ip().is_loopback() {
            warn!(
                "Security: rejected non-loopback connection to domain router from {}",
                peer
            );
            return Ok(());
        }
    }

    let _ = client.set_nodelay(true);

    let header_read_fut = async {
        let mut head = Vec::with_capacity(2048);
        let mut buf = [0u8; 4096];

        let header_end = loop {
            let n = client
                .read(&mut buf)
                .await
                .context("Failed reading from client socket")?;
            if n == 0 {
                return Ok::<_, anyhow::Error>(None);
            }
            head.extend_from_slice(&buf[..n]);
            if let Some(idx) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                break idx + 4;
            }
            if head.len() > MAX_HEAD_SIZE {
                let resp = "HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
                let _ = client.write_all(resp.as_bytes()).await;
                return Ok::<_, anyhow::Error>(None);
            }
        };

        Ok::<_, anyhow::Error>(Some((head, header_end)))
    };

    // Protect against Slowloris DoS attacks by capping header read time to 30 seconds
    let read_result =
        tokio::time::timeout(std::time::Duration::from_secs(30), header_read_fut).await;
    let (head, header_end) = match read_result {
        Ok(Ok(Some((h, end)))) => (h, end),
        _ => return Ok(()),
    };

    let text = String::from_utf8_lossy(&head[..header_end]);
    let host = text
        .lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_default();

    let target = resolve_domain(&data_dir, &host);

    match target {
        Some(mapping) => {
            let dest_addr = format!("127.0.0.1:{}", mapping.target_host_port);
            match TcpStream::connect(&dest_addr).await {
                Ok(mut upstream) => {
                    // Send the already buffered request head
                    upstream
                        .write_all(&head)
                        .await
                        .context("Failed writing head to upstream microVM")?;

                    // Bi-directional TCP stream copy
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                }
                Err(err) => {
                    warn!("Failed connecting to upstream {}: {}", dest_addr, err);
                    let body = format!(
                        "<!doctype html><title>502 Bad Gateway</title><h1>Bad Gateway</h1>\
                         <p>Could not connect to microVM target on <code>{}</code>: {}</p>",
                        dest_addr,
                        html_escape(&err.to_string())
                    );
                    let resp = format!(
                        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = client.write_all(resp.as_bytes()).await;
                }
            }
        }
        None => {
            let active = list_active_domains(&data_dir);
            let body = generate_404_html(&host, &active, bind_port);
            let resp = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = client.write_all(resp.as_bytes()).await;
        }
    }

    Ok(())
}

/// Runs the local domain ingress server on the configured port.
pub async fn run_domain_router(
    config: DomainRouterConfig,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    let port = config.bind_port;
    let bind_addr = config
        .bind_addr
        .clone()
        .unwrap_or_else(|| "0.0.0.0".to_string());
    let listen_target = format!("{}:{}", bind_addr, port);

    let listener = TcpListener::bind(&listen_target)
        .await
        .with_context(|| format!("Failed to bind domain router on {}", listen_target))?;

    info!("cro domain ingress router listening on {}", listen_target);

    let dns_handle = if config.enable_dns {
        match super::dns::MicroDnsServer::start("127.0.0.1", config.dns_port).await {
            Ok(h) => {
                info!(
                    "cro micro-DNS server listening on 127.0.0.1:{} (*.cro.local -> 127.0.0.1)",
                    config.dns_port
                );
                Some(h)
            }
            Err(e) => {
                warn!(
                    "Could not start embedded Micro-DNS server on port {}: {}",
                    config.dns_port, e
                );
                None
            }
        }
    } else {
        None
    };

    let data_dir = Arc::new(config.data_dir);
    let conn_semaphore = Arc::new(tokio::sync::Semaphore::new(512));

    loop {
        tokio::select! {
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((socket, _peer)) => {
                        let permit = match conn_semaphore.clone().try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                warn!("Domain router connection limit reached (512 concurrent connections)");
                                continue;
                            }
                        };
                        let data_dir = Arc::clone(&data_dir);
                        tokio::spawn(async move {
                            let _permit = permit;
                            if let Err(e) = handle_connection(socket, (*data_dir).clone(), port).await {
                                debug!("Connection handler terminated: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Error accepting TCP connection: {}", e);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Domain router shutdown signal received.");
                if let Some(ref h) = dns_handle {
                    h.shutdown();
                }
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PortForward;
    use tempfile::tempdir;

    #[test]
    fn test_sanitize_domain_label() {
        assert_eq!(sanitize_domain_label("nginx:alpine"), "nginx");
        assert_eq!(
            sanitize_domain_label("ghcr.io/org/llama-3.2-1b:latest"),
            "llama-3-2-1b"
        );
        assert_eq!(sanitize_domain_label("my_service_name"), "my-service-name");
    }

    #[test]
    fn test_extract_routing_keys() {
        let keys = extract_routing_keys("llama.cro.local:8080");
        assert_eq!(keys, vec!["llama.cro.local", "llama"]);

        let keys_sub = extract_routing_keys("web.myproject.cro.local");
        assert_eq!(keys_sub, vec!["web.myproject.cro.local", "web.myproject"]);

        let keys_localhost = extract_routing_keys("test.localhost");
        assert_eq!(keys_localhost, vec!["test.localhost", "test"]);
    }

    #[test]
    fn test_list_and_resolve_domain() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path();

        let state_mgr = StateManager::new(data_dir);
        let vm = crate::state::VmState {
            id: "vm-a1b2c3d4e5f6".to_string(),
            pid: std::process::id(),
            image: "nginx:alpine".to_string(),
            created_at: 1000,
            port_forwards: vec![PortForward::new(8080, 80)],
            instance_dir: data_dir.join("instances/vm-a1b2c3d4e5f6"),
            status: VmStatus::Running,
            vcpus: Some(2),
            memory_mib: Some(512),
        };
        state_mgr.save_state(&vm).unwrap();

        let mappings = list_active_domains(data_dir);
        assert!(!mappings.is_empty());
        assert!(mappings
            .iter()
            .any(|m| m.domain == "vm-a1b2c3d4e5.cro.local" || m.domain.starts_with("vm-a1b2c3")));
        assert!(mappings.iter().any(|m| m.domain == "nginx.cro.local"));

        let resolved = resolve_domain(data_dir, "nginx.cro.local:8080");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().target_host_port, 8080);

        let resolved_prefix = resolve_domain(data_dir, "nginx");
        assert!(resolved_prefix.is_some());

        let not_found = resolve_domain(data_dir, "unknown.cro.local");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_generate_404_html() {
        let mappings = vec![DomainMapping {
            domain: "app.cro.local".to_string(),
            target_host_port: 3000,
            instance_id: "vm-1234".to_string(),
            image_or_service: "node-app".to_string(),
            status: "running".to_string(),
        }];

        let html = generate_404_html("missing.cro.local", &mappings, 8080);
        assert!(html.contains("cro: Domain Not Found"));
        assert!(html.contains("missing.cro.local"));
        assert!(html.contains("app.cro.local"));
        assert!(html.contains("127.0.0.1:3000"));
    }
}
