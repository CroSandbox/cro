// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Command-line interface for the microvm lightweight virtualization system.

use anyhow::Result;
use clap::Parser;

mod args;
mod commands;
#[cfg(test)]
mod tests;
mod utils;

use args::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    commands::dispatch(cli.command).await
}
