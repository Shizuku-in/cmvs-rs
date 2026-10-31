//! Game-agnostic headless probe for CMVS builds. The design and its rules are in
//! docs/design/probe.md: no game identity, script name or script address appears in this
//! crate; state is judged from engine-level observations, build differences are
//! capabilities, and every result states the verdict level it reached.
pub mod checks;
pub mod device;
pub mod json;
pub mod observe;
pub mod report;
pub mod session;
pub mod target;
