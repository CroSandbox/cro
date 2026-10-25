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

pub use crate::net::approval::{
    ApprovalPolicy, ApprovalStatus, ApprovalStore, ApprovalTrigger, PendingApproval,
    RequestSummarizer,
};
pub use crate::net::host_matcher::{
    host_pattern_matches, host_pattern_matches_strict_subdomain, host_pattern_matches_with_apex,
};
pub use crate::net::injection::{CredentialInjectionRule, HeaderInjectionEngine, InjectionAction};
pub use crate::secrets::VaultResolver;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_reexports_facade() {
        let engine = HeaderInjectionEngine::new();
        assert_eq!(engine.find_injections("openai.com", "/").len(), 0);

        let policy = ApprovalPolicy::new();
        assert!(!policy.requires_approval("GET", "example.com", "/"));

        let res = VaultResolver::resolve("direct_val").unwrap();
        assert_eq!(res, "direct_val");
    }
}
