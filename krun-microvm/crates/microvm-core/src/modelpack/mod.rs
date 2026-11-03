// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! CNCF ModelPack Specification Implementation for `cro`.
//!
//! Provides model packaging, registry distribution, and microVM zero-copy execution.

pub mod discover;
pub mod engine;
pub mod fit;
pub mod pack;
pub mod spec;

pub use discover::{DiscoveredModel, ModelDiscovery, ModelSource};
pub use engine::{CachedModelMetadata, ModelEngine};
pub use fit::{FitVerdict, HardwareFit};
pub use pack::{ModelPackOptions, ModelPacker};
pub use spec::{
    ModelConfig, ModelDescriptor, ModelFileEntry, ModelFileRole, ModelFormat, ModelManifest,
    MODELPACK_CONFIG_LAYER_MEDIA_TYPE, MODELPACK_CONFIG_MEDIA_TYPE, MODELPACK_DOC_LAYER_MEDIA_TYPE,
    MODELPACK_LEGACY_ARTIFACT_TYPE, MODELPACK_MANIFEST_ARTIFACT_TYPE,
    MODELPACK_TOKENIZER_LAYER_MEDIA_TYPE, MODELPACK_WEIGHT_LAYER_MEDIA_TYPE,
    OCI_IMAGE_MANIFEST_MEDIA_TYPE,
};
