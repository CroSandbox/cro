// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Secret resolution from external sources.
//!
//! Resolves secrets dynamically from 1Password (`op://...`), Bitwarden (`bw://...`),
//! environment variables (`env:...`), local files (`file:...`), or plain strings.

mod vault;

pub use vault::VaultResolver;
