//! Strict OCI digest validation before network access or filesystem path construction.
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};

pub(crate) fn sha256_hex(digest: &str) -> Result<&str> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        bail!("Unsupported OCI digest: expected sha256");
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("Invalid OCI SHA-256 digest: expected 64 lowercase hexadecimal characters");
    }
    Ok(hex)
}

pub(crate) fn verify_sha256(digest: &str, bytes: &[u8]) -> Result<()> {
    let expected = sha256_hex(digest)?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        bail!("Cryptographic digest mismatch: expected {digest}, got sha256:{actual}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_digests_cannot_become_paths() {
        for digest in [
            "sha256:../../outside",
            "sha256:/etc/passwd",
            "sha512:abcd",
            "sha256:abc",
            "",
            "sha256:fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffG",
        ] {
            assert!(sha256_hex(digest).is_err(), "{digest}");
        }
    }

    #[test]
    fn content_integrity_is_verified() {
        let digest = format!("sha256:{:x}", Sha256::digest(b"trusted"));
        assert!(verify_sha256(&digest, b"trusted").is_ok());
        assert!(verify_sha256(&digest, b"tampered").is_err());
    }
}
