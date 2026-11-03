// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Automatic local domain ingress (`*.cro.local`) reverse router and embedded Micro-DNS.

pub mod dns;
pub mod heuristics;
pub mod router;

pub use dns::*;
pub use heuristics::*;
pub use router::*;
