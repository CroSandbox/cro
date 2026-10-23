// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Zero-Trust Agent Gateway & Security Suite inspired by OneCLI.
//!
//! Provides:
//! 1. **Dynamic In-Flight Credential & Header Injection**:
//!    Injects API keys, Bearer tokens, and custom headers directly into outbound
//!    requests destined for known AI providers (Anthropic, OpenAI, Gemini, GitHub, etc.)
//!    without exposing real credentials inside the guest microVM filesystem or env.
//! 2. **Human-in-the-Loop (HITL) Policy & Approval Interception**:
//!    Holds sensitive or mutating HTTP requests (e.g. POST/DELETE/PUT, git push, Stripe, AWS)
//!    in an async pause state while notifying human operators via CLI or WebUI to Approve/Deny.
//! 3. **Smart Request Summarizers**:
//!    Parses opaque request payloads into concise, actionable summaries for human review.
//! 4. **External Vault Resolvers**:
//!    Resolves secrets dynamically from 1Password (`op://...`), Bitwarden (`bw://...`),
//!    files (`file:...`), or environment (`env:...`).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{oneshot, RwLock};

// ── External Vault Resolvers ─────────────────────────────────────────────

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
            bail!("1Password CLI failed to resolve '{}': {}", op_uri, err.trim());
        }

        let val = String::from_utf8(output.stdout)
            .context("1Password output is not valid UTF-8")?;
        Ok(val)
    }

    fn resolve_bitwarden(spec: &str) -> Result<String> {
        let parts: Vec<&str> = spec.split('/').collect();
        let (item, field) = match parts.as_slice() {
            [item, field] => (*item, Some(*field)),
            [item] => (*item, None),
            _ => bail!("Invalid Bitwarden spec '{}'. Expected 'bw://item/field' or 'bw://item'", spec),
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
                let output = cmd.output().context("Failed to execute Bitwarden CLI ('bw')")?;
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
                bail!("Custom field '{}' not found in Bitwarden item '{}'", custom_field, item);
            }
        }

        let output = cmd.output().context("Failed to invoke Bitwarden CLI ('bw')")?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!("Bitwarden CLI failed to resolve '{}': {}", spec, err.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

// ── Credential & Header Injection Engine ─────────────────────────────────

/// Action to perform when an outbound request matches an injection rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InjectionAction {
    /// Injects an HTTP header if not already present or overrides it.
    SetHeader { name: String, value: String },
    /// Injects `Authorization: Bearer <token>`.
    SetBearerToken(String),
    /// Injects a provider-specific API key (e.g. `x-api-key: <key>`).
    SetApiKey { header_name: String, key: String },
    /// Injects a query parameter into the request URI.
    SetQueryParam { name: String, value: String },
}

/// A declarative rule for in-flight credential injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialInjectionRule {
    /// Target host pattern (e.g. `api.anthropic.com`, `*.openai.com`, `github.com`).
    pub host_pattern: String,
    /// Optional path prefix or regex (e.g. `/v1/*`). Empty matches all paths.
    pub path_prefix: Option<String>,
    /// Action to execute on the request.
    pub action: InjectionAction,
}

impl CredentialInjectionRule {
    pub fn matches(&self, target_host: &str, target_path: &str) -> bool {
        let host_matches = if self.host_pattern == "*" || self.host_pattern == "*:*" {
            true
        } else if let Some(suffix) = self.host_pattern.strip_prefix("*.") {
            target_host.ends_with(suffix) || target_host == suffix
        } else {
            target_host.eq_ignore_ascii_case(&self.host_pattern)
        };

        if !host_matches {
            return false;
        }

        if let Some(ref prefix) = self.path_prefix {
            if !target_path.starts_with(prefix) {
                return false;
            }
        }

        true
    }
}

/// Manages and applies in-flight credential injection rules.
#[derive(Debug, Clone, Default)]
pub struct HeaderInjectionEngine {
    rules: Vec<CredentialInjectionRule>,
}

impl HeaderInjectionEngine {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(&mut self, rule: CredentialInjectionRule) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// Automatically provisions standard AI and developer provider injection rules
    /// from standard secret names.
    ///
    /// This enables zero-configuration agent sandboxing:
    /// Passing `ANTHROPIC_API_KEY` automatically authenticates `api.anthropic.com`
    /// without the guest agent needing to know or hold any credentials.
    pub fn auto_configure_from_secrets(&mut self, secrets: &[(String, String)]) {
        for (key, val) in secrets {
            match key.to_uppercase().as_str() {
                "ANTHROPIC_API_KEY" => {
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "api.anthropic.com".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetApiKey {
                            header_name: "x-api-key".to_string(),
                            key: val.clone(),
                        },
                    });
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "api.anthropic.com".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetHeader {
                            name: "anthropic-version".to_string(),
                            value: "2023-06-01".to_string(),
                        },
                    });
                }
                "OPENAI_API_KEY" => {
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "api.openai.com".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetBearerToken(val.clone()),
                    });
                }
                "GEMINI_API_KEY" => {
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "generativelanguage.googleapis.com".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetApiKey {
                            header_name: "x-goog-api-key".to_string(),
                            key: val.clone(),
                        },
                    });
                }
                "GITHUB_TOKEN" | "GH_TOKEN" => {
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "api.github.com".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetBearerToken(val.clone()),
                    });
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "api.github.com".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetHeader {
                            name: "User-Agent".to_string(),
                            value: "cro-sandboxed-agent".to_string(),
                        },
                    });
                }
                "GROQ_API_KEY" => {
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "api.groq.com".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetBearerToken(val.clone()),
                    });
                }
                "DEEPSEEK_API_KEY" => {
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "api.deepseek.com".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetBearerToken(val.clone()),
                    });
                }
                "MISTRAL_API_KEY" => {
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "api.mistral.ai".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetBearerToken(val.clone()),
                    });
                }
                "OPENROUTER_API_KEY" => {
                    self.add_rule(CredentialInjectionRule {
                        host_pattern: "openrouter.ai".to_string(),
                        path_prefix: None,
                        action: InjectionAction::SetBearerToken(val.clone()),
                    });
                }
                _ => {}
            }
        }
    }

    /// Finds all injection actions matching a given host and path.
    pub fn find_injections(&self, host: &str, path: &str) -> Vec<InjectionAction> {
        let mut actions = Vec::new();
        for rule in &self.rules {
            if rule.matches(host, path) {
                actions.push(rule.action.clone());
            }
        }
        actions
    }
}

// ── Human-in-the-Loop (HITL) Approvals ───────────────────────────────────

/// State of an in-flight manual approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    TimedOut,
}

/// An intercepted outbound request awaiting human approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub id: String,
    pub timestamp: u64,
    pub agent_or_vm_id: String,
    pub client_addr: String,
    pub method: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub summary: String,
    pub preview: Option<String>,
    pub status: ApprovalStatus,
}

/// Policy triggers that require human approval before an outbound request is forwarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalTrigger {
    /// Intercepts all state-mutating HTTP methods: POST, PUT, DELETE, PATCH.
    AllMutating,
    /// Intercepts requests containing high-risk keywords in their path (e.g. /delete, /refund, /drop, git-receive-pack).
    DestructiveKeywords,
    /// Intercepts requests targeting specific hosts/paths.
    ExplicitRule {
        host_pattern: String,
        path_prefix: Option<String>,
        method: Option<String>,
    },
}

/// Configures when requests must be held for human review.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    pub enabled: bool,
    pub default_timeout_secs: u64,
    pub triggers: Vec<ApprovalTrigger>,
}

impl ApprovalPolicy {
    pub fn new() -> Self {
        Self {
            enabled: false,
            default_timeout_secs: 60,
            triggers: Vec::new(),
        }
    }

    pub fn with_trigger(mut self, trigger: ApprovalTrigger) -> Self {
        self.enabled = true;
        self.triggers.push(trigger);
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.default_timeout_secs = secs;
        self
    }

    pub fn requires_approval(&self, method: &str, host: &str, path: &str) -> bool {
        if !self.enabled {
            return false;
        }

        let m_upper = method.to_ascii_uppercase();

        for trigger in &self.triggers {
            match trigger {
                ApprovalTrigger::AllMutating => {
                    if m_upper == "POST" || m_upper == "PUT" || m_upper == "DELETE" || m_upper == "PATCH" {
                        return true;
                    }
                }
                ApprovalTrigger::DestructiveKeywords => {
                    let path_lower = path.to_ascii_lowercase();
                    if path_lower.contains("delete")
                        || path_lower.contains("destroy")
                        || path_lower.contains("terminate")
                        || path_lower.contains("refund")
                        || path_lower.contains("drop")
                        || path_lower.contains("git-receive-pack")
                        || path_lower.contains("purge")
                    {
                        return true;
                    }
                }
                ApprovalTrigger::ExplicitRule {
                    host_pattern,
                    path_prefix,
                    method: rule_method,
                } => {
                    let host_ok = if host_pattern == "*" {
                        true
                    } else if let Some(suffix) = host_pattern.strip_prefix("*.") {
                        host.ends_with(suffix) || host == suffix
                    } else {
                        host.eq_ignore_ascii_case(host_pattern)
                    };

                    let path_ok = path_prefix.as_ref().map_or(true, |p| path.starts_with(p));
                    let method_ok = rule_method.as_ref().map_or(true, |m| m.eq_ignore_ascii_case(method));

                    if host_ok && path_ok && method_ok {
                        return true;
                    }
                }
            }
        }

        false
    }
}

/// Generates human-friendly action summaries for approval cards.
pub struct RequestSummarizer;

impl RequestSummarizer {
    pub fn summarize(method: &str, host: &str, path: &str, body_snippet: Option<&str>) -> String {
        let m = method.to_ascii_uppercase();

        // 1. Anthropic API
        if host.contains("anthropic.com") {
            if path.contains("/messages") {
                return format!("🤖 Anthropic Claude · Generate Model Completion ({m} {path})");
            }
        }

        // 2. OpenAI API
        if host.contains("openai.com") {
            if path.contains("/chat/completions") {
                return format!("🤖 OpenAI GPT · Generate Chat Completion ({m} {path})");
            }
        }

        // 3. GitHub API
        if host.contains("github.com") {
            if path.contains("/git-receive-pack") {
                return "🐙 GitHub Git Push · Pushing Commits to Remote Repository".to_string();
            }
            if path.contains("/pulls") && m == "POST" {
                return "🐙 GitHub · Create Pull Request".to_string();
            }
            if path.contains("/issues") && m == "POST" {
                return "🐙 GitHub · Create Issue".to_string();
            }
        }

        // 4. Stripe API
        if host.contains("stripe.com") {
            if path.contains("/refunds") {
                return format!("💳 Stripe · Issue Customer Refund ({m} {path})");
            }
            if path.contains("/charges") || path.contains("/payment_intents") {
                return format!("💳 Stripe · Create/Modify Payment ({m} {path})");
            }
        }

        // 5. AWS / Cloud APIs
        if host.contains("amazonaws.com") || host.contains("googleapis.com") {
            if m == "DELETE" {
                return format!("☁️ Cloud Provider · DELETE Resource ({host}{path})");
            }
        }

        // 6. Generic preview
        if let Some(snippet) = body_snippet {
            let truncated = if snippet.len() > 80 {
                format!("{}...", &snippet[..80])
            } else {
                snippet.to_string()
            };
            format!("{m} https://{host}{path} · Payload: {truncated}")
        } else {
            format!("{m} https://{host}{path}")
        }
    }
}

use std::path::{Path, PathBuf};

/// Global storage for holding and deciding pending approvals, supporting both in-process
/// oneshot channels and cross-process file-backed synchronization.
pub struct ApprovalStore {
    id_counter: AtomicU64,
    pending: RwLock<HashMap<String, (PendingApproval, oneshot::Sender<bool>)>>,
    history: RwLock<Vec<PendingApproval>>,
    storage_dir: Option<PathBuf>,
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self {
            id_counter: AtomicU64::new(1),
            pending: RwLock::new(HashMap::new()),
            history: RwLock::new(Vec::new()),
            storage_dir: None,
        }
    }

    pub fn with_storage_dir(mut self, path: impl Into<PathBuf>) -> Self {
        let p = path.into();
        let _ = std::fs::create_dir_all(p.join("pending"));
        let _ = std::fs::create_dir_all(p.join("decisions"));
        let _ = std::fs::create_dir_all(p.join("history"));
        self.storage_dir = Some(p);
        self
    }

    /// Submits a request for human approval, returning a channel receiver that completes
    /// when approved (true) or denied/timed out (false).
    pub async fn submit(
        &self,
        agent_or_vm_id: impl Into<String>,
        client_addr: impl Into<String>,
        method: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        path: impl Into<String>,
        preview: Option<String>,
    ) -> (String, oneshot::Receiver<bool>) {
        let id_num = self.id_counter.fetch_add(1, Ordering::SeqCst);
        let id = format!("appr_{:06x}", id_num);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let m = method.into();
        let h = host.into();
        let p = path.into();
        let summary = RequestSummarizer::summarize(&m, &h, &p, preview.as_deref());

        let pending_item = PendingApproval {
            id: id.clone(),
            timestamp: now,
            agent_or_vm_id: agent_or_vm_id.into(),
            client_addr: client_addr.into(),
            method: m,
            host: h,
            port,
            path: p,
            summary,
            preview,
            status: ApprovalStatus::Pending,
        };

        if let Some(ref dir) = self.storage_dir {
            let file_path = dir.join("pending").join(format!("{}.json", id));
            if let Ok(data) = serde_json::to_string_pretty(&pending_item) {
                let _ = std::fs::write(&file_path, data);
            }
        }

        let (tx, rx) = oneshot::channel();
        self.pending.write().await.insert(id.clone(), (pending_item, tx));

        (id, rx)
    }

    /// Decides on an approval request.
    /// Returns Ok(true) if the request was found and updated, Ok(false) if not found.
    pub async fn decide(&self, id: &str, approved: bool) -> Result<bool> {
        let item_opt = self.pending.write().await.remove(id);

        if let Some((mut item, tx)) = item_opt {
            item.status = if approved {
                ApprovalStatus::Approved
            } else {
                ApprovalStatus::Denied
            };

            if let Some(ref dir) = self.storage_dir {
                let _ = std::fs::remove_file(dir.join("pending").join(format!("{}.json", id)));
                let history_file = dir.join("history").join(format!("{}.json", id));
                if let Ok(data) = serde_json::to_string_pretty(&item) {
                    let _ = std::fs::write(&history_file, data);
                }
            }

            self.history.write().await.push(item);
            let _ = tx.send(approved);
            Ok(true)
        } else {
            // Check file-backed store if present
            if let Some(ref dir) = self.storage_dir {
                Self::cli_decide(dir, id, approved)?;
                return Ok(true);
            }
            Ok(false)
        }
    }

    /// Marks an in-flight approval as timed out.
    pub async fn mark_timeout(&self, id: &str) {
        if let Some((mut item, tx)) = self.pending.write().await.remove(id) {
            item.status = ApprovalStatus::TimedOut;
            if let Some(ref dir) = self.storage_dir {
                let _ = std::fs::remove_file(dir.join("pending").join(format!("{}.json", id)));
                let history_file = dir.join("history").join(format!("{}.json", id));
                if let Ok(data) = serde_json::to_string_pretty(&item) {
                    let _ = std::fs::write(&history_file, data);
                }
            }
            self.history.write().await.push(item);
            let _ = tx.send(false);
        }
    }

    /// Lists all approvals currently awaiting decision.
    pub async fn list_pending(&self) -> Vec<PendingApproval> {
        let in_memory: Vec<PendingApproval> = self
            .pending
            .read()
            .await
            .values()
            .map(|(item, _)| item.clone())
            .collect();

        if !in_memory.is_empty() {
            return in_memory;
        }

        if let Some(ref dir) = self.storage_dir {
            if let Ok(disk_pending) = Self::cli_list_pending(dir) {
                return disk_pending;
            }
        }

        Vec::new()
    }

    /// Retrieves historical approvals (approved, denied, timed out).
    pub async fn get_history(&self) -> Vec<PendingApproval> {
        let mut list = self.history.read().await.clone();
        if list.is_empty() {
            if let Some(ref dir) = self.storage_dir {
                let history_dir = dir.join("history");
                if let Ok(entries) = std::fs::read_dir(history_dir) {
                    for entry in entries.flatten() {
                        if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                                if let Ok(item) = serde_json::from_str::<PendingApproval>(&content) {
                                    list.push(item);
                                }
                            }
                        }
                    }
                }
            }
        }
        list
    }

    /// CLI/Cross-process helper: lists pending approvals directly from a storage directory.
    pub fn cli_list_pending(storage_dir: &Path) -> Result<Vec<PendingApproval>> {
        let pending_dir = storage_dir.join("pending");
        if !pending_dir.exists() {
            return Ok(Vec::new());
        }

        let mut list = Vec::new();
        for entry in std::fs::read_dir(&pending_dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(item) = serde_json::from_str::<PendingApproval>(&content) {
                        list.push(item);
                    }
                }
            }
        }
        list.sort_by_key(|item| item.timestamp);
        Ok(list)
    }

    /// CLI/Cross-process helper: writes an external decision for a pending approval.
    pub fn cli_decide(storage_dir: &Path, id: &str, approved: bool) -> Result<()> {
        let pending_file = storage_dir.join("pending").join(format!("{}.json", id));
        if !pending_file.exists() {
            bail!("Approval request '{}' not found in pending queue", id);
        }

        let content = std::fs::read_to_string(&pending_file)?;
        let mut item: PendingApproval = serde_json::from_str(&content)?;
        item.status = if approved {
            ApprovalStatus::Approved
        } else {
            ApprovalStatus::Denied
        };

        // Write decision notification file for waiting process
        let decision_file = storage_dir.join("decisions").join(format!("{}.json", id));
        let decision_data = serde_json::json!({
            "id": id,
            "approved": approved,
            "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        });
        std::fs::write(&decision_file, serde_json::to_string_pretty(&decision_data)?)?;

        // Move from pending to history
        let _ = std::fs::remove_file(&pending_file);
        let history_file = storage_dir.join("history").join(format!("{}.json", id));
        let _ = std::fs::write(&history_file, serde_json::to_string_pretty(&item)?);

        Ok(())
    }
}

// ── Unit Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn test_vault_resolver_env() {
        std::env::set_var("TEST_CRO_SECRET_123", "secret_value_xyz");
        let res = VaultResolver::resolve("env:TEST_CRO_SECRET_123").unwrap();
        assert_eq!(res, "secret_value_xyz");
    }

    #[test]
    fn test_header_injection_auto_configure() {
        let mut engine = HeaderInjectionEngine::new();
        let secrets = vec![
            ("ANTHROPIC_API_KEY".to_string(), "sk-ant-test12345".to_string()),
            ("OPENAI_API_KEY".to_string(), "sk-proj-openai999".to_string()),
        ];
        engine.auto_configure_from_secrets(&secrets);

        // Anthropic check
        let ant_injections = engine.find_injections("api.anthropic.com", "/v1/messages");
        assert_eq!(ant_injections.len(), 2);
        assert!(ant_injections.contains(&InjectionAction::SetApiKey {
            header_name: "x-api-key".to_string(),
            key: "sk-ant-test12345".to_string(),
        }));
        assert!(ant_injections.contains(&InjectionAction::SetHeader {
            name: "anthropic-version".to_string(),
            value: "2023-06-01".to_string(),
        }));

        // OpenAI check
        let openai_injections = engine.find_injections("api.openai.com", "/v1/chat/completions");
        assert_eq!(openai_injections.len(), 1);
        assert_eq!(
            openai_injections[0],
            InjectionAction::SetBearerToken("sk-proj-openai999".to_string())
        );

        // Unknown host should get no injections
        let other_injections = engine.find_injections("example.com", "/");
        assert!(other_injections.is_empty());
    }

    #[test]
    fn test_approval_policy_triggers() {
        let policy = ApprovalPolicy::new()
            .with_trigger(ApprovalTrigger::AllMutating)
            .with_trigger(ApprovalTrigger::DestructiveKeywords);

        // GET should not trigger
        assert!(!policy.requires_approval("GET", "api.github.com", "/repos/owner/repo"));

        // POST should trigger (AllMutating)
        assert!(policy.requires_approval("POST", "api.github.com", "/repos/owner/repo/pulls"));

        // GET with delete keyword should trigger (DestructiveKeywords)
        assert!(policy.requires_approval("GET", "my-api.com", "/v1/records/delete"));
    }

    #[tokio::test]
    async fn test_approval_store_flow() {
        let store = Arc::new(ApprovalStore::new());

        let (id, rx) = store
            .submit(
                "vm-1",
                "127.0.0.1",
                "POST",
                "api.github.com",
                443,
                "/repos/CroSandbox/cro/pulls",
                Some(r#"{"title":"Add OneCLI features"}"#.to_string()),
            )
            .await;

        let pending = store.list_pending().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        assert_eq!(pending[0].summary, "🐙 GitHub · Create Pull Request");

        // Approve in background
        let store_clone = store.clone();
        let id_clone = id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store_clone.decide(&id_clone, true).await.unwrap();
        });

        let decision = rx.await.unwrap();
        assert!(decision);

        let pending_after = store.list_pending().await;
        assert!(pending_after.is_empty());

        let history = store.get_history().await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, ApprovalStatus::Approved);
    }

    #[tokio::test]
    async fn test_approval_store_file_backed() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(ApprovalStore::new().with_storage_dir(temp_dir.path()));

        let (id, _rx) = store
            .submit(
                "agent-claude",
                "127.0.0.1",
                "POST",
                "api.stripe.com",
                443,
                "/v1/refunds",
                Some(r#"{"charge":"ch_123","amount":1000}"#.to_string()),
            )
            .await;

        let pending_disk = ApprovalStore::cli_list_pending(temp_dir.path()).unwrap();
        assert_eq!(pending_disk.len(), 1);
        assert_eq!(pending_disk[0].id, id);
        assert!(pending_disk[0].summary.contains("Stripe"));

        // Simulate external CLI deciding
        ApprovalStore::cli_decide(temp_dir.path(), &id, true).unwrap();

        let pending_after = ApprovalStore::cli_list_pending(temp_dir.path()).unwrap();
        assert!(pending_after.is_empty());

        let decision_file = temp_dir.path().join("decisions").join(format!("{}.json", id));
        assert!(decision_file.exists());
    }
}
