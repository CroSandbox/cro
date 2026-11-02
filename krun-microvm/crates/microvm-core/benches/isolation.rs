// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! MicroVM Isolation and Zero-Trust Proxy Micro-benchmarks.
//!
//! Measures:
//! - Egress Policy evaluation throughput (wildcard, exact, and default-deny)
//! - In-flight Secret Substitution buffer throughput
//! - Cryptographic approval payload digest computation latency

use microvm_core::net::approval::compute_payload_digest;
use microvm_core::net::egress::{EgressPolicy, SecretSubstitution};
use microvm_core::secrets::CredentialScope;
use std::time::Instant;

fn main() {
    println!("\n=======================================================");
    println!("   cro (microvm-core): Isolation & Zero-Trust Benchmarks");
    println!("=======================================================\n");

    bench_egress_policy_evaluation();
    bench_secret_substitution_throughput();
    bench_approval_digest_computation();

    println!("\nAll isolation benchmarks completed successfully.\n");
}

fn bench_egress_policy_evaluation() {
    let policy = EgressPolicy::default_deny().with_default_deny(true);
    let mut policy = policy;
    policy.allow_hosts.push("api.openai.com:443".to_string());
    policy.allow_hosts.push("*.github.com:443".to_string());
    policy.allow_hosts.push("10.0.0.0/8".to_string());

    let iterations = 200_000;
    let start = Instant::now();

    for _ in 0..iterations {
        let _ = policy.is_allowed("api.openai.com", 443);
        let _ = policy.is_allowed("raw.github.com", 443);
        let _ = policy.is_allowed("evil-exfil.com", 80);
    }

    let elapsed = start.elapsed();
    let total_checks = iterations * 3;
    let ns_per_op = elapsed.as_nanos() as f64 / total_checks as f64;
    let ops_per_sec = total_checks as f64 / elapsed.as_secs_f64();

    println!("⚡ Egress Policy Evaluation (Allowed & Blocked Destinations):");
    println!("   Total Checks:     {}", total_checks);
    println!("   Total time:       {:.2?}", elapsed);
    println!("   Latency / check:  {:.2} ns", ns_per_op);
    println!("   Throughput:       {:.2} checks/sec\n", ops_per_sec);
}

fn bench_secret_substitution_throughput() {
    let secrets = vec![
        (
            "OPENAI_KEY".to_string(),
            "mock-openai-key-sample-12345".to_string(),
        ),
        (
            "STRIPE_KEY".to_string(),
            "mock-stripe-secret-key-99999".to_string(),
        ),
    ];
    let scope = CredentialScope::new("OPENAI_KEY")
        .with_host("api.openai.com")
        .with_method("POST");

    let engine = SecretSubstitution::new(&secrets).with_scope(scope);

    let http_payload = b"POST /v1/chat/completions HTTP/1.1\r\n\
Host: api.openai.com\r\n\
Authorization: Bearer krun-secret:OPENAI_KEY\r\n\
Content-Type: application/json\r\n\
Content-Length: 54\r\n\r\n\
{\"model\":\"gpt-4o\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}";

    let iterations = 100_000;
    let total_bytes = http_payload.len() * iterations;
    let start = Instant::now();

    for _ in 0..iterations {
        let _ = engine.substitute_authorized(
            http_payload,
            "POST",
            "api.openai.com",
            "/v1/chat/completions",
        );
    }

    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    let mb_per_sec = (total_bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();

    println!("⚡ In-Flight Secret Substitution with Scope Authorization:");
    println!("   Iterations:       {}", iterations);
    println!("   Total time:       {:.2?}", elapsed);
    println!("   Latency / op:     {:.2} ns", ns_per_op);
    println!("   Throughput:       {:.2} MB/s\n", mb_per_sec);
}

fn bench_approval_digest_computation() {
    let payload =
        b"{\"action\":\"execute_transaction\",\"amount\":50000,\"recipient\":\"account_123\"}";
    let iterations = 150_000;
    let start = Instant::now();

    for _ in 0..iterations {
        let _ = compute_payload_digest("POST", "bank.internal", "/api/transfer", payload);
    }

    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("⚡ HITL Cryptographic Payload Digest Binding (SHA-256):");
    println!("   Iterations:       {}", iterations);
    println!("   Total time:       {:.2?}", elapsed);
    println!("   Latency / digest: {:.2} ns", ns_per_op);
    println!("   Throughput:       {:.2} digests/sec\n", ops_per_sec);
}
