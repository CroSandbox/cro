// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Multi-service declarative specification and parsing for Docker Compose and Kubernetes manifests.

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Represents a multi-service declarative specification, compatible with Docker Compose YAML formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComposeSpec {
    #[serde(default = "default_version")]
    pub version: String,

    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub services: BTreeMap<String, ServiceSpec>,

    #[serde(default)]
    pub volumes: BTreeMap<String, ComposeVolumeSpec>,

    #[serde(default)]
    pub networks: BTreeMap<String, ComposeNetworkSpec>,
}

fn default_version() -> String {
    "krun/v1".to_string()
}

/// Specifications for an individual microVM service within a compose manifest.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ServiceSpec {
    /// OCI image reference (e.g. alpine:latest, nginx:alpine)
    #[serde(default)]
    pub image: String,

    /// Number of virtual CPUs (defaults to 2 if unspecified)
    #[serde(default)]
    pub cpus: Option<u8>,

    /// Memory string (e.g. "512M", "1G", "2GiB", defaults to "512M")
    #[serde(default)]
    pub memory: Option<String>,

    /// Command override (supports array ["sh", "-c", "echo hi"] or string "echo hi")
    #[serde(default, deserialize_with = "deserialize_optional_cmd")]
    pub cmd: Option<Vec<String>>,

    /// Entrypoint override
    #[serde(default, deserialize_with = "deserialize_optional_cmd")]
    pub entrypoint: Option<Vec<String>>,

    /// Environment variables (supports map { "KEY": "VAL" } or list ["KEY=VAL"])
    #[serde(default, deserialize_with = "deserialize_env_map")]
    pub env: BTreeMap<String, String>,

    /// Port forward mappings (e.g. ["8080:80", "9000:9000", "3000"])
    #[serde(default)]
    pub ports: Vec<String>,

    /// Volume mounts (e.g. ["./data:/data", "./workspace:/workspace:rw"])
    #[serde(default)]
    pub volumes: Vec<String>,

    /// Working directory inside guest
    #[serde(default, rename = "working_dir", alias = "workdir")]
    pub working_dir: Option<String>,

    /// Network isolation mode: "gvproxy", "tsi", "none", "cni"
    #[serde(default, rename = "network_mode")]
    pub network_mode: Option<String>,

    /// Services that must be booted prior to this service
    #[serde(default, deserialize_with = "deserialize_depends_on")]
    pub depends_on: Vec<String>,

    /// Virtual networks attached to this service
    #[serde(default, deserialize_with = "deserialize_depends_on")]
    pub networks: Vec<String>,

    /// Restart policy: "no", "always", "on-failure"
    #[serde(default)]
    pub restart: Option<String>,

    /// Hardware-accelerated virtio-gpu (Metal on Apple Silicon, DRM on Linux)
    #[serde(default)]
    pub gpu: Option<bool>,

    /// Shared memory vRAM window size for virtio-gpu (e.g. "2G", "4Gi")
    #[serde(default, rename = "gpu_shm_size")]
    pub gpu_shm_size: Option<String>,

    /// Size of /dev/shm (e.g. "64m", "2g")
    #[serde(default, rename = "shm_size")]
    pub shm_size: Option<String>,

    /// VirtioFS DAX window size (e.g. "4G")
    #[serde(default)]
    pub dax: Option<String>,

    /// Secret injection list (e.g. ["API_KEY=val", "SECRET_KEY"])
    #[serde(default)]
    pub secrets: Vec<String>,

    /// Allowed outbound egress destinations for zero-trust egress control
    #[serde(default, rename = "allow_hosts")]
    pub allow_hosts: Vec<String>,

    /// Host sandboxing (default: true)
    #[serde(default)]
    pub sandbox: Option<bool>,

    /// User metadata labels
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ComposeVolumeSpec {
    #[serde(default)]
    pub driver: Option<String>,
    #[serde(default)]
    pub external: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ComposeNetworkSpec {
    #[serde(default)]
    pub driver: Option<String>,
}

/// Kubernetes-style `MicroVm` Custom Resource YAML definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct K8sMicroVmManifest {
    #[serde(rename = "apiVersion", default = "default_api_version")]
    pub api_version: String,

    pub kind: String,

    pub metadata: K8sMetadata,

    pub spec: K8sMicroVmSpec,
}

fn default_api_version() -> String {
    "krun.io/v1alpha1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sMetadata {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sMicroVmSpec {
    pub image: String,
    #[serde(default)]
    pub vcpus: Option<u8>,
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub cmd: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<Vec<K8sEnvVar>>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub ports: Option<Vec<K8sPortMapping>>,
    #[serde(default, rename = "workspaceCow")]
    pub workspace_cow: Option<String>,
    #[serde(default, rename = "daxWindowSize")]
    pub dax_window_size: Option<String>,
    #[serde(default)]
    pub gpu: Option<bool>,
    #[serde(default, rename = "gpuShmSize")]
    pub gpu_shm_size: Option<String>,
    #[serde(default, rename = "shmSize")]
    pub shm_size: Option<String>,
    #[serde(default, rename = "networkMode")]
    pub network_mode: Option<String>,
    #[serde(default, rename = "allowEgress")]
    pub allow_egress: Option<Vec<String>>,
    #[serde(default)]
    pub sandbox: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sEnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sPortMapping {
    pub host: u16,
    pub guest: u16,
}

/// Standard Kubernetes `Pod` YAML definition (apiVersion: v1, kind: Pod).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct K8sPodManifest {
    #[serde(rename = "apiVersion", default = "default_pod_api_version")]
    pub api_version: String,

    pub kind: String,

    pub metadata: K8sMetadata,

    pub spec: K8sPodSpec,
}

fn default_pod_api_version() -> String {
    "v1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sPodSpec {
    pub containers: Vec<K8sContainerSpec>,
    #[serde(default, rename = "restartPolicy")]
    pub restart_policy: Option<String>,
    #[serde(default)]
    pub volumes: Option<Vec<K8sPodVolume>>,
    #[serde(default, rename = "hostNetwork")]
    pub host_network: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sContainerSpec {
    pub name: String,
    pub image: String,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<Vec<K8sEnvVar>>,
    #[serde(default)]
    pub ports: Option<Vec<K8sContainerPort>>,
    #[serde(default, rename = "workingDir")]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub resources: Option<K8sResourceRequirements>,
    #[serde(default, rename = "volumeMounts")]
    pub volume_mounts: Option<Vec<K8sVolumeMount>>,
    #[serde(default, rename = "shmSize")]
    pub shm_size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sContainerPort {
    #[serde(default, rename = "containerPort")]
    pub container_port: u16,
    #[serde(default, rename = "hostPort")]
    pub host_port: Option<u16>,
    #[serde(default)]
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sResourceRequirements {
    #[serde(default)]
    pub limits: Option<K8sResourceLimits>,
    #[serde(default)]
    pub requests: Option<K8sResourceLimits>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sResourceLimits {
    pub cpu: Option<String>,
    pub memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sVolumeMount {
    pub name: String,
    #[serde(rename = "mountPath")]
    pub mount_path: String,
    #[serde(default, rename = "readOnly")]
    pub read_only: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sPodVolume {
    pub name: String,
    #[serde(default, rename = "hostPath")]
    pub host_path: Option<K8sHostPathVolumeSource>,
    #[serde(default, rename = "emptyDir")]
    pub empty_dir: Option<K8sEmptyDirVolumeSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sHostPathVolumeSource {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct K8sEmptyDirVolumeSource {
    #[serde(default, rename = "medium")]
    pub medium: Option<String>,
    #[serde(default, rename = "sizeLimit")]
    pub size_limit: Option<String>,
}

/// Unified manifest abstraction: auto-detects Docker Compose, Kubernetes CRD, or Pod formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Manifest {
    Pod(Box<K8sPodManifest>),
    K8s(Box<K8sMicroVmManifest>),
    Compose(ComposeSpec),
}

impl Manifest {
    /// Parses a YAML manifest from raw string content.
    pub fn parse(yaml_str: &str) -> Result<Self> {
        let manifest: Manifest = serde_yaml::from_str(yaml_str)
            .context("Failed to parse YAML manifest (neither Compose nor MicroVm CRD format)")?;
        Ok(manifest)
    }

    /// Reads and parses a YAML manifest from a filesystem path.
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read YAML file at {}", path.as_ref().display()))?;
        Self::parse(&content)
    }

    /// Converts this manifest into a standardized `ComposeSpec`.
    pub fn into_compose_spec(self) -> ComposeSpec {
        match self {
            Manifest::Compose(spec) => spec,
            Manifest::Pod(pod) => {
                let mut vol_lookup: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                if let Some(ref vols) = pod.spec.volumes {
                    for v in vols {
                        if let Some(ref hp) = v.host_path {
                            vol_lookup.insert(v.name.clone(), hp.path.clone());
                        }
                    }
                }

                let mut services = BTreeMap::new();
                for (idx, c) in pod.spec.containers.iter().enumerate() {
                    let mut env_map = BTreeMap::new();
                    if let Some(ref envs) = c.env {
                        for e in envs {
                            env_map.insert(e.name.clone(), e.value.clone());
                        }
                    }

                    let mut port_strings = Vec::new();
                    if let Some(ref ports) = c.ports {
                        for p in ports {
                            if let Some(hp) = p.host_port {
                                port_strings.push(format!("{hp}:{}", p.container_port));
                            } else {
                                port_strings.push(format!("{}", p.container_port));
                            }
                        }
                    }

                    let mut volumes = Vec::new();
                    if let Some(ref mounts) = c.volume_mounts {
                        for m in mounts {
                            if let Some(host_path) = vol_lookup.get(&m.name) {
                                let ro_suffix = if m.read_only.unwrap_or(false) {
                                    ":ro"
                                } else {
                                    ""
                                };
                                volumes
                                    .push(format!("{}:{}{}", host_path, m.mount_path, ro_suffix));
                            }
                        }
                    }

                    let cpus = c
                        .resources
                        .as_ref()
                        .and_then(|r| r.limits.as_ref().and_then(|l| l.cpu.as_ref()))
                        .or_else(|| {
                            c.resources
                                .as_ref()
                                .and_then(|r| r.requests.as_ref().and_then(|req| req.cpu.as_ref()))
                        })
                        .and_then(|s| s.parse::<u8>().ok());

                    let memory = c
                        .resources
                        .as_ref()
                        .and_then(|r| r.limits.as_ref().and_then(|l| l.memory.clone()))
                        .or_else(|| {
                            c.resources.as_ref().and_then(|r| {
                                r.requests.as_ref().and_then(|req| req.memory.clone())
                            })
                        });

                    let svc = ServiceSpec {
                        image: c.image.clone(),
                        cpus,
                        memory,
                        cmd: c.args.clone(),
                        entrypoint: c.command.clone(),
                        env: env_map,
                        ports: port_strings,
                        volumes,
                        working_dir: c.working_dir.clone(),
                        network_mode: if pod.spec.host_network.unwrap_or(false) {
                            Some("host".to_string())
                        } else {
                            None
                        },
                        depends_on: if idx > 0 {
                            vec![pod.spec.containers[idx - 1].name.clone()]
                        } else {
                            Vec::new()
                        },
                        networks: Vec::new(),
                        restart: pod.spec.restart_policy.clone(),
                        gpu: None,
                        gpu_shm_size: None,
                        shm_size: c.shm_size.clone(),
                        dax: None,
                        secrets: Vec::new(),
                        allow_hosts: Vec::new(),
                        sandbox: None,
                        labels: pod.metadata.labels.clone(),
                    };

                    services.insert(c.name.clone(), svc);
                }

                ComposeSpec {
                    version: "krun/k8s-pod".to_string(),
                    name: Some(pod.metadata.name),
                    services,
                    volumes: BTreeMap::new(),
                    networks: BTreeMap::new(),
                }
            }
            Manifest::K8s(k8s) => {
                let mut env_map = BTreeMap::new();
                if let Some(envs) = k8s.spec.env {
                    for e in envs {
                        env_map.insert(e.name, e.value);
                    }
                }

                let mut port_strings = Vec::new();
                if let Some(single_port) = k8s.spec.port {
                    port_strings.push(format!("{single_port}:{single_port}"));
                }
                if let Some(multi_ports) = k8s.spec.ports {
                    for p in multi_ports {
                        port_strings.push(format!("{}:{}", p.host, p.guest));
                    }
                }

                let svc = ServiceSpec {
                    image: k8s.spec.image,
                    cpus: k8s.spec.vcpus,
                    memory: k8s.spec.memory,
                    cmd: k8s.spec.cmd,
                    entrypoint: None,
                    env: env_map,
                    ports: port_strings,
                    volumes: Vec::new(),
                    working_dir: None,
                    network_mode: k8s.spec.network_mode,
                    depends_on: Vec::new(),
                    networks: Vec::new(),
                    restart: None,
                    gpu: k8s.spec.gpu,
                    gpu_shm_size: k8s.spec.gpu_shm_size,
                    shm_size: k8s.spec.shm_size,
                    dax: k8s.spec.dax_window_size,
                    secrets: Vec::new(),
                    allow_hosts: k8s.spec.allow_egress.unwrap_or_default(),
                    sandbox: k8s.spec.sandbox,
                    labels: k8s.metadata.labels,
                };

                let mut services = BTreeMap::new();
                services.insert(k8s.metadata.name.clone(), svc);

                ComposeSpec {
                    version: "krun/v1".to_string(),
                    name: Some(k8s.metadata.name),
                    services,
                    volumes: BTreeMap::new(),
                    networks: BTreeMap::new(),
                }
            }
        }
    }
}

// Serde helper deserializers for permissive compose parsing

fn deserialize_optional_cmd<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CmdValue {
        List(Vec<String>),
        Str(String),
    }

    match Option::<CmdValue>::deserialize(deserializer)? {
        None => Ok(None),
        Some(CmdValue::List(list)) => Ok(Some(list)),
        Some(CmdValue::Str(s)) => {
            let parts: Vec<String> = s.split_whitespace().map(|s| s.to_string()).collect();
            Ok(Some(parts))
        }
    }
}

fn deserialize_env_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum EnvValue {
        Map(BTreeMap<String, serde_yaml::Value>),
        List(Vec<String>),
    }

    let mut res = BTreeMap::new();
    match Option::<EnvValue>::deserialize(deserializer)? {
        None => Ok(res),
        Some(EnvValue::Map(map)) => {
            for (k, v) in map {
                let val_str = match v {
                    serde_yaml::Value::String(s) => s,
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    other => serde_json::to_string(&other).unwrap_or_default(),
                };
                res.insert(k, val_str);
            }
            Ok(res)
        }
        Some(EnvValue::List(list)) => {
            for item in list {
                if let Some((k, v)) = item.split_once('=') {
                    res.insert(k.trim().to_string(), v.to_string());
                } else {
                    res.insert(item.trim().to_string(), String::new());
                }
            }
            Ok(res)
        }
    }
}

fn deserialize_depends_on<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum DepValue {
        List(Vec<String>),
        Map(BTreeMap<String, serde_yaml::Value>),
    }

    match Option::<DepValue>::deserialize(deserializer)? {
        None => Ok(Vec::new()),
        Some(DepValue::List(list)) => Ok(list),
        Some(DepValue::Map(map)) => Ok(map.into_keys().collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_docker_compose_yaml_standard() {
        let yaml = r#"
version: "3.8"
name: test-app

services:
  web:
    image: nginx:alpine
    cpus: 2
    memory: 512M
    ports:
      - "8080:80"
    volumes:
      - "./html:/usr/share/nginx/html:ro"
    env:
      APP_ENV: production
      PORT: 80
    restart: always

  db:
    image: redis:alpine
    cpus: 1
    memory: 256MiB
    ports:
      - "6379:6379"
    cmd: "redis-server --appendonly yes"
"#;

        let manifest = Manifest::parse(yaml).expect("Failed to parse standard compose YAML");
        let spec = manifest.into_compose_spec();

        assert_eq!(spec.name.as_deref(), Some("test-app"));
        assert_eq!(spec.services.len(), 2);

        let web = &spec.services["web"];
        assert_eq!(web.image, "nginx:alpine");
        assert_eq!(web.cpus, Some(2));
        assert_eq!(web.memory.as_deref(), Some("512M"));
        assert_eq!(web.ports, vec!["8080:80"]);
        assert_eq!(web.volumes, vec!["./html:/usr/share/nginx/html:ro"]);
        assert_eq!(
            web.env.get("APP_ENV").map(|s| s.as_str()),
            Some("production")
        );
        assert_eq!(web.env.get("PORT").map(|s| s.as_str()), Some("80"));

        let db = &spec.services["db"];
        assert_eq!(db.image, "redis:alpine");
        assert_eq!(
            db.cmd,
            Some(vec![
                "redis-server".to_string(),
                "--appendonly".to_string(),
                "yes".to_string()
            ])
        );
    }

    #[test]
    fn test_parse_k8s_microvm_crd_yaml() {
        let yaml = r#"
apiVersion: krun.io/v1alpha1
kind: MicroVm
metadata:
  name: isolated-ai-agent
  labels:
    tier: inference
spec:
  image: ghcr.io/ericlbuehler/mistral.rs:cpu-latest
  vcpus: 4
  memory: 8Gi
  cmd: ["mistralrs-server", "--port", "8000"]
  port: 8000
  gpu: true
  daxWindowSize: 4Gi
  allowEgress:
    - "api.anthropic.com:443"
"#;

        let manifest = Manifest::parse(yaml).expect("Failed to parse Kubernetes MicroVm CRD");
        let spec = manifest.into_compose_spec();

        assert_eq!(spec.name.as_deref(), Some("isolated-ai-agent"));
        let svc = &spec.services["isolated-ai-agent"];
        assert_eq!(svc.image, "ghcr.io/ericlbuehler/mistral.rs:cpu-latest");
        assert_eq!(svc.cpus, Some(4));
        assert_eq!(svc.memory.as_deref(), Some("8Gi"));
        assert_eq!(svc.gpu, Some(true));
        assert_eq!(svc.dax.as_deref(), Some("4Gi"));
        assert_eq!(svc.ports, vec!["8000:8000"]);
        assert_eq!(svc.allow_hosts, vec!["api.anthropic.com:443"]);
    }

    #[test]
    fn test_parse_env_list_and_map() {
        let yaml_list = r#"
image: alpine
env:
  - FOO=BAR
  - BAZ=123
"#;
        let svc: ServiceSpec = serde_yaml::from_str(yaml_list).unwrap();
        assert_eq!(svc.env.get("FOO").unwrap(), "BAR");
        assert_eq!(svc.env.get("BAZ").unwrap(), "123");

        let yaml_map = r#"
image: alpine
env:
  FOO: BAR
  BAZ: 123
"#;
        let svc2: ServiceSpec = serde_yaml::from_str(yaml_map).unwrap();
        assert_eq!(svc2.env.get("FOO").unwrap(), "BAR");
        assert_eq!(svc2.env.get("BAZ").unwrap(), "123");
    }

    #[test]
    fn test_parse_k8s_pod_yaml_standard() {
        let yaml = r#"
apiVersion: v1
kind: Pod
metadata:
  name: fullstack-pod
  labels:
    app: fullstack
spec:
  restartPolicy: Always
  volumes:
    - name: shared-data
      hostPath:
        path: /tmp/cro-data
  containers:
    - name: backend
      image: node:20-alpine
      command: ["node"]
      args: ["server.js"]
      ports:
        - containerPort: 3000
          hostPort: 3000
      env:
        - name: NODE_ENV
          value: production
      resources:
        limits:
          cpu: "2"
          memory: "1024Mi"
      volumeMounts:
        - name: shared-data
          mountPath: /data
      shmSize: "128m"
    - name: frontend
      image: nginx:alpine
      ports:
        - containerPort: 80
          hostPort: 8080
"#;

        let manifest = Manifest::parse(yaml).expect("Failed to parse Kubernetes Pod manifest");
        let compose = manifest.into_compose_spec();

        assert_eq!(compose.name, Some("fullstack-pod".to_string()));
        assert_eq!(compose.services.len(), 2);

        let backend = compose.services.get("backend").unwrap();
        assert_eq!(backend.image, "node:20-alpine");
        assert_eq!(backend.entrypoint, Some(vec!["node".to_string()]));
        assert_eq!(backend.cmd, Some(vec!["server.js".to_string()]));
        assert_eq!(backend.ports, vec!["3000:3000"]);
        assert_eq!(backend.env.get("NODE_ENV").unwrap(), "production");
        assert_eq!(backend.cpus, Some(2));
        assert_eq!(backend.memory, Some("1024Mi".to_string()));
        assert_eq!(backend.shm_size, Some("128m".to_string()));
        assert_eq!(backend.volumes, vec!["/tmp/cro-data:/data"]);
        assert!(backend.depends_on.is_empty());

        let frontend = compose.services.get("frontend").unwrap();
        assert_eq!(frontend.image, "nginx:alpine");
        assert_eq!(frontend.ports, vec!["8080:80"]);
        assert_eq!(frontend.depends_on, vec!["backend".to_string()]);
    }
}
