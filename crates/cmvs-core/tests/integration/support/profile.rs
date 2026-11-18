use cmvs_core::{
    format::exe::Arch,
    profile::{Binary, CommandStatus, EngineProfile, Family},
};

/// Every command is present so playback tests can exercise any registered handler.
pub fn synthetic_profile(id: &str) -> EngineProfile {
    EngineProfile::new(
        id.into(),
        Family::Cmvs,
        None,
        vec![Binary {
            arch: Arch::X64,
            file: "cmvs64.exe".into(),
            sha256: [0; 32],
        }],
        vec![CommandStatus::Base; 2048],
        vec![],
        vec![],
    )
}
