// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Subcommand handler modules and centralized command dispatch for microvm-cli.

use crate::args::Commands;
use anyhow::Result;

pub mod approvals;
pub mod artifact;
pub mod compose;
pub mod containerd;
pub mod kube;
pub mod lifecycle;
pub mod metrics;
pub mod network;
pub mod run;
pub mod sandbox;
pub mod system;

/// Dispatches parsed CLI commands to their respective dedicated handler modules.
pub async fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Run(run_args) => run::handle_run(*run_args).await?,

        Commands::Unikernel {
            kernel,
            cpus,
            memory,
            cmdline,
            disks,
            data_dir,
        } => sandbox::handle_unikernel(kernel, cpus, memory, cmdline, disks, data_dir).await?,

        Commands::Sandbox {
            agent,
            workspace,
            repo,
            cherry_pick,
            apply_to_host,
            image,
            cpus,
            memory,
            secrets,
            allow_hosts,
            max_tokens,
            hitl,
            gpu,
            gpu_shm_size,
            shm_size,
            data_dir,
            cmd,
        } => {
            sandbox::handle_sandbox(
                agent,
                workspace,
                repo,
                cherry_pick,
                apply_to_host,
                image,
                cpus,
                memory,
                secrets,
                allow_hosts,
                max_tokens,
                hitl,
                gpu,
                gpu_shm_size,
                shm_size,
                data_dir,
                cmd,
            )
            .await?
        }

        Commands::CherryPick {
            id,
            workspace,
            data_dir,
        } => sandbox::handle_cherry_pick(id, workspace, data_dir).await?,

        Commands::Apply {
            file,
            detach,
            data_dir,
        } => sandbox::handle_apply(file, detach, data_dir).await?,

        Commands::Ps {
            all,
            no_trunc,
            data_dir,
        } => lifecycle::handle_ps(all, no_trunc, data_dir)?,

        Commands::Exec {
            id,
            env,
            workdir,
            tty,
            data_dir,
            cmd,
        } => system::handle_exec(id, env, workdir, tty, data_dir, cmd).await?,

        Commands::Stop { id, data_dir } => lifecycle::handle_stop(id, data_dir)?,

        Commands::Pause { id, data_dir } => lifecycle::handle_pause(id, data_dir)?,

        Commands::Resume { id, data_dir } => lifecycle::handle_resume(id, data_dir)?,

        Commands::Rm {
            id,
            force,
            data_dir,
        } => lifecycle::handle_rm(id, force, data_dir)?,

        Commands::Inspect { id, json, data_dir } => lifecycle::handle_inspect(id, json, data_dir)?,

        Commands::Stats {
            id,
            no_stream,
            json,
            data_dir,
        } => system::handle_stats(id, no_stream, json, data_dir).await?,

        Commands::Top { id, data_dir } => lifecycle::handle_top(id, data_dir)?,

        Commands::Cp {
            src,
            dest,
            data_dir,
        } => lifecycle::handle_cp(src, dest, data_dir)?,

        Commands::Logs {
            id,
            follow,
            data_dir,
        } => lifecycle::handle_logs(id, follow, data_dir).await?,

        Commands::Prune { data_dir } => lifecycle::handle_prune(data_dir)?,

        Commands::Pull { image, data_dir } => system::handle_pull(image, data_dir).await?,

        Commands::Preflight { port } => system::handle_preflight(port)?,

        Commands::Info => system::handle_info()?,

        Commands::Artifact { command } => artifact::handle_artifact_command(command).await?,

        Commands::Snapshot {
            id,
            output,
            data_dir,
        } => system::handle_snapshot(id, output, data_dir)?,

        Commands::Restore {
            snapshot,
            name,
            data_dir,
        } => system::handle_restore(snapshot, name, data_dir)?,

        Commands::Resize {
            id,
            memory,
            cpus,
            data_dir,
        } => lifecycle::handle_resize(id, memory, cpus, data_dir)?,

        Commands::Metrics {
            listen,
            json,
            data_dir,
        } => metrics::handle_metrics_command(listen, json, data_dir).await?,

        Commands::Containerd { command } => containerd::handle_containerd_command(command)?,

        Commands::Compose { command } => compose::handle_compose_command(command).await?,

        Commands::Network(net_cmd) => network::handle_network_command(net_cmd).await?,

        Commands::Build {
            tag,
            file,
            no_cache,
            context,
            data_dir,
        } => system::handle_build(tag, file, no_cache, context, data_dir).await?,

        Commands::Kube { command } => kube::handle_kube_command(command).await?,

        Commands::Approval { command } => approvals::handle_approval_command(command).await?,
    }

    Ok(())
}
