// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Canonical wildcard host pattern matching used across the networking layer.
//!
//! Consolidates the host matching logic that was previously duplicated in
//! `CredentialInjectionRule::matches()`, `ApprovalPolicy::requires_approval()`,
//! and `EgressPolicy::host_matches()`.

/// Matches a target hostname against a pattern string, specifying whether apex domain
/// (e.g. `example.com` for pattern `*.example.com`) should also match.
pub fn host_pattern_matches_with_apex(pattern: &str, target: &str, match_apex: bool) -> bool {
    let pattern = pattern.trim().trim_matches(|c| c == '[' || c == ']');
    let target = target.trim().trim_matches(|c| c == '[' || c == ']');

    if pattern == "*" || pattern == "*:*" {
        return true;
    }

    if pattern.eq_ignore_ascii_case(target) {
        return true;
    }

    if let Some(suffix) = pattern.strip_prefix("*.") {
        if match_apex && target.eq_ignore_ascii_case(suffix) {
            return true;
        }
        let suffix_lower = suffix.to_ascii_lowercase();
        let target_lower = target.to_ascii_lowercase();
        if target_lower.ends_with(&suffix_lower)
            && target_lower.len() > suffix_lower.len()
            && target_lower.as_bytes()[target_lower.len() - suffix_lower.len() - 1] == b'.'
        {
            return true;
        }
    }

    false
}

/// Matches a target hostname against a pattern string (including apex domain for wildcard patterns).
///
/// Supported patterns:
/// - `"*"` or `"*:*"` → matches everything
/// - `"*.example.com"` → matches `sub.example.com` and `example.com` itself
/// - `"api.openai.com"` → exact match (case-insensitive)
pub fn host_pattern_matches(pattern: &str, target: &str) -> bool {
    host_pattern_matches_with_apex(pattern, target, true)
}

/// Matches a target hostname against a pattern string, strictly requiring subdomains for `*.` patterns.
///
/// For pattern `"*.example.com"`, `sub.example.com` matches, but `example.com` does NOT match.
pub fn host_pattern_matches_strict_subdomain(pattern: &str, target: &str) -> bool {
    host_pattern_matches_with_apex(pattern, target, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert!(host_pattern_matches("api.openai.com", "api.openai.com"));
        assert!(host_pattern_matches("API.OpenAI.COM", "api.openai.com"));
        assert!(!host_pattern_matches("api.openai.com", "other.com"));
    }

    #[test]
    fn test_wildcard_all() {
        assert!(host_pattern_matches("*", "anything.com"));
        assert!(host_pattern_matches("*:*", "anything.com"));
    }

    #[test]
    fn test_glob_prefix_with_apex() {
        assert!(host_pattern_matches("*.github.com", "api.github.com"));
        assert!(host_pattern_matches("*.github.com", "github.com"));
        assert!(host_pattern_matches("*.github.com", "sub.api.github.com"));
        assert!(!host_pattern_matches("*.github.com", "notgithub.com"));
        assert!(!host_pattern_matches("*.github.com", "evilgithub.com"));
    }

    #[test]
    fn test_glob_prefix_strict_subdomain() {
        assert!(host_pattern_matches_strict_subdomain(
            "*.github.com",
            "api.github.com"
        ));
        assert!(!host_pattern_matches_strict_subdomain(
            "*.github.com",
            "github.com"
        ));
        assert!(!host_pattern_matches_strict_subdomain(
            "*.github.com",
            "notgithub.com"
        ));
        assert!(!host_pattern_matches_strict_subdomain(
            "*.github.com",
            "evilgithub.com"
        ));
    }

    #[test]
    fn test_ipv6_bracket_stripping() {
        assert!(host_pattern_matches("[::1]", "::1"));
        assert!(host_pattern_matches("::1", "[::1]"));
    }

    #[test]
    fn test_whitespace_trimming() {
        assert!(host_pattern_matches("  api.openai.com  ", "api.openai.com"));
    }
}
