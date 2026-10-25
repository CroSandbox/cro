// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! External vault resolvers for 1Password, Bitwarden, environment, and file-backed secrets.

use anyhow::{bail, Context, Result};

/// Resolves secret values from external sources (1Password, Bitwarden, env, file).
pub struct VaultResolver;

impl VaultResolver {
    /// Resolves a secret specification string.
    ///
    /// Formats supported:
    /// - `op://vault/item/field` -> 1Password CLI (`op read`)
    /// - `bw://item/field` or `bw://item` -> Bitwarden CLI (`bw get`)
    /// - `env:VAR_NAME` -> Host environment variable
    /// - `file:/path/to/secret` -> Local file contents
    /// - Plain string -> returned as-is
    pub fn resolve(spec: &str) -> Result<String> {
        let trimmed = spec.trim();

        if let Some(op_path) = trimmed.strip_prefix("op://") {
            Self::resolve_1password(&format!("op://{}", op_path))
        } else if let Some(bw_spec) = trimmed.strip_prefix("bw://") {
            Self::resolve_bitwarden(bw_spec)
        } else if let Some(var_name) = trimmed.strip_prefix("env:") {
            std::env::var(var_name)
                .with_context(|| format!("Environment variable '{}' not found on host", var_name))
        } else if let Some(file_path) = trimmed.strip_prefix("file:") {
            std::fs::read_to_string(file_path)
                .map(|s| s.trim().to_string())
                .with_context(|| format!("Failed to read secret file at '{}'", file_path))
        } else {
            Ok(trimmed.to_string())
        }
    }

    fn resolve_1password(op_uri: &str) -> Result<String> {
        let output = std::process::Command::new("op")
            .args(["read", op_uri, "--no-newline"])
            .output()
            .context("Failed to invoke 1Password CLI ('op'). Is 1Password CLI installed and authenticated?")?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!(
                "1Password CLI failed to resolve '{}': {}",
                op_uri,
                err.trim()
            );
        }

        let val =
            String::from_utf8(output.stdout).context("1Password output is not valid UTF-8")?;
        Ok(val)
    }

    fn resolve_bitwarden(spec: &str) -> Result<String> {
        let parts: Vec<&str> = spec.split('/').collect();
        let (item, field) = match parts.as_slice() {
            [item, field] => (*item, Some(*field)),
            [item] => (*item, None),
            _ => bail!(
                "Invalid Bitwarden spec '{}'. Expected 'bw://item/field' or 'bw://item'",
                spec
            ),
        };

        let mut cmd = std::process::Command::new("bw");
        match field {
            Some("password") | None => {
                cmd.args(["get", "password", item]);
            }
            Some("username") => {
                cmd.args(["get", "username", item]);
            }
            Some("uri") => {
                cmd.args(["get", "uri", item]);
            }
            Some(custom_field) => {
                cmd.args(["get", "item", item]);
                let output = cmd
                    .output()
                    .context("Failed to execute Bitwarden CLI ('bw')")?;
                if !output.status.success() {
                    bail!("Bitwarden CLI failed for item '{}'", item);
                }
                let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
                if let Some(fields) = json.get("fields").and_then(|f| f.as_array()) {
                    for f in fields {
                        if f.get("name").and_then(|n| n.as_str()) == Some(custom_field) {
                            if let Some(val) = f.get("value").and_then(|v| v.as_str()) {
                                return Ok(val.to_string());
                            }
                        }
                    }
                }
                bail!(
                    "Custom field '{}' not found in Bitwarden item '{}'",
                    custom_field,
                    item
                );
            }
        }

        let output = cmd
            .output()
            .context("Failed to invoke Bitwarden CLI ('bw')")?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!("Bitwarden CLI failed to resolve '{}': {}", spec, err.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_resolver_env() {
        std::env::set_var("TEST_CRO_SECRET_123", "secret_value_xyz");
        let res = VaultResolver::resolve("env:TEST_CRO_SECRET_123").unwrap();
        assert_eq!(res, "secret_value_xyz");
    }

    #[test]
    fn test_vault_resolver_plain_string() {
        let res = VaultResolver::resolve("just-a-plain-value").unwrap();
        assert_eq!(res, "just-a-plain-value");
    }

    #[test]
    fn test_vault_resolver_trimming() {
        std::env::set_var("TEST_CRO_TRIM_VAR", "trimmed");
        let res = VaultResolver::resolve("  env:TEST_CRO_TRIM_VAR  ").unwrap();
        assert_eq!(res, "trimmed");
    }
}
