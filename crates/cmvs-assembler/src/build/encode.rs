use super::*;
use cmvs_core::format::{
    digest::{Sha256, sha256, sha256_reader, to_hex},
    pb3::{Compression, EncodeDepth, ImageType},
};
use std::io::BufRead;
const ENCODER: &[u8] = b"cmvs-game/resource-v1/lzss";

pub(super) fn edited(
    cat: &Catalog,
    id: usize,
    base: Option<&RgbaImage>,
    project: &Path,
) -> Result<PathBuf> {
    let asset = &cat.assets[id];
    let input = asset.edit.as_ref().ok_or("missing edit path")?;
    let open = || Ok::<_, std::io::Error>(BufReader::new(File::open(input)?));
    if asset.codec == "raw" {
        return Ok(input.clone());
    }
    // Include all source PNGs and part names before deciding whether encoding is needed.
    let collection = if asset.codec.starts_with("pb2/5/") || asset.codec.starts_with("pb2/6/") {
        let h = crate::pb::Collection::read(input, Some(project))?;
        if asset.codec != format!("pb2/{}/{}", h.kind, h.bpp) {
            return Err("PB2 collection type or depth differs from project".into());
        }
        Some(h)
    } else {
        None
    };
    let mut key = Sha256::new();
    for field in [
        ENCODER,
        cat.profile.id.as_bytes(),
        asset.codec.as_bytes(),
        asset.reference.as_slice(),
    ] {
        key.update(&(field.len() as u64).to_le_bytes());
        key.update(field);
    }
    let fingerprint = || -> Result<[u8; 32]> {
        if let Some(c) = &collection {
            c.fingerprint()
        } else {
            Ok(sha256_reader(open()?)?)
        }
    };
    let before = fingerprint()?;
    key.update(&before);
    if let Some(img) = base {
        key.update(&img.width().to_le_bytes());
        key.update(&img.height().to_le_bytes());
        key.update(img.pixels());
    }
    let cache = project.join("cache");
    let path = cache.join(to_hex(&key.finish()));
    let file = path.join("resource");
    let check = || -> Result<()> {
        if fs::read(path.join("sha256"))? != to_hex(&sha256_reader(File::open(&file)?)?).as_bytes()
        {
            return Err("cached resource digest mismatch".into());
        }
        Ok(())
    };
    if path.try_exists()? {
        check()?;
        return Ok(file);
    }
    let raw = if let Some(c) = &collection {
        c.pack(Compression::Lzss)?
    } else if asset.codec.starts_with("pb") {
        let img = crate::image::read_image(open()?)?;
        let fields: Vec<_> = asset.codec.split('/').collect();
        let [family, kind, bpp] = fields.as_slice() else {
            return Err("invalid project PB format".into());
        };
        let depth = EncodeDepth::from_bpp(bpp.parse()?)?;
        let (raw, checked) = if *family == "pb2" {
            let kind = match *kind {
                "1" => pb2::StaticType::Swizzled,
                "2" | "3" | "4" => pb2::StaticType::Blocked,
                _ => return Err("unsupported PB2 edit".into()),
            };
            let raw = pb2::encode_static(&img, kind, depth, Compression::Lzss)?;
            let checked = pb2::decode(&raw)?;
            (raw, checked)
        } else {
            let format = match *kind {
                "1" | "2" | "3" => ImageType::Blocked(depth),
                "5" => ImageType::Delta,
                "7" => ImageType::Interleaved,
                "6" | "8" => ImageType::Overlay {
                    kind: kind.parse()?,
                    stem: asset
                        .reference
                        .strip_suffix(b".pb3")
                        .ok_or("missing PB3 reference")?,
                    base: base.ok_or("missing PB3 base image")?,
                },
                _ => return Err("unsupported PB3 edit".into()),
            };
            let raw = pb3::encode_image(&img, format, Compression::Lzss)?;
            let checked = if let Some(base) = base {
                pb3::decode_with_base(&raw, base.clone())?
            } else {
                pb3::decode(&raw)?
            };
            (raw, checked)
        };
        if img != checked {
            return Err("encoded image pixels differ".into());
        }
        raw
    } else {
        match asset.codec.as_str() {
            "msk" => crate::image::png_to_msk(open()?)?,
            "cur" => crate::cursor::assemble(open()?)?,
            "ps2a" | "pss0" => {
                let mut reader = open()?;
                let raw = if matches!(reader.fill_buf()?.get(..4), Some(b"PS2A" | b"PSS0")) {
                    read_bounded(input, MAX_PAYLOAD + 48)?
                } else {
                    crate::script::assemble(reader, Some(cat.profile), None)?.0
                };
                let s = Script::decode(&raw)?;
                if cat.profile.is_cvns()
                    != (s.header().container == cmvs_core::format::ps3::Container::Pss0)
                {
                    return Err("script container differs from profile".into());
                }
                raw
            }
            _ => return Err("unsupported editable resource format".into()),
        }
    };
    if before != fingerprint()? {
        return Err("edit changed during encoding".into());
    }
    fs::create_dir_all(cache)?;
    let publish = write_new_dir(&path, |dir| {
        fs::write(dir.join("resource"), &raw)?;
        Ok(fs::write(dir.join("sha256"), to_hex(&sha256(&raw)))?)
    });
    if let Err(e) = publish {
        if !path.try_exists()? {
            return Err(e);
        }
        check()?;
    }
    Ok(file)
}
