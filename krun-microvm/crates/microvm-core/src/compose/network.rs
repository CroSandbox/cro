// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Inter-MicroVM Private Overlay Network and Service Discovery for Compose.
//!
//! Provides isolated virtual subnets (`10.88.x.0/24`), deterministic IP/MAC allocation,
//! `/etc/hosts` peer injection, and 12-factor service discovery environment variables.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use super::spec::ComposeSpec;

/// Service endpoint attached to the compose private overlay network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayEndpoint {
    /// Service name in the compose manifest (e.g. `db`, `web`, `redis`).
    pub service_name: String,
    /// Assigned private IPv4 address on the overlay subnet.
    pub ip: Ipv4Addr,
    /// Virtual MAC address (e.g. `52:54:00:88:01:02`).
    pub mac_address: String,
    /// Exposed primary guest ports.
    pub ports: Vec<u16>,
    /// Domain aliases resolvable within the project.
    pub aliases: Vec<String>,
}

/// Private overlay network managing inter-microVM routing and service discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeOverlayNetwork {
    /// Project name.
    pub project_name: String,
    /// Network name (e.g. `default`, `backend`).
    pub network_name: String,
    /// Subnet CIDR (e.g. `10.88.42.0/24`).
    pub subnet: String,
    /// Gateway address (e.g. `10.88.42.1`).
    pub gateway: Ipv4Addr,
    /// Subnet index (3rd octet: `10.88.<index>.0`).
    pub subnet_octet: u8,
    /// Registered endpoints by service name.
    pub endpoints: BTreeMap<String, OverlayEndpoint>,
}

impl ComposeOverlayNetwork {
    /// Constructs a private overlay network for a `ComposeSpec`.
    ///
    /// Computes a deterministic subnet from the project name hash (e.g. `10.88.<hash % 254 + 1>.0/24`)
    /// unless explicitly overridden in the compose network specification.
    pub fn from_project(project_name: &str, spec: &ComposeSpec) -> Self {
        // Deterministic subnet hash
        let hash: u32 = project_name.bytes().fold(2166136261u32, |acc, b| {
            (acc ^ (b as u32)).wrapping_mul(16777619)
        });
        let subnet_octet = ((hash % 250) + 1) as u8;

        let gateway = Ipv4Addr::new(10, 88, subnet_octet, 1);
        let subnet_cidr = format!("10.88.{}.0/24", subnet_octet);

        let mut network = Self {
            project_name: project_name.to_string(),
            network_name: "default".to_string(),
            subnet: subnet_cidr,
            gateway,
            subnet_octet,
            endpoints: BTreeMap::new(),
        };

        // Check if any service specifies gvproxy network mode
        let any_gvproxy = spec.services.values().any(|s| {
            s.network_mode
                .as_deref()
                .map(|m| m.eq_ignore_ascii_case("gvproxy"))
                .unwrap_or(false)
        });

        for (idx, (svc_name, svc)) in spec.services.iter().enumerate() {
            let host_num = (2 + idx) as u8;

            let ip = if any_gvproxy {
                Ipv4Addr::new(192, 168, 127, host_num)
            } else {
                Ipv4Addr::new(10, 88, subnet_octet, host_num)
            };

            let mac = format!("52:54:00:88:{:02x}:{:02x}", subnet_octet, host_num);

            // Extract primary ports from ports list e.g. "8080:80" -> 80
            let mut ports = Vec::new();
            for p_str in &svc.ports {
                let parts: Vec<&str> = p_str.split(':').collect();
                let guest_port = match parts.len() {
                    1 => parts[0].parse::<u16>().ok(),
                    2 => parts[1].parse::<u16>().ok(),
                    3 => parts[2].parse::<u16>().ok(),
                    _ => None,
                };
                if let Some(gp) = guest_port {
                    if !ports.contains(&gp) {
                        ports.push(gp);
                    }
                }
            }

            let aliases = vec![
                svc_name.clone(),
                format!("{}.{}", svc_name, project_name),
                format!("{}.{}.cro.local", svc_name, project_name),
                format!("{}.cro.local", svc_name),
                format!("{}.local", svc_name),
            ];

            network.endpoints.insert(
                svc_name.clone(),
                OverlayEndpoint {
                    service_name: svc_name.clone(),
                    ip,
                    mac_address: mac,
                    ports,
                    aliases,
                },
            );
        }

        network
    }

    /// Resolves a domain or service query to an assigned overlay IP.
    pub fn resolve(&self, query: &str) -> Option<Ipv4Addr> {
        let query_lower = query.to_ascii_lowercase();
        for ep in self.endpoints.values() {
            if ep.service_name.eq_ignore_ascii_case(&query_lower) {
                return Some(ep.ip);
            }
            for alias in &ep.aliases {
                if alias.eq_ignore_ascii_case(&query_lower) {
                    return Some(ep.ip);
                }
            }
        }
        None
    }

    /// Generates `/etc/hosts` mapping pairs to inject into a given service microVM.
    /// Excludes the target service's own identity to avoid self-shadowing.
    pub fn hosts_entries_for(&self, current_service: &str) -> Vec<(String, String)> {
        let mut entries = Vec::new();

        for (name, ep) in &self.endpoints {
            if name != current_service {
                let ip_str = ep.ip.to_string();
                // 1. Direct service name
                entries.push((name.clone(), ip_str.clone()));
                // 2. Project-qualified service name
                entries.push((format!("{}.{}", name, self.project_name), ip_str.clone()));
                // 3. Project-qualified cro.local domain
                entries.push((
                    format!("{}.{}.cro.local", name, self.project_name),
                    ip_str.clone(),
                ));
                // 4. Clean local alias
                entries.push((format!("{}.local", name), ip_str));
            }
        }

        entries
    }

    /// Generates 12-factor service discovery environment variables for all services.
    ///
    /// Exposes:
    /// - `<NAME>_HOST=<ip>`
    /// - `<NAME>_PORT=<port>`
    /// - `<NAME>_URL=http://<ip>:<port>`
    pub fn service_discovery_env(&self) -> Vec<(String, String)> {
        let mut envs = Vec::new();

        for (name, ep) in &self.endpoints {
            let normalized = name.to_ascii_uppercase().replace('-', "_");
            let ip_str = ep.ip.to_string();

            envs.push((format!("{}_HOST", normalized), ip_str.clone()));

            if let Some(&first_port) = ep.ports.first() {
                envs.push((format!("{}_PORT", normalized), first_port.to_string()));
                envs.push((
                    format!("{}_URL", normalized),
                    format!("http://{}:{}", ip_str, first_port),
                ));
            }
        }

        envs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::spec::ServiceSpec;

    #[test]
    fn test_compose_overlay_network_allocation() {
        let mut services = BTreeMap::new();
        services.insert(
            "frontend".to_string(),
            ServiceSpec {
                image: "frontend:latest".to_string(),
                ports: vec!["3000:80".to_string()],
                ..Default::default()
            },
        );
        services.insert(
            "backend".to_string(),
            ServiceSpec {
                image: "backend:latest".to_string(),
                ports: vec!["8080:8080".to_string()],
                ..Default::default()
            },
        );
        services.insert(
            "postgres".to_string(),
            ServiceSpec {
                image: "postgres:16".to_string(),
                ports: vec!["5432:5432".to_string()],
                ..Default::default()
            },
        );

        let spec = ComposeSpec {
            version: "krun/v1".to_string(),
            name: Some("myapp".to_string()),
            services,
            volumes: BTreeMap::new(),
            networks: BTreeMap::new(),
        };

        let overlay = ComposeOverlayNetwork::from_project("myapp", &spec);
        assert_eq!(overlay.endpoints.len(), 3);

        // Endpoints exist
        let fe = overlay.endpoints.get("frontend").unwrap();
        let be = overlay.endpoints.get("backend").unwrap();
        let db = overlay.endpoints.get("postgres").unwrap();

        assert_eq!(fe.ports, vec![80]);
        assert_eq!(be.ports, vec![8080]);
        assert_eq!(db.ports, vec![5432]);

        // IPs are on overlay subnet (10.88.x.y) and unique
        assert_eq!(fe.ip.octets()[0..2], [10, 88]);
        assert_ne!(fe.ip, be.ip);
        assert_ne!(be.ip, db.ip);

        // Resolution by service name and FQDNs
        assert_eq!(overlay.resolve("backend"), Some(be.ip));
        assert_eq!(overlay.resolve("backend.myapp"), Some(be.ip));
        assert_eq!(overlay.resolve("backend.myapp.cro.local"), Some(be.ip));
        assert_eq!(overlay.resolve("postgres.cro.local"), Some(db.ip));
        assert_eq!(overlay.resolve("unknown"), None);

        // Hosts entries for frontend includes backend and postgres, not frontend itself
        let hosts = overlay.hosts_entries_for("frontend");
        assert!(hosts.iter().any(|(h, _)| h == "backend"));
        assert!(hosts.iter().any(|(h, _)| h == "backend.myapp.cro.local"));
        assert!(hosts.iter().any(|(h, _)| h == "postgres"));
        assert!(!hosts.iter().any(|(h, _)| h == "frontend"));

        // 12-factor env vars
        let envs = overlay.service_discovery_env();
        assert!(envs.iter().any(|(k, v)| k == "BACKEND_PORT" && v == "8080"));
        assert!(envs
            .iter()
            .any(|(k, v)| k == "POSTGRES_PORT" && v == "5432"));
        assert!(envs.iter().any(|(k, _)| k == "POSTGRES_HOST"));
    }
}
