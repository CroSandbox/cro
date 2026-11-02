// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Fine-grained credential scoping and API action authorization.
//!
//! Restricts secrets to specific upstream domains, permitted HTTP verbs (e.g. read-only GET),
//! and exact API path endpoints, preventing prompt-injected agents or untrusted sandboxes
//! from exfiltrating credentials or executing unauthorized actions (e.g. issuing refunds or deleting resources).

use crate::net::host_matcher::host_pattern_matches;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Fine-grained authorization constraints on where and how a credential (secret) may be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialScope {
    /// Secret identifier (e.g. "OPENAI_API_KEY", "STRIPE_KEY", "CRM_API_TOKEN").
    pub key: String,
    /// Permitted destination host patterns (e.g. `["api.openai.com", "*.stripe.com"]`).
    /// If empty, any host passing egress policy is accepted.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Permitted HTTP methods (e.g. `["GET"]` for read-only credentials, `["GET", "POST"]`).
    /// Case-insensitive. If empty, all methods are permitted.
    #[serde(default)]
    pub allowed_methods: Vec<String>,
    /// Permitted API path prefixes (e.g. `["/v1/customers", "/v1/charges"]`).
    /// If empty, all paths on the permitted host are accepted.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Explicitly forbidden API path prefixes (e.g. `["/v1/refunds", "/admin"]`).
    /// Takes precedence over `allowed_paths`.
    #[serde(default)]
    pub denied_paths: Vec<String>,
}

impl CredentialScope {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            allowed_hosts: Vec::new(),
            allowed_methods: Vec::new(),
            allowed_paths: Vec::new(),
            denied_paths: Vec::new(),
        }
    }

    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.allowed_hosts.push(host.into());
        self
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.allowed_methods
            .push(method.into().to_ascii_uppercase());
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.allowed_paths.push(path.into());
        self
    }

    pub fn with_denied_path(mut self, path: impl Into<String>) -> Self {
        self.denied_paths.push(path.into());
        self
    }

    /// Evaluates whether an outbound HTTP request (method, host, path) is authorized to use this credential.
    pub fn authorize(&self, method: &str, host: &str, path: &str) -> Result<()> {
        let m_upper = method.to_ascii_uppercase();

        // 1. Host authorization
        if !self.allowed_hosts.is_empty() {
            let matched_host = self
                .allowed_hosts
                .iter()
                .any(|pattern| host_pattern_matches(pattern, host));
            if !matched_host {
                bail!(
                    "Credential scope violation: secret '{}' cannot be transmitted to host '{}' (allowed: {:?})",
                    self.key,
                    host,
                    self.allowed_hosts
                );
            }
        }

        // 2. HTTP method authorization
        if !self.allowed_methods.is_empty() {
            let matched_method = self
                .allowed_methods
                .iter()
                .any(|m| m.eq_ignore_ascii_case(&m_upper));
            if !matched_method {
                bail!(
                    "Credential scope violation: secret '{}' cannot be used with HTTP method '{}' (allowed: {:?})",
                    self.key,
                    m_upper,
                    self.allowed_methods
                );
            }
        }

        // 3. Denied paths check (strict precedence)
        for denied in &self.denied_paths {
            if path.starts_with(denied) {
                bail!(
                    "Credential scope violation: secret '{}' is explicitly forbidden from path '{}' (denied rule: '{}')",
                    self.key,
                    path,
                    denied
                );
            }
        }

        // 4. Allowed paths check
        if !self.allowed_paths.is_empty() {
            let matched_path = self
                .allowed_paths
                .iter()
                .any(|allowed| path.starts_with(allowed));
            if !matched_path {
                bail!(
                    "Credential scope violation: secret '{}' cannot be used with path '{}' (allowed paths: {:?})",
                    self.key,
                    path,
                    self.allowed_paths
                );
            }
        }

        Ok(())
    }
}

/// Registry and authorizer managing credential scopes across multiple secrets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeAuthorizer {
    scopes: HashMap<String, CredentialScope>,
}

impl ScopeAuthorizer {
    pub fn new() -> Self {
        Self {
            scopes: HashMap::new(),
        }
    }

    pub fn with_scope(mut self, scope: CredentialScope) -> Self {
        self.scopes.insert(scope.key.clone(), scope);
        self
    }

    pub fn add_scope(&mut self, scope: CredentialScope) {
        self.scopes.insert(scope.key.clone(), scope);
    }

    pub fn get_scope(&self, key: &str) -> Option<&CredentialScope> {
        self.scopes.get(key)
    }

    /// Authorizes an action for a specific secret key.
    pub fn authorize(&self, key: &str, method: &str, host: &str, path: &str) -> Result<()> {
        if let Some(scope) = self.scopes.get(key) {
            scope.authorize(method, host, path)
        } else {
            // Unscoped secrets default to permitted (backwards compatible)
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_scope_host_restriction() {
        let scope = CredentialScope::new("STRIPE_SECRET")
            .with_host("api.stripe.com")
            .with_host("*.stripe.com");

        assert!(scope
            .authorize("GET", "api.stripe.com", "/v1/customers")
            .is_ok());
        assert!(scope
            .authorize("GET", "checkout.stripe.com", "/v1/sessions")
            .is_ok());

        // Evil exfil host rejected
        let res = scope.authorize("GET", "evil.attacker.com", "/exfil");
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("cannot be transmitted to host"));
    }

    #[test]
    fn test_credential_scope_method_restriction() {
        let read_only = CredentialScope::new("GITHUB_TOKEN")
            .with_host("api.github.com")
            .with_method("GET");

        assert!(read_only
            .authorize("GET", "api.github.com", "/repos/owner/repo")
            .is_ok());

        // Mutating methods rejected
        assert!(read_only
            .authorize("POST", "api.github.com", "/repos/owner/repo/issues")
            .is_err());
        assert!(read_only
            .authorize("DELETE", "api.github.com", "/repos/owner/repo")
            .is_err());
    }

    #[test]
    fn test_credential_scope_path_restriction() {
        let crm_scope = CredentialScope::new("CRM_API_KEY")
            .with_host("crm.internal")
            .with_path("/api/v1/customers")
            .with_denied_path("/api/v1/customers/delete");

        assert!(crm_scope
            .authorize("GET", "crm.internal", "/api/v1/customers")
            .is_ok());
        assert!(crm_scope
            .authorize("GET", "crm.internal", "/api/v1/customers/123")
            .is_ok());

        // Path not in allowed_paths
        assert!(crm_scope
            .authorize("GET", "crm.internal", "/api/v1/financials")
            .is_err());

        // Denied path takes precedence
        assert!(crm_scope
            .authorize("POST", "crm.internal", "/api/v1/customers/delete")
            .is_err());
    }
}
