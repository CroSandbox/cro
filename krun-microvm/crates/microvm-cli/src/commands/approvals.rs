// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Human-in-the-Loop (HITL) approval review and decision command handlers.

use crate::args::ApprovalCommands;
use crate::utils::default_data_dir;
use anyhow::Result;

pub async fn handle_approval_command(command: ApprovalCommands) -> Result<()> {
    match command {
        ApprovalCommands::List {
            all,
            json,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let approval_dir = base_data_dir.join("approvals");

            if all {
                let store = microvm_core::net::ApprovalStore::new().with_storage_dir(&approval_dir);
                let history = store.get_history().await;
                let pending = microvm_core::net::ApprovalStore::cli_list_pending(&approval_dir)
                    .unwrap_or_default();
                let mut combined = pending;
                combined.extend(history);

                if json {
                    println!("{}", serde_json::to_string_pretty(&combined)?);
                } else if combined.is_empty() {
                    println!("No pending or historical approval requests found.");
                } else {
                    println!(
                        "{:<14} {:<12} {:<8} {:<28} {:<10} SUMMARY",
                        "ID", "AGENT/VM", "METHOD", "DESTINATION", "STATUS"
                    );
                    println!("{}", "-".repeat(90));
                    for item in &combined {
                        let status_str = format!("{:?}", item.status);
                        let dest = format!("{}:{}", item.host, item.port);
                        println!(
                            "{:<14} {:<12} {:<8} {:<28} {:<10} {}",
                            item.id,
                            item.agent_or_vm_id,
                            item.method,
                            dest,
                            status_str,
                            item.summary
                        );
                    }
                }
            } else {
                let pending = microvm_core::net::ApprovalStore::cli_list_pending(&approval_dir)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&pending)?);
                } else if pending.is_empty() {
                    println!("✨ No pending requests awaiting approval.");
                } else {
                    println!(
                        "{:<14} {:<12} {:<8} {:<28} SUMMARY",
                        "ID", "AGENT/VM", "METHOD", "DESTINATION"
                    );
                    println!("{}", "-".repeat(80));
                    for item in &pending {
                        let dest = format!("{}:{}", item.host, item.port);
                        println!(
                            "{:<14} {:<12} {:<8} {:<28} {}",
                            item.id, item.agent_or_vm_id, item.method, dest, item.summary
                        );
                    }
                    println!("\nTo approve: cro approval approve <id>");
                    println!("To deny:    cro approval deny <id>");
                }
            }
        }
        ApprovalCommands::Approve { id, data_dir } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let approval_dir = base_data_dir.join("approvals");
            microvm_core::net::ApprovalStore::cli_decide(&approval_dir, &id, true)?;
            println!(
                "✅ Approved request '{}'! Intercepted connection resumed.",
                id
            );
        }
        ApprovalCommands::Deny {
            id,
            reason,
            data_dir,
        } => {
            let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
            let approval_dir = base_data_dir.join("approvals");
            microvm_core::net::ApprovalStore::cli_decide(&approval_dir, &id, false)?;
            if let Some(r) = reason {
                println!(
                    "❌ Denied request '{}' (Reason: {}). Connection terminated with HTTP 403.",
                    id, r
                );
            } else {
                println!(
                    "❌ Denied request '{}'. Connection terminated with HTTP 403.",
                    id
                );
            }
        }
    }
    Ok(())
}
