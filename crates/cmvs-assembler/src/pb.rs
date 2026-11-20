//! PB command handling and editable animation/part collections.
use crate::{
    Result,
    image::{read_image, write_rgba},
};
use cmvs_core::{
    MAX_PAYLOAD,
    format::{
        pb2,
        pb3::{self, Compression, EncodeDepth, ImageType},
    },
    render::raster::RgbaImage,
};
use cmvs_tooling::{
    cli::Arguments,
    read_bounded,
    table::{Encoding, escape, unescape},
    write_new, write_new_dir,
};
use std::{
    ffi::OsString,
    fs::File,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

fn picture(path: &Path) -> Result<RgbaImage> {
    read_image(BufReader::new(File::open(path)?))
}

pub fn run(resource: &str, action: &str, argv: &[OsString]) -> Result<()> {
    let options: &[&str] = match (resource, action) {
        ("pb2", "encode") => &["--profile", "--type", "--bpp"],
        ("pb3", "encode") => &["--profile", "--type", "--bpp", "--base", "--reference"],
        ("pb3", "decode") => &["--profile", "--base"],
        ("pb2", "decode" | "export" | "pack") => &["--profile"],
        _ => return Err("unknown PB action".into()),
    };
    let flags: &[&str] = if matches!(action, "encode" | "pack") {
        &["--uncompressed"]
    } else {
        &[]
    };
    let args = Arguments::parse(argv, options, flags)?;
    if args
        .profile()?
        .is_some_and(|p| p.is_cvns() != (resource == "pb2"))
    {
        return Err("image format differs from selected profile family".into());
    }
    let [input, output] = args.paths.as_slice() else {
        return Err("expected input and new output paths".into());
    };
    let (input, output) = (Path::new(input), Path::new(output));
    let mode = if args.option("--uncompressed").is_some() {
        Compression::Literals
    } else {
        Compression::Lzss
    };
    match action {
        "export" => export_collection(&read_bounded(input, MAX_PAYLOAD)?, output),
        "pack" => {
            let raw = pack_collection(input, mode)?;
            write_new(output, |f| Ok(f.write_all(&raw)?))
        }
        "decode" => {
            let raw = read_bounded(input, MAX_PAYLOAD)?;
            if resource == "pb2" {
                return write_new(output, |f| crate::image::pb2_to_png(&raw, f));
            }
            let img = match (pb3::reference_name(&raw)?, args.option("--base")) {
                (Some(_), Some(path)) => pb3::decode_with_base(&raw, picture(Path::new(path))?)?,
                (None, None) => pb3::decode(&raw)?,
                _ => return Err("supply --base <decoded base image> only for PB3 overlays".into()),
            };
            write_new(output, |f| write_rgba(&img, f))
        }
        "encode" => {
            let img = picture(input)?;
            let depth = EncodeDepth::from_bpp(
                args.option("--bpp")
                    .map(|v| v.to_str().ok_or("invalid bit depth"))
                    .transpose()?
                    .unwrap_or("32")
                    .parse()?,
            )?;
            let kind = args
                .option("--type")
                .map(|v| v.to_str().ok_or("invalid PB type"))
                .transpose()?;
            let (raw, decoded) = if resource == "pb2" {
                let kind = match kind.unwrap_or("2") {
                    "1" => pb2::StaticType::Swizzled,
                    "2" => pb2::StaticType::Blocked,
                    _ => return Err("expected static PB2 type 1 or 2".into()),
                };
                let raw = pb2::encode_static(&img, kind, depth, mode)?;
                let checked = pb2::decode(&raw)?;
                (raw, checked)
            } else {
                let kind = kind.unwrap_or("1");
                let overlay = matches!(kind, "6" | "8");
                if kind != "1" && depth != EncodeDepth::Rgba32 {
                    return Err("PB3 types 5 through 8 require 32 bits".into());
                }
                if overlay != args.option("--base").is_some()
                    || overlay != args.option("--reference").is_some()
                {
                    return Err("PB3 overlay requires both --base and --reference".into());
                }
                let base = args
                    .option("--base")
                    .map(|v| picture(Path::new(v)))
                    .transpose()?;
                let stem = args
                    .option("--reference")
                    .map(|v| {
                        unescape(
                            v.to_str().ok_or("invalid reference name")?,
                            Encoding::ShiftJis,
                        )
                    })
                    .transpose()?;
                let format = match (kind, &base, &stem) {
                    ("1", _, _) => ImageType::Blocked(depth),
                    ("5", _, _) => ImageType::Delta,
                    ("7", _, _) => ImageType::Interleaved,
                    ("6" | "8", Some(base), Some(stem)) => ImageType::Overlay {
                        kind: kind.parse()?,
                        stem,
                        base,
                    },
                    _ => return Err("expected PB3 type 1, 5, 6, 7 or 8".into()),
                };
                let raw = pb3::encode_image(&img, format, mode)?;
                let decoded = if let Some(base) = base {
                    pb3::decode_with_base(&raw, base)?
                } else {
                    pb3::decode(&raw)?
                };
                (raw, decoded)
            };
            if img != decoded {
                return Err("encoded PB pixels differ from input".into());
            }
            write_new(output, |f| Ok(f.write_all(&raw)?))
        }
        _ => unreachable!(),
    }
}

pub(crate) fn export_collection(raw: &[u8], destination: &Path) -> Result<()> {
    let h = pb2::Header::parse(raw)?;
    if !matches!(h.kind, 5 | 6) || h.count == 0 {
        return Err("expected a PB2 animation or named parts".into());
    }
    let names = pb2::part_names(raw)?;
    write_new_dir(destination, |dir| {
        let mut manifest = File::create(dir.join("images.tsv"))?;
        writeln!(manifest, "cmvs-pb2-v1\t{}\t{}", h.kind, h.bpp)?;
        let mut parts = names.iter();
        for i in 0..h.count as usize {
            let path = format!("{i:04}.png");
            let img = if h.kind == 5 {
                writeln!(manifest, "frame\t{i}\t{path}")?;
                pb2::decode_frame(raw, i)?
            } else {
                writeln!(
                    manifest,
                    "part\t{}\t{path}",
                    escape(
                        parts.next().ok_or("missing PB2 part name")?,
                        Encoding::ShiftJis
                    )
                )?;
                pb2::decode_part_index(raw, i)?
            };
            write_rgba(&img, File::create(dir.join(path))?)?;
        }
        Ok(())
    })
}

pub(crate) fn pack_collection(input: &Path, mode: Compression) -> Result<Vec<u8>> {
    Collection::read(input, None)?.pack(mode)
}

pub(crate) struct Collection {
    pub kind: u16,
    pub bpp: u16,
    images: Vec<(Option<Vec<u8>>, PathBuf)>,
}
impl Collection {
    pub fn read(input: &Path, project: Option<&Path>) -> Result<Self> {
        let dir = input.parent().unwrap_or(Path::new("."));
        let mut lines = BufReader::new(File::open(input)?).lines();
        let header = lines.next().ok_or("missing PB2 manifest header")??;
        let fields: Vec<_> = header.split('\t').collect();
        let ["cmvs-pb2-v1", kind @ ("5" | "6"), bpp] = fields.as_slice() else {
            return Err("invalid PB2 manifest header".into());
        };
        let bpp = bpp.parse()?;
        let mut images = Vec::new();
        for (i, line) in lines.enumerate() {
            let line = line?;
            let fields: Vec<_> = line.split('\t').collect();
            let (name, path) = match fields.as_slice() {
                ["frame", number, path] if *kind == "5" && number.parse::<usize>()? == i => {
                    (None, *path)
                }
                ["part", name, path] if *kind == "6" => {
                    (Some(unescape(name, Encoding::ShiftJis)?), *path)
                }
                _ => return Err(format!("invalid PB2 manifest row {}", i + 2).into()),
            };
            let path = match project {
                Some(root) => crate::manifest_input(root, dir, Path::new(path))?,
                None => dir.join(path),
            };
            images.push((name, path));
        }
        if images.is_empty() {
            return Err("empty PB2 collection".into());
        }
        Ok(Self {
            kind: kind.parse()?,
            bpp,
            images,
        })
    }
    pub fn fingerprint(&self) -> Result<[u8; 32]> {
        use cmvs_core::format::digest::{Sha256, sha256_reader};
        let mut digest = Sha256::new();
        for (name, path) in &self.images {
            let bytes = name.as_deref().unwrap_or_default();
            digest.update(&(bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
            digest.update(&sha256_reader(File::open(path)?)?);
        }
        Ok(digest.finish())
    }
    pub fn pack(&self, mode: Compression) -> Result<Vec<u8>> {
        let mut writer: Option<pb2::CollectionWriter> = None;
        let mut hashes = Vec::new();
        for (name, path) in &self.images {
            let img = picture(path)?;

            hashes.push(img.content_digest());
            match &mut writer {
                Some(w) => w.push(&img, name.as_deref())?,
                None => {
                    writer = Some(pb2::CollectionWriter::new(
                        img,
                        name.as_deref(),
                        self.bpp,
                        mode,
                    )?)
                }
            }
        }
        let raw = writer.ok_or("empty PB2 collection")?.finish();
        for (i, hash) in hashes.into_iter().enumerate() {
            let img = if self.kind == 5 {
                pb2::decode_frame(&raw, i)?
            } else {
                pb2::decode_part_index(&raw, i)?
            };
            if img.content_digest() != hash {
                return Err(format!("PB2 image {i} differs after encoding").into());
            }
        }
        Ok(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_collection_inputs_can_share_images_within_the_project() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let project = root.join("project");
        let dir = project.join("frames");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("images.tsv");
        let load = |path: &str| {
            std::fs::write(&manifest, format!("cmvs-pb2-v1\t5\t32\nframe\t0\t{path}\n")).unwrap();
            Collection::read(&manifest, Some(&project))
        };
        std::fs::write(project.join("shared.png"), []).unwrap();
        assert!(load("../shared.png").is_ok());
        std::fs::write(root.join("outside.png"), []).unwrap();
        assert!(load("../../outside.png").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("outside.png"), dir.join("link.png")).unwrap();
            assert!(load("link.png").is_err());
        }
    }
}
