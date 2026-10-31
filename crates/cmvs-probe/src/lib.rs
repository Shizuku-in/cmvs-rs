//! Game-agnostic headless probe for CMVS builds. No game identity, script name or script
//! address appears in this crate; state is judged from engine-level observations, build
//! differences are capabilities, and every result states the verdict level it reached.
pub mod checks;
pub mod controls;
pub mod device;
pub mod json;
pub mod manifest;
pub mod observe;
pub mod report;
pub mod scenario;
pub mod session;
pub mod target;
