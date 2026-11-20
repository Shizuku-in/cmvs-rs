//! Centralized registry of build-specific engine quirks bound to verified binaries.

use super::EngineProfile;
use crate::digest::parse_hex;

/// Builds whose runtime quirks or save layouts have been independently inspected
/// and bound to exact `(id, base, sha256)` profile identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectedBuild {
    Cvns20040515,
    Cvns20040730,
    Cvns20050216,
    Cvns20050727,
    Cmvs20070119,
    Cmvs20071117,
    Cmvs20080719,
    Cmvs20080903,
    Cvns20090203,
    Cmvs20090815,
    Cmvs20100314,
    Cmvs20101211,
    Cmvs20110525,
    Cmvs20120309,
    Cmvs20130131,
    Cmvs20140217,
    Cmvs20150405,
    Cmvs20160201,
    Cmvs20171114,
    Cmvs20190305,
    Cmvs20190405,
    Cmvs20211006,
    Cmvs20221105,
}

const fn sha256_hex(hex: &str) -> [u8; 32] {
    parse_hex(hex).expect("valid 64-digit SHA-256 hex")
}

struct BuildIdentity {
    build: InspectedBuild,
    id: &'static str,
    base: Option<&'static str>,
    sha256: [u8; 32],
}

const INSPECTED_BUILDS: &[BuildIdentity] = &[
    BuildIdentity {
        build: InspectedBuild::Cvns20040515,
        id: "cvns-20040515",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("8d5abea307067f2789ae3a7fb4a9cd09aa4e48393a06e3d172ca4250dc56274f"),
    },
    BuildIdentity {
        build: InspectedBuild::Cvns20040730,
        id: "cvns-20040730",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("2bfcb306f094a3968af7bc88e37a091305c628b06ddfbf0fce14268d66bf3a07"),
    },
    BuildIdentity {
        build: InspectedBuild::Cvns20050216,
        id: "cvns-20050216",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("1e0d17db759650c32e94a9bc1eb6a143463df22638fba687b420644dce969e42"),
    },
    BuildIdentity {
        build: InspectedBuild::Cvns20050727,
        id: "cvns-20050727",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("fd475dda79e4a559d934b214494d314539915afde7a910b3d35269dd9cd4a028"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20070119,
        id: "cmvs-20070119",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("5ce421db393cbc5ca443fa405c5a9e31c348416109cb1279d91718f490c0e602"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20071117,
        id: "cmvs-20071117",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("caff7901a7dc4ec963d7fde358028cdd68dfd87fc7925c9715a20e7a5f36d02b"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20080719,
        id: "cmvs-20080719",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("afe3b3a895e7b3ba5f78e6f6464296f1d028e87307c8de5a568b88fd6086491e"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20080903,
        id: "cmvs-20080903",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("f77ecbfd39d1076c66915544d73b9c2d2c5876fbabeaef3d589050b6094d13ac"),
    },
    BuildIdentity {
        build: InspectedBuild::Cvns20090203,
        id: "cvns-20090203",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("8e6044466b928d77f0a94a0d0f32fe48190596bf628115598881c2aa3a4885aa"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20090815,
        id: "cmvs-20090815",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("36fa780a5d2b8df4ac6a6e48a2f78708b79aa10d22eb575a5ae74742d902f71a"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20100314,
        id: "cmvs-20100314",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("97575da34a284fa534084ca8452741e0e176650f3396308f92c2c8cf7ffc9bc8"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20101211,
        id: "cmvs-20101211",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("158409c71cf14c3dedb7d051585e5f48e56dbc621bb19e56d084cd4fe8d4352f"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20110525,
        id: "cmvs-20110525",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("9870e6d5cbd5242b9c686aae20fe10697e87328a66d322cfdb2c80021cb645c5"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20120309,
        id: "cmvs-20120309",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("f93d90d45dad95588f91425fa5d4fe20be0d97e1978ccb4ae0e83ca6e09aa15e"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20130131,
        id: "cmvs-20130131",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("637d2928efab89e7b5fc68b2d0801cfdefbfa65558b69378580c145c52b8c05b"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20140217,
        id: "cmvs-20140217",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("04e37c27be28aa0ffe5661a2757e766488316ccdb8c8698da2ea7677675b5aaa"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20150405,
        id: "cmvs-20150405",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("d1c4ade9551b827b8e401fc10f68a73e2525a67a217b63747898f5797894911d"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20160201,
        id: "cmvs-20160201",
        base: None,
        sha256: sha256_hex("dc74d2001387ae1aefbdc4204216de5ebb9d3896009280720c4e7a91b42d84f5"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20171114,
        id: "cmvs-20171114",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("3bccda5d7611412c6938c0c9b06b2195914cfb78763a53f2c235bb6479395ee0"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20190305,
        id: "cmvs-20190305",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("02b71aac3d3d1ced1d0312b6c37748cf12df9cffd9585fb07787e6ca43199f00"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20190405,
        id: "cmvs-20190405",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("c1742b5873e6d03f5cc7bc6f5b3b23655ec67e7cddf307deefa75349caed33be"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20211006,
        id: "cmvs-20211006",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("58e993a82e741ea73d3d1b5d89363dcaf435e72e6d505324e1de0c0ba5486631"),
    },
    BuildIdentity {
        build: InspectedBuild::Cmvs20221105,
        id: "cmvs-20221105",
        base: Some("cmvs-20160201"),
        sha256: sha256_hex("dfbabd2abac5055d3ab2144a983b6c441b87b294d19607e9a4829e2ea954e86c"),
    },
];

impl EngineProfile {
    /// Returns the verified inspected-build tag if `id`, `base`, and recorded binary
    /// SHA-256 all match the independently inspected executable evidence.
    pub(crate) fn inspected_build(&self) -> Option<InspectedBuild> {
        INSPECTED_BUILDS.iter().find_map(|entry| {
            (self.id == entry.id
                && self.base.as_deref() == entry.base
                && self.binary(&entry.sha256).is_some())
            .then_some(entry.build)
        })
    }

    /// Verified loader zero-fill bank sizes, in integer/bit/float/string order.
    /// Keyed on declared profile ID rather than `inspected_build()` because synthetic
    /// startup profiles reuse the canonical IDs without the real binary SHA-256.
    pub(crate) fn initial_globals(&self) -> Option<[u32; 4]> {
        match self.id.as_str() {
            "cmvs-20071117" | "cmvs-20080719" | "cmvs-20080903" | "cmvs-20090815"
            | "cmvs-20100314" | "cmvs-20101211" | "cmvs-20110525" | "cmvs-20120309"
            | "cmvs-20130131" | "cmvs-20140217" | "cmvs-20150405" | "cmvs-20160201"
            | "cmvs-20171114" | "cmvs-20190305" | "cmvs-20190405" | "cmvs-20221105" => {
                Some([4096, 262144, 2048, 128])
            }
            // docs/games/puriring: .data file bytes end at 0x4ae000; the banks at 0x4b6708,
            // 0x4dc710 (0x8000 bytes of packed bits, 0x440be0), 0x4da710 and 0x4ba710.
            "cmvs-20070119" => Some([4096, 262144, 2048, 128]),
            // docs/games/maji/README.md (`sub_411670`/`sub_412170`), majifd/README.md (`sub_4117E0`),
            // aki/README.md (`sub_412980`/`sub_411F00`), akifd/README.md (`sub_412FE0`/`sub_413AC0`) & miharu/README.md:
            // integers 4096, bits 0x8000*8=262144, floats 0, strings 256.
            "cvns-20040515" | "cvns-20040730" | "cvns-20050216" | "cvns-20050727"
            | "cvns-20090203" => Some([4096, 262144, 0, 256]),
            _ => None,
        }
    }

    /// Independently inspected x86 sine interpolation; docs/games/{asukimi,harouse}.
    pub(crate) fn uses_legacy_sine_modes(&self) -> bool {
        matches!(
            self.inspected_build(),
            Some(
                InspectedBuild::Cmvs20071117
                    | InspectedBuild::Cmvs20080719
                    | InspectedBuild::Cmvs20080903
                    | InspectedBuild::Cmvs20090815
                    | InspectedBuild::Cmvs20100314
            )
        )
    }

    /// Whether `SYSTEM_INFO.WINDOW_FULL_MOVE_USE` is read/written in the user INI.
    pub(crate) fn uses_window_full_move(&self) -> bool {
        self.inspected_build() == Some(InspectedBuild::Cmvs20070119)
    }

    /// Whether screen size comes from `CMVS_SYSTEM_MAIN.WINDOW_SIZE` rather than
    /// `WINDOW_RESOLVING.BASE`.
    pub(crate) fn uses_legacy_window_size_ini(&self) -> bool {
        matches!(
            self.inspected_build(),
            Some(
                InspectedBuild::Cvns20040515
                    | InspectedBuild::Cvns20040730
                    | InspectedBuild::Cvns20050216
                    | InspectedBuild::Cvns20050727
                    | InspectedBuild::Cvns20090203
                    | InspectedBuild::Cmvs20070119
                    | InspectedBuild::Cmvs20071117
                    | InspectedBuild::Cmvs20080719
                    | InspectedBuild::Cmvs20080903
                    | InspectedBuild::Cmvs20090815
                    | InspectedBuild::Cmvs20100314
            )
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Registry;

    #[test]
    fn every_builtin_profile_resolves_its_inspected_build_and_rejects_mutations() {
        let registry = Registry::builtin();
        assert_eq!(registry.profiles().len(), INSPECTED_BUILDS.len());
        for entry in INSPECTED_BUILDS {
            let profile = registry.profile(entry.id).unwrap();
            assert_eq!(profile.inspected_build(), Some(entry.build));

            let mut wrong_id = profile.clone();
            wrong_id.id = "uninspected".into();
            assert_eq!(wrong_id.inspected_build(), None);

            let mut wrong_base = profile.clone();
            wrong_base.base = match profile.base {
                None => Some("unexpected-base".into()),
                Some(_) => None,
            };
            assert_eq!(wrong_base.inspected_build(), None);

            let mut wrong_hash = profile.clone();
            for binary in &mut wrong_hash.binaries {
                binary.sha256 = [0; 32];
            }
            assert_eq!(wrong_hash.inspected_build(), None);
        }
    }
}
