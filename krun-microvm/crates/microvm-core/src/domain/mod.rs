// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Automatic local domain ingress (`*.cro.local`) reverse router.

pub mod heuristics;
pub mod router;

pub use heuristics::*;
pub use router::*;
