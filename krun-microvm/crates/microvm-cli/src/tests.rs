// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

#![cfg(test)]

use clap::Parser;
use std::path::PathBuf;

use crate::args::*;
use crate::utils::*;
#[test]
fn test_cli_parse_run_artifact_and_cow() {
    let args = vec![
        "microvm",
        "run",
        "--artifact",
        "ghcr.io/owner/model:v1:weights:ro",
        "--workspace-cow",
        "/tmp/test-project:workspace",
        "alpine:latest",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert_eq!(run.artifacts, vec!["ghcr.io/owner/model:v1:weights:ro"]);
            assert_eq!(run.workspace_cow, vec!["/tmp/test-project:workspace"]);
            assert_eq!(run.image, "alpine:latest");
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_run_rosetta_and_platform() {
    let args = vec![
        "cro",
        "run",
        "--rosetta",
        "--platform",
        "linux/amd64",
        "alpine:latest",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert!(run.rosetta);
            assert_eq!(run.platform.as_deref(), Some("linux/amd64"));
            assert_eq!(run.image, "alpine:latest");
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_domain_subcommands() {
    // 1. Start
    let start_args = vec!["cro", "domain", "start", "--port", "8080", "-d"];
    let cli = Cli::try_parse_from(start_args).unwrap();
    match cli.command {
        Commands::Domain {
            command: DomainCommands::Start { port, detach, .. },
        } => {
            assert_eq!(port, 8080);
            assert!(detach);
        }
        _ => panic!("Expected Commands::Domain::Start"),
    }

    // 2. List
    let list_args = vec!["cro", "domain", "list", "--json"];
    let cli = Cli::try_parse_from(list_args).unwrap();
    match cli.command {
        Commands::Domain {
            command: DomainCommands::List { json, .. },
        } => {
            assert!(json);
        }
        _ => panic!("Expected Commands::Domain::List"),
    }

    // 3. Stop
    let stop_args = vec!["cro", "domain", "stop"];
    let cli = Cli::try_parse_from(stop_args).unwrap();
    match cli.command {
        Commands::Domain {
            command: DomainCommands::Stop { .. },
        } => {}
        _ => panic!("Expected Commands::Domain::Stop"),
    }

    // 4. SetupResolver
    let setup_args = vec!["cro", "domain", "setup-resolver", "--dns-port", "5354"];
    let cli_setup = Cli::try_parse_from(setup_args).unwrap();
    match cli_setup.command {
        Commands::Domain {
            command: DomainCommands::SetupResolver { dns_port },
        } => {
            assert_eq!(dns_port, 5354);
        }
        _ => panic!("Expected Commands::Domain::SetupResolver"),
    }

    // 5. Resolve
    let resolve_args = vec!["cro", "domain", "resolve", "llama.cro.local"];
    let cli_res = Cli::try_parse_from(resolve_args).unwrap();
    match cli_res.command {
        Commands::Domain {
            command: DomainCommands::Resolve { domain, dns_port },
        } => {
            assert_eq!(domain, "llama.cro.local");
            assert_eq!(dns_port, 5354);
        }
        _ => panic!("Expected Commands::Domain::Resolve"),
    }
}

#[test]
fn test_cli_parse_machine_subcommands() {
    // 1. Create
    let create_args = vec![
        "cro",
        "machine",
        "create",
        "dev",
        "--distro",
        "ubuntu",
        "-c",
        "4",
        "-m",
        "4096",
        "--rosetta",
    ];
    let cli = Cli::try_parse_from(create_args).unwrap();
    match cli.command {
        Commands::Machine {
            command:
                MachineCommands::Create {
                    name,
                    distro,
                    cpus,
                    memory,
                    rosetta,
                    ..
                },
        } => {
            assert_eq!(name, "dev");
            assert_eq!(distro, "ubuntu");
            assert_eq!(cpus, Some(4));
            assert_eq!(memory, Some(4096));
            assert!(rosetta);
        }
        _ => panic!("Expected Commands::Machine::Create"),
    }

    // 2. Shell with trailing command
    let shell_args = vec!["cro", "machine", "shell", "dev", "--", "uname", "-m"];
    let cli = Cli::try_parse_from(shell_args).unwrap();
    match cli.command {
        Commands::Machine {
            command: MachineCommands::Shell { name, cmd, .. },
        } => {
            assert_eq!(name, "dev");
            assert_eq!(cmd, vec!["uname", "-m"]);
        }
        _ => panic!("Expected Commands::Machine::Shell"),
    }

    // 3. List
    let list_args = vec!["cro", "machine", "list", "--json"];
    let cli = Cli::try_parse_from(list_args).unwrap();
    match cli.command {
        Commands::Machine {
            command: MachineCommands::List { json, .. },
        } => {
            assert!(json);
        }
        _ => panic!("Expected Commands::Machine::List"),
    }

    // 4. Start
    let start_args = vec!["cro", "machine", "start", "dev"];
    let cli = Cli::try_parse_from(start_args).unwrap();
    match cli.command {
        Commands::Machine {
            command: MachineCommands::Start { name, .. },
        } => {
            assert_eq!(name, "dev");
        }
        _ => panic!("Expected Commands::Machine::Start"),
    }

    // 5. Stop
    let stop_args = vec!["cro", "machine", "stop", "dev"];
    let cli = Cli::try_parse_from(stop_args).unwrap();
    match cli.command {
        Commands::Machine {
            command: MachineCommands::Stop { name, .. },
        } => {
            assert_eq!(name, "dev");
        }
        _ => panic!("Expected Commands::Machine::Stop"),
    }

    // 6. Rm
    let rm_args = vec!["cro", "machine", "rm", "dev", "-f"];
    let cli = Cli::try_parse_from(rm_args).unwrap();
    match cli.command {
        Commands::Machine {
            command: MachineCommands::Rm { name, force, .. },
        } => {
            assert_eq!(name, "dev");
            assert!(force);
        }
        _ => panic!("Expected Commands::Machine::Rm"),
    }
}

#[test]
fn test_cli_parse_model_subcommands() {
    // 1. Model Pull
    let pull_args = vec!["cro", "model", "pull", "ghcr.io/org/llama-3.2-1b:latest"];
    let cli = Cli::try_parse_from(pull_args).unwrap();
    match cli.command {
        Commands::Model {
            command: ModelCommands::Pull { model, data_dir },
        } => {
            assert_eq!(model, "ghcr.io/org/llama-3.2-1b:latest");
            assert!(data_dir.is_none());
        }
        _ => panic!("Expected Commands::Model::Pull"),
    }

    // 2. Model List
    let list_args = vec!["cro", "model", "list", "--json"];
    let cli = Cli::try_parse_from(list_args).unwrap();
    match cli.command {
        Commands::Model {
            command: ModelCommands::List { json, .. },
        } => {
            assert!(json);
        }
        _ => panic!("Expected Commands::Model::List"),
    }

    // 3. Model Info
    let info_args = vec!["cro", "model", "info", "llama-3.2-1b"];
    let cli = Cli::try_parse_from(info_args).unwrap();
    match cli.command {
        Commands::Model {
            command: ModelCommands::Info { model, json, .. },
        } => {
            assert_eq!(model, "llama-3.2-1b");
            assert!(!json);
        }
        _ => panic!("Expected Commands::Model::Info"),
    }

    // 4. Model Pack
    let pack_args = vec![
        "cro",
        "model",
        "pack",
        "-s",
        "/path/to/weights",
        "-o",
        "/path/to/layout",
        "-n",
        "my-llama",
        "-f",
        "gguf",
        "-a",
        "llama",
        "-p",
        "q4_k_m",
        "--context-length",
        "8192",
        "-t",
        "v1.0",
    ];
    let cli = Cli::try_parse_from(pack_args).unwrap();
    match cli.command {
        Commands::Model {
            command:
                ModelCommands::Pack {
                    source,
                    output,
                    name,
                    format,
                    architecture,
                    precision,
                    context_length,
                    tag,
                    ..
                },
        } => {
            assert_eq!(source, PathBuf::from("/path/to/weights"));
            assert_eq!(output, PathBuf::from("/path/to/layout"));
            assert_eq!(name, "my-llama");
            assert_eq!(format, "gguf");
            assert_eq!(architecture.as_deref(), Some("llama"));
            assert_eq!(precision.as_deref(), Some("q4_k_m"));
            assert_eq!(context_length, Some(8192));
            assert_eq!(tag.as_deref(), Some("v1.0"));
        }
        _ => panic!("Expected Commands::Model::Pack"),
    }

    // 5. Model Discover
    let discover_args = vec![
        "cro",
        "model",
        "discover",
        "--path",
        "/custom/models",
        "--json",
    ];
    let cli = Cli::try_parse_from(discover_args).unwrap();
    match cli.command {
        Commands::Model {
            command: ModelCommands::Discover {
                custom_paths, json, ..
            },
        } => {
            assert_eq!(custom_paths, vec![PathBuf::from("/custom/models")]);
            assert!(json);
        }
        _ => panic!("Expected Commands::Model::Discover"),
    }

    // 6. Model Fit
    let fit_args = vec![
        "cro",
        "model",
        "fit",
        "/path/to/model.gguf",
        "--context-length",
        "8192",
        "--json",
    ];
    let cli = Cli::try_parse_from(fit_args).unwrap();
    match cli.command {
        Commands::Model {
            command:
                ModelCommands::Fit {
                    model,
                    context_length,
                    json,
                },
        } => {
            assert_eq!(model, "/path/to/model.gguf");
            assert_eq!(context_length, 8192);
            assert!(json);
        }
        _ => panic!("Expected Commands::Model::Fit"),
    }
}

#[test]
fn test_cli_parse_run_with_model() {
    let args = vec![
        "cro",
        "run",
        "--model",
        "ghcr.io/org/llama-3.2-1b:latest:/models/llama",
        "--model",
        "./local-model",
        "alpine:latest",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert_eq!(run.models.len(), 2);
            assert_eq!(
                run.models[0],
                "ghcr.io/org/llama-3.2-1b:latest:/models/llama"
            );
            assert_eq!(run.models[1], "./local-model");
            assert_eq!(run.image, "alpine:latest");
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_serve_and_vllm() {
    // 1. cro serve with defaults
    let serve_args = vec!["cro", "serve", "ghcr.io/org/llama-3.2-1b:latest"];
    let cli = Cli::try_parse_from(serve_args).unwrap();
    match cli.command {
        Commands::Serve(args) => {
            assert_eq!(args.model, "ghcr.io/org/llama-3.2-1b:latest");
            assert_eq!(args.engine, "auto");
            assert_eq!(args.port, 8000);
            assert_eq!(args.cpus, 4);
            assert_eq!(args.memory, 8192);
            assert!(args.gpu);
            assert!(args.wait);
        }
        _ => panic!("Expected Commands::Serve"),
    }

    // 2. cro serve with explicit vllm flags
    let custom_serve = vec![
        "cro",
        "serve",
        "./models/mistral",
        "--engine",
        "vllm",
        "-p",
        "8080",
        "-c",
        "8",
        "-m",
        "16384",
        "--api-key",
        "mock-key",
        "--max-model-len",
        "8192",
        "--tensor-parallel-size",
        "2",
        "--enforce-eager",
        "--alias",
        "mistral-custom",
        "--plugin",
        "/tmp/custom_plugins",
        "--vllm-plugin",
        "plugin_module_x",
        "--trust-remote-code",
        "--quantization",
        "fp8",
        "--chat-template",
        "/tmp/chat.jinja",
        "-e",
        "HF_TOKEN=hf_mock",
        "-d",
    ];
    let cli2 = Cli::try_parse_from(custom_serve).unwrap();
    match cli2.command {
        Commands::Serve(args) => {
            assert_eq!(args.model, "./models/mistral");
            assert_eq!(args.engine, "vllm");
            assert_eq!(args.port, 8080);
            assert_eq!(args.cpus, 8);
            assert_eq!(args.memory, 16384);
            assert_eq!(args.api_key.as_deref(), Some("mock-key"));
            assert_eq!(args.max_model_len, Some(8192));
            assert_eq!(args.tensor_parallel_size, 2);
            assert!(args.enforce_eager);
            assert_eq!(args.served_model_name.as_deref(), Some("mistral-custom"));
            assert_eq!(args.plugins, vec![PathBuf::from("/tmp/custom_plugins")]);
            assert_eq!(args.vllm_plugins, vec!["plugin_module_x".to_string()]);
            assert!(args.trust_remote_code);
            assert_eq!(args.quantization.as_deref(), Some("fp8"));
            assert_eq!(args.chat_template, Some(PathBuf::from("/tmp/chat.jinja")));
            assert_eq!(args.env, vec!["HF_TOKEN=hf_mock".to_string()]);
            assert!(args.detach);
        }
        _ => panic!("Expected Commands::Serve"),
    }

    // 3. cro vllm serve subcommand
    let vllm_args = vec!["cro", "vllm", "serve", "my-model", "-p", "9000"];
    let cli3 = Cli::try_parse_from(vllm_args).unwrap();
    match cli3.command {
        Commands::Vllm {
            command: VllmCommands::Serve(sargs),
        } => {
            assert_eq!(sargs.model, "my-model");
            assert_eq!(sargs.port, 9000);
        }
        _ => panic!("Expected Commands::Vllm"),
    }

    // 4. cro serve with warm-start snapshots
    let warm_args = vec![
        "cro",
        "serve",
        "llama-3.2-1b",
        "--warm",
        "--snapshot-on-ready",
        "--warmup-snapshot-tag",
        "v1-prod",
    ];
    let cli4 = Cli::try_parse_from(warm_args).unwrap();
    match cli4.command {
        Commands::Serve(args) => {
            assert_eq!(args.model, "llama-3.2-1b");
            assert!(args.warm);
            assert!(args.snapshot_on_ready);
            assert_eq!(args.warmup_snapshot_tag.as_deref(), Some("v1-prod"));
        }
        _ => panic!("Expected Commands::Serve"),
    }

    // 5. cro serve with scale-to-zero and idle timeout
    let scale_zero_args = vec![
        "cro",
        "serve",
        "llama-3.2-1b",
        "--scale-to-zero",
        "--idle-timeout",
        "120",
    ];
    let cli5 = Cli::try_parse_from(scale_zero_args).unwrap();
    match cli5.command {
        Commands::Serve(args) => {
            assert!(args.scale_to_zero);
            assert_eq!(args.idle_timeout, 120);
        }
        _ => panic!("Expected Commands::Serve"),
    }
}

#[test]
fn test_cli_parse_egress_and_secrets() {
    let args = vec![
        "microvm",
        "run",
        "--allow-host",
        "api.openai.com:443",
        "--allow-host",
        "*.anthropic.com:443",
        "--secret",
        "OPENAI_API_KEY=sk-test-12345",
        "--max-tokens",
        "50000",
        "python:3.11-slim",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert_eq!(
                run.allow_hosts,
                vec!["api.openai.com:443", "*.anthropic.com:443"]
            );
            assert_eq!(run.secrets, vec!["OPENAI_API_KEY=sk-test-12345"]);
            assert_eq!(run.max_tokens, Some(50000));
            assert_eq!(run.image, "python:3.11-slim");
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_default_deny() {
    let args = vec!["microvm", "run", "--default-deny", "alpine:latest"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert!(run.default_deny);
            assert!(run.allow_hosts.is_empty());
        }
        _ => panic!("Expected Commands::Run"),
    }

    let sandbox_args = vec!["microvm", "sandbox", "claude", "--deny-all"];
    let cli_sb = Cli::try_parse_from(sandbox_args).unwrap();
    match cli_sb.command {
        Commands::Sandbox { default_deny, .. } => {
            assert!(default_deny);
        }
        _ => panic!("Expected Commands::Sandbox"),
    }
}

#[test]
fn test_cli_parse_disk_parallel_reads_and_fs_options() {
    let args = vec![
        "microvm",
        "run",
        "--disk",
        "/var/lib/disk.raw",
        "--disk-parallel-reads",
        "--disk-read-pool-size",
        "16",
        "--fs-cache",
        "always",
        "--fs-permissions",
        "simplified",
        "--balloon-reporting",
        "replace",
        "ubuntu:24.04",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert_eq!(run.disks, vec!["/var/lib/disk.raw"]);
            assert!(run.disk_parallel_reads);
            assert_eq!(run.disk_read_pool_size, Some(16));
            assert_eq!(run.fs_cache, Some("always".to_string()));
            assert_eq!(run.fs_permissions, Some("simplified".to_string()));
            assert_eq!(run.balloon_reporting, Some("replace".to_string()));
            assert_eq!(run.image, "ubuntu:24.04");
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_parse_secret_arg() {
    // Direct value
    let (k, v) = parse_secret_arg("FOO=bar").unwrap();
    assert_eq!(k, "FOO");
    assert_eq!(v, "bar");

    // Environment variable
    std::env::set_var("TEST_SECRET_ENV_VAR", "super-secret-token");
    let (k2, v2) = parse_secret_arg("API_KEY=env:TEST_SECRET_ENV_VAR").unwrap();
    assert_eq!(k2, "API_KEY");
    assert_eq!(v2, "super-secret-token");

    // Missing env var
    assert!(parse_secret_arg("API_KEY=env:NON_EXISTENT_VAR_123456").is_err());

    // File-based
    let temp_dir = tempfile::tempdir().unwrap();
    let secret_file = temp_dir.path().join("secret.txt");
    std::fs::write(&secret_file, "file-token-content\n").unwrap();
    let arg = format!("FILE_KEY=file:{}", secret_file.display());
    let (k3, v3) = parse_secret_arg(&arg).unwrap();
    assert_eq!(k3, "FILE_KEY");
    assert_eq!(v3, "file-token-content");

    // Invalid format
    assert!(parse_secret_arg("NO_EQUALS_SIGN").is_err());
    assert!(parse_secret_arg("=NO_KEY").is_err());
}

#[test]
fn test_cli_parse_artifact_commands() {
    let pull_args = vec!["microvm", "artifact", "pull", "ghcr.io/owner/weights:v1"];
    let cli = Cli::try_parse_from(pull_args).unwrap();
    match cli.command {
        Commands::Artifact {
            command: ArtifactCommands::Pull { artifact, .. },
        } => {
            assert_eq!(artifact, "ghcr.io/owner/weights:v1");
        }
        _ => panic!("Expected ArtifactCommands::Pull"),
    }

    let list_args = vec!["microvm", "artifact", "list"];
    let cli = Cli::try_parse_from(list_args).unwrap();
    match cli.command {
        Commands::Artifact {
            command: ArtifactCommands::List { .. },
        } => {}
        _ => panic!("Expected ArtifactCommands::List"),
    }
}

#[test]
fn test_cli_parse_stats_and_inspect() {
    let stats_args = vec!["microvm", "stats", "--no-stream", "--json"];
    let cli = Cli::try_parse_from(stats_args).unwrap();
    match cli.command {
        Commands::Stats {
            id,
            no_stream,
            json,
            ..
        } => {
            assert!(id.is_none());
            assert!(no_stream);
            assert!(json);
        }
        _ => panic!("Expected Commands::Stats"),
    }

    let inspect_args = vec!["microvm", "inspect", "test-vm-123", "--json"];
    let cli = Cli::try_parse_from(inspect_args).unwrap();
    match cli.command {
        Commands::Inspect { id, json, .. } => {
            assert_eq!(id, "test-vm-123");
            assert!(json);
        }
        _ => panic!("Expected Commands::Inspect"),
    }
}

#[test]
fn test_cli_parse_rm_and_top() {
    let rm_args = vec!["microvm", "rm", "-f", "test-vm-123"];
    let cli = Cli::try_parse_from(rm_args).unwrap();
    match cli.command {
        Commands::Rm { id, force, .. } => {
            assert_eq!(id, "test-vm-123");
            assert!(force);
        }
        _ => panic!("Expected Commands::Rm"),
    }

    let top_args = vec!["microvm", "top", "test-vm-123"];
    let cli = Cli::try_parse_from(top_args).unwrap();
    match cli.command {
        Commands::Top {
            id, interactive, ..
        } => {
            assert_eq!(id, Some("test-vm-123".to_string()));
            assert!(!interactive);
        }
        _ => panic!("Expected Commands::Top"),
    }

    let top_interactive_args = vec!["microvm", "top", "-i"];
    let cli = Cli::try_parse_from(top_interactive_args).unwrap();
    match cli.command {
        Commands::Top {
            id, interactive, ..
        } => {
            assert_eq!(id, None);
            assert!(interactive);
        }
        _ => panic!("Expected Commands::Top"),
    }

    let ui_args = vec!["microvm", "ui"];
    let cli = Cli::try_parse_from(ui_args).unwrap();
    match cli.command {
        Commands::Ui { .. } => {}
        _ => panic!("Expected Commands::Ui"),
    }

    let cockpit_args = vec!["microvm", "cockpit"];
    let cli = Cli::try_parse_from(cockpit_args).unwrap();
    match cli.command {
        Commands::Ui { .. } => {}
        _ => panic!("Expected Commands::Ui alias cockpit"),
    }
}

#[test]
fn test_format_bytes() {
    assert_eq!(format_bytes(500), "500 B");
    assert_eq!(format_bytes(1024), "1.0 KiB");
    assert_eq!(format_bytes(1024 * 1024 * 10), "10.0 MiB");
    assert_eq!(format_bytes(1024 * 1024 * 1024 * 2), "2.00 GiB");
}

#[test]
fn test_cli_parse_pause_and_resume() {
    let pause_args = vec!["microvm", "pause", "vm-1234"];
    let cli = Cli::try_parse_from(pause_args).unwrap();
    match cli.command {
        Commands::Pause { id, .. } => {
            assert_eq!(id, "vm-1234");
        }
        _ => panic!("Expected Commands::Pause"),
    }

    let resume_args = vec!["microvm", "resume", "vm-1234"];
    let cli = Cli::try_parse_from(resume_args).unwrap();
    match cli.command {
        Commands::Resume { id, .. } => {
            assert_eq!(id, "vm-1234");
        }
        _ => panic!("Expected Commands::Resume"),
    }
}

#[test]
fn test_cli_parse_run_dax() {
    let run_args = vec!["microvm", "run", "--dax", "4G", "alpine:latest"];
    let cli = Cli::try_parse_from(run_args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert_eq!(run.image, "alpine:latest");
            assert_eq!(run.dax.as_deref(), Some("4G"));
        }
        _ => panic!("Expected Commands::Run with --dax"),
    }
}

#[test]
fn test_cli_parse_run_lazy_load() {
    let run_args = vec![
        "microvm",
        "run",
        "--lazy-load",
        "--nydus-bootstrap",
        "/tmp/bootstrap.rafs",
        "--nydus-cache",
        "/var/cache/nydus",
        "--chunk-size",
        "64M",
        "--dax",
        "2G",
        "alpine:latest",
    ];
    let cli = Cli::try_parse_from(run_args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert!(run.lazy_load);
            assert_eq!(
                run.nydus_bootstrap,
                Some(PathBuf::from("/tmp/bootstrap.rafs"))
            );
            assert_eq!(run.nydus_cache, Some(PathBuf::from("/var/cache/nydus")));
            assert_eq!(run.chunk_size.as_deref(), Some("64M"));
            assert_eq!(run.dax.as_deref(), Some("2G"));
            assert_eq!(run.image, "alpine:latest");
        }
        _ => panic!("Expected Commands::Run with --lazy-load"),
    }
}

#[test]
fn test_cli_parse_run_gpu() {
    let run_args = vec![
        "microvm",
        "run",
        "--gpu",
        "--gpu-shm-size",
        "4G",
        "ghcr.io/ericlbuehler/mistral.rs:latest",
    ];
    let cli = Cli::try_parse_from(run_args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert!(run.gpu);
            assert_eq!(run.gpu_shm_size.as_deref(), Some("4G"));
            assert_eq!(run.image, "ghcr.io/ericlbuehler/mistral.rs:latest");
        }
        _ => panic!("Expected Commands::Run with --gpu"),
    }
}

#[test]
fn test_cli_parse_exec() {
    let exec_args = vec![
        "microvm",
        "exec",
        "-t",
        "-w",
        "/app",
        "-e",
        "PORT=9000",
        "vm-12345",
        "uname",
        "-a",
    ];
    let cli = Cli::try_parse_from(exec_args).unwrap();
    match cli.command {
        Commands::Exec {
            id,
            env,
            workdir,
            tty,
            cmd,
            ..
        } => {
            assert_eq!(id, "vm-12345");
            assert_eq!(env, vec!["PORT=9000"]);
            assert_eq!(workdir.as_deref(), Some("/app"));
            assert!(tty);
            assert_eq!(cmd, vec!["uname", "-a"]);
        }
        _ => panic!("Expected Commands::Exec"),
    }
}

#[test]
fn test_cli_parse_snapshot_and_restore() {
    let snap_args = vec![
        "microvm",
        "snapshot",
        "vm-12345",
        "--output",
        "/tmp/snap.tar",
    ];
    let cli = Cli::try_parse_from(snap_args).unwrap();
    match cli.command {
        Commands::Snapshot { id, output, .. } => {
            assert_eq!(id, "vm-12345");
            assert_eq!(output, Some(PathBuf::from("/tmp/snap.tar")));
        }
        _ => panic!("Expected Commands::Snapshot"),
    }

    let restore_args = vec![
        "microvm",
        "restore",
        "/tmp/snap.tar",
        "--name",
        "vm-restored-99",
    ];
    let cli = Cli::try_parse_from(restore_args).unwrap();
    match cli.command {
        Commands::Restore { snapshot, name, .. } => {
            assert_eq!(snapshot, PathBuf::from("/tmp/snap.tar"));
            assert_eq!(name.as_deref(), Some("vm-restored-99"));
        }
        _ => panic!("Expected Commands::Restore"),
    }
}

#[test]
fn test_cli_parse_run_no_sandbox() {
    let args = vec!["microvm", "run", "--no-sandbox", "alpine:latest"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert!(run.no_sandbox);
            assert_eq!(run.image, "alpine:latest");
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_resize() {
    let args = vec![
        "microvm",
        "resize",
        "vm-test-1",
        "--memory",
        "1024",
        "--cpus",
        "4",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Resize {
            id, memory, cpus, ..
        } => {
            assert_eq!(id, "vm-test-1");
            assert_eq!(memory, Some(1024));
            assert_eq!(cpus, Some(4));
        }
        _ => panic!("Expected Commands::Resize"),
    }
}

#[test]
fn test_cli_parse_metrics() {
    let args = vec!["microvm", "metrics", "--listen", "127.0.0.1:9090"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Metrics { listen, json, .. } => {
            assert_eq!(listen.as_deref(), Some("127.0.0.1:9090"));
            assert!(!json);
        }
        _ => panic!("Expected Commands::Metrics"),
    }

    let json_args = vec!["microvm", "metrics", "--json"];
    let cli = Cli::try_parse_from(json_args).unwrap();
    match cli.command {
        Commands::Metrics { listen, json, .. } => {
            assert!(listen.is_none());
            assert!(json);
        }
        _ => panic!("Expected Commands::Metrics"),
    }
}

#[test]
fn test_cli_parse_multi_boot_run() {
    let args = vec![
        "microvm",
        "run",
        "--kernel",
        "/boot/vmlinuz",
        "--kernel-format",
        "gz",
        "--initrd",
        "/boot/initrd.img",
        "--cmdline",
        "console=ttyS0 root=/dev/vda",
        "--disk",
        "rootfs.raw",
        "--disk",
        "data:data.raw:ro",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert_eq!(run.kernel, Some(PathBuf::from("/boot/vmlinuz")));
            assert_eq!(run.kernel_format, Some("gz".to_string()));
            assert_eq!(run.initrd, Some(PathBuf::from("/boot/initrd.img")));
            assert_eq!(run.cmdline, Some("console=ttyS0 root=/dev/vda".to_string()));
            assert_eq!(
                run.disks,
                vec!["rootfs.raw".to_string(), "data:data.raw:ro".to_string()]
            );
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_asahi_m1n1_kernel_run() {
    let args = vec![
        "microvm",
        "run",
        "--kernel",
        "/opt/asahi/m1n1.bin",
        "--kformat",
        "raw",
        "--cmdline",
        "console=ttyAMA0 earlycon",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert_eq!(run.kernel, Some(PathBuf::from("/opt/asahi/m1n1.bin")));
            assert_eq!(run.kernel_format, Some("raw".to_string()));
            assert_eq!(run.cmdline, Some("console=ttyAMA0 earlycon".to_string()));
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_unikernel_and_sandbox() {
    let unikernel_args = vec![
        "microvm",
        "unikernel",
        "app.unikraft",
        "-c",
        "1",
        "-m",
        "256",
        "--cmdline",
        "netdev.ipv4_addr=192.168.1.2",
        "--disk",
        "data.img",
    ];
    let cli = Cli::try_parse_from(unikernel_args).unwrap();
    match cli.command {
        Commands::Unikernel {
            kernel,
            cpus,
            memory,
            cmdline,
            disks,
            ..
        } => {
            assert_eq!(kernel, PathBuf::from("app.unikraft"));
            assert_eq!(cpus, 1);
            assert_eq!(memory, 256);
            assert_eq!(cmdline, Some("netdev.ipv4_addr=192.168.1.2".to_string()));
            assert_eq!(disks, vec!["data.img".to_string()]);
        }
        _ => panic!("Expected Commands::Unikernel"),
    }

    // Test with --params alias
    let unikernel_params_args = vec![
        "microvm",
        "unikernel",
        "app.unikraft",
        "--params",
        "console=ttyS0",
    ];
    let cli_unik = Cli::try_parse_from(unikernel_params_args).unwrap();
    match cli_unik.command {
        Commands::Unikernel { cmdline, .. } => {
            assert_eq!(cmdline, Some("console=ttyS0".to_string()));
        }
        _ => panic!("Expected Commands::Unikernel"),
    }

    let sandbox_args = vec![
        "microvm",
        "sandbox",
        "claude",
        "--workspace",
        ".",
        "-c",
        "4",
        "-m",
        "2048",
        "--secret",
        "CUSTOM_KEY=secretval",
        "--repo",
        "https://github.com/example/project.git",
    ];
    let cli2 = Cli::try_parse_from(sandbox_args).unwrap();
    match cli2.command {
        Commands::Sandbox {
            agent,
            workspace,
            repo,
            cpus,
            memory,
            secrets,
            ..
        } => {
            assert_eq!(agent, "claude");
            assert_eq!(workspace, PathBuf::from("."));
            assert_eq!(
                repo,
                Some("https://github.com/example/project.git".to_string())
            );
            assert_eq!(cpus, 4);
            assert_eq!(memory, 2048);
            assert_eq!(secrets, vec!["CUSTOM_KEY=secretval".to_string()]);
        }
        _ => panic!("Expected Commands::Sandbox"),
    }
}

#[test]
fn test_cli_parse_sandbox_enhanced_features() {
    let sandbox_args = vec![
        "microvm",
        "sandbox",
        "deepseek",
        "--workspace",
        "/tmp/test-ws",
        "--allow-host",
        "api.deepseek.com:443",
        "--allow-host",
        "pypi.org:443",
        "--max-tokens",
        "75000",
        "--gpu",
        "--gpu-shm-size",
        "4G",
        "--shm-size",
        "1g",
    ];
    let cli = Cli::try_parse_from(sandbox_args).unwrap();
    match cli.command {
        Commands::Sandbox {
            agent,
            workspace,
            allow_hosts,
            max_tokens,
            gpu,
            gpu_shm_size,
            shm_size,
            ..
        } => {
            assert_eq!(agent, "deepseek");
            assert_eq!(workspace, PathBuf::from("/tmp/test-ws"));
            assert_eq!(
                allow_hosts,
                vec![
                    "api.deepseek.com:443".to_string(),
                    "pypi.org:443".to_string()
                ]
            );
            assert_eq!(max_tokens, Some(75000));
            assert!(gpu);
            assert_eq!(gpu_shm_size.as_deref(), Some("4G"));
            assert_eq!(shm_size.as_deref(), Some("1g"));
        }
        _ => panic!("Expected Commands::Sandbox with enhanced features"),
    }
}

#[test]
fn test_cli_parse_sandbox_hitl() {
    let sandbox_args = vec![
        "microvm",
        "sandbox",
        "claude",
        "--hitl",
        "--secret",
        "ANTHROPIC_API_KEY=env:ANTHROPIC_API_KEY",
    ];
    let cli = Cli::try_parse_from(sandbox_args).unwrap();
    match cli.command {
        Commands::Sandbox {
            agent,
            hitl,
            secrets,
            ..
        } => {
            assert_eq!(agent, "claude");
            assert!(hitl);
            assert_eq!(
                secrets,
                vec!["ANTHROPIC_API_KEY=env:ANTHROPIC_API_KEY".to_string()]
            );
        }
        _ => panic!("Expected Commands::Sandbox with hitl"),
    }
}

#[test]
fn test_cli_parse_approval_commands() {
    // 1. List
    let list_args = vec!["microvm", "approval", "ls", "--all"];
    let cli = Cli::try_parse_from(list_args).unwrap();
    match cli.command {
        Commands::Approval { command } => match command {
            ApprovalCommands::List { all, .. } => assert!(all),
            _ => panic!("Expected ApprovalCommands::List"),
        },
        _ => panic!("Expected Commands::Approval"),
    }

    // 2. Approve
    let approve_args = vec!["microvm", "approval", "approve", "appr_000001"];
    let cli2 = Cli::try_parse_from(approve_args).unwrap();
    match cli2.command {
        Commands::Approval { command } => match command {
            ApprovalCommands::Approve { id, .. } => assert_eq!(id, "appr_000001"),
            _ => panic!("Expected ApprovalCommands::Approve"),
        },
        _ => panic!("Expected Commands::Approval"),
    }

    // 3. Deny
    let deny_args = vec![
        "microvm",
        "approval",
        "deny",
        "appr_000001",
        "-r",
        "Dangerous action",
    ];
    let cli3 = Cli::try_parse_from(deny_args).unwrap();
    match cli3.command {
        Commands::Approval { command } => match command {
            ApprovalCommands::Deny { id, reason, .. } => {
                assert_eq!(id, "appr_000001");
                assert_eq!(reason, Some("Dangerous action".to_string()));
            }
            _ => panic!("Expected ApprovalCommands::Deny"),
        },
        _ => panic!("Expected Commands::Approval"),
    }
}

#[test]
fn test_cli_parse_compose_up_and_down() {
    let up_args = vec![
        "microvm",
        "compose",
        "up",
        "-f",
        "krun-compose.yaml",
        "-d",
        "web",
        "redis",
    ];
    let cli = Cli::try_parse_from(up_args).unwrap();
    match cli.command {
        Commands::Compose {
            command:
                ComposeCommands::Up {
                    file,
                    detach,
                    services,
                    ..
                },
        } => {
            assert_eq!(file, Some(PathBuf::from("krun-compose.yaml")));
            assert!(detach);
            assert_eq!(services, vec!["web".to_string(), "redis".to_string()]);
        }
        _ => panic!("Expected ComposeCommands::Up"),
    }

    let down_args = vec!["microvm", "compose", "down", "-v"];
    let cli_down = Cli::try_parse_from(down_args).unwrap();
    match cli_down.command {
        Commands::Compose {
            command: ComposeCommands::Down { file, volumes, .. },
        } => {
            assert!(file.is_none());
            assert!(volumes);
        }
        _ => panic!("Expected ComposeCommands::Down"),
    }
}

#[test]
fn test_cli_parse_compose_ps_and_logs() {
    let ps_args = vec!["microvm", "compose", "ps", "-f", "compose.yaml"];
    let cli_ps = Cli::try_parse_from(ps_args).unwrap();
    match cli_ps.command {
        Commands::Compose {
            command: ComposeCommands::Ps { file, .. },
        } => {
            assert_eq!(file, Some(PathBuf::from("compose.yaml")));
        }
        _ => panic!("Expected ComposeCommands::Ps"),
    }

    let logs_args = vec![
        "microvm",
        "compose",
        "logs",
        "--file",
        "compose.yaml",
        "-f",
        "api",
    ];
    let cli_logs = Cli::try_parse_from(logs_args).unwrap();
    match cli_logs.command {
        Commands::Compose {
            command:
                ComposeCommands::Logs {
                    file,
                    service,
                    follow,
                    ..
                },
        } => {
            assert_eq!(file, Some(PathBuf::from("compose.yaml")));
            assert_eq!(service, Some("api".to_string()));
            assert!(follow);
        }
        _ => panic!("Expected ComposeCommands::Logs"),
    }
}

#[test]
fn test_cli_parse_compose_restart_and_config() {
    let restart_args = vec!["microvm", "compose", "restart", "web"];
    let cli_res = Cli::try_parse_from(restart_args).unwrap();
    match cli_res.command {
        Commands::Compose {
            command: ComposeCommands::Restart { services, .. },
        } => {
            assert_eq!(services, vec!["web".to_string()]);
        }
        _ => panic!("Expected ComposeCommands::Restart"),
    }

    let config_args = vec!["microvm", "compose", "config", "-f", "krun-compose.yaml"];
    let cli_cfg = Cli::try_parse_from(config_args).unwrap();
    match cli_cfg.command {
        Commands::Compose {
            command: ComposeCommands::Config { file, .. },
        } => {
            assert_eq!(file, Some(PathBuf::from("krun-compose.yaml")));
        }
        _ => panic!("Expected ComposeCommands::Config"),
    }
}

#[test]
fn test_cli_parse_apply_and_run_file() {
    let apply_args = vec!["microvm", "apply", "-f", "examples/microvm.yaml", "-d"];
    let cli = Cli::try_parse_from(apply_args).unwrap();
    match cli.command {
        Commands::Apply { file, detach, .. } => {
            assert_eq!(file, PathBuf::from("examples/microvm.yaml"));
            assert!(detach);
        }
        _ => panic!("Expected Commands::Apply"),
    }

    let run_args = vec!["microvm", "run", "-f", "examples/krun-compose.yaml", "-d"];
    let cli_run = Cli::try_parse_from(run_args).unwrap();
    match cli_run.command {
        Commands::Run(run) => {
            assert_eq!(run.file, Some(PathBuf::from("examples/krun-compose.yaml")));
            assert!(run.detach);
        }
        _ => panic!("Expected Commands::Run with -f"),
    }
}

#[test]
fn test_cli_parse_sandbox_cherry_pick() {
    let args = vec![
        "microvm",
        "sandbox",
        "claude",
        "--workspace",
        "/path/to/project",
        "--cherry-pick",
        "a1b2c3d4",
        "--apply-to-host",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Sandbox {
            agent,
            workspace,
            cherry_pick,
            apply_to_host,
            ..
        } => {
            assert_eq!(agent, "claude");
            assert_eq!(workspace, PathBuf::from("/path/to/project"));
            assert_eq!(cherry_pick, Some("a1b2c3d4".to_string()));
            assert!(apply_to_host);
        }
        _ => panic!("Expected Commands::Sandbox with cherry-pick"),
    }
}

#[test]
fn test_cli_parse_cherry_pick_command() {
    let args = vec![
        "microvm",
        "cherry-pick",
        "vm-test1234",
        "-w",
        "/host/workspace",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::CherryPick {
            id,
            workspace,
            data_dir,
        } => {
            assert_eq!(id, "vm-test1234");
            assert_eq!(workspace, PathBuf::from("/host/workspace"));
            assert!(data_dir.is_none());
        }
        _ => panic!("Expected Commands::CherryPick"),
    }
}

#[test]
fn test_cli_parse_run_mac_and_mtu() {
    let args = vec![
        "microvm",
        "run",
        "--mac",
        "5a:94:ef:e4:0c:ee",
        "--mtu",
        "9000",
        "alpine:latest",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run(run) => {
            assert_eq!(run.mac, Some("5a:94:ef:e4:0c:ee".to_string()));
            assert_eq!(run.mtu, Some(9000));
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_network_commands() {
    // network ls
    let cli = Cli::try_parse_from(vec!["microvm", "network", "ls", "--json"]).unwrap();
    match cli.command {
        Commands::Network(NetworkCommands::Ls { json, .. }) => assert!(json),
        _ => panic!("Expected NetworkCommands::Ls"),
    }

    // network inspect
    let cli = Cli::try_parse_from(vec!["microvm", "network", "inspect", "vm-net-123"]).unwrap();
    match cli.command {
        Commands::Network(NetworkCommands::Inspect { id, json, .. }) => {
            assert_eq!(id, "vm-net-123");
            assert!(!json);
        }
        _ => panic!("Expected NetworkCommands::Inspect"),
    }

    // network ports
    let cli = Cli::try_parse_from(vec!["microvm", "network", "ports"]).unwrap();
    match cli.command {
        Commands::Network(NetworkCommands::Ports { id, .. }) => assert!(id.is_none()),
        _ => panic!("Expected NetworkCommands::Ports"),
    }

    // network test
    let cli = Cli::try_parse_from(vec![
        "microvm",
        "network",
        "test",
        "vm-net-123",
        "api.openai.com",
    ])
    .unwrap();
    match cli.command {
        Commands::Network(NetworkCommands::Test { id, target, .. }) => {
            assert_eq!(id, "vm-net-123");
            assert_eq!(target, "api.openai.com");
        }
        _ => panic!("Expected NetworkCommands::Test"),
    }

    // network pingora
    let cli = Cli::try_parse_from(vec![
        "microvm",
        "network",
        "pingora",
        "--listen",
        "127.0.0.1:9090",
        "--route",
        "/api=127.0.0.1:3000",
        "--allow-host",
        "api.openai.com:443",
        "--strip-prefix",
        "--max-tokens",
        "50000",
    ])
    .unwrap();
    match cli.command {
        Commands::Network(NetworkCommands::Pingora {
            listen,
            route,
            allow_hosts,
            strip_prefix,
            max_tokens,
            ..
        }) => {
            assert_eq!(listen, "127.0.0.1:9090");
            assert_eq!(route, vec!["/api=127.0.0.1:3000".to_string()]);
            assert_eq!(allow_hosts, vec!["api.openai.com:443".to_string()]);
            assert!(strip_prefix);
            assert_eq!(max_tokens, Some(50000));
        }
        _ => panic!("Expected NetworkCommands::Pingora"),
    }

    // network proxy
    let cli = Cli::try_parse_from(vec![
        "microvm",
        "network",
        "proxy",
        "--listen",
        "127.0.0.1:8888",
        "--allow-host",
        "api.anthropic.com:443",
        "--secret",
        "ANTHROPIC_API_KEY=sk-ant-test1234",
        "--max-tokens",
        "25000",
    ])
    .unwrap();
    match cli.command {
        Commands::Network(NetworkCommands::Proxy {
            listen,
            allow_hosts,
            secrets,
            max_tokens,
        }) => {
            assert_eq!(listen, "127.0.0.1:8888");
            assert_eq!(allow_hosts, vec!["api.anthropic.com:443".to_string()]);
            assert_eq!(secrets.len(), 1);
            assert_eq!(secrets[0].0, "ANTHROPIC_API_KEY");
            assert_eq!(secrets[0].1, "sk-ant-test1234");
            assert_eq!(max_tokens, Some(25000));
        }
        _ => panic!("Expected NetworkCommands::Proxy"),
    }

    // network doctor
    let cli = Cli::try_parse_from(vec!["microvm", "network", "doctor", "--json"]).unwrap();
    match cli.command {
        Commands::Network(NetworkCommands::Doctor { json }) => assert!(json),
        _ => panic!("Expected NetworkCommands::Doctor"),
    }
}

#[test]
fn test_truncate_str() {
    assert_eq!(truncate_str("hello", 10), "hello");
    assert_eq!(truncate_str("superlongstringexample", 8), "super...");
}

#[test]
fn test_cli_parse_shm_size() {
    let cli = Cli::try_parse_from(vec![
        "cro",
        "run",
        "--shm-size",
        "2g",
        "pytorch/pytorch:latest",
    ])
    .unwrap();
    match cli.command {
        Commands::Run(run_box) => {
            assert_eq!(run_box.shm_size.as_deref(), Some("2g"));
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_cli_parse_build() {
    let cli = Cli::try_parse_from(vec![
        "cro",
        "build",
        "-t",
        "custom-app:1.0",
        "-f",
        "custom.Dockerfile",
        "--no-cache",
        "./src",
    ])
    .unwrap();
    match cli.command {
        Commands::Build {
            tag,
            file,
            no_cache,
            context,
            ..
        } => {
            assert_eq!(tag.as_deref(), Some("custom-app:1.0"));
            assert_eq!(file, Some(PathBuf::from("custom.Dockerfile")));
            assert!(no_cache);
            assert_eq!(context, PathBuf::from("./src"));
        }
        _ => panic!("Expected Commands::Build"),
    }
}

#[test]
fn test_cli_parse_kube() {
    // kube play
    let cli = Cli::try_parse_from(vec!["cro", "kube", "play", "-d", "pod.yaml"]).unwrap();
    match cli.command {
        Commands::Kube {
            command: KubeCommands::Play { file, detach, .. },
        } => {
            assert_eq!(file, PathBuf::from("pod.yaml"));
            assert!(detach);
        }
        _ => panic!("Expected KubeCommands::Play"),
    }

    // kube down
    let cli = Cli::try_parse_from(vec!["cro", "kube", "down", "pod.yaml"]).unwrap();
    match cli.command {
        Commands::Kube {
            command: KubeCommands::Down { file, .. },
        } => {
            assert_eq!(file, PathBuf::from("pod.yaml"));
        }
        _ => panic!("Expected KubeCommands::Down"),
    }
}
