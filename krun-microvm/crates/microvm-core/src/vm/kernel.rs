// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

/// Parses a string representation of a kernel payload format (raw, elf, gz, bz2, zstd, pe).
pub fn parse_kernel_format(s: &str) -> Option<u32> {
    match s.trim().to_lowercase().as_str() {
        "raw" | "bin" | "m1n1" => {
            #[cfg(target_arch = "aarch64")]
            {
                Some(krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ)
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                Some(krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_RAW)
            }
        }
        "elf" | "vmlinux" => Some(krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_ELF),
        "gz" | "gzip" | "image.gz" | "vmlinuz" => {
            #[cfg(target_arch = "aarch64")]
            {
                Some(krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ)
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                Some(krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_GZ)
            }
        }
        "pe" | "pe_gz" => Some(krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ),
        "bz2" | "bzip2" => Some(krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_BZ2),
        "zstd" | "zst" => Some(krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_ZSTD),
        _ => None,
    }
}

/// Automatically inspects binary magic bytes and file extension to determine the kernel format.
/// Supports Asahi Linux gzip kernels (Image.gz / vmlinuz-asahi) and m1n1 raw payloads.
pub fn detect_kernel_format(path: &Path) -> u32 {
    if let Ok(mut file) = std::fs::File::open(path) {
        use std::io::Read;
        let mut magic = [0u8; 4];
        if file.read_exact(&mut magic).is_ok() {
            // Gzip magic (0x1f, 0x8b) - standard compressed kernel Image.gz / vmlinuz-asahi
            if magic[0] == 0x1f && magic[1] == 0x8b {
                #[cfg(target_arch = "aarch64")]
                return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ;
                #[cfg(not(target_arch = "aarch64"))]
                return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_GZ;
            }
            // Zstandard magic (0x28, 0xb5, 0x2f, 0xfd)
            if magic[0] == 0x28 && magic[1] == 0xb5 && magic[2] == 0x2f && magic[3] == 0xfd {
                return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_ZSTD;
            }
            // Bzip2 magic ("BZ")
            if magic[0] == b'B' && magic[1] == b'Z' {
                return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_BZ2;
            }
            // Standard ELF header
            if magic[0] == 0x7f && magic[1] == b'E' && magic[2] == b'L' && magic[3] == b'F' {
                return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_ELF;
            }
        }
    }

    // Filename extension inspection fallback
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let lower = name.to_lowercase();
        if lower.ends_with(".gz") || lower.contains("vmlinuz") {
            #[cfg(target_arch = "aarch64")]
            return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ;
            #[cfg(not(target_arch = "aarch64"))]
            return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_GZ;
        }
        if lower.ends_with(".zst") || lower.ends_with(".zstd") {
            return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_ZSTD;
        }
        if lower.ends_with(".bz2") {
            return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_IMAGE_BZ2;
        }
        if lower.ends_with(".bin") || lower.contains("m1n1") || lower.ends_with(".raw") {
            #[cfg(target_arch = "aarch64")]
            return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ;
            #[cfg(not(target_arch = "aarch64"))]
            return krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_RAW;
        }
    }

    // Architecture-dependent default (AArch64 defaults to PE_GZ for flat arm64 Image / m1n1 payloads)
    #[cfg(target_arch = "aarch64")]
    {
        krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_PE_GZ
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        krun_sys::kernel_formats::KRUN_KERNEL_FORMAT_ELF
    }
}

/// Locates the Rosetta for Linux runtime directory on the host (macOS Apple Silicon or via ROSETTA_PATH).
pub fn find_rosetta_path() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("ROSETTA_PATH") {
        let p = PathBuf::from(env_path);
        if p.exists() {
            return Some(p);
        }
    }
    let candidate = PathBuf::from("/Library/Apple/usr/libexec/oah/RosettaLinux");
    if candidate.exists() && candidate.join("rosetta").exists() {
        return Some(candidate);
    }
    None
}
