//! Transport-independent OCI manifest data and image configuration parsing.
use crate::config::OciConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct ManifestIndex {
    pub(crate) manifests: Option<Vec<ManifestIndexEntry>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ManifestIndexEntry {
    pub(crate) digest: String,
    pub(crate) platform: Option<Platform>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Platform {
    pub(crate) architecture: String,
    pub(crate) os: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct SingleManifest {
    #[serde(rename = "artifactType")]
    pub(crate) artifact_type: Option<String>,
    pub(crate) config: BlobDescriptor,
    pub(crate) layers: Vec<BlobDescriptor>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct BlobDescriptor {
    #[serde(rename = "mediaType")]
    pub(crate) media_type: Option<String>,
    pub(crate) digest: String,
    pub(crate) size: u64,
    pub(crate) annotations: Option<std::collections::HashMap<String, String>>,
}

pub(crate) fn parse_oci_config_from_json(json: &serde_json::Value) -> OciConfig {
    let config_node = json.get("config").unwrap_or(json);

    let entrypoint = config_node
        .get("Entrypoint")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let cmd = config_node
        .get("Cmd")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let env = config_node
        .get("Env")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let working_dir = config_node
        .get("WorkingDir")
        .and_then(|v| v.as_str())
        .map(String::from);

    let user = config_node
        .get("User")
        .and_then(|v| v.as_str())
        .map(String::from);

    OciConfig {
        entrypoint,
        cmd,
        env,
        working_dir,
        user,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_oci_config() {
        let json = serde_json::json!({
            "config": {
                "Entrypoint": ["/bin/sh", "-c"],
                "Cmd": ["echo hello"],
                "Env": ["PATH=/usr/bin:/bin", "FOO=BAR"],
                "WorkingDir": "/workspace",
                "User": "1000:1000"
            }
        });
        let parsed = parse_oci_config_from_json(&json);
        assert_eq!(parsed.entrypoint, vec!["/bin/sh", "-c"]);
        assert_eq!(parsed.cmd, vec!["echo hello"]);
        assert_eq!(parsed.env, vec!["PATH=/usr/bin:/bin", "FOO=BAR"]);
        assert_eq!(parsed.working_dir, Some("/workspace".to_string()));
        assert_eq!(parsed.user, Some("1000:1000".to_string()));
    }
}
