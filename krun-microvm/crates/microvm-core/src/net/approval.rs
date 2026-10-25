// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Human-in-the-Loop (HITL) Policy & Approval Interception.
//!
//! Holds sensitive or mutating HTTP requests (e.g. POST/DELETE/PUT, git push, Stripe, AWS)
//! in an async pause state while notifying human operators via CLI or WebUI to Approve/Deny.

use crate::net::host_matcher::host_pattern_matches;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{oneshot, RwLock};

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
                    if m_upper == "POST"
                        || m_upper == "PUT"
                        || m_upper == "DELETE"
                        || m_upper == "PATCH"
                    {
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
                    let host_ok = host_pattern_matches(host_pattern, host);
                    let path_ok = path_prefix.as_ref().is_none_or(|p| path.starts_with(p));
                    let method_ok = rule_method
                        .as_ref()
                        .is_none_or(|m| m.eq_ignore_ascii_case(method));

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
        if host.contains("anthropic.com") && path.contains("/messages") {
            return format!("🤖 Anthropic Claude · Generate Model Completion ({m} {path})");
        }

        // 2. OpenAI API
        if host.contains("openai.com") && path.contains("/chat/completions") {
            return format!("🤖 OpenAI GPT · Generate Chat Completion ({m} {path})");
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
        if (host.contains("amazonaws.com") || host.contains("googleapis.com")) && m == "DELETE" {
            return format!("☁️ Cloud Provider · DELETE Resource ({host}{path})");
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

/// Global storage for holding and deciding pending approvals, supporting both in-process
/// oneshot channels and cross-process file-backed synchronization.
pub struct ApprovalStore {
    id_counter: AtomicU64,
    pending: RwLock<HashMap<String, (PendingApproval, oneshot::Sender<bool>)>>,
    history: RwLock<Vec<PendingApproval>>,
    storage_dir: Option<PathBuf>,
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self::new()
    }
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
        if let Err(e) = std::fs::create_dir_all(p.join("pending")) {
            tracing::warn!("Failed to create pending approvals dir: {e}");
        }
        if let Err(e) = std::fs::create_dir_all(p.join("decisions")) {
            tracing::warn!("Failed to create decisions approvals dir: {e}");
        }
        if let Err(e) = std::fs::create_dir_all(p.join("history")) {
            tracing::warn!("Failed to create history approvals dir: {e}");
        }
        self.storage_dir = Some(p);
        self
    }

    /// Submits a request for human approval, returning a channel receiver that completes
    /// when approved (true) or denied/timed out (false).
    #[allow(clippy::too_many_arguments)]
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
            match serde_json::to_string_pretty(&pending_item) {
                Ok(data) => {
                    if let Err(e) = std::fs::write(&file_path, data) {
                        tracing::warn!(
                            "Failed to persist pending approval file '{file_path:?}': {e}"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to serialize pending approval: {e}");
                }
            }
        }

        let (tx, rx) = oneshot::channel();
        self.pending
            .write()
            .await
            .insert(id.clone(), (pending_item, tx));

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
                let pending_path = dir.join("pending").join(format!("{}.json", id));
                if let Err(e) = std::fs::remove_file(&pending_path) {
                    tracing::warn!("Failed to remove resolved approval from pending dir '{pending_path:?}': {e}");
                }
                let history_file = dir.join("history").join(format!("{}.json", id));
                match serde_json::to_string_pretty(&item) {
                    Ok(data) => {
                        if let Err(e) = std::fs::write(&history_file, data) {
                            tracing::warn!(
                                "Failed to write history approval file '{history_file:?}': {e}"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to serialize approval history item: {e}");
                    }
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
                let pending_path = dir.join("pending").join(format!("{}.json", id));
                if let Err(e) = std::fs::remove_file(&pending_path) {
                    tracing::warn!("Failed to remove timed out approval '{pending_path:?}': {e}");
                }
                let history_file = dir.join("history").join(format!("{}.json", id));
                if let Ok(data) = serde_json::to_string_pretty(&item) {
                    if let Err(e) = std::fs::write(&history_file, data) {
                        tracing::warn!("Failed to write history file for timed out approval '{history_file:?}': {e}");
                    }
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
                                if let Ok(item) = serde_json::from_str::<PendingApproval>(&content)
                                {
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
        std::fs::write(
            &decision_file,
            serde_json::to_string_pretty(&decision_data)?,
        )?;

        // Move from pending to history
        if let Err(e) = std::fs::remove_file(&pending_file) {
            tracing::warn!("Failed to remove pending file '{pending_file:?}': {e}");
        }
        let history_file = storage_dir.join("history").join(format!("{}.json", id));
        if let Err(e) = std::fs::write(&history_file, serde_json::to_string_pretty(&item)?) {
            tracing::warn!("Failed to write history file '{history_file:?}': {e}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

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

    #[test]
    fn test_approval_policy_explicit_rule() {
        let policy = ApprovalPolicy::new().with_trigger(ApprovalTrigger::ExplicitRule {
            host_pattern: "*.internal.net".to_string(),
            path_prefix: Some("/admin".to_string()),
            method: Some("POST".to_string()),
        });

        assert!(policy.requires_approval("POST", "service.internal.net", "/admin/reset"));
        assert!(!policy.requires_approval("GET", "service.internal.net", "/admin/reset"));
        assert!(!policy.requires_approval("POST", "other.net", "/admin/reset"));
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

        let decision_file = temp_dir
            .path()
            .join("decisions")
            .join(format!("{}.json", id));
        assert!(decision_file.exists());
    }

    #[tokio::test]
    async fn test_approval_store_timeout() {
        let store = Arc::new(ApprovalStore::new());
        let (id, rx) = store
            .submit(
                "vm-timeout",
                "127.0.0.1",
                "DELETE",
                "api.aws.com",
                443,
                "/v1/instance",
                None,
            )
            .await;

        store.mark_timeout(&id).await;
        let decision = rx.await.unwrap();
        assert!(!decision);

        let history = store.get_history().await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, ApprovalStatus::TimedOut);
    }
}
