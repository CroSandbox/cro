// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Dynamic in-flight credential & header injection engine.
//!
//! Injects API keys, Bearer tokens, and custom headers directly into outbound
//! requests destined for known AI providers (Anthropic, OpenAI, Gemini, GitHub, etc.)
//! without exposing real credentials inside the guest microVM filesystem or env.

use crate::net::host_matcher::host_pattern_matches;
use serde::{Deserialize, Serialize};

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
    /// Tests whether this rule applies to the given `(host, path)` pair.
    pub fn matches(&self, target_host: &str, target_path: &str) -> bool {
        if !host_pattern_matches(&self.host_pattern, target_host) {
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
        self.rules
            .iter()
            .filter(|rule| rule.matches(host, path))
            .map(|rule| rule.action.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_injection_auto_configure() {
        let mut engine = HeaderInjectionEngine::new();
        let secrets = vec![
            (
                "ANTHROPIC_API_KEY".to_string(),
                "sk-ant-test12345".to_string(),
            ),
            (
                "OPENAI_API_KEY".to_string(),
                "sk-proj-openai999".to_string(),
            ),
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
    fn test_rule_path_prefix_matching() {
        let rule = CredentialInjectionRule {
            host_pattern: "api.openai.com".to_string(),
            path_prefix: Some("/v1/".to_string()),
            action: InjectionAction::SetBearerToken("test".to_string()),
        };

        assert!(rule.matches("api.openai.com", "/v1/chat/completions"));
        assert!(!rule.matches("api.openai.com", "/v2/models"));
        assert!(!rule.matches("other.com", "/v1/foo"));
    }
}
