//! Shared cache lifecycle: lock, read completed metadata, stage privately, publish.
//!
//! Callers own transport and extraction. This module owns filesystem publication
//! and cancellation cleanup. Cache paths and metadata formats remain compatible.
use super::digest::sha256_hex;
use crate::rootfs::tar_security::write_file_in_root;
use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::{
    fs::{OpenOptionsExt, PermissionsExt},
    io::AsRawFd,
};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::{NamedTempFile, TempDir};

#[derive(Clone, Copy)]
pub(crate) enum CacheKind {
    Image,
    Model,
    Artifact,
}

impl CacheKind {
    fn directory(self) -> &'static str {
        match self {
            Self::Image => "rootfs",
            Self::Model => "models",
            Self::Artifact => "artifacts",
        }
    }

    fn marker(self) -> Option<&'static str> {
        match self {
            Self::Image => None,
            Self::Model => Some(".modelpack.json"),
            Self::Artifact => Some(".krun_artifact.json"),
        }
    }
}

struct FileLock(File);

impl FileLock {
    fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self(
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .with_context(|| format!("Opening cache lock {}", path.display()))?,
        ))
    }

    fn try_lock(&self, nonblocking: bool) -> io::Result<()> {
        let flags = libc::LOCK_EX | if nonblocking { libc::LOCK_NB } else { 0 };
        // SAFETY: this guard owns the descriptor for the duration of flock.
        if unsafe { libc::flock(self.0.as_raw_fd(), flags) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn acquire(path: &Path) -> Result<Self> {
        let lock = Self::open(path)?;
        loop {
            match lock.try_lock(false) {
                Ok(()) => return Ok(lock),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    async fn acquire_async(path: &Path) -> Result<Self> {
        let lock = Self::open(path)?;
        loop {
            match lock.try_lock(true) {
                Ok(()) => return Ok(lock),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

// Closing the descriptor releases flock, including on async cancellation.

pub(crate) struct CacheEntry {
    base: PathBuf,
    kind: CacheKind,
    target: PathBuf,
    metadata: PathBuf,
    _lock: FileLock,
}

impl CacheEntry {
    fn paths(base: &Path, kind: CacheKind, digest: &str) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let key = format!("sha256_{}", sha256_hex(digest)?);
        let target = base.join(kind.directory()).join(&key);
        let (metadata, lock_name) = match kind.marker() {
            Some(marker) => (
                target.join(marker),
                format!("{}-{key}.lock", kind.directory()),
            ),
            None => (
                base.join("configs").join(format!("{key}.json")),
                format!("{key}.lock"),
            ),
        };
        Ok((target, metadata, base.join("locks").join(lock_name)))
    }

    pub(crate) fn acquire(base: &Path, kind: CacheKind, digest: &str) -> Result<Self> {
        let (target, metadata, lock_path) = Self::paths(base, kind, digest)?;
        let lock = FileLock::acquire(&lock_path)?;
        Ok(Self {
            base: base.into(),
            kind,
            target,
            metadata,
            _lock: lock,
        })
    }

    pub(crate) async fn acquire_async(base: &Path, kind: CacheKind, digest: &str) -> Result<Self> {
        let (target, metadata, lock_path) = Self::paths(base, kind, digest)?;
        let lock = FileLock::acquire_async(&lock_path).await?;
        Ok(Self {
            base: base.into(),
            kind,
            target,
            metadata,
            _lock: lock,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.target
    }

    pub(crate) fn metadata_path(&self) -> &Path {
        &self.metadata
    }

    pub(crate) fn read_metadata<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        if self.target.is_symlink() || !self.target.is_dir() || self.metadata.is_symlink() {
            return Ok(None);
        }
        let bytes = match fs::read(&self.metadata) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("Reading cache completion metadata"),
        };
        match serde_json::from_slice(&bytes) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) => {
                tracing::warn!(path = %self.metadata.display(), %error, "Ignoring incomplete cache metadata");
                Ok(None)
            }
        }
    }

    pub(crate) fn stage(&self) -> Result<CacheTransaction<'_>> {
        let parent = self.base.join("staging");
        fs::create_dir_all(&parent)?;
        let directory = tempfile::Builder::new()
            .prefix(self.kind.directory())
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(parent)?;
        // Keep the wrapper private while allowing the payload root to retain
        // normal filesystem permissions when it becomes a guest rootfs.
        let payload = directory.path().join("data");
        fs::create_dir(&payload)?;
        Ok(CacheTransaction {
            entry: self,
            directory,
            payload,
        })
    }
}

pub(crate) struct CacheTransaction<'a> {
    entry: &'a CacheEntry,
    directory: TempDir,
    payload: PathBuf,
}

impl CacheTransaction<'_> {
    pub(crate) fn path(&self) -> &Path {
        &self.payload
    }

    pub(crate) fn commit<T: Serialize>(self, metadata: &T) -> Result<PathBuf> {
        // Serialization and metadata writes must succeed before replacing existing data.
        let bytes = serde_json::to_vec_pretty(metadata)?;
        let external_metadata = if let Some(marker) = self.entry.kind.marker() {
            write_file_in_root(self.path(), Path::new(marker), &bytes)?;
            None
        } else {
            let parent = self
                .entry
                .metadata
                .parent()
                .context("Missing metadata parent")?;
            fs::create_dir_all(parent)?;
            let mut file = NamedTempFile::new_in(parent)?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            Some(file)
        };

        let publication = DirectoryPublication::install(
            self.path(),
            self.entry.path(),
            self.directory
                .path()
                .parent()
                .context("Missing staging parent")?,
        )?;
        if let Some(file) = external_metadata {
            // Readers hold the same entry lock, so they cannot see a mismatched pair.
            file.persist(&self.entry.metadata)
                .context("Publishing image cache metadata")?;
        }
        publication.finish();
        Ok(self.entry.target.clone())
    }
}

/// Rolls back a directory replacement if publishing its completion marker fails.
struct DirectoryPublication {
    target: PathBuf,
    backup: Option<TempDir>,
    installed: bool,
    finished: bool,
}

impl DirectoryPublication {
    fn install(staged: &Path, target: &Path, backup_parent: &Path) -> Result<Self> {
        let parent = target.parent().context("Missing cache parent")?;
        fs::create_dir_all(parent)?;
        let mut publication = Self {
            target: target.into(),
            backup: None,
            installed: false,
            finished: false,
        };
        if fs::symlink_metadata(target).is_ok() {
            // Keep backups outside namespace directories so discovery cannot list them.
            let backup = tempfile::Builder::new()
                .prefix("backup-")
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir_in(backup_parent)?;
            fs::rename(target, backup.path().join("previous"))?;
            publication.backup = Some(backup);
        }
        fs::rename(staged, target).context("Publishing staged cache directory")?;
        publication.installed = true;
        Ok(publication)
    }

    fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for DirectoryPublication {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if self.installed {
            if let Err(error) = fs::remove_dir_all(&self.target) {
                tracing::error!(%error, path = %self.target.display(), "Failed to remove incomplete cache publication");
            }
        }
        if let Some(backup) = self.backup.take() {
            if let Err(error) = fs::rename(backup.path().join("previous"), &self.target) {
                let recovery = backup.keep();
                tracing::error!(%error, path = %recovery.display(), "Cache rollback retained for recovery");
            }
        }
    }
}

/// Publish a single cache file through a sibling temporary file on the same filesystem.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing cache file parent")?;
    fs::create_dir_all(parent)?;
    let mut file = NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.persist(path)
        .with_context(|| format!("Publishing cache file {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn digest() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    #[test]
    fn artifact_discovery_never_sees_staged_metadata() {
        use crate::oci::{ArtifactMetadata, OciArtifact};
        let temp = tempfile::tempdir().unwrap();
        let entry = CacheEntry::acquire(temp.path(), CacheKind::Artifact, &digest()).unwrap();
        let transaction = entry.stage().unwrap();
        let metadata = ArtifactMetadata {
            reference: "example/model:latest".into(),
            digest: digest(),
            files: vec!["weights".into()],
            created_at: 1,
        };
        fs::write(transaction.path().join("weights"), b"data").unwrap();
        fs::write(
            transaction.path().join(".krun_artifact.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        assert!(OciArtifact::list_cached(temp.path()).unwrap().is_empty());
        transaction.commit(&metadata).unwrap();
        assert_eq!(OciArtifact::list_cached(temp.path()).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cancelling_an_active_transaction_removes_staging_and_releases_lock() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().to_path_buf();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let entry = CacheEntry::acquire_async(&base, CacheKind::Artifact, &digest())
                .await
                .unwrap();
            let transaction = entry.stage().unwrap();
            sender.send(transaction.path().to_path_buf()).unwrap();
            std::future::pending::<()>().await;
            drop(transaction);
        });
        let staging = receiver.await.unwrap();
        assert!(staging.exists());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!staging.exists());
        let _next = tokio::time::timeout(
            Duration::from_secs(2),
            CacheEntry::acquire_async(temp.path(), CacheKind::Artifact, &digest()),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[test]
    fn completed_entries_keep_existing_paths_and_metadata_formats() {
        let temp = tempfile::tempdir().unwrap();
        for kind in [CacheKind::Image, CacheKind::Model, CacheKind::Artifact] {
            let entry = CacheEntry::acquire(temp.path(), kind, &digest()).unwrap();
            assert!(entry.read_metadata::<Value>().unwrap().is_none());
            let transaction = entry.stage().unwrap();
            use std::os::unix::fs::PermissionsExt;
            let wrapper_mode = fs::metadata(transaction.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                wrapper_mode & 0o077,
                0,
                "staging wrapper must remain private"
            );
            fs::set_permissions(transaction.path(), fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(transaction.path().join("content"), b"complete").unwrap();
            let metadata = json!({"digest": digest()});
            let target = transaction.commit(&metadata).unwrap();
            assert_eq!(
                target,
                temp.path()
                    .join(kind.directory())
                    .join(format!("sha256_{}", "a".repeat(64)))
            );
            assert_eq!(fs::read(target.join("content")).unwrap(), b"complete");
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o755
            );
            assert_eq!(entry.read_metadata::<Value>().unwrap(), Some(metadata));
        }
    }

    #[test]
    fn abandoned_transactions_do_not_touch_completed_entries() {
        let temp = tempfile::tempdir().unwrap();
        let entry = CacheEntry::acquire(temp.path(), CacheKind::Model, &digest()).unwrap();
        let transaction = entry.stage().unwrap();
        fs::write(transaction.path().join("weights"), b"original").unwrap();
        transaction.commit(&json!({"version": 1})).unwrap();
        let first = entry.stage().unwrap();
        let second = entry.stage().unwrap();
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();
        fs::write(first.path().join("partial"), b"unfinished").unwrap();
        drop(first);
        assert!(!first_path.exists());
        assert!(second_path.exists());
        assert_eq!(fs::read(entry.path().join("weights")).unwrap(), b"original");
        assert_eq!(
            entry.read_metadata::<Value>().unwrap(),
            Some(json!({"version": 1}))
        );
    }

    #[test]
    fn metadata_publication_failure_restores_previous_directory() {
        let temp = tempfile::tempdir().unwrap();
        let entry = CacheEntry::acquire(temp.path(), CacheKind::Image, &digest()).unwrap();
        fs::create_dir_all(entry.path()).unwrap();
        fs::write(entry.path().join("original"), b"keep").unwrap();
        // Force the final metadata rename to fail after directory installation.
        fs::create_dir_all(entry.metadata_path()).unwrap();
        let transaction = entry.stage().unwrap();
        let staging = transaction.path().to_path_buf();
        fs::write(transaction.path().join("replacement"), b"new").unwrap();
        assert!(transaction.commit(&json!({})).is_err());
        assert_eq!(fs::read(entry.path().join("original")).unwrap(), b"keep");
        assert!(!entry.path().join("replacement").exists());
        assert!(!staging.exists());
        assert_eq!(
            fs::read_dir(temp.path().join("staging")).unwrap().count(),
            0
        );
    }

    #[test]
    fn serialization_failure_never_publishes_a_directory() {
        struct InvalidMetadata;
        impl Serialize for InvalidMetadata {
            fn serialize<S: serde::Serializer>(
                &self,
                _serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom(
                    "deliberate serialization failure",
                ))
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let entry = CacheEntry::acquire(temp.path(), CacheKind::Artifact, &digest()).unwrap();
        let transaction = entry.stage().unwrap();
        let staging = transaction.path().to_path_buf();
        assert!(transaction.commit(&InvalidMetadata).is_err());
        assert!(!staging.exists());
        assert!(!entry.path().exists());
    }

    #[test]
    fn corrupt_and_missing_metadata_are_cache_misses() {
        let temp = tempfile::tempdir().unwrap();
        let entry = CacheEntry::acquire(temp.path(), CacheKind::Model, &digest()).unwrap();
        fs::create_dir_all(entry.path()).unwrap();
        assert!(entry.read_metadata::<Value>().unwrap().is_none());
        fs::write(entry.metadata_path(), b"{partial").unwrap();
        assert!(entry.read_metadata::<Value>().unwrap().is_none());
    }

    #[test]
    fn invalid_digests_create_no_cache_state() {
        let temp = tempfile::tempdir().unwrap();
        assert!(CacheEntry::acquire(temp.path(), CacheKind::Image, "sha256:../../escape").is_err());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lock_contention_does_not_block_the_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let entry = CacheEntry::acquire(temp.path(), CacheKind::Image, &digest()).unwrap();
        let path = temp.path().to_path_buf();
        let waiter = tokio::spawn(async move {
            let next = CacheEntry::acquire_async(&path, CacheKind::Image, &digest())
                .await
                .unwrap();
            next.read_metadata::<Value>().unwrap()
        });
        // This timer must remain runnable while the same-entry waiter is pending.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(!waiter.is_finished());
        entry
            .stage()
            .unwrap()
            .commit(&json!({"published": true}))
            .unwrap();
        drop(entry);
        let cached = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cached, Some(json!({"published": true})));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_lock_waiters_do_not_leak_locks() {
        let temp = tempfile::tempdir().unwrap();
        let first = CacheEntry::acquire(temp.path(), CacheKind::Model, &digest()).unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            CacheEntry::acquire_async(temp.path(), CacheKind::Model, &digest()),
        )
        .await;
        assert!(result.is_err());
        drop(first);
        let _next = tokio::time::timeout(
            Duration::from_secs(2),
            CacheEntry::acquire_async(temp.path(), CacheKind::Model, &digest()),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[test]
    fn staged_metadata_cannot_overwrite_a_host_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let host_file = temp.path().join("host");
        fs::write(&host_file, b"host data").unwrap();
        let entry = CacheEntry::acquire(temp.path(), CacheKind::Model, &digest()).unwrap();
        let transaction = entry.stage().unwrap();
        std::os::unix::fs::symlink(&host_file, transaction.path().join(".modelpack.json")).unwrap();
        transaction.commit(&json!({"safe": true})).unwrap();
        assert_eq!(fs::read(host_file).unwrap(), b"host data");
        assert_eq!(
            entry.read_metadata::<Value>().unwrap(),
            Some(json!({"safe": true}))
        );
    }
}
