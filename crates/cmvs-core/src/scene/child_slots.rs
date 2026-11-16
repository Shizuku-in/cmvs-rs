//! Runtime child-table layout. Snapshot codecs are selected independently.
use crate::profile::{EngineProfile, InspectedBuild};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ChildSlots {
    Slots256,
    Slots512,
    #[default]
    Slots768,
    Slots1024,
}
impl ChildSlots {
    pub(crate) fn for_profile(profile: &EngineProfile) -> Self {
        match profile.inspected_build() {
            // Independent x86 256-entry helpers: docs/games/{puriring,asukimi}.
            Some(
                InspectedBuild::Cmvs20070119
                | InspectedBuild::Cmvs20071117
                | InspectedBuild::Cmvs20080903
                | InspectedBuild::Cmvs20090815
                | InspectedBuild::Cmvs20100314
                | InspectedBuild::Cmvs20101211,
            ) => Self::Slots256,
            // Independent 512-entry create/destroy helpers: docs/games/kazokubu.
            Some(InspectedBuild::Cmvs20120309) => Self::Slots512,
            // Independent x64 evidence: docs/games/{realive,seishunf,kunado,hapymaher_re}.
            Some(
                InspectedBuild::Cmvs20190305
                | InspectedBuild::Cmvs20190405
                | InspectedBuild::Cmvs20211006
                | InspectedBuild::Cmvs20221105,
            ) => Self::Slots1024,
            _ => Self::Slots768,
        }
    }
    pub(crate) fn count(self) -> u16 {
        match self {
            Self::Slots256 => 256,
            Self::Slots512 => 512,
            Self::Slots768 => 768,
            Self::Slots1024 => 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_table_capacity_requires_the_inspected_profile_and_binary_identity() {
        let registry = crate::profile::Registry::builtin();
        let profile = registry.profile("cmvs-20071117").unwrap();
        assert_eq!(ChildSlots::for_profile(profile).count(), 256);
        assert!(crate::runtime::snapshot_layout_supported(profile));
        let mut changed = profile.clone();
        changed.binaries[0].sha256 = [0; 32];
        assert_eq!(ChildSlots::for_profile(&changed).count(), 768);
        let mut changed = profile.clone();
        changed.base = Some("unverified-base".into());
        assert_eq!(ChildSlots::for_profile(&changed).count(), 768);
        assert!(!crate::runtime::snapshot_layout_supported(&changed));
        let mut scene = super::super::SceneHost::for_profile(profile);
        assert!(scene.config.allow_filter_mode_one);
        assert!(scene.config.allow_legacy_sine_modes);
        scene.restore(
            Default::default(),
            Default::default(),
            None,
            None,
            None,
            None,
        );
        assert!(scene.config.allow_filter_mode_one);
        assert!(scene.config.allow_legacy_sine_modes);
        let scene = super::super::SceneHost::for_profile(&changed);
        assert!(!scene.config.allow_filter_mode_one);
        assert!(!scene.config.allow_legacy_sine_modes);
        for (id, count) in [
            ("cmvs-20101211", 256),
            ("cmvs-20120309", 512),
            ("cmvs-20190305", 1024),
            ("cmvs-20190405", 1024),
            ("cmvs-20211006", 1024),
            ("cmvs-20221105", 1024),
        ] {
            let profile = registry.profile(id).unwrap();
            assert_eq!(ChildSlots::for_profile(profile).count(), count);
            assert!(crate::runtime::snapshot_layout_supported(profile));
            let mut scene = super::super::SceneHost::for_profile(profile);
            scene.restore(
                Default::default(),
                Default::default(),
                None,
                None,
                None,
                None,
            );
            assert_eq!(scene.config.child_slots.count(), count);

            for id in ["cmvs-20160201", "cmvs-20171114"] {
                assert_eq!(
                    ChildSlots::for_profile(registry.profile(id).unwrap()).count(),
                    768
                );
            }
            let mut changed = profile.clone();
            for binary in &mut changed.binaries {
                binary.sha256 = [0; 32];
            }
            assert_eq!(ChildSlots::for_profile(&changed).count(), 768);
            assert!(!crate::runtime::snapshot_layout_supported(&changed));
        }
    }
}
