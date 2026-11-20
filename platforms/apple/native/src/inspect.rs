use crate::Result;
use cmvs_core::{host::settings::Configuration, profile::Registry};
use cmvs_font::text_codec;
use cmvs_host::{EncodingPolicy, GameBootstrap, default_host_fs};
use serde_json::{Value, json};
use std::path::Path;

pub fn bootstrap(root: &Path, profile: Option<&str>, encoding: &str) -> Result<GameBootstrap> {
    let policy = match encoding {
        "auto" => EncodingPolicy::AutoDefaultShiftJis,
        name => EncodingPolicy::Explicit(text_codec::parse(name)?),
    };
    GameBootstrap::load(
        default_host_fs(),
        root,
        profile.filter(|p| !p.is_empty()),
        policy,
    )
}

pub fn inspect(root: &Path, profile: Option<&str>, encoding: &str) -> Result<Value> {
    let game = bootstrap(root, profile, encoding)?;
    let selection = game.game.selection;
    let profile = selection.profile;
    let config = Configuration::read(&root.join(profile.config_file_name()))?
        .unwrap_or(Configuration::new(Vec::new())?);
    Ok(json!({
        "game": selection.record.map(|r| &r.game),
        "profile": profile.id,
        "family": if profile.is_cvns() {"CVNS"} else {"CMVS"},
        "basis": format!("{:?}",selection.basis),
        "encoding": text_codec::name(game.encoding),
        "size": config.screen_size_for_profile(profile)?.pixels(),
        "savedata": root.join("savedata").is_dir()
    }))
}
pub fn profiles() -> Value {
    let registry = Registry::builtin();
    registry.profiles().iter().map(|p| json!({
        "id": p.id, "games": registry.games().iter().filter(|g| g.profile == p.id).map(|g| &g.game).collect::<Vec<_>>()
    })).collect()
}
