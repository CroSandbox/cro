// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CLI handler for `microvm run` command.

use anyhow::{bail, Context, Result};
use microvm_core::compose::ComposeProject;
use microvm_core::{detect_kernel_format, parse_kernel_format, MicroVmBuilder, NetworkMode};
use std::path::PathBuf;

use crate::args::RunArgs;
use crate::commands::compose::handle_compose_up;
use crate::utils::{default_data_dir, parse_disk_arg, parse_secret_arg};

pub async fn handle_run(run: RunArgs) -> Result<()> {
    let RunArgs {
        image,
        bundle,
        cpus,
        memory,
        ports,
        volumes,
        artifacts,
        workspace_cow,
        env,
        workdir,
        data_dir,
        no_preflight,
        log_level,
        interactive,
        tty,
        detach,
        no_network,
        net,
        dns,
        hostname,
        mac,
        mtu,
        rlimits,
        dax,
        lazy_load,
        nydus_bootstrap,
        nydus_cache,
        chunk_size,
        gpu,
        gpu_shm_size,
        no_sandbox,
        allow_hosts,
        secrets,
        max_tokens,
        kernel,
        kernel_format,
        initrd,
        cmdline,
        firmware,
        disks,
        shm_size,
        cmd,
        file,
    } = run;

    if let Some(ref manifest_path) = file {
        let base_data_dir = data_dir.clone().unwrap_or_else(default_data_dir);
        let project = ComposeProject::load(manifest_path, Some(base_data_dir.clone()))?;
        return handle_compose_up(&project, manifest_path, &base_data_dir, detach, &Vec::new())
            .await;
    }

    let mut builder = if let Some(ref b) = bundle {
        if !detach {
            println!("📦 Loading MicroVM from OCI bundle: {}", b.display());
        }
        MicroVmBuilder::from_bundle(b)?
    } else if let Some(ref kpath) = kernel {
        if !detach {
            println!("🚀 Direct kernel boot: {}", kpath.display());
        }
        let format_val = if let Some(ref fmt_str) = kernel_format {
            parse_kernel_format(fmt_str).ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid kernel format '{}'. Supported formats: raw, bin, m1n1, elf, vmlinux, gz, image.gz, vmlinuz, bz2, zstd, pe",
                    fmt_str
                )
            })?
        } else {
            detect_kernel_format(kpath)
        };
        MicroVmBuilder::new("").kernel_with_format(kpath.clone(), format_val, initrd, cmdline)
    } else if let Some(ref fpath) = firmware {
        if !detach {
            println!("🚀 UEFI firmware boot: {}", fpath.display());
        }
        MicroVmBuilder::new("").firmware(fpath.clone())
    } else if !image.is_empty() {
        if !detach {
            println!("🚀 Launching MicroVM for image '{}'...", image);
        }
        MicroVmBuilder::new(image)
    } else {
        bail!("Please specify an image reference (e.g. 'alpine:latest'), '--kernel <path>', '--firmware <path>', or '--bundle <path>'");
    };

    for (i, d) in disks.iter().enumerate() {
        let (id, path, ro) = parse_disk_arg(d, i);
        builder = builder.disk(id, path, ro);
    }

    builder = builder
        .cpus(cpus)
        .memory_mb(memory)
        .preflight(!no_preflight)
        .interactive(interactive)
        .tty(tty)
        .detach(detach)
        .no_network(no_network)
        .sandbox(!no_sandbox);

    for h in allow_hosts {
        builder = builder.allow_host(h);
    }

    for s in secrets {
        let (k, v) = parse_secret_arg(&s)?;
        builder = builder.secret(k, v);
    }

    if let Some(tokens) = max_tokens {
        builder = builder.max_tokens(tokens);
    }

    if gpu {
        builder = builder.gpu(true);
        if let Some(shm) = gpu_shm_size {
            let bytes = microvm_core::parse_size_to_bytes(&shm)
                .context("Invalid --gpu-shm-size format (e.g. 2G, 512M)")?;
            builder = builder.gpu_shm_size(bytes);
        }
    }

    if let Some(dax_size) = dax {
        builder = builder.dax_window_size_str(&dax_size)?;
    }

    if lazy_load || nydus_bootstrap.is_some() {
        builder = builder.lazy_load(true);
    }
    if let Some(bootstrap) = nydus_bootstrap {
        builder = builder.nydus_bootstrap(bootstrap);
    }
    if let Some(cache) = nydus_cache {
        builder = builder.chunk_cache_dir(cache);
    }
    if let Some(chunk_sz) = chunk_size {
        builder = builder.chunk_size_str(&chunk_sz)?;
    }

    if let Some(rlim) = rlimits {
        builder = builder.rlimits(rlim);
    }

    if let Some(dd) = data_dir {
        builder = builder.data_dir(dd);
    }

    if let Some(w) = workdir {
        builder = builder.workdir(w);
    }

    if let Some(shm) = shm_size {
        builder = builder.shm_size(shm);
    }

    if let Some(lvl) = log_level {
        builder = builder.log_level(lvl);
    }

    if no_network || net == "none" {
        builder = builder.network_mode(NetworkMode::None);
    } else if net == "gvproxy" {
        builder = builder.network_mode(NetworkMode::Gvproxy);
    } else if let Some(path_str) = net.strip_prefix("unix:") {
        builder = builder.network_mode(NetworkMode::UnixStream(PathBuf::from(path_str)));
    } else if let Some(netns_str) = net.strip_prefix("cni:") {
        builder = builder.network_mode(NetworkMode::Cni {
            netns: PathBuf::from(netns_str),
            socket_path: None,
        });
    } else {
        builder = builder.network_mode(NetworkMode::Tsi);
    }

    if !dns.is_empty() {
        builder = builder.dns_servers(dns);
    }

    if let Some(h) = hostname {
        builder = builder.hostname(h);
    }

    if let Some(m) = mac {
        builder = builder.mac_address(m);
    }

    if let Some(mtu_val) = mtu {
        builder = builder.mtu(mtu_val);
    }

    if !cmd.is_empty() {
        builder = builder.cmd(cmd);
    }

    for p in ports {
        let parts: Vec<&str> = p.split(':').collect();
        if parts.len() != 2 {
            bail!("Invalid port forward format '{}', expected host:guest", p);
        }
        let host: u16 = parts[0].parse().context("Invalid host port")?;
        let guest: u16 = parts[1].parse().context("Invalid guest port")?;
        builder = builder.port_forward(host, guest);
    }

    for v in volumes {
        let parts: Vec<&str> = v.split(':').collect();
        if parts.len() < 2 {
            bail!("Invalid volume format '{}', expected host_path:tag[:ro]", v);
        }
        let host_path = PathBuf::from(parts[0]);
        let tag = parts[1];
        let ro = parts.get(2).is_some_and(|&s| s == "ro");
        builder = builder.virtiofs(tag, host_path, ro);
    }

    for a in artifacts {
        let (rem, ro) = if let Some(stripped) = a.strip_suffix(":ro") {
            (stripped, true)
        } else if let Some(stripped) = a.strip_suffix(":rw") {
            (stripped, false)
        } else {
            (a.as_str(), false)
        };

        let (reference, tag) = match rem.rfind(':') {
            Some(idx) => (&rem[..idx], &rem[idx + 1..]),
            None => bail!(
                "Invalid artifact format '{}', expected <reference>:<tag>[:ro]",
                a
            ),
        };
        builder = builder.attach_artifact(reference, tag, ro);
    }

    for w in workspace_cow {
        let (host_path, tag) = match w.rfind(':') {
            Some(idx) => (&w[..idx], &w[idx + 1..]),
            None => bail!(
                "Invalid workspace-cow format '{}', expected <host_path>:<tag>",
                w
            ),
        };
        builder = builder.workspace_cow(PathBuf::from(host_path), tag);
    }

    for e in env {
        if let Some((k, v)) = e.split_once('=') {
            builder = builder.env(k, v);
        } else {
            bail!("Invalid env var format '{}', expected KEY=VALUE", e);
        }
    }

    let mut vm = builder.run().await.context("Failed to start microVM")?;
    if detach {
        println!("{}", vm.id());
        return Ok(());
    }

    println!("✅ MicroVM active! ID: {}, PID: {:?}", vm.id(), vm.pid());

    // Concurrently wait for VM exit or Ctrl+C signal
    let exit_code = tokio::select! {
        status_res = vm.wait() => {
            let status = status_res?;
            println!("🛑 MicroVM exited with status: {}", status);
            status.code().unwrap_or(0)
        }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\n⚠️ Received interrupt (Ctrl+C). Gracefully stopping microVM {}...", vm.id());
            let _ = vm.stop().await;
            130 // Standard exit code: 128 + SIGINT
        }
    };

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}
