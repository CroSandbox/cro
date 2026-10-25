// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Shared formatting, path resolution, and argument parsing utilities for the CLI.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cache/krun-microvm")
}

pub fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn format_duration_since(timestamp: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now < timestamp {
        return "just now".to_string();
    }
    let elapsed = now - timestamp;
    if elapsed < 60 {
        format!("{}s ago", elapsed)
    } else if elapsed < 3600 {
        format!("{}m ago", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{}h ago", elapsed / 3600)
    } else {
        format!("{}d ago", elapsed / 86400)
    }
}

pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(meta) = p.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

pub fn host_memory_bytes() -> Option<u64> {
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages > 0 && page_size > 0 {
        Some((pages as u64).saturating_mul(page_size as u64))
    } else {
        None
    }
}

/// Parses a `--disk` CLI argument into (id, path, read_only).
///
/// Accepted formats:
///   `/path/to/disk.raw`                  → auto-ID, rw
///   `/path/to/disk.raw:ro`               → auto-ID, ro
///   `myid:/path/to/disk.raw`             → explicit ID, rw
///   `myid:/path/to/disk.raw:ro`          → explicit ID, ro
///
/// `disk_index` is used to generate unique auto-IDs (`disk0`, `disk1`, ...).
pub fn parse_disk_arg(arg: &str, disk_index: usize) -> (String, PathBuf, bool) {
    let parts: Vec<&str> = arg.split(':').collect();
    match parts.len() {
        1 => (format!("disk{disk_index}"), PathBuf::from(parts[0]), false),
        2 => {
            if parts[1] == "ro" {
                (format!("disk{disk_index}"), PathBuf::from(parts[0]), true)
            } else if parts[1] == "rw" {
                (format!("disk{disk_index}"), PathBuf::from(parts[0]), false)
            } else {
                (parts[0].to_string(), PathBuf::from(parts[1]), false)
            }
        }
        _ => (
            parts[0].to_string(),
            PathBuf::from(parts[1]),
            parts[2] == "ro",
        ),
    }
}

pub fn parse_secret_arg(arg: &str) -> Result<(String, String)> {
    let (key, value_spec) = arg.split_once('=').ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid secret format '{}'. Expected KEY=VALUE, KEY=env:VAR_NAME, KEY=op://..., KEY=bw://..., or KEY=file:/path",
            arg
        )
    })?;

    let key = key.trim();
    if key.is_empty() {
        bail!("Secret key cannot be empty in '{}'", arg);
    }

    let value = microvm_core::secrets::VaultResolver::resolve(value_spec)?;
    Ok((key.to_string(), value))
}

pub fn get_git_head(repo_path: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

pub fn get_git_commits_between(repo_path: &Path, base: &str, head: &str) -> Vec<String> {
    let out = std::process::Command::new("git")
        .args(["log", "--oneline", &format!("{base}..{head}")])
        .current_dir(repo_path)
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect(),
        _ => Vec::new(),
    }
}

pub fn cherry_pick_sandbox_to_host(
    sandbox_ws: &Path,
    host_ws: &Path,
    base_commit: &str,
    target_commit: &str,
) -> Result<()> {
    let patch_output = std::process::Command::new("git")
        .args([
            "format-patch",
            &format!("{base_commit}..{target_commit}"),
            "--stdout",
        ])
        .current_dir(sandbox_ws)
        .output()
        .context("Failed to format patch from sandbox git repository")?;

    if !patch_output.status.success() {
        bail!(
            "Failed to format patch from sandbox: {}",
            String::from_utf8_lossy(&patch_output.stderr)
        );
    }

    if patch_output.stdout.is_empty() {
        println!("No git commits to cherry-pick.");
        return Ok(());
    }

    use std::io::Write;
    let mut child = std::process::Command::new("git")
        .args(["am", "--3way"])
        .current_dir(host_ws)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn 'git am' on host repository")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&patch_output.stdout)?;
    }

    let result = child.wait_with_output()?;
    if !result.status.success() {
        bail!(
            "git am failed: {}\n(You may run 'git am --abort' to reset the host repository state)",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1024 * 1024 * 10), "10.0 MiB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 2), "2.00 GiB");
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("superlongstringexample", 8), "super...");
    }

    #[test]
    fn test_parse_secret_arg() {
        // Direct value
        let (k, v) = parse_secret_arg("FOO=bar").unwrap();
        assert_eq!(k, "FOO");
        assert_eq!(v, "bar");

        // Environment variable
        std::env::set_var("TEST_SECRET_ENV_VAR_2", "super-secret-token");
        let (k2, v2) = parse_secret_arg("API_KEY=env:TEST_SECRET_ENV_VAR_2").unwrap();
        assert_eq!(k2, "API_KEY");
        assert_eq!(v2, "super-secret-token");

        // Invalid format
        assert!(parse_secret_arg("NO_EQUALS_SIGN").is_err());
        assert!(parse_secret_arg("=NO_KEY").is_err());
    }

    #[test]
    fn test_parse_disk_arg() {
        let (id1, path1, ro1) = parse_disk_arg("/tmp/disk.img", 0);
        assert_eq!(id1, "disk0");
        assert_eq!(path1, PathBuf::from("/tmp/disk.img"));
        assert!(!ro1);

        let (id2, path2, ro2) = parse_disk_arg("/tmp/disk.img:ro", 1);
        assert_eq!(id2, "disk1");
        assert_eq!(path2, PathBuf::from("/tmp/disk.img"));
        assert!(ro2);

        let (id3, path3, ro3) = parse_disk_arg("mydata:/data/store.raw:ro", 2);
        assert_eq!(id3, "mydata");
        assert_eq!(path3, PathBuf::from("/data/store.raw"));
        assert!(ro3);
    }
}
