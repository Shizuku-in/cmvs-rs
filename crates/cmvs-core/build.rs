use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
struct FormatError(pub &'static str);

#[allow(dead_code)]
const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

#[path = "src/cvns.rs"]
#[allow(dead_code)]
mod cvns;

#[path = "src/digest.rs"]
#[allow(dead_code)]
mod digest;

#[path = "src/exe.rs"]
#[allow(dead_code)]
mod exe;

#[path = "src/lzss.rs"]
#[allow(dead_code)]
mod lzss;

#[path = "src/profile"]
#[allow(dead_code)]
mod profile {
    #[path = "binary.rs"]
    pub mod binary;
    #[path = "contract.rs"]
    pub mod contract;
    #[path = "model.rs"]
    pub mod model;
    #[path = "text.rs"]
    pub mod text;
    #[path = "validate.rs"]
    pub mod validate;

    pub use contract::{ContractProof, ContractRules};
    pub use model::*;
}

fn read_manifest_text(manifest_dir: &Path, relative: &str) -> String {
    assert!(
        Path::new(relative)
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "build input path must stay inside CARGO_MANIFEST_DIR: {relative}"
    );
    println!("cargo:rerun-if-changed={relative}");
    let path = manifest_dir.join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn write_bundle(path: &Path, raw: &[u8]) {
    let raw_len = u32::try_from(raw.len()).expect("builtin bundle fits u32");
    let compressed = lzss::encode(raw);
    let mut blob = Vec::with_capacity(4 + compressed.len());
    blob.extend_from_slice(&raw_len.to_le_bytes());
    blob.extend_from_slice(&compressed);
    fs::write(path, blob).expect("write builtin bundle");
}

fn parse_command_literal(token: &str) -> u16 {
    if let Some(hex) = token.strip_prefix("0x") {
        u16::from_str_radix(hex, 16)
            .unwrap_or_else(|err| panic!("invalid hex command literal `{token}`: {err}"))
    } else {
        token
            .parse::<u16>()
            .unwrap_or_else(|err| panic!("invalid command literal `{token}`: {err}"))
    }
}

fn parse_entry_tables(source: &str, label: &str) -> BTreeMap<String, BTreeSet<u16>> {
    let mut tables = BTreeMap::new();
    let mut current: Option<String> = None;
    for raw_line in source.lines() {
        let code = raw_line
            .split_once("//")
            .map_or(raw_line, |(head, _)| head)
            .trim();
        if let Some(rest) = code.strip_prefix("pub static ") {
            let (name, _) = rest
                .split_once(':')
                .unwrap_or_else(|| panic!("malformed static header in {label}: `{code}`"));
            let name = name.trim().to_owned();
            assert!(
                tables.insert(name.clone(), BTreeSet::new()).is_none(),
                "duplicate static table `{name}` in {label}"
            );
            current = Some(name);
        }
        if let Some(table_name) = current.as_deref()
            && let Some((_, after)) = code.split_once("command:")
        {
            let literal = after.split_once(',').map_or(after, |(head, _)| head).trim();
            tables
                .get_mut(table_name)
                .expect("active table")
                .insert(parse_command_literal(literal));
        }
        if current.is_some() && code.ends_with("];") {
            current = None;
        }
    }
    assert!(current.is_none(), "unclosed static table in {label}");
    tables
}

fn main() {
    for source in [
        "build.rs",
        "profiles",
        "src/cvns.rs",
        "src/digest.rs",
        "src/exe.rs",
        "src/lzss.rs",
        "src/profile/model.rs",
        "src/profile/text.rs",
        "src/profile/contract.rs",
        "src/profile/binary.rs",
        "src/profile/validate.rs",
    ] {
        println!("cargo:rerun-if-changed={source}");
    }

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let rules_text = read_manifest_text(&manifest_dir, "profiles/contract_rules.txt");
    let rules = profile::ContractRules::parse(&rules_text)
        .unwrap_or_else(|err| panic!("failed to parse profiles/contract_rules.txt: {err}"));

    let cvns_source = read_manifest_text(&manifest_dir, "src/dispatch/registry/cvns.rs");
    let cvns_tables = parse_entry_tables(&cvns_source, "src/dispatch/registry/cvns.rs");
    let cvns_commands = cvns_tables
        .get("CVNS")
        .expect("CVNS static table in src/dispatch/registry/cvns.rs");

    let variant_source = read_manifest_text(&manifest_dir, "src/dispatch/registry/variants.rs");
    let raw_variants = parse_entry_tables(&variant_source, "src/dispatch/registry/variants.rs");
    let mut variant_commands = BTreeMap::new();
    for (table_name, commands) in raw_variants {
        let kebab = table_name.to_ascii_lowercase().replace('_', "-");
        let id = profile::VariantId::parse(&kebab).unwrap_or_else(|| {
            panic!("unknown variant table `{table_name}` in src/dispatch/registry/variants.rs")
        });
        variant_commands.insert(id.name().to_owned(), commands);
    }

    let mut profile_files = fs::read_dir(manifest_dir.join("profiles"))
        .expect("read profiles directory")
        .map(|entry| entry.expect("profiles directory entry").file_name())
        .filter_map(|name| {
            let name = name.into_string().expect("ASCII profile file name");
            name.ends_with(".profile")
                .then(|| format!("profiles/{name}"))
        })
        .collect::<Vec<_>>();
    profile_files.sort();
    assert!(
        !profile_files.is_empty(),
        "expected embedded .profile files"
    );

    let sources: Vec<_> = profile_files
        .iter()
        .map(|relative| read_manifest_text(&manifest_dir, relative))
        .collect();
    let profiles = profile_files
        .iter()
        .zip(&sources)
        .map(|(relative, text)| {
            let parsed = profile::text::parse_profile(text)
                .unwrap_or_else(|err| panic!("failed to parse {relative}: {err}"));
            profile::validate::validate_profile(&parsed, |id, command| {
                variant_commands
                    .get(id.name())
                    .is_some_and(|commands| commands.contains(&command))
            })
            .unwrap_or_else(|err| panic!("failed to validate {relative}: {err}"));
            parsed
        })
        .collect::<Vec<_>>();
    let games_text = read_manifest_text(&manifest_dir, "profiles/games.txt");
    let games = profile::text::parse_games(&games_text)
        .unwrap_or_else(|err| panic!("failed to parse profiles/games.txt: {err}"));
    let contracts_text = read_manifest_text(&manifest_dir, "profiles/contracts.txt");
    let contracts = profile::contract::parse_catalog(&contracts_text, std::slice::from_ref(&rules))
        .unwrap_or_else(|err| panic!("failed to parse profiles/contracts.txt: {err}"));

    profile::validate::validate_registry(&profiles, &games, &contracts, |target| {
        cvns_commands.contains(&target)
    })
    .unwrap_or_else(|err| panic!("failed to validate builtin profile registry: {err}"));

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    write_bundle(
        &out_dir.join("builtin_registry.bin"),
        &profile::binary::encode_bundle(&profiles, &games),
    );
    if env::var_os("CARGO_FEATURE_PROFILE_RESEARCH").is_some() {
        // Profile text is parsed by the standalone research tool on demand.
        assert!(sources.iter().all(|source| !source.contains('\0')));
        write_bundle(
            &out_dir.join("builtin_research_profiles.bin"),
            sources.join("\0").as_bytes(),
        );
    }
}
