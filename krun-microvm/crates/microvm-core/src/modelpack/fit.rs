// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Pre-flight hardware memory profiling and fit estimation for local models.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Suitability verdict for running a model within available host memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitVerdict {
    /// Model comfortably fits with ample headroom (< 70% available RAM).
    Comfortable,
    /// Model will run but consumes most available memory (70% - 95% available RAM).
    Tight,
    /// Model will exceed available host RAM and likely cause an Out-Of-Memory (OOM) crash.
    ExceedsMemory,
}

impl std::fmt::Display for FitVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Comfortable => write!(f, "✅ Comfortable Fit (Optimal)"),
            Self::Tight => write!(f, "⚠️ Tight Fit (Low Headroom)"),
            Self::ExceedsMemory => write!(f, "❌ Exceeds Host Memory (OOM Risk)"),
        }
    }
}

/// Pre-flight memory analysis and recommended microVM sizing for a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareFit {
    /// Raw on-disk model weight size in bytes.
    pub model_size_bytes: u64,
    /// Target context length in tokens.
    pub context_length: u64,
    /// Estimated memory required for KV cache at target context length.
    pub estimated_kv_cache_bytes: u64,
    /// Hypervisor and Linux guest runtime overhead in bytes.
    pub runtime_overhead_bytes: u64,
    /// Total estimated working memory required.
    pub total_required_bytes: u64,
    /// Total host physical memory in bytes.
    pub host_total_bytes: u64,
    /// Currently available/free host memory in bytes.
    pub host_available_bytes: u64,
    /// Recommended guest RAM allocation in MiB (`--memory`).
    pub recommended_guest_ram_mib: u32,
    /// Recommended Virtio-GPU shared memory vRAM window in GiB (`--gpu-shm-size`).
    pub recommended_gpu_shm_gib: u32,
    /// Suitability verdict.
    pub verdict: FitVerdict,
}

impl HardwareFit {
    /// Pure calculation of memory requirement against host capacity.
    pub fn calculate(
        model_size_bytes: u64,
        context_length: u64,
        host_total_bytes: u64,
        host_available_bytes: u64,
    ) -> Self {
        // Standard KV cache estimation heuristic:
        // For a typical model, KV cache takes ~256 KB per token at 16-bit or ~128 KB with quantization.
        // Approx 256 * 1024 bytes per token per 1024 context tokens.
        let estimated_kv_cache_bytes = (context_length as f64 * 256.0 * 1024.0) as u64;

        // Guest kernel + supervisor overhead: 1024 MiB (1 GB)
        let runtime_overhead_bytes = 1024 * 1024 * 1024;

        let total_required_bytes =
            model_size_bytes + estimated_kv_cache_bytes + runtime_overhead_bytes;

        // Add 512 MiB safety headroom for guest allocations and convert to MiB
        let recommended_guest_ram_mib =
            ((total_required_bytes + (512 * 1024 * 1024)) / (1024 * 1024)) as u32;

        // Calculate GPU shared memory vRAM: model weights + KV cache rounded up to next GiB
        let recommended_gpu_shm_gib = (((model_size_bytes + estimated_kv_cache_bytes) as f64
            / (1024.0 * 1024.0 * 1024.0))
            .ceil() as u32)
            .max(1);

        let verdict = if total_required_bytes <= (host_available_bytes * 70) / 100 {
            FitVerdict::Comfortable
        } else if total_required_bytes <= host_available_bytes {
            FitVerdict::Tight
        } else {
            FitVerdict::ExceedsMemory
        };

        Self {
            model_size_bytes,
            context_length,
            estimated_kv_cache_bytes,
            runtime_overhead_bytes,
            total_required_bytes,
            host_total_bytes,
            host_available_bytes,
            recommended_guest_ram_mib,
            recommended_gpu_shm_gib,
            verdict,
        }
    }

    /// Profiles a model file or directory against local host hardware.
    pub fn profile(model_path: &Path, context_length: Option<u64>) -> Result<Self> {
        if !model_path.exists() {
            bail!("Model path does not exist: {}", model_path.display());
        }

        let model_size_bytes = if model_path.is_file() {
            fs::metadata(model_path)?.len()
        } else {
            Self::dir_size(model_path)
        };

        let ctx_len = context_length.unwrap_or(4096);
        let (host_total, host_available) = Self::get_host_memory();

        Ok(Self::calculate(
            model_size_bytes,
            ctx_len,
            host_total,
            host_available,
        ))
    }

    fn dir_size(dir: &Path) -> u64 {
        let mut total = 0;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(m) = entry.metadata() {
                    if m.is_file() {
                        total += m.len();
                    } else if m.is_dir() {
                        total += Self::dir_size(&entry.path());
                    }
                }
            }
        }
        total
    }

    /// Returns (host_total_bytes, host_available_bytes).
    fn get_host_memory() -> (u64, u64) {
        #[cfg(target_os = "macos")]
        {
            use std::mem;
            let mut total: u64 = 0;
            let mut size = mem::size_of::<u64>();
            let mib = [libc::CTL_HW, libc::HW_MEMSIZE];
            let ret = unsafe {
                libc::sysctl(
                    mib.as_ptr() as *mut libc::c_int,
                    2,
                    &mut total as *mut u64 as *mut libc::c_void,
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                )
            };
            let host_total = if ret == 0 {
                total
            } else {
                16 * 1024 * 1024 * 1024
            };
            // On macOS, estimate free/available as ~75% of total if inactive pages can be reclaimed
            let host_available = (host_total * 75) / 100;
            (host_total, host_available)
        }

        #[cfg(target_os = "linux")]
        {
            let mut total = 16 * 1024 * 1024 * 1024;
            let mut available = 12 * 1024 * 1024 * 1024;
            if let Ok(content) = fs::read_to_string("/proc/meminfo") {
                for line in content.lines() {
                    if line.starts_with("MemTotal:") {
                        if let Some(val) = Self::parse_meminfo_line(line) {
                            total = val;
                        }
                    } else if line.starts_with("MemAvailable:") {
                        if let Some(val) = Self::parse_meminfo_line(line) {
                            available = val;
                        }
                    }
                }
            }
            (total, available)
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            (16 * 1024 * 1024 * 1024, 8 * 1024 * 1024 * 1024)
        }
    }

    #[cfg(target_os = "linux")]
    fn parse_meminfo_line(line: &str) -> Option<u64> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            parts[1].parse::<u64>().ok().map(|kb| kb * 1024)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_fit_verdicts() {
        let host_total = 16 * 1024 * 1024 * 1024; // 16 GB
        let host_available = 12 * 1024 * 1024 * 1024; // 12 GB

        // 1. Small 2 GB model -> Comfortable Fit
        let fit1 = HardwareFit::calculate(2 * 1024 * 1024 * 1024, 2048, host_total, host_available);
        assert_eq!(fit1.verdict, FitVerdict::Comfortable);
        assert!(fit1.recommended_guest_ram_mib > 2000);
        assert!(fit1.recommended_gpu_shm_gib >= 2);

        // 2. 8 GB model -> Tight Fit
        let fit2 = HardwareFit::calculate(8 * 1024 * 1024 * 1024, 4096, host_total, host_available);
        assert_eq!(fit2.verdict, FitVerdict::Tight);

        // 3. 24 GB model -> Exceeds Memory
        let fit3 =
            HardwareFit::calculate(24 * 1024 * 1024 * 1024, 4096, host_total, host_available);
        assert_eq!(fit3.verdict, FitVerdict::ExceedsMemory);
    }
}
