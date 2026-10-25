// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Multi-microVM deployment orchestrator and lifecycle engine.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::parse_size_to_bytes;
use crate::vm::MicroVmBuilder;

use super::spec::{ComposeSpec, Manifest};
use super::state::ComposeProjectState;

/// Orchestrates multi-microVM deployments from a `ComposeSpec`.
pub struct ComposeProject {
    pub name: String,
    pub spec: ComposeSpec,
    pub base_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl ComposeProject {
    pub fn new(
        name: impl Into<String>,
        spec: ComposeSpec,
        base_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            spec,
            base_dir: base_dir.into(),
            data_dir: data_dir.into(),
        }
    }

    /// Auto-discovers compose file (`krun-compose.yaml`, `compose.yaml`, `docker-compose.yaml`)
    /// in the current or specified directory.
    pub fn discover_compose_file(dir: &Path) -> Result<PathBuf> {
        let candidates = [
            "krun-compose.yaml",
            "krun-compose.yml",
            "microvm-compose.yaml",
            "microvm-compose.yml",
            "compose.yaml",
            "compose.yml",
            "docker-compose.yaml",
            "docker-compose.yml",
            "microvm.yaml",
            "microvm.yml",
        ];

        for c in &candidates {
            let p = dir.join(c);
            if p.exists() {
                return Ok(p);
            }
        }

        bail!(
            "No compose manifest found in '{}'. (Looked for: krun-compose.yaml, compose.yaml, microvm.yaml)",
            dir.display()
        );
    }

    /// Loads project from a file path. Defaults project name to directory name if not specified in spec.
    pub fn load(path: &Path, data_dir: Option<PathBuf>) -> Result<Self> {
        let manifest = Manifest::load_file(path)?;
        let spec = manifest.into_compose_spec();

        let base_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .unwrap_or_else(|_| {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            });

        let project_name = spec.name.clone().unwrap_or_else(|| {
            base_dir
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "default".to_string())
        });

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let final_data_dir =
            data_dir.unwrap_or_else(|| PathBuf::from(home).join(".cache/krun-microvm"));

        Ok(Self::new(project_name, spec, base_dir, final_data_dir))
    }

    /// Resolves the topological launch order of services based on `depends_on`.
    /// Detects circular dependencies and returns ordered service names.
    pub fn resolve_launch_order(&self) -> Result<Vec<String>> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

        for name in self.spec.services.keys() {
            in_degree.insert(name.as_str(), 0);
            adj.insert(name.as_str(), Vec::new());
        }

        for (name, svc) in &self.spec.services {
            for dep in &svc.depends_on {
                if !self.spec.services.contains_key(dep) {
                    bail!("Service '{}' depends on undefined service '{}'", name, dep);
                }
                adj.get_mut(dep.as_str()).unwrap().push(name.as_str());
                *in_degree.get_mut(name.as_str()).unwrap() += 1;
            }
        }

        // Kahn's algorithm
        let mut queue: Vec<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&name, _)| name)
            .collect();
        queue.sort(); // Stable ordering

        let mut order = Vec::new();
        while let Some(u) = queue.pop() {
            order.push(u.to_string());
            if let Some(neighbors) = adj.get(u) {
                for &v in neighbors {
                    let deg = in_degree.get_mut(v).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(v);
                    }
                }
            }
        }

        if order.len() != self.spec.services.len() {
            bail!("Circular dependency detected in compose services depends_on graph");
        }

        Ok(order)
    }

    /// Returns the virtual network IP assigned to a service.
    ///
    /// If any service in the compose spec uses isolated virtual networking (e.g. `gvproxy`),
    /// assigns sequential IP addresses on the `192.168.127.0/24` subnet (e.g. `192.168.127.2`, `192.168.127.3`, ...).
    /// Under TSI / host networking, returns `127.0.0.1`.
    pub fn service_ip(&self, service_name: &str) -> String {
        let any_gvproxy = self.spec.services.values().any(|s| {
            s.network_mode
                .as_deref()
                .map(|m| m.eq_ignore_ascii_case("gvproxy"))
                .unwrap_or(false)
        });

        if any_gvproxy {
            let idx = self
                .spec
                .services
                .keys()
                .position(|k| k == service_name)
                .unwrap_or(0);
            format!("192.168.127.{}", 2 + idx)
        } else {
            "127.0.0.1".to_string()
        }
    }

    /// Builds a `MicroVmBuilder` configured for a specific service in the project.
    pub fn build_service_vm(&self, service_name: &str) -> Result<MicroVmBuilder> {
        let svc = self
            .spec
            .services
            .get(service_name)
            .with_context(|| format!("Service '{service_name}' not found in compose spec"))?;

        if svc.image.is_empty() {
            bail!("Service '{service_name}' has no image defined");
        }

        let mut builder = MicroVmBuilder::new(&svc.image)
            .data_dir(self.data_dir.clone())
            .detach(true) // Compose services run detached
            .sandbox(svc.sandbox.unwrap_or(true));

        if let Some(c) = svc.cpus {
            builder = builder.cpus(c);
        } else {
            builder = builder.cpus(2);
        }

        if let Some(ref mem_str) = svc.memory {
            let bytes = parse_size_to_bytes(mem_str).with_context(|| {
                format!("Invalid memory value '{mem_str}' for service '{service_name}'")
            })?;
            builder = builder.memory_mb((bytes / (1024 * 1024)) as u32);
        } else {
            builder = builder.memory_mb(512);
        }

        if let Some(ref cmd) = svc.cmd {
            builder = builder.cmd(cmd.clone());
        }

        if let Some(ref workdir) = svc.working_dir {
            builder = builder.workdir(workdir);
        }

        // Environment variables
        for (k, v) in &svc.env {
            builder = builder.env(k, v);
        }

        // Project and service tracking metadata env
        builder = builder.env("KRUN_COMPOSE_PROJECT", &self.name);
        builder = builder.env("KRUN_COMPOSE_SERVICE", service_name);

        // Port forwards
        for p in &svc.ports {
            let (host, guest) = parse_port_forward(p)?;
            builder = builder.port_forward(host, guest);
        }

        // Volumes
        for v in &svc.volumes {
            let (host_path, tag, ro) = self.parse_volume_mount(v)?;
            builder = builder.virtiofs(&tag, host_path, ro);
        }

        // GPU
        if svc.gpu.unwrap_or(false) {
            builder = builder.gpu(true);
            if let Some(ref shm) = svc.gpu_shm_size {
                let bytes = parse_size_to_bytes(shm)?;
                builder = builder.gpu_shm_size(bytes);
            }
        }

        // DAX
        if let Some(ref dax_str) = svc.dax {
            builder = builder.dax_window_size_str(dax_str)?;
        }

        // Shared memory (/dev/shm)
        if let Some(ref shm) = svc.shm_size {
            builder = builder.shm_size(shm);
        }

        // Secrets
        for s in &svc.secrets {
            let (k, v) = if let Some((k, v)) = s.split_once('=') {
                (k.to_string(), v.to_string())
            } else if let Ok(val) = std::env::var(s) {
                (s.clone(), val)
            } else {
                (s.clone(), String::new())
            };
            builder = builder.secret(k, v);
        }

        // Allow hosts
        for h in &svc.allow_hosts {
            builder = builder.allow_host(h);
        }

        // Network mode
        if let Some(ref net) = svc.network_mode {
            match net.to_lowercase().as_str() {
                "none" => builder = builder.network_mode(crate::net::NetworkMode::None),
                "gvproxy" => builder = builder.network_mode(crate::net::NetworkMode::Gvproxy),
                "tsi" => builder = builder.network_mode(crate::net::NetworkMode::Tsi),
                _ => {}
            }
        }

        // Inject peer service names into /etc/hosts for seamless inter-service discovery
        let mut extra_hosts = Vec::new();
        for peer_name in self.spec.services.keys() {
            if peer_name != service_name {
                let peer_ip = self.service_ip(peer_name);
                extra_hosts.push((peer_name.clone(), peer_ip));
            }
        }
        builder = builder.extra_hosts(extra_hosts);

        Ok(builder)
    }

    fn parse_volume_mount(&self, vol_str: &str) -> Result<(PathBuf, String, bool)> {
        let parts: Vec<&str> = vol_str.split(':').collect();
        if parts.is_empty() {
            bail!("Empty volume string");
        }

        let host_src = parts[0];
        let tag = if parts.len() > 1 {
            parts[1].trim_start_matches('/').replace('/', "_")
        } else {
            "vol".to_string()
        };
        let ro = parts.get(2).is_some_and(|&s| s == "ro");

        let host_path = if host_src.starts_with('.') || !host_src.starts_with('/') {
            self.base_dir.join(host_src)
        } else {
            PathBuf::from(host_src)
        };

        if !host_path.exists() {
            let _ = std::fs::create_dir_all(&host_path);
        }

        Ok((host_path, tag, ro))
    }

    /// Path to project state file.
    pub fn state_file_path(&self) -> PathBuf {
        self.data_dir
            .join("compose")
            .join(&self.name)
            .join("project_state.json")
    }

    /// Loads persisted project state from cache directory.
    pub fn load_state(&self) -> Result<Option<ComposeProjectState>> {
        let p = self.state_file_path();
        if !p.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(&p)?;
        let state: ComposeProjectState = serde_json::from_str(&data)?;
        Ok(Some(state))
    }

    /// Saves project state to cache directory.
    pub fn save_state(&self, state: &ComposeProjectState) -> Result<()> {
        let p = self.state_file_path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(state)?;
        std::fs::write(&p, json)?;
        Ok(())
    }

    /// Deletes project state file from disk.
    pub fn remove_state(&self) -> Result<()> {
        let p = self.state_file_path();
        if p.exists() {
            let _ = std::fs::remove_file(&p);
        }
        Ok(())
    }
}

fn parse_port_forward(p: &str) -> Result<(u16, u16)> {
    let parts: Vec<&str> = p.split(':').collect();
    match parts.len() {
        1 => {
            let port: u16 = parts[0].parse().context("Invalid port number")?;
            Ok((port, port))
        }
        2 => {
            let host: u16 = parts[0].parse().context("Invalid host port")?;
            let guest: u16 = parts[1].parse().context("Invalid guest port")?;
            Ok((host, guest))
        }
        _ => bail!("Invalid port forward specification: '{p}', expected host:guest or port"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::spec::ServiceSpec;
    use std::collections::BTreeMap;

    #[test]
    fn test_topological_sort_launch_order() {
        let mut services = BTreeMap::new();

        let frontend = ServiceSpec {
            image: "frontend:latest".to_string(),
            depends_on: vec!["api".to_string()],
            ..Default::default()
        };

        let api = ServiceSpec {
            image: "api:latest".to_string(),
            depends_on: vec!["db".to_string()],
            ..Default::default()
        };

        let db = ServiceSpec {
            image: "db:latest".to_string(),
            ..Default::default()
        };

        services.insert("frontend".to_string(), frontend);
        services.insert("api".to_string(), api);
        services.insert("db".to_string(), db);

        let spec = ComposeSpec {
            version: "krun/v1".to_string(),
            name: Some("test-project".to_string()),
            services,
            volumes: BTreeMap::new(),
            networks: BTreeMap::new(),
        };

        let project = ComposeProject::new("test-project", spec, "/tmp", "/tmp");
        let order = project
            .resolve_launch_order()
            .expect("Dependency sort failed");

        assert_eq!(order, vec!["db", "api", "frontend"]);
    }

    #[test]
    fn test_circular_dependency_detected() {
        let mut services = BTreeMap::new();

        let a = ServiceSpec {
            image: "a:latest".to_string(),
            depends_on: vec!["b".to_string()],
            ..Default::default()
        };

        let b = ServiceSpec {
            image: "b:latest".to_string(),
            depends_on: vec!["a".to_string()],
            ..Default::default()
        };

        services.insert("a".to_string(), a);
        services.insert("b".to_string(), b);

        let spec = ComposeSpec {
            version: "krun/v1".to_string(),
            name: Some("cyclic".to_string()),
            services,
            volumes: BTreeMap::new(),
            networks: BTreeMap::new(),
        };

        let project = ComposeProject::new("cyclic", spec, "/tmp", "/tmp");
        assert!(project.resolve_launch_order().is_err());
    }

    #[test]
    fn test_compose_service_ip_assignment() {
        let mut services = BTreeMap::new();
        services.insert(
            "api".to_string(),
            ServiceSpec {
                image: "api:latest".to_string(),
                network_mode: Some("gvproxy".to_string()),
                ..Default::default()
            },
        );
        services.insert(
            "db".to_string(),
            ServiceSpec {
                image: "postgres:16".to_string(),
                network_mode: Some("gvproxy".to_string()),
                ..Default::default()
            },
        );

        let spec = ComposeSpec {
            version: "krun/v1".to_string(),
            name: Some("gvproxy-project".to_string()),
            services,
            volumes: BTreeMap::new(),
            networks: BTreeMap::new(),
        };

        let project = ComposeProject::new("gvproxy-project", spec, "/tmp", "/tmp");
        assert_eq!(project.service_ip("api"), "192.168.127.2");
        assert_eq!(project.service_ip("db"), "192.168.127.3");

        // TSI or default returns 127.0.0.1
        let mut tsi_services = BTreeMap::new();
        tsi_services.insert(
            "web".to_string(),
            ServiceSpec {
                image: "nginx:latest".to_string(),
                network_mode: Some("tsi".to_string()),
                ..Default::default()
            },
        );
        let tsi_spec = ComposeSpec {
            version: "krun/v1".to_string(),
            name: Some("tsi-project".to_string()),
            services: tsi_services,
            volumes: BTreeMap::new(),
            networks: BTreeMap::new(),
        };
        let tsi_proj = ComposeProject::new("tsi-project", tsi_spec, "/tmp", "/tmp");
        assert_eq!(tsi_proj.service_ip("web"), "127.0.0.1");
    }
}
