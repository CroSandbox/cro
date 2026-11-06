//! Content identity for completed build filesystems, without following symlinks.
use crate::config::OciConfig;
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::os::unix::{
    ffi::OsStrExt,
    fs::{OpenOptionsExt, PermissionsExt},
};
use std::path::Path;

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

pub(crate) fn build_digest(root: &Path, config: &OciConfig) -> Result<String> {
    let mut hash = Sha256::new();
    field(&mut hash, b"cro-build-filesystem-v1");
    field(&mut hash, &serde_json::to_vec(config)?);
    let root_metadata = fs::symlink_metadata(root)?;
    if !root_metadata.is_dir() {
        bail!("Build rootfs must be a directory");
    }
    hash.update((root_metadata.permissions().mode() & 0o7777).to_be_bytes());
    hash_directory(root, root, &mut hash)?;
    Ok(format!("sha256:{:x}", hash.finalize()))
}

fn hash_directory(root: &Path, directory: &Path, hash: &mut Sha256) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        field(hash, path.strip_prefix(root)?.as_os_str().as_bytes());
        hash.update((metadata.permissions().mode() & 0o7777).to_be_bytes());
        if metadata.is_symlink() {
            field(hash, b"symlink");
            field(hash, fs::read_link(&path)?.as_os_str().as_bytes());
        } else if metadata.is_dir() {
            field(hash, b"directory");
            hash_directory(root, &path, hash)?;
        } else if metadata.is_file() {
            field(hash, b"file");
            hash.update(metadata.len().to_be_bytes());
            let mut file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)?;
            let mut buffer = [0; 65536];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
        } else {
            bail!(
                "Build output contains unsupported special file: {}",
                path.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_output_files_cannot_alias_the_same_config() {
        let temp = tempfile::tempdir().unwrap();
        let config = OciConfig::default();
        fs::write(temp.path().join("file"), b"first").unwrap();
        let first = build_digest(temp.path(), &config).unwrap();
        fs::write(temp.path().join("file"), b"second").unwrap();
        assert_ne!(first, build_digest(temp.path(), &config).unwrap());
    }

    #[test]
    fn identity_is_independent_of_entry_creation_order() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for name in ["a", "b"] {
            fs::write(a.path().join(name), name).unwrap();
        }
        for name in ["b", "a"] {
            fs::write(b.path().join(name), name).unwrap();
        }
        assert_eq!(
            build_digest(a.path(), &OciConfig::default()).unwrap(),
            build_digest(b.path(), &OciConfig::default()).unwrap()
        );
    }

    #[test]
    fn symlink_identity_does_not_depend_on_host_target_contents() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let host = temp.path().join("host");
        fs::write(&host, b"first").unwrap();
        std::os::unix::fs::symlink(&host, root.join("link")).unwrap();
        let digest = build_digest(&root, &OciConfig::default()).unwrap();
        fs::write(&host, b"second").unwrap();
        assert_eq!(digest, build_digest(&root, &OciConfig::default()).unwrap());
    }
}
