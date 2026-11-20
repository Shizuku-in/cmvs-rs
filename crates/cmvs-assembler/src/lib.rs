//! Offline resource conversion, script assembly and verified archive replacement.
pub mod archive;
pub mod image;
pub mod script;

pub use cmvs_tooling::{Result, write_new};
pub mod build;
pub mod cursor;
pub mod pb;

use std::path::{Component, Path, PathBuf};

/// Resolve an input relative to its manifest, within a canonical project root.
fn manifest_input(root: &Path, dir: &Path, input: &Path) -> Result<PathBuf> {
    if input
        .components()
        .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    {
        return Err("expected a manifest-relative input path".into());
    }
    let path = dir.join(input).canonicalize()?;
    if !path.starts_with(root) {
        return Err("input resolves outside the project".into());
    }
    Ok(path)
}
