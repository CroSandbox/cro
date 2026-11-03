// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CNCF ModelPack CLI commands (pull, list, info, pack).

use crate::args::ModelCommands;
use crate::utils::{default_data_dir, truncate_str};
use anyhow::{Context, Result};
use microvm_core::{
    modelpack::{HardwareFit, ModelDiscovery},
    ImageReference, ModelEngine, ModelFormat, ModelPackOptions, ModelPacker, OciClient,
};
use std::path::PathBuf;

pub async fn handle_model_command(command: ModelCommands) -> Result<()> {
    match command {
        ModelCommands::Pull { model, data_dir } => {
            let cache_base = data_dir.unwrap_or_else(default_data_dir);
            let reference = ImageReference::parse(&model)?;
            let client = OciClient::new();

            println!(
                "⏳ Pulling CNCF ModelPack model '{}'...",
                reference.canonical_name()
            );
            let dest = client.pull_model(&reference, &cache_base).await?;
            println!("✅ CNCF ModelPack model pulled and cached successfully!");
            println!("   Location: {}", dest.display());
        }

        ModelCommands::List { json, data_dir } => {
            let cache_base = data_dir.unwrap_or_else(default_data_dir);
            let models = ModelEngine::list_cached(&cache_base)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&models)?);
                return Ok(());
            }

            if models.is_empty() {
                println!("No locally cached CNCF ModelPack models found.");
                println!("Pull one using: cro model pull <registry/model:tag>");
                return Ok(());
            }

            println!(
                "{:<24} {:<12} {:<14} {:<12} {:<10} {:<12} {:<30}",
                "NAME", "FORMAT", "ARCHITECTURE", "PRECISION", "CONTEXT", "SIZE", "REFERENCE"
            );
            println!("{:-<120}", "");

            for m in models {
                let arch = m.architecture.as_deref().unwrap_or("-");
                let prec = m.precision.as_deref().unwrap_or("-");
                let ctx = m
                    .context_length
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let size_str = format_bytes(m.total_size);

                println!(
                    "{:<24} {:<12} {:<14} {:<12} {:<10} {:<12} {:<30}",
                    m.name,
                    m.format.to_string(),
                    arch,
                    prec,
                    ctx,
                    size_str,
                    m.reference
                );
            }
        }

        ModelCommands::Info {
            model,
            json,
            data_dir,
        } => {
            let cache_base = data_dir.unwrap_or_else(default_data_dir);
            let info = ModelEngine::inspect_model(&model, &cache_base)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
                return Ok(());
            }

            println!("📦 ModelPack: {}", info.name);
            println!("   Reference:     {}", info.reference);
            println!("   Digest:        {}", info.digest);
            println!("   Format:        {}", info.format);
            println!(
                "   Architecture:  {}",
                info.architecture.as_deref().unwrap_or("-")
            );
            println!(
                "   Precision:     {}",
                info.precision.as_deref().unwrap_or("-")
            );
            println!(
                "   Context Len:   {}",
                info.context_length
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
            println!("   Total Size:    {}", format_bytes(info.total_size));
            println!("   Path:          {}", info.path.display());
            println!("   Files ({}):", info.files.len());
            for f in &info.files {
                println!(
                    "     - {:<30} [{:?}, {}]",
                    f.path,
                    f.role,
                    format_bytes(f.size)
                );
            }
        }

        ModelCommands::Pack {
            source,
            output,
            name,
            format,
            version,
            architecture,
            precision,
            context_length,
            tag,
        } => {
            let fmt: ModelFormat = format
                .parse()
                .map_err(|e: std::convert::Infallible| anyhow::anyhow!("{:?}", e))?;

            let options = ModelPackOptions {
                source_dir: source.clone(),
                output_dir: output.clone(),
                name: name.clone(),
                format: fmt,
                version,
                architecture,
                precision,
                context_length,
                tag,
            };

            println!(
                "⏳ Packaging ModelPack '{}' from {}...",
                name,
                source.display()
            );
            let manifest = ModelPacker::pack(&options)
                .with_context(|| format!("Failed to package model from {}", source.display()))?;

            println!("✅ ModelPack packaged successfully!");
            println!("   Config digest:   {}", manifest.config.digest);
            println!("   Total layers:    {}", manifest.layers.len());
            println!("   OCI layout at:   {}", output.display());
        }

        ModelCommands::Discover {
            custom_paths,
            json,
            data_dir,
        } => {
            let cache_base = data_dir.unwrap_or_else(default_data_dir);
            let models = ModelDiscovery::discover(&cache_base, &custom_paths);
            if json {
                println!("{}", serde_json::to_string_pretty(&models)?);
                return Ok(());
            }

            if models.is_empty() {
                println!("🔍 No local models found across standard directories.");
                println!("Scanned locations: Hugging Face hub, LM Studio, Ollama, ~/models, and cro cache.");
                println!("Pass custom search paths with: cro model discover --path <dir>");
                return Ok(());
            }

            println!(
                "{:<32} {:<14} {:<12} {:<18} LOCATION",
                "NAME", "FORMAT", "SIZE", "ECOSYSTEM"
            );
            println!("{:-<110}", "");
            for m in models {
                println!(
                    "{:<32} {:<14} {:<12} {:<18} {}",
                    truncate_str(&m.name, 30),
                    m.format,
                    format_bytes(m.size_bytes),
                    m.source,
                    m.path.display()
                );
            }
        }

        ModelCommands::Fit {
            model,
            context_length,
            json,
        } => {
            let path = PathBuf::from(&model);
            let fit = HardwareFit::profile(&path, Some(context_length))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&fit)?);
                return Ok(());
            }

            println!("Hardware Memory Profiling & Fit Analysis");
            println!("{:-<60}", "");
            println!("Model:                  {}", model);
            println!(
                "Model Weights Size:     {}",
                format_bytes(fit.model_size_bytes)
            );
            println!("Target Context:         {} tokens", fit.context_length);
            println!(
                "Estimated KV Cache:     {}",
                format_bytes(fit.estimated_kv_cache_bytes)
            );
            println!(
                "Runtime Overhead:       {}",
                format_bytes(fit.runtime_overhead_bytes)
            );
            println!(
                "Total Memory Needed:    {}",
                format_bytes(fit.total_required_bytes)
            );
            println!("{:-<60}", "");
            println!(
                "Host Total Memory:      {}",
                format_bytes(fit.host_total_bytes)
            );
            println!(
                "Host Available RAM:     {}",
                format_bytes(fit.host_available_bytes)
            );
            println!("{:-<60}", "");
            println!(
                "Recommended Guest RAM:  {} MiB",
                fit.recommended_guest_ram_mib
            );
            println!(
                "Recommended GPU vRAM:   {} GiB (--gpu-shm-size)",
                fit.recommended_gpu_shm_gib
            );
            println!("{:-<60}", "");
            println!("Suitability Verdict:    {}", fit.verdict);
        }
    }

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
