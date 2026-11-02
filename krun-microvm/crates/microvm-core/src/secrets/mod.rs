// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Secret resolution from external sources.
//!
//! Resolves secrets dynamically from 1Password (`op://...`), Bitwarden (`bw://...`),
//! environment variables (`env:...`), local files (`file:...`), or plain strings.

mod scope;
mod vault;

pub use scope::{CredentialScope, ScopeAuthorizer};
pub use vault::VaultResolver;
