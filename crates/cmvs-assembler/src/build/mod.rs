//! Incremental construction from a byte-bound source directory and explicit resource edits.
mod catalog;
mod encode;
use crate::{Result, archive};
use catalog::Catalog;
pub use catalog::export;
use cmvs_core::{
    MAX_PAYLOAD,
    format::{cpz::ArchiveIndex, pb2, pb3, ps3::Script},
    host::image::{MAX_REFERENCE_BYTES, MAX_REFERENCE_DEPTH, charge_pb3},
    profile::{EngineProfile, GameFiles},
    render::raster::RgbaImage,
    runtime::script_control::Startup,
};
use cmvs_tooling::{read_bounded, write_new, write_new_dir};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufReader, Write},
    path::{Path, PathBuf},
};
type Outputs = BTreeMap<usize, PathBuf>;

fn reference_chain(
    id: usize,
    parent: impl Fn(usize) -> Result<Option<usize>>,
) -> Result<Vec<usize>> {
    let mut chain = vec![id];
    let mut current = id;
    while let Some(base) = parent(current)? {
        if chain.len() >= MAX_REFERENCE_DEPTH || chain.contains(&base) {
            return Err(format!("PB3 reference cycle or depth limit at asset {id}").into());
        }
        chain.push(base);
        current = base;
    }
    Ok(chain)
}

fn dependencies(cat: &Catalog, start: &Startup) -> Result<(Vec<Option<usize>>, Vec<usize>)> {
    let parents: Vec<_> = (0..cat.assets.len())
        .map(|id| cat.base(id, start))
        .collect::<Result<_>>()?;
    let mut depth = Vec::with_capacity(parents.len());
    for id in 0..parents.len() {
        depth.push(reference_chain(id, |i| Ok(parents[i]))?.len() - 1);
    }
    Ok((parents, depth))
}
fn pixels(
    cat: &Catalog,
    chain: &[usize],
    outputs: &Outputs,
    budget: &mut usize,
) -> Result<RgbaImage> {
    let (&id, bases) = chain.split_first().ok_or("empty PB3 reference chain")?;
    let raw = match outputs.get(&id) {
        Some(path) => read_bounded(path, MAX_PAYLOAD)?,
        None => cat.raw(id)?,
    };
    charge_pb3(&raw, budget)?;
    if bases.is_empty() {
        Ok(pb3::decode(&raw)?)
    } else {
        Ok(pb3::decode_with_base(
            &raw,
            pixels(cat, bases, outputs, budget)?,
        )?)
    }
}
pub fn extract(source: &Path, project: &Path, id: usize, out: &Path) -> Result<()> {
    let cat = catalog::load(source, project, None, false)?;
    let asset = cat.assets.get(id).ok_or("asset ID outside manifest")?;
    cat.validate_asset(id)?;
    if asset.codec.starts_with("pb3/") {
        let chain = reference_chain(id, |i| {
            let parent = cat.base(i, &cat.startup)?;
            if let Some(p) = parent {
                cat.validate_asset(p)?;
            }
            Ok(parent)
        })?;
        let img = pixels(&cat, &chain, &Outputs::new(), &mut 0)?;
        return write_new(out, |f| crate::image::write_rgba(&img, f));
    }
    if asset.codec.starts_with("pb2/") {
        let raw = cat.raw(id)?;
        if matches!(pb2::Header::parse(&raw)?.kind, 5 | 6) {
            return crate::pb::export_collection(&raw, out);
        }
        return write_new(out, |f| crate::image::pb2_to_png(&raw, f));
    }
    write_new(out, |dest| match asset.codec.as_str() {
        "cur" => crate::cursor::export(&cat.raw(id)?, dest),
        "msk" => crate::image::msk_to_png(&cat.raw(id)?, dest),
        _ => {
            let file = &cat.files[asset.file];
            file.verify(&cat.root)?;
            let mut source = File::open(cat.root.join(&file.relative))?;
            if let Some(n) = asset.ordinal {
                file.index
                    .as_ref()
                    .unwrap()
                    .copy_entry_verified(&mut source, n, dest)?;
            } else {
                std::io::copy(&mut source, dest)?;
            }
            Ok(())
        }
    })
}

pub fn assemble(source: &Path, project: &Path, out: &Path, selected: Option<&str>) -> Result<()> {
    let mut cat = catalog::load(source, project, selected, true)?;
    let game = GameFiles::discover(source)?;
    let project = project.canonicalize()?;
    let mut outputs = Outputs::new();
    let mut start = None;
    let start_id = cat
        .files
        .iter()
        .find(|f| f.relative == game.start_script)
        .ok_or("missing startup script")?
        .first;
    if cat.assets[start_id].edit.is_some() {
        let path = encode::edited(&cat, start_id, None, &project)?;
        let script = Script::decode(&read_bounded(&path, MAX_PAYLOAD + 48)?)?;
        start = Some(Startup::execute(&script, cat.profile)?);
        outputs.insert(start_id, path);
    }
    let startup = start.as_ref().unwrap_or(&cat.startup);
    let (parents, depth) = dependencies(&cat, startup)?;
    let original_parents = if start.is_some() {
        dependencies(&cat, &cat.startup)?.0
    } else {
        parents.clone()
    };
    let mut affected = BTreeSet::new();
    let workers = std::thread::available_parallelism()?.get().min(4);
    for layer in 0..=depth.iter().copied().max().unwrap_or(0) {
        let jobs: Vec<_> = (0..depth.len())
            .filter(|&i| {
                depth[i] == layer
                    && i != start_id
                    && (cat.assets[i].edit.is_some()
                        || parents[i] != original_parents[i]
                        || parents[i].is_some_and(|p| affected.contains(&p)))
            })
            .collect();
        if jobs.is_empty() {
            continue;
        }
        let perform = |ids: &[usize]| -> Result<Vec<(usize, Option<PathBuf>)>> {
            let mut done = Vec::new();
            for &i in ids {
                let base = parents[i]
                    .map(|p| {
                        let chain = reference_chain(p, |j| Ok(parents[j]))?;
                        pixels(&cat, &chain, &outputs, &mut 0)
                    })
                    .transpose()?;
                let path = if cat.assets[i].edit.is_some() {
                    Some(encode::edited(&cat, i, base.as_ref(), &project)?)
                } else {
                    // Untouched overlay bytes keep their original replacement tiles on the new base.
                    let raw = cat.raw(i)?;
                    pb3::decode_with_base(&raw, base.ok_or("missing overlay base")?)?;
                    None
                };
                done.push((i, path));
            }
            Ok(done)
        };
        let results = std::thread::scope(|scope| -> Result<Vec<_>> {
            let threads: Vec<_> = jobs
                .chunks(jobs.len().div_ceil(workers))
                .map(|chunk| scope.spawn(move || perform(chunk)))
                .collect();
            threads
                .into_iter()
                .map(|t| t.join().map_err(|_| "image encoder thread failed")?)
                .collect()
        })?;
        for (id, path) in results.into_iter().flatten() {
            affected.insert(id);
            if let Some(p) = path {
                if cat.assets[id].codec.starts_with("pb3/") {
                    let mut bytes = 0;
                    charge_pb3(&read_bounded(&p, MAX_PAYLOAD)?, &mut bytes)?;
                    cat.assets[id].pb3_bytes = bytes;
                }
                outputs.insert(id, p);
            }
        }
    }
    for (id, parent) in parents.iter().enumerate() {
        if parent.is_some() {
            let chain = reference_chain(id, |i| Ok(parents[i]))?;
            let bytes: usize = chain.iter().map(|&i| cat.assets[i].pb3_bytes).sum();
            if bytes > MAX_REFERENCE_BYTES {
                return Err(
                    format!("asset {id}: PB3 reference chain exceeds the byte budget").into(),
                );
            }
        }
    }
    write_new_dir(out, |stage| {
        for f in &cat.files {
            let dest = stage.join(&f.relative);
            fs::create_dir_all(dest.parent().unwrap())?;
            let original = cat.root.join(&f.relative);
            if let Some(index) = &f.index {
                let mut source = File::open(&original)?;
                if start.is_some() {
                    let checked = startup.open_relative(&mut source, &cat.root, &f.relative)?;
                    if checked.directory_format() != index.directory_format() {
                        return Err(format!(
                            "edited startup changes archive keys for {}",
                            f.relative.display()
                        )
                        .into());
                    }
                }
                let paths = outputs
                    .range(f.first..f.first + index.entries().len())
                    .map(|(id, p)| (id - f.first, p.as_path()))
                    .collect::<BTreeMap<_, _>>();
                if !paths.is_empty() {
                    archive::replace_files(&mut source, index, &dest, &paths, false)?;
                    continue;
                }
            } else if let Some(edit) = outputs.get(&f.first) {
                fs::copy(edit, dest)?;
                continue;
            }
            f.copy_unchanged(&original, &dest)?;
        }
        Ok(())
    })
}
