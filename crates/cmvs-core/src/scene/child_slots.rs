//! Runtime child-table layout. Snapshot codecs are selected independently.
use crate::profile::EngineProfile;

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
        // Independent x86 256-entry helpers: docs/games/asukimi.
        if profile.is_inspected_20071117() || profile.is_inspected_20080903() {
            return Self::Slots256;
        }
        // Independent 512-entry create/destroy helpers: docs/games/kazokubu.
        if profile.id == "cmvs-20120309"
            && profile
                .binary(
                    &crate::digest::parse_hex(
                        "f93d90d45dad95588f91425fa5d4fe20be0d97e1978ccb4ae0e83ca6e09aa15e",
                    )
                    .expect("static SHA-256"),
                )
                .is_some()
        {
            return Self::Slots512;
        }
        // Independent x64 evidence: docs/games/{realive,seishunf,kunado,hapymaher_re}.
        if [
            (
                "cmvs-20190305",
                "02b71aac3d3d1ced1d0312b6c37748cf12df9cffd9585fb07787e6ca43199f00",
            ),
            (
                "cmvs-20190405",
                "c1742b5873e6d03f5cc7bc6f5b3b23655ec67e7cddf307deefa75349caed33be",
            ),
            (
                "cmvs-20211006",
                "58e993a82e741ea73d3d1b5d89363dcaf435e72e6d505324e1de0c0ba5486631",
            ),
            (
                "cmvs-20221105",
                "dfbabd2abac5055d3ab2144a983b6c441b87b294d19607e9a4829e2ea954e86c",
            ),
        ]
        .iter()
        .any(|(id, hash)| {
            profile.id == *id
                && profile
                    .binary(&crate::digest::parse_hex(hash).expect("static SHA-256"))
                    .is_some()
        }) {
            Self::Slots1024
        } else {
            Self::Slots768
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
        assert!(crate::runner::snapshot_layout_supported(profile));
        let mut changed = profile.clone();
        changed.binaries[0].sha256 = [0; 32];
        assert_eq!(ChildSlots::for_profile(&changed).count(), 768);
        let mut changed = profile.clone();
        changed.base = Some("unverified-base".into());
        assert_eq!(ChildSlots::for_profile(&changed).count(), 768);
        assert!(!crate::runner::snapshot_layout_supported(&changed));
        let mut scene = super::super::SceneHost::for_profile(profile);
        assert!(scene.allow_filter_mode_one);
        assert!(scene.allow_legacy_sine_modes);
        scene.restore(
            Default::default(),
            Default::default(),
            None,
            None,
            None,
            None,
        );
        assert!(scene.allow_filter_mode_one);
        assert!(scene.allow_legacy_sine_modes);
        let scene = super::super::SceneHost::for_profile(&changed);
        assert!(!scene.allow_filter_mode_one);
        assert!(!scene.allow_legacy_sine_modes);
        for (id, count) in [
            ("cmvs-20120309", 512),
            ("cmvs-20190305", 1024),
            ("cmvs-20190405", 1024),
            ("cmvs-20211006", 1024),
            ("cmvs-20221105", 1024),
        ] {
            let profile = registry.profile(id).unwrap();
            assert_eq!(ChildSlots::for_profile(profile).count(), count);
            assert!(crate::runner::snapshot_layout_supported(profile));
            let mut scene = super::super::SceneHost::for_profile(profile);
            scene.restore(
                Default::default(),
                Default::default(),
                None,
                None,
                None,
                None,
            );
            assert_eq!(scene.child_slots.count(), count);

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
            assert!(!crate::runner::snapshot_layout_supported(&changed));
        }
    }
}
