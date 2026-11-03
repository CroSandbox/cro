// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

pub mod builder;
pub mod kernel;
pub mod lifecycle;
pub mod runtime;

#[cfg(test)]
mod tests;

pub use builder::MicroVmBuilder;
pub use kernel::{detect_kernel_format, find_rosetta_path, parse_kernel_format};
pub use runtime::MicroVm;

pub use crate::types::{
    ArtifactMount, BootPayload, CowWorkspace, DiskAttachment, FirmwarePayload,
    FreePageReportingMode, FsOverlayEntry, KernelPayload, ModelAttachment, PortForward,
    RunnerConfig, VirtioFsMount, VsockPort,
};
