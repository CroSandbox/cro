//! Execute Dockerfile RUN instructions in dedicated, foreground microVMs.
use crate::config::KrunConfig;
use crate::types::RunnerConfig;
use crate::vm::runtime::find_runner_binary;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Host-side request for the runner's isolated build-step entry point.
/// This file is kept outside the guest rootfs; only that rootfs is shared with the VM.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildStepConfig {
    pub vm: RunnerConfig,
    pub command: KrunConfig,
}

impl BuildStepConfig {
    pub(crate) fn new(rootfs: &Path, mut command: KrunConfig) -> Result<Self> {
        if command.cmd.is_empty() || command.cmd[0].is_empty() {
            bail!("Dockerfile RUN requires a nonempty command");
        }
        if !command.env.iter().any(|value| value.starts_with("PATH=")) {
            command
                .env
                .push("PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into());
        }
        let rootfs = rootfs.canonicalize().context("Resolving build rootfs")?;
        let vm = serde_json::from_value(serde_json::json!({
            "root_path": rootfs, "num_vcpus": 1, "ram_mib": 512,
            "sandbox": true, "detach": false,
        }))?;
        Ok(Self { vm, command })
    }
}

pub(crate) struct BuildVmExecutor {
    pub(crate) timeout: Duration,
    // A test-only executable seam; production always resolves the trusted runner.
    #[cfg(test)]
    pub(crate) runner: Option<PathBuf>,
}

impl Default for BuildVmExecutor {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            #[cfg(test)]
            runner: None,
        }
    }
}

impl BuildVmExecutor {
    pub(crate) async fn execute(
        &self,
        rootfs: &Path,
        command: KrunConfig,
        cache_base: &Path,
    ) -> Result<()> {
        let request = BuildStepConfig::new(rootfs, command)?;
        let runner = self.runner_binary()?;
        let runs = cache_base.join("build-runs");
        fs::create_dir_all(&runs)?;
        let directory = tempfile::Builder::new()
            .prefix("run-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(&runs)?;
        let config = directory.path().join("request.json");
        fs::write(&config, serde_json::to_vec(&request)?)?;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600))?;

        let mut guest_config = GuestConfigGuard::prepare(&request.vm.root_path, directory.path())?;

        // The only host executable is the runner. Command argv/env are data in
        // its request file, never host arguments, shell input, or host environment.
        let mut process = Command::new(runner);
        process.env_clear();
        // Loader configuration belongs to the host runner. Guest env is supplied
        // separately in the guest init JSON, without inheriting host env.
        for name in [
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH",
        ] {
            if let Some(value) = std::env::var_os(name) {
                process.env(name, value);
            }
        }
        #[cfg(target_os = "macos")]
        if std::env::var_os("DYLD_FALLBACK_LIBRARY_PATH").is_none() {
            process.env(
                "DYLD_FALLBACK_LIBRARY_PATH",
                "/opt/homebrew/lib:/usr/local/lib:/usr/lib",
            );
        }
        let mut child = process
            .arg("--build-step")
            .arg(&config)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("Starting isolated build VM; host execution is not supported")?;
        let status = match tokio::time::timeout(self.timeout, child.wait()).await {
            Ok(result) => result.context("Waiting for isolated build VM")?,
            Err(_) => {
                child
                    .kill()
                    .await
                    .context("Terminating timed-out build VM")?;
                child.wait().await.context("Reaping timed-out build VM")?;
                bail!(
                    "Dockerfile RUN exceeded the build VM timeout of {:?}",
                    self.timeout
                );
            }
        };
        guest_config.restore()?;
        if !status.success() {
            bail!("Dockerfile RUN failed in isolated build VM ({status})");
        }
        Ok(())
    }

    fn runner_binary(&self) -> Result<PathBuf> {
        #[cfg(test)]
        if let Some(ref runner) = self.runner {
            return Ok(runner.clone());
        }
        find_runner_binary().context("Dockerfile RUN requires microvm-runner and native libkrun; no host fallback is available")
    }
}

/// Preserve the image's init config outside the guest share during a RUN step.
struct GuestConfigGuard {
    path: PathBuf,
    original: Option<PathBuf>,
    active: bool,
}

impl GuestConfigGuard {
    fn prepare(rootfs: &Path, private: &Path) -> Result<Self> {
        let path = rootfs.join(KrunConfig::FILE_NAME);
        let original = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let backup = private.join("original-init-config");
                fs::rename(&path, &backup).context("Preserving original guest init config")?;
                Some(backup)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            original,
            active: true,
        })
    }

    fn restore(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(&self.path)?,
            Ok(_) => fs::remove_file(&self.path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(original) = &self.original {
            fs::rename(original, &self.path).context("Restoring original guest init config")?;
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for GuestConfigGuard {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            tracing::warn!(%error, "Failed to restore temporary build init config");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(directory: &Path, body: &str) -> PathBuf {
        let path = directory.join("runner");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn guest_command_is_data_not_a_host_executable() {
        let temp = tempfile::tempdir().unwrap();
        let rootfs = temp.path().join("rootfs");
        fs::create_dir(&rootfs).unwrap();
        let capture = temp.path().join("captured.json");
        let mode = temp.path().join("mode");
        let executable = runner(
            temp.path(),
            &format!(
                "printf '%s' \"$1\" > '{}'; /bin/cp \"$2\" '{}'",
                mode.display(),
                capture.display()
            ),
        );
        let executor = BuildVmExecutor {
            runner: Some(executable),
            ..Default::default()
        };
        let command = KrunConfig::new(
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo '$HOME; touch /host'".into(),
            ],
            vec!["BUILD_VALUE=quoted '$; value".into()],
            Some("/app".into()),
        );
        executor
            .execute(&rootfs, command.clone(), temp.path())
            .await
            .unwrap();
        let captured: BuildStepConfig =
            serde_json::from_slice(&fs::read(&capture).unwrap()).unwrap();
        assert_eq!(fs::read_to_string(mode).unwrap(), "--build-step");
        assert_eq!(captured.command.cmd, command.cmd);
        assert_eq!(captured.command.working_dir, "/app");
        assert!(captured.command.env.contains(&command.env[0]));
        assert!(captured
            .command
            .env
            .iter()
            .any(|env| env.starts_with("PATH=")));
        assert!(captured.vm.virtiofs_mounts.is_empty());
        assert!(captured.vm.disks.is_empty());
        assert!(captured.vm.port_forwards.is_empty());
        assert!(captured.vm.sandbox);
        assert!(!captured.vm.detach);
        assert_eq!(captured.vm.root_path, rootfs.canonicalize().unwrap());
        assert_eq!(
            fs::read_dir(temp.path().join("build-runs"))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn failed_runner_never_falls_back_to_host_execution() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = temp.path().join("must-not-exist");
        let executable = runner(temp.path(), "exit 73");
        let executor = BuildVmExecutor {
            runner: Some(executable),
            ..Default::default()
        };
        let command = KrunConfig::new(
            vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("touch '{}'", sentinel.display()),
            ],
            vec![],
            None,
        );
        assert!(executor
            .execute(temp.path(), command, temp.path())
            .await
            .is_err());
        assert!(!sentinel.exists());
    }

    #[tokio::test]
    async fn missing_runner_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let executor = BuildVmExecutor {
            runner: Some(temp.path().join("missing")),
            ..Default::default()
        };
        let command = KrunConfig::new(vec!["/bin/true".into()], vec![], None);
        assert!(executor
            .execute(temp.path(), command, temp.path())
            .await
            .is_err());
        assert_eq!(
            fs::read_dir(temp.path().join("build-runs"))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn timed_out_runner_is_terminated_and_request_removed() {
        let temp = tempfile::tempdir().unwrap();
        let executable = runner(temp.path(), "exec /bin/sleep 30");
        let executor = BuildVmExecutor {
            runner: Some(executable),
            timeout: Duration::from_millis(50),
        };
        let command = KrunConfig::new(vec!["/bin/true".into()], vec![], None);
        let error = executor
            .execute(temp.path(), command, temp.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timeout"));
        assert_eq!(
            fs::read_dir(temp.path().join("build-runs"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn empty_run_command_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        assert!(BuildStepConfig::new(temp.path(), KrunConfig::new(vec![], vec![], None)).is_err());
    }

    #[test]
    fn original_init_config_is_restored_without_following_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let private = temp.path().join("private");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&private).unwrap();
        let host = temp.path().join("host");
        fs::write(&host, b"host data").unwrap();
        let config = root.join(KrunConfig::FILE_NAME);
        std::os::unix::fs::symlink(&host, &config).unwrap();
        {
            let mut guard = GuestConfigGuard::prepare(&root, &private).unwrap();
            KrunConfig::new(vec!["/bin/true".into()], vec![], None)
                .write_to(&root)
                .unwrap();
            assert!(!config.is_symlink());
            guard.restore().unwrap();
        }
        assert_eq!(fs::read_link(config).unwrap(), host);
        assert_eq!(fs::read(host).unwrap(), b"host data");
    }

    #[test]
    fn temporary_init_config_is_removed_after_failure() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let private = temp.path().join("private");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&private).unwrap();
        {
            let _guard = GuestConfigGuard::prepare(&root, &private).unwrap();
            KrunConfig::new(vec!["/bin/false".into()], vec![], None)
                .write_to(&root)
                .unwrap();
        }
        assert!(!root.join(KrunConfig::FILE_NAME).exists());
    }
}
