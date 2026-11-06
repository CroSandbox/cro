//! Dockerfile builder with VM-isolated RUN steps and a local image registry.
//!
//! Allows constructing OCI container rootfs images directly from Dockerfiles
//! without requiring an external container runtime daemon.

mod digest;
pub mod dockerignore;
pub mod executor;
pub mod parser;
mod run;

pub use run::BuildStepConfig;

pub use dockerignore::Dockerignore;
pub use executor::{
    try_fetch_local_tag, BuildEngine, BuildOptions, BuildResult, LocalImageRecord,
    LocalImageRegistry,
};
pub use parser::{CommandForm, Dockerfile, Instruction};
