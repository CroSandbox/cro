// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Subcommand handlers for model inference serving (`cro serve` and `cro vllm`).

use crate::args::{ServeArgs, VllmCommands};
use anyhow::Result;
use microvm_core::{parse_size_to_bytes, ServeConfig, ServeManager};

pub async fn handle_serve(args: ServeArgs) -> Result<()> {
    let mut config = ServeConfig::new(args.model);
    config.engine = args
        .engine
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{}", e))?;
    config.host_port = args.port;
    config.host = args.host;
    config.container_image = args.image;
    config.cpus = args.cpus;
    config.memory_mb = args.memory;
    config.gpu = args.gpu;
    config.gpu_shm_size = args.gpu_shm_size;
    config.dax_window_size = args.dax.map(|s| parse_size_to_bytes(&s)).transpose()?;
    config.max_model_len = args.max_model_len;
    config.tensor_parallel_size = args.tensor_parallel_size;
    config.enforce_eager = args.enforce_eager;
    config.api_key = args.api_key;
    config.served_model_name = args.served_model_name;
    config.plugins = args.plugins;
    config.vllm_plugins = args.vllm_plugins;
    config.trust_remote_code = args.trust_remote_code;
    config.quantization = args.quantization;
    config.chat_template = args.chat_template;
    for env_str in args.env {
        if let Some((k, v)) = env_str.split_once('=') {
            config.env_vars.push((k.to_string(), v.to_string()));
        } else {
            config.env_vars.push((env_str, String::new()));
        }
    }
    config.data_dir = args.data_dir;
    config.extra_args = args.extra_args;
    config.snapshot_on_ready = args.snapshot_on_ready;
    config.warm = args.warm;
    config.warmup_snapshot_tag = args.warmup_snapshot_tag;
    config.scale_to_zero = args.scale_to_zero;
    if args.scale_to_zero || args.idle_timeout != 60 {
        config.idle_timeout_secs = Some(args.idle_timeout);
        config.scale_to_zero = true;
    }

    if args.no_wait && args.snapshot_on_ready {
        anyhow::bail!(
            "--snapshot-on-ready requires probing readiness before capturing snapshot; cannot be used with --no-wait"
        );
    }

    let wait = args.snapshot_on_ready || (!args.no_wait && args.wait);
    let mut handle = ServeManager::start(&config, args.detach, wait).await?;

    if !args.detach {
        handle.wait_with_sigint().await?;
    }

    Ok(())
}

pub async fn handle_vllm(command: VllmCommands) -> Result<()> {
    match command {
        VllmCommands::Serve(mut args) => {
            args.engine = "vllm".to_string();
            handle_serve(*args).await
        }
    }
}
