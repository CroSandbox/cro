// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CLI handlers for AI agent sandboxing, cherry-picking, applying manifests, and unikernels.

use anyhow::{bail, Context, Result};
use microvm_core::compose::ComposeProject;
use microvm_core::{MicroVmBuilder, StateManager};
use std::path::PathBuf;

use crate::commands::compose::handle_compose_up;
use crate::utils::{
    cherry_pick_sandbox_to_host, default_data_dir, get_git_commits_between, get_git_head,
    parse_disk_arg, parse_secret_arg,
};

#[allow(clippy::too_many_arguments)]
pub async fn handle_sandbox(
    agent: String,
    workspace: PathBuf,
    repo: Option<String>,
    cherry_pick: Option<String>,
    apply_to_host: bool,
    image: String,
    cpus: u8,
    memory: u32,
    secrets: Vec<String>,
    allow_hosts: Vec<String>,
    max_tokens: Option<u64>,
    hitl: bool,
    gpu: bool,
    gpu_shm_size: Option<String>,
    shm_size: Option<String>,
    data_dir: Option<PathBuf>,
    cmd: Vec<String>,
) -> Result<()> {
    let abs_ws = std::fs::canonicalize(&workspace).unwrap_or(workspace);
    println!(
        "🛡️  Initializing microVM Sandbox for AI Agent '{}'...",
        agent
    );
    println!("📁 Isolated Workspace: {}", abs_ws.display());

    let mut builder = MicroVmBuilder::new(&image)
        .cpus(cpus)
        .memory_mb(memory)
        .interactive(true)
        .tty(true)
        .workspace_cow(&abs_ws, "workspace")
        .workdir("/workspace");

    let effective_data_dir = data_dir.clone().unwrap_or_else(default_data_dir);
    if hitl {
        let approval_dir = effective_data_dir.join("approvals");
        let store = std::sync::Arc::new(
            microvm_core::net::ApprovalStore::new().with_storage_dir(&approval_dir),
        );
        builder = builder.hitl(true).approval_store(store);
        println!(
            "🔒 Human-in-the-Loop (HITL) Interception: ENABLED (review via 'cro approval list')"
        );
    }

    if let Some(ref dd) = data_dir {
        builder = builder.data_dir(dd.clone());
    }

    // Inject host git credentials so git operations (cherry-pick, commit) succeed
    let host_git_name = std::process::Command::new("git")
        .args(["config", "user.name"])
        .current_dir(&abs_ws)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Sandbox Developer".to_string());

    let host_git_email = std::process::Command::new("git")
        .args(["config", "user.email"])
        .current_dir(&abs_ws)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "developer@libkrun.local".to_string());

    builder = builder
        .env("GIT_AUTHOR_NAME", &host_git_name)
        .env("GIT_AUTHOR_EMAIL", &host_git_email)
        .env("GIT_COMMITTER_NAME", &host_git_name)
        .env("GIT_COMMITTER_EMAIL", &host_git_email);

    // Inject any explicit secrets passed via --secret
    for s in &secrets {
        let (k, v) = parse_secret_arg(s)?;
        builder = builder.secret(k, v);
    }

    // Configure AI Agent specific egress rules and environment
    match agent.to_lowercase().as_str() {
        "claude" => {
            builder = builder
                .allow_host("api.anthropic.com:443")
                .allow_host("cdn.anthropic.com:443")
                .allow_host("github.com:443")
                .allow_host("api.github.com:443")
                .allow_host("registry.npmjs.org:443");
            if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                builder = builder.secret("ANTHROPIC_API_KEY", &key);
            } else if !secrets.iter().any(|s| s.starts_with("ANTHROPIC_API_KEY")) {
                eprintln!("⚠️  ANTHROPIC_API_KEY not found in host environment. Agent may fail to authenticate.");
            }
        }
        "gemini" => {
            builder = builder
                .allow_host("generativelanguage.googleapis.com:443")
                .allow_host("github.com:443")
                .allow_host("api.github.com:443");
            if let Ok(key) = std::env::var("GEMINI_API_KEY") {
                builder = builder.secret("GEMINI_API_KEY", &key);
            } else if !secrets.iter().any(|s| s.starts_with("GEMINI_API_KEY")) {
                eprintln!("⚠️  GEMINI_API_KEY not found in host environment. Agent may fail to authenticate.");
            }
        }
        "codex" | "openai" => {
            builder = builder
                .allow_host("api.openai.com:443")
                .allow_host("github.com:443")
                .allow_host("api.github.com:443");
            if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                builder = builder.secret("OPENAI_API_KEY", &key);
            } else if !secrets.iter().any(|s| s.starts_with("OPENAI_API_KEY")) {
                eprintln!("⚠️  OPENAI_API_KEY not found in host environment. Agent may fail to authenticate.");
            }
        }
        "deepseek" => {
            builder = builder
                .allow_host("api.deepseek.com:443")
                .allow_host("github.com:443")
                .allow_host("api.github.com:443");
            if let Ok(key) = std::env::var("DEEPSEEK_API_KEY") {
                builder = builder.secret("DEEPSEEK_API_KEY", &key);
            } else if !secrets.iter().any(|s| s.starts_with("DEEPSEEK_API_KEY")) {
                eprintln!("⚠️  DEEPSEEK_API_KEY not found in host environment. Agent may fail to authenticate.");
            }
        }
        "openrouter" => {
            builder = builder
                .allow_host("openrouter.ai:443")
                .allow_host("github.com:443")
                .allow_host("api.github.com:443");
            if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
                builder = builder.secret("OPENROUTER_API_KEY", &key);
            } else if !secrets.iter().any(|s| s.starts_with("OPENROUTER_API_KEY")) {
                eprintln!("⚠️  OPENROUTER_API_KEY not found in host environment. Agent may fail to authenticate.");
            }
        }
        "groq" => {
            builder = builder
                .allow_host("api.groq.com:443")
                .allow_host("github.com:443")
                .allow_host("api.github.com:443");
            if let Ok(key) = std::env::var("GROQ_API_KEY") {
                builder = builder.secret("GROQ_API_KEY", &key);
            } else if !secrets.iter().any(|s| s.starts_with("GROQ_API_KEY")) {
                eprintln!("⚠️  GROQ_API_KEY not found in host environment. Agent may fail to authenticate.");
            }
        }
        "mistral" => {
            builder = builder
                .allow_host("api.mistral.ai:443")
                .allow_host("github.com:443")
                .allow_host("api.github.com:443");
            if let Ok(key) = std::env::var("MISTRAL_API_KEY") {
                builder = builder.secret("MISTRAL_API_KEY", &key);
            } else if !secrets.iter().any(|s| s.starts_with("MISTRAL_API_KEY")) {
                eprintln!("⚠️  MISTRAL_API_KEY not found in host environment. Agent may fail to authenticate.");
            }
        }
        _ => {
            builder = builder
                .allow_host("github.com:443")
                .allow_host("api.github.com:443");
        }
    }

    for host in &allow_hosts {
        builder = builder.allow_host(host);
    }

    if let Some(tokens) = max_tokens {
        builder = builder.max_tokens(tokens);
    }

    if gpu {
        builder = builder.gpu(true);
        if let Some(ref shm) = gpu_shm_size {
            let bytes = microvm_core::parse_size_to_bytes(shm)
                .context("Invalid --gpu-shm-size format (e.g. 2G, 512M)")?;
            builder = builder.gpu_shm_size(bytes);
        }
    }

    if let Some(ref shm) = shm_size {
        builder = builder.shm_size(shm);
    }

    let mut script_parts = Vec::new();
    script_parts.push(
        r#"if command -v git >/dev/null 2>&1; then
    git config --global --add safe.directory '*' 2>/dev/null || true
    [ -n "$GIT_AUTHOR_NAME" ] && git config --global user.name "$GIT_AUTHOR_NAME" 2>/dev/null || true
    [ -n "$GIT_AUTHOR_EMAIL" ] && git config --global user.email "$GIT_AUTHOR_EMAIL" 2>/dev/null || true
fi"#.to_string(),
    );

    if let Some(ref git_repo) = repo {
        script_parts.push(format!(
            r#"if [ ! -d .git ]; then
    echo "📦 Cloning {git_repo} into workspace..."
    git clone "{git_repo}" .
    git config --global --add safe.directory '*' 2>/dev/null || true
fi"#
        ));
    }

    if let Some(ref cp_ref) = cherry_pick {
        script_parts.push(format!(
            r#"if command -v git >/dev/null 2>&1 && [ -d .git ]; then
    echo "🍒 Cherry-picking commit '{cp_ref}' in isolated sandbox..."
    if git cherry-pick "{cp_ref}"; then
        echo "✅ Successfully cherry-picked '{cp_ref}' in sandbox!"
    else
        echo "⚠️  git cherry-pick encountered conflicts. The sandbox is ready for inspection/resolution."
    fi
else
    echo "⚠️  Cannot cherry-pick: git is not installed or workspace is not a git repository."
fi"#
        ));
    }

    if !cmd.is_empty() {
        let joined_cmd = cmd
            .iter()
            .map(|s| format!("'{}'", s.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ");
        script_parts.push(format!("exec {joined_cmd}"));
    } else {
        script_parts.push("exec /bin/sh".to_string());
    }

    let startup_script = script_parts.join("\n");
    builder = builder.cmd(vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        startup_script,
    ]);

    let mut vm = builder.run().await.context("Failed to start sandbox")?;
    let status = vm.wait().await?;
    println!("🛑 Sandbox session closed with status: {}", status);

    let base_dir = data_dir.unwrap_or_else(default_data_dir);
    let cow_ws = base_dir
        .join("instances")
        .join(vm.id())
        .join("workspaces")
        .join("workspace");

    if abs_ws.join(".git").exists() && cow_ws.join(".git").exists() {
        let host_head = get_git_head(&abs_ws);
        let sandbox_head = get_git_head(&cow_ws);

        if let (Some(ref h_head), Some(ref s_head)) = (&host_head, &sandbox_head) {
            if h_head != s_head {
                let commits = get_git_commits_between(&cow_ws, h_head, s_head);
                if !commits.is_empty() {
                    println!("\n📦 Sandbox produced {} new git commit(s):", commits.len());
                    for c in &commits {
                        println!("   • {c}");
                    }

                    if apply_to_host {
                        println!("🚀 Cherry-picking sandbox commit(s) to host workspace...");
                        match cherry_pick_sandbox_to_host(&cow_ws, &abs_ws, h_head, s_head) {
                            Ok(_) => {
                                println!(
                                    "✅ Successfully cherry-picked sandbox changes to host branch!"
                                )
                            }
                            Err(e) => eprintln!("⚠️ Failed to cherry-pick to host: {e}"),
                        }
                    } else {
                        println!("\n💡 Tip: To apply these commits to your host branch, run:");
                        println!("   microvm cherry-pick {}", vm.id());
                    }
                }
            }
        }
    }

    if status.code().unwrap_or(0) != 0 {
        std::process::exit(status.code().unwrap_or(0));
    }
    Ok(())
}

pub async fn handle_cherry_pick(
    id: String,
    workspace: PathBuf,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let abs_host_ws = std::fs::canonicalize(&workspace).unwrap_or(workspace);

    let vm = match StateManager::find(&base, &id)? {
        Some(v) => v,
        None => bail!("MicroVM instance '{}' not found in state", id),
    };

    let cow_ws = vm.instance_dir.join("workspaces").join("workspace");
    if !cow_ws.exists() {
        bail!(
            "No Copy-on-Write workspace found for microVM '{}' at {}",
            id,
            cow_ws.display()
        );
    }

    let host_head = get_git_head(&abs_host_ws)
        .context("Host workspace is not a valid git repository (no HEAD found)")?;
    let sandbox_head =
        get_git_head(&cow_ws).context("Sandbox workspace is not a valid git repository")?;

    if host_head == sandbox_head {
        println!(
            "No new commits in sandbox '{}' compared to host HEAD ({})",
            id,
            &host_head[..8.min(host_head.len())]
        );
        return Ok(());
    }

    let commits = get_git_commits_between(&cow_ws, &host_head, &sandbox_head);
    println!(
        "🍒 Cherry-picking {} commit(s) from sandbox '{}' to host:",
        commits.len(),
        id
    );
    for c in &commits {
        println!("   • {c}");
    }

    cherry_pick_sandbox_to_host(&cow_ws, &abs_host_ws, &host_head, &sandbox_head)?;
    println!("✅ Successfully cherry-picked sandbox commits to host workspace!");
    Ok(())
}

pub async fn handle_apply(file: PathBuf, detach: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let base_data_dir = data_dir.unwrap_or_else(default_data_dir);
    if !file.exists() {
        bail!("Manifest file '{}' does not exist", file.display());
    }
    let project = ComposeProject::load(&file, Some(base_data_dir.clone()))?;
    handle_compose_up(&project, &file, &base_data_dir, detach, &[]).await?;
    Ok(())
}

pub async fn handle_unikernel(
    kernel: PathBuf,
    cpus: u8,
    memory: u32,
    cmdline: Option<String>,
    disks: Vec<String>,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let mut builder = MicroVmBuilder::new("")
        .cpus(cpus)
        .memory_mb(memory)
        .unikernel(kernel, cmdline)
        .interactive(true)
        .tty(true);

    if let Some(dd) = data_dir {
        builder = builder.data_dir(dd);
    }

    for (i, d) in disks.iter().enumerate() {
        let (id, path, ro) = parse_disk_arg(d, i);
        builder = builder.disk(id, path, ro);
    }

    println!(
        "🚀 Launching unikernel in microVM ({} vCPUs, {} MiB RAM)...",
        cpus, memory
    );
    let mut vm = builder.run().await.context("Failed to boot unikernel")?;
    let status = vm.wait().await?;
    println!("🛑 Unikernel exited with status: {}", status);
    if status.code().unwrap_or(0) != 0 {
        std::process::exit(status.code().unwrap_or(0));
    }
    Ok(())
}
