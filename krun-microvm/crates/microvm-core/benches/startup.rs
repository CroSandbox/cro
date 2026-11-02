// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! MicroVM Startup and Initialization Micro-benchmarks.
//!
//! Measures:
//! - Configuration building and validation latency
//! - Capability policy evaluation throughput
//! - Safe path containment and lexical normalization latency

use microvm_core::builder::executor::safe_normalize_relative;
use microvm_core::capability::{CapabilityPolicy, FsAccessMode};
use microvm_core::vm::MicroVmBuilder;
use std::path::Path;
use std::time::Instant;

fn main() {
    println!("\n=======================================================");
    println!("   cro (microvm-core): Startup & Containment Benchmarks ");
    println!("=======================================================\n");

    bench_config_instantiation();
    bench_capability_policy_evaluation();
    bench_path_containment_normalization();

    println!("\nAll startup benchmarks completed successfully.\n");
}

fn bench_config_instantiation() {
    let iterations = 100_000;
    let start = Instant::now();

    for _ in 0..iterations {
        let _builder = MicroVmBuilder::new("alpine:latest")
            .cpus(4)
            .memory_mb(1024)
            .workdir("/workspace")
            .allow_host("api.openai.com:443")
            .default_deny(true);
    }

    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("⚡ MicroVmBuilder Instantiation & Security Config:");
    println!("   Iterations:       {}", iterations);
    println!("   Total time:       {:.2?}", elapsed);
    println!("   Latency / op:     {:.2} ns", ns_per_op);
    println!("   Throughput:       {:.2} ops/sec\n", ops_per_sec);
}

fn bench_capability_policy_evaluation() {
    let policy = CapabilityPolicy::hardened_agent_sandbox();
    let iterations = 200_000;
    let test_path = Path::new("/workspace/src/lib.rs");
    let test_cmd = ["python3".to_string(), "script.py".to_string()];

    let start = Instant::now();

    for _ in 0..iterations {
        let _ = policy.authorize_fs(test_path, FsAccessMode::Read);
        let _ = policy.authorize_exec(&test_cmd);
    }

    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("⚡ Capability Policy Dual Check (FS Read + Process Exec):");
    println!("   Iterations:       {}", iterations);
    println!("   Total time:       {:.2?}", elapsed);
    println!("   Latency / op:     {:.2} ns", ns_per_op);
    println!("   Throughput:       {:.2} checks/sec\n", ops_per_sec);
}

fn bench_path_containment_normalization() {
    let iterations = 250_000;
    let raw_path = Path::new("./app/../app/src/components/../../src/main.rs");

    let start = Instant::now();

    for _ in 0..iterations {
        let _ = safe_normalize_relative(raw_path);
    }

    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("⚡ Dockerfile COPY/ADD Path Containment Normalization:");
    println!("   Iterations:       {}", iterations);
    println!("   Total time:       {:.2?}", elapsed);
    println!("   Latency / op:     {:.2} ns", ns_per_op);
    println!("   Throughput:       {:.2} ops/sec\n", ops_per_sec);
}
