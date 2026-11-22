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
    let backup_identity = backup_identity(&game);
    let selection = game.game.selection;
    let profile = selection.profile;
    let config = Configuration::read(&root.join(profile.config_file_name()))?
        .unwrap_or(Configuration::new(Vec::new())?);
    Ok(json!({
        "backupIdentity": backup_identity,
        "game": selection.record.map(|r| &r.game),
        "profile": profile.id,
        "family": if profile.is_cvns() {"CVNS"} else {"CMVS"},
        "basis": format!("{:?}",selection.basis),
        "encoding": text_codec::name(game.encoding),
        "size": config.screen_size_for_profile(profile)?.pixels(),
        "savedata": root.join("savedata").is_dir()
    }))
}

pub fn font(path: &Path) -> Result<Value> {
    let bytes = crate::fonts::read_file(path)?;
    let count = match ttf_parser::fonts_in_collection(&bytes) {
        Some(count) if count as usize <= bytes.len().saturating_sub(12) / 4 => count,
        Some(_) => return Err("invalid font collection header".into()),
        None => 1,
    };
    Ok((0..count)
        .filter_map(|index| {
            let font = ttf_parser::Face::parse(&bytes, index).ok()?;
            let draws = |ch| {
                font.glyph_index(ch)
                    .filter(|id| id.0 != 0)
                    .and_then(|id| font.glyph_bounding_box(id))
                    .is_some_and(|bounds| {
                        bounds.x_min < bounds.x_max && bounds.y_min < bounds.y_max
                    })
            };
            let cjk = u8::from(draws('中')) + u8::from(draws('あ'));
            if cjk == 0 && !draws('A') {
                return None;
            }
            let name = font
                .names()
                .into_iter()
                .filter(|record| record.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
                .find_map(|record| {
                    record
                        .to_string()
                        .or_else(|| {
                            (!record.name.is_empty()
                                && record.name.iter().all(u8::is_ascii_graphic))
                            .then(|| String::from_utf8_lossy(record.name).into_owned())
                        })
                        .filter(|name| !name.is_empty())
                })?;
            Some(json!({"index": index, "name": name, "cjk": cjk}))
        })
        .collect())
}

pub fn profiles() -> Value {
    let registry = Registry::builtin();
    registry.profiles().iter().map(|p| json!({
        "id": p.id, "games": registry.games().iter().filter(|g| g.profile == p.id).map(|g| &g.game).collect::<Vec<_>>()
    })).collect()
}

pub fn backup_identity(game: &GameBootstrap) -> String {
    use cmvs_core::format::digest::{sha256, to_hex};
    match game.game.selection.record {
        Some(record) => record.game.clone(),
        None => to_hex(&sha256(game.script.payload())),
    }
}
