//! Runtime child-table layout. Snapshot codecs are selected independently.
use crate::profile::EngineProfile;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ChildSlots {
    #[default]
    Base,
    Extended,
}
impl ChildSlots {
    pub(crate) fn for_profile(profile: &EngineProfile) -> Self {
        // Independent x64 evidence: docs/games/{realive,seishunf,kunado}.
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
        ]
        .iter()
        .any(|(id, hash)| {
            profile.id == *id
                && profile
                    .binary(&crate::digest::parse_hex(hash).expect("static SHA-256"))
                    .is_some()
        }) {
            Self::Extended
        } else {
            Self::Base
        }
    }
    pub(crate) fn count(self) -> u16 {
        match self {
            Self::Base => 768,
            Self::Extended => 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_table_requires_the_inspected_profile_and_binary_identity() {
        let registry = crate::profile::Registry::builtin();
        for id in ["cmvs-20190305", "cmvs-20190405", "cmvs-20211006"] {
            let profile = registry.profile(id).unwrap();
            assert_eq!(ChildSlots::for_profile(profile).count(), 1024);
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
            assert_eq!(scene.child_slots.count(), 1024);

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
