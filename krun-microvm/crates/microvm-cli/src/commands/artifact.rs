// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Detached OCI artifact (models, datasets, toolchains) pull and cache commands.

use crate::args::ArtifactCommands;
use crate::utils::default_data_dir;
use anyhow::Result;
use microvm_core::{ImageReference, OciArtifact, OciClient};

pub async fn handle_artifact_command(command: ArtifactCommands) -> Result<()> {
    match command {
        ArtifactCommands::Pull { artifact, data_dir } => {
            let cache_base = data_dir.unwrap_or_else(default_data_dir);
            let reference = ImageReference::parse(&artifact)?;
            let client = OciClient::new();
            let dest = client.pull_artifact(&reference, &cache_base).await?;
            println!("✅ OCI artifact pulled and cached successfully!");
            println!("   Artifact directory: {}", dest.display());
        }
        ArtifactCommands::List { data_dir } => {
            let cache_base = data_dir.unwrap_or_else(default_data_dir);
            let artifacts = OciArtifact::list_cached(&cache_base)?;
            if artifacts.is_empty() {
                println!("No locally cached OCI artifacts found.");
                return Ok(());
            }

            println!(
                "{:<40} {:<20} {:<12} {:<20}",
                "REFERENCE", "DIGEST", "FILES", "PULLED AT"
            );
            println!("{:-<95}", "");
            for a in artifacts {
                let short_digest = if a.digest.len() > 19 {
                    format!("{}...", &a.digest[..16])
                } else {
                    a.digest.clone()
                };
                let time_str = match std::time::UNIX_EPOCH
                    .checked_add(std::time::Duration::from_secs(a.created_at))
                {
                    Some(t) => {
                        let dur = std::time::SystemTime::now()
                            .duration_since(t)
                            .unwrap_or_default();
                        if dur.as_secs() < 60 {
                            format!("{}s ago", dur.as_secs())
                        } else if dur.as_secs() < 3600 {
                            format!("{}m ago", dur.as_secs() / 60)
                        } else if dur.as_secs() < 86400 {
                            format!("{}h ago", dur.as_secs() / 3600)
                        } else {
                            format!("{}d ago", dur.as_secs() / 86400)
                        }
                    }
                    None => "Unknown".to_string(),
                };
                println!(
                    "{:<40} {:<20} {:<12} {:<20}",
                    a.reference,
                    short_digest,
                    format!("{} file(s)", a.files.len()),
                    time_str,
                );
            }
        }
    }
    Ok(())
}
