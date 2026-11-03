// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! High-throughput model serving subsystem for `cro serve` and `cro vllm`.

pub mod config;
pub mod gateway;
pub mod manager;

pub use config::{ServeConfig, ServeEngine};
pub use gateway::{
    find_available_loopback_port, GatewayStatsSnapshot, ServeGateway, ServeGatewayHandle,
};
pub use manager::{ServeHandle, ServeManager};
