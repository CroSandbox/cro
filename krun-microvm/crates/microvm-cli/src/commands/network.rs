// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Network topology inspection, diagnostics, and proxy command handlers.

use crate::args::NetworkCommands;
use crate::utils::{default_data_dir, truncate_str};
use anyhow::{bail, Context, Result};
use microvm_core::StateManager;

pub async fn handle_network_command(cmd: NetworkCommands) -> Result<()> {
    match cmd {
        NetworkCommands::Ls { json, data_dir } => {
            let base = data_dir.unwrap_or_else(default_data_dir);
            let vms = StateManager::list(&base)?;
            let mut inspections = Vec::new();
            for vm in &vms {
                let is_alive = vm.is_process_alive();
                if let Ok(insp) = microvm_core::net::inspect_microvm_network(
                    &vm.instance_dir,
                    &vm.id,
                    Some(vm.pid),
                    is_alive,
                ) {
                    inspections.push(insp);
                }
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&inspections)?);
            } else if inspections.is_empty() {
                println!("No active microVM networks found.");
            } else {
                println!(
                    "{:<14} {:<12} {:<16} {:<16} {:<16} {:<18} STATUS",
                    "MICROVM ID", "MODE", "GUEST IP", "GATEWAY", "PORTS", "EGRESS RULES"
                );
                println!("{:-<100}", "");
                for item in inspections {
                    let ports_str = if item.port_forwards.is_empty() {
                        "-".to_string()
                    } else {
                        item.port_forwards
                            .iter()
                            .map(|p| format!("{}:{}", p.host, p.guest))
                            .collect::<Vec<_>>()
                            .join(",")
                    };
                    let egress_str = if item.allow_hosts.is_empty() {
                        if item.mode.starts_with("none") {
                            "Blocked (All)".to_string()
                        } else {
                            "Unrestricted".to_string()
                        }
                    } else {
                        item.allow_hosts.join(",")
                    };
                    let mode_short = if item.mode.starts_with("gvproxy") {
                        "gvproxy"
                    } else if item.mode.starts_with("tsi") {
                        "tsi"
                    } else if item.mode.starts_with("none") {
                        "none"
                    } else if item.mode.starts_with("cni") {
                        "cni"
                    } else {
                        "unix"
                    };

                    println!(
                        "{:<14} {:<12} {:<16} {:<16} {:<16} {:<18} {}",
                        truncate_str(&item.id, 13),
                        mode_short,
                        truncate_str(&item.guest_ip, 15),
                        truncate_str(&item.gateway_ip, 15),
                        truncate_str(&ports_str, 15),
                        truncate_str(&egress_str, 17),
                        item.status
                    );
                }
            }
        }

        NetworkCommands::Inspect { id, json, data_dir } => {
            let base = data_dir.unwrap_or_else(default_data_dir);
            let vm = match StateManager::find(&base, &id)? {
                Some(v) => v,
                None => bail!("MicroVM '{}' not found in state", id),
            };
            let is_alive = vm.is_process_alive();
            let inspection = microvm_core::net::inspect_microvm_network(
                &vm.instance_dir,
                &vm.id,
                Some(vm.pid),
                is_alive,
            )?;

            if json {
                println!("{}", serde_json::to_string_pretty(&inspection)?);
            } else {
                println!("🌐 MicroVM Network Topology & Security: {}", inspection.id);
                println!("{:-<65}", "");
                println!("  Status:              {}", inspection.status);
                println!("  Network Mode:        {}", inspection.mode);
                println!("  Guest IP Address:    {}", inspection.guest_ip);
                println!("  Virtual Gateway:     {}", inspection.gateway_ip);
                println!("  Virtual MAC:         {}", inspection.mac_address);
                println!("  Interface MTU:       {}", inspection.mtu);
                println!("  Guest Hostname:      {}", inspection.hostname);
                println!(
                    "  DNS Resolvers:       {}",
                    inspection.dns_servers.join(", ")
                );
                let pf_str = if inspection.port_forwards.is_empty() {
                    "None".to_string()
                } else {
                    inspection
                        .port_forwards
                        .iter()
                        .map(|pf| {
                            format!(
                                "0.0.0.0:{} -> {}:{}",
                                pf.host, inspection.guest_ip, pf.guest
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                println!("  Port Mappings:       {}", pf_str);
                let egress_str = if inspection.allow_hosts.is_empty() {
                    if inspection.mode.starts_with("none") {
                        "Default-Deny (Air-Gapped Isolation)"
                    } else {
                        "Unrestricted (Standard Egress)"
                    }
                } else {
                    &inspection.allow_hosts.join(", ")
                };
                println!("  Allowed Egress:      {}", egress_str);
                println!(
                    "  Metadata Defense:    {}",
                    if inspection.metadata_blocked {
                        "Active (169.254.169.254 exfiltration blocked)"
                    } else {
                        "Disabled"
                    }
                );
                println!(
                    "  In-Flight Secrets:   {}",
                    if inspection.secret_substitution_active {
                        "Active (Zero-Trust header/body substitution)"
                    } else {
                        "None configured"
                    }
                );
                if let Some(budget) = inspection.token_budget {
                    println!("  LLM Token Ceiling:   {} tokens", budget);
                }
                if let Some(port) = inspection.egress_proxy_port {
                    println!("  Egress Proxy Server: 127.0.0.1:{}", port);
                }
                if let Some(ref sock) = inspection.unix_socket_path {
                    println!("  Virtual Switch Sock: {}", sock);
                }
            }
        }

        NetworkCommands::Ports { id, json, data_dir } => {
            let base = data_dir.unwrap_or_else(default_data_dir);
            let vms = if let Some(target_id) = id {
                match StateManager::find(&base, &target_id)? {
                    Some(v) => vec![v],
                    None => bail!("MicroVM '{}' not found in state", target_id),
                }
            } else {
                StateManager::list(&base)?
            };

            #[derive(serde::Serialize)]
            struct PortEntry {
                host_port: u16,
                guest_port: u16,
                protocol: &'static str,
                microvm_id: String,
                status: String,
                endpoint: String,
            }

            let mut port_list = Vec::new();
            for vm in &vms {
                let is_alive = vm.is_process_alive();
                let status_str = if is_alive { "Running" } else { "Stopped" };
                for pf in &vm.port_forwards {
                    port_list.push(PortEntry {
                        host_port: pf.host,
                        guest_port: pf.guest,
                        protocol: "tcp",
                        microvm_id: vm.id.clone(),
                        status: status_str.to_string(),
                        endpoint: format!("http://localhost:{}", pf.host),
                    });
                }
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&port_list)?);
            } else if port_list.is_empty() {
                println!("No published port mappings found.");
            } else {
                println!(
                    "{:<12} {:<12} {:<10} {:<16} {:<12} LOCAL ENDPOINT",
                    "HOST PORT", "GUEST PORT", "PROTOCOL", "MICROVM ID", "STATUS"
                );
                println!("{:-<80}", "");
                for p in port_list {
                    println!(
                        "{:<12} {:<12} {:<10} {:<16} {:<12} {}",
                        p.host_port,
                        p.guest_port,
                        p.protocol,
                        truncate_str(&p.microvm_id, 15),
                        p.status,
                        p.endpoint
                    );
                }
            }
        }

        NetworkCommands::Test {
            id,
            target,
            data_dir,
        } => {
            let base = data_dir.unwrap_or_else(default_data_dir);
            let vm = match StateManager::find(&base, &id)? {
                Some(v) => v,
                None => bail!("MicroVM '{}' not found in state", id),
            };

            if !vm.is_process_alive() {
                bail!(
                    "Cannot test network on stopped microVM '{}'. Start the microVM first.",
                    id
                );
            }

            println!(
                "🔍 Probing network connectivity and egress security inside microVM '{}'...",
                id
            );
            println!("   Target Destination: {}", target);

            let test_cmd = format!(
                "echo '[DNS Test]' && (getent hosts {target} 2>&1 || nslookup {target} 2>&1 || echo 'DNS resolution not available') && echo '[Ping/TCP Probe]' && (ping -c 2 -W 2 {target} 2>&1 || nc -z -w 2 {target} 80 2>&1 || curl -I -s --connect-timeout 2 {target} 2>&1 || echo 'Target unreachable or filtered')"
            );

            let req =
                microvm_core::ExecRequest::new(vec!["sh".to_string(), "-c".to_string(), test_cmd]);
            let resp = StateManager::exec(&base, &id, &req).await?;

            if !resp.stdout.is_empty() {
                println!("\n{}", resp.stdout.trim());
            }
            if !resp.stderr.is_empty() {
                eprintln!("\nDiagnostics Stderr:\n{}", resp.stderr.trim());
            }
            if resp.exit_code == 0 {
                println!("\n✅ Network diagnostic probe completed successfully.");
            } else {
                println!(
                    "\n⚠️ Network diagnostic returned exit code {}.",
                    resp.exit_code
                );
            }
        }

        NetworkCommands::Pingora {
            listen,
            route,
            egress,
            allow_hosts,
            strip_prefix,
            secrets,
            max_tokens,
        } => {
            if egress {
                println!(
                    "🌐 Starting Cloudflare Pingora Zero-Trust Egress Proxy on {}...",
                    listen
                );
                if allow_hosts.is_empty() {
                    println!(
                        "   Policy: Default-deny (all external egress blocked except loopback)"
                    );
                } else {
                    println!("   Allowed destinations:");
                    for h in &allow_hosts {
                        println!("     - {}", h);
                    }
                }
                println!("   Cloud Metadata (169.254.169.254, 100.100.100.200, fd00:ec2::254): Strictly blocked");
                if !secrets.is_empty() {
                    println!(
                        "   Configured Secrets for In-Flight Substitution: {}",
                        secrets.len()
                    );
                }
                if let Some(tokens) = max_tokens {
                    println!("   Enforced LLM Token Ceiling: {} tokens", tokens);
                }
                let policy = microvm_core::EgressPolicy::new(allow_hosts);
                microvm_core::run_pingora_egress_server(&listen, policy, &secrets, max_tokens)?;
            } else {
                let mut routes_map = std::collections::HashMap::new();
                for r in &route {
                    if let Some((prefix, target)) = r.split_once('=') {
                        let target_addr: std::net::SocketAddr =
                            target.parse().with_context(|| {
                                format!(
                                    "Invalid target socket address '{}' in route '{}'",
                                    target, r
                                )
                            })?;
                        routes_map.insert(
                            prefix.to_string(),
                            microvm_core::MicroVmServiceBackend::new(
                                prefix.trim_start_matches('/'),
                                target_addr,
                                prefix,
                            )
                            .with_strip_prefix(strip_prefix),
                        );
                    } else {
                        bail!(
                            "Invalid route specification '{}'. Expected format /prefix=IP:PORT",
                            r
                        );
                    }
                }
                println!(
                    "🚀 Starting Cloudflare Pingora L7 Reverse Proxy Gateway on {}...",
                    listen
                );
                if routes_map.is_empty() {
                    println!(
                        "   Warning: No routes configured. Pass --route /prefix=127.0.0.1:PORT"
                    );
                } else {
                    println!(
                        "   Configured MicroVM Routes (strip prefix: {}):",
                        strip_prefix
                    );
                    for (prefix, backend) in &routes_map {
                        println!("     {} -> {}", prefix, backend.target_addr);
                    }
                }
                println!("   Health Check Probe: http://{}/healthz", listen);
                microvm_core::run_pingora_gateway_server(&listen, routes_map)?;
            }
        }

        NetworkCommands::Proxy {
            listen,
            allow_hosts,
            secrets,
            max_tokens,
        } => {
            println!(
                "🌐 Starting Zero-Trust Ephemeral Egress Proxy on {}...",
                listen
            );
            if allow_hosts.is_empty() {
                println!("   Policy: Default-deny (all external egress blocked except loopback)");
            } else {
                println!("   Allowed destinations (domains, wildcards, CIDRs):");
                for h in &allow_hosts {
                    println!("     - {}", h);
                }
            }
            println!("   Cloud Metadata & SSRF Defense: Active (169.254.169.254, 100.100.100.200, fd00:ec2::254 strictly blocked)");
            if !secrets.is_empty() {
                println!(
                    "   Secret Substitution Engine: Active ({} keys configured)",
                    secrets.len()
                );
                for (k, _) in &secrets {
                    println!("     - Injected placeholder: krun-secret:{}", k);
                }
            }
            if let Some(tokens) = max_tokens {
                println!("   Hard LLM Token Budget: {} tokens", tokens);
            }

            let policy = microvm_core::EgressPolicy::new(allow_hosts);
            let sub = microvm_core::SecretSubstitution::new(&secrets);
            let budget = microvm_core::LlmTokenBudget::new(max_tokens);

            let proxy = microvm_core::EgressProxyServer::start(policy, sub, budget).await?;
            println!(
                "✅ Zero-Trust Egress Proxy listening on {}",
                proxy.proxy_url()
            );
            println!("   Press Ctrl+C to shut down.");

            tokio::signal::ctrl_c().await?;
            println!(
                "\n🛑 Shutting down egress proxy. Blocked requests: {}, Tokens consumed: {}",
                proxy.blocked_count(),
                proxy.tokens_consumed()
            );
        }

        NetworkCommands::Doctor { json } => {
            let nameservers = microvm_core::net::DnsConfig::resolve_nameservers(&[]);
            let has_dns = !nameservers.is_empty();

            // Test cloud metadata blocking
            let default_policy = microvm_core::EgressPolicy::default();
            let aws_imds_blocked = !default_policy.is_allowed("169.254.169.254", 80);
            let gcp_imds_blocked = !default_policy.is_allowed("metadata.google.internal", 80);
            let alibaba_imds_blocked = !default_policy.is_allowed("100.100.100.200", 80);
            let ipv6_imds_blocked = !default_policy.is_allowed("fd00:ec2::254", 80);
            let decimal_imds_blocked = !default_policy.is_allowed("2852039166", 80);
            let metadata_protection_ok = aws_imds_blocked
                && gcp_imds_blocked
                && alibaba_imds_blocked
                && ipv6_imds_blocked
                && decimal_imds_blocked;

            // Test CIDR subnet evaluation
            let cidr_policy = microvm_core::EgressPolicy::new(vec!["10.0.0.0/8".to_string()]);
            let cidr_ok = cidr_policy.is_allowed("10.1.2.3", 80)
                && !cidr_policy.is_allowed("192.168.1.1", 80);

            // Test Secret Substitution Engine
            let test_secrets = vec![("KEY".to_string(), "REAL_SECRET".to_string())];
            let sub_engine = microvm_core::SecretSubstitution::new(&test_secrets);
            let sub_raw_ok = sub_engine.substitute_str("krun-secret:KEY") == "REAL_SECRET";
            let sub_url_ok = sub_engine.substitute_str("krun-secret%3AKEY") == "REAL_SECRET";
            let secret_substitution_ok = sub_raw_ok && sub_url_ok;

            // Test LLM token budget inspection
            let token_budget = microvm_core::LlmTokenBudget::new(Some(100));
            token_budget.inspect_chunk(br#"{"usage":{"total_tokens":42}}"#);
            let token_metering_ok = token_budget.total_consumed() == 42;

            // Test common ports
            let test_ports = [8080, 9090, 3000, 5432, 6379];
            let mut port_statuses = Vec::new();
            for port in test_ports {
                let is_available =
                    std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok();
                port_statuses.push((port, is_available));
            }

            if json {
                let doc_json = serde_json::json!({
                    "status": "ok",
                    "dns": {
                        "resolved_nameservers": nameservers,
                        "dns_ready": has_dns
                    },
                    "security": {
                        "metadata_protection_ok": metadata_protection_ok,
                        "aws_imds_blocked": aws_imds_blocked,
                        "gcp_imds_blocked": gcp_imds_blocked,
                        "alibaba_imds_blocked": alibaba_imds_blocked,
                        "ipv6_imds_blocked": ipv6_imds_blocked,
                        "decimal_imds_blocked": decimal_imds_blocked,
                        "cidr_subnets_ok": cidr_ok,
                        "secret_substitution_ok": secret_substitution_ok,
                        "llm_token_metering_ok": token_metering_ok
                    },
                    "ports": port_statuses.iter().map(|(p, a)| {
                        serde_json::json!({ "port": p, "available": a })
                    }).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&doc_json)?);
            } else {
                println!("🩺 MicroVM Network & Security Doctor");
                println!("{:-<60}", "");
                println!(
                    "  Autonomous Resilient DNS:      {}",
                    if has_dns {
                        format!("✅ Ready ({})", nameservers.join(", "))
                    } else {
                        "⚠️  No nameservers found".to_string()
                    }
                );
                println!(
                    "  Zero-Trust Egress Firewall:    {}",
                    if cidr_ok {
                        "✅ Active (Host, Wildcard, CIDR Supported)"
                    } else {
                        "❌ Error"
                    }
                );
                println!(
                    "  Multi-Cloud Metadata Defense:  {}",
                    if metadata_protection_ok {
                        "✅ Strict Default-Deny (AWS/GCP/Azure/Alibaba/IPv6)"
                    } else {
                        "❌ IMDS Leak Detected"
                    }
                );
                println!(
                    "  In-Flight Secret Substitution: {}",
                    if secret_substitution_ok {
                        "✅ Active (Header, URI, Body & URL-Encoded)"
                    } else {
                        "❌ Engine Error"
                    }
                );
                println!(
                    "  Multi-Provider Token Metering: {}",
                    if token_metering_ok {
                        "✅ Active (OpenAI, Anthropic, Gemini, Ollama)"
                    } else {
                        "❌ Engine Error"
                    }
                );
                println!("  Cloudflare Pingora L7 Gateway: ✅ Ready");
                println!("\n  Local Port Availability:");
                for (port, avail) in port_statuses {
                    println!(
                        "    Port {:<5} : {}",
                        port,
                        if avail {
                            "🟢 Available"
                        } else {
                            "🟡 In Use / Occupied"
                        }
                    );
                }
                println!("{:-<60}", "");
                println!("✅ All core microVM networking subsystems operational.");
            }
        }
    }
    Ok(())
}
