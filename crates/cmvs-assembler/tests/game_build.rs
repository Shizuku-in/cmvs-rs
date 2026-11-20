//! Offline projects retain tile inheritance across archives and observed VM registrations.
use cmvs_assembler::{build, image};
use cmvs_core::{
    format::{
        cpz::{ArchiveIndex, Cpz3Index, DirectoryFormat, DirectoryNames},
        pb3::{self, Compression, ImageType},
        ps3::{Container, Script, ScriptFormat},
    },
    host::platform::RecordingBackend,
    profile::Registry,
    render::raster::RgbaImage,
    runtime::{Runtime, script_control::Startup},
};
use std::{
    fs::{self, File},
    io::BufReader,
    path::Path,
};

fn script(words: &[u16], text: &[u8]) -> Script {
    script_as(Container::Ps2a, words, text)
}
fn script_as(container: Container, words: &[u16], text: &[u8]) -> Script {
    let code: Vec<_> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    Script::from_segments(
        ScriptFormat {
            container,
            version_raw: 0x20004,
            key: 0,
            reserved: 0,
        },
        0,
        &[],
        &code,
        &[],
        text,
    )
    .unwrap()
}
fn push(words: &mut Vec<u16>, string: bool, value: u32) {
    words.extend([
        if string { 0x201 } else { 0x200 },
        if string { 0x120 } else { 0x100 },
        value as u16,
        (value >> 16) as u16,
        0x20f,
        0x430,
    ]);
}
fn pack(path: &Path, name: &[u8], bytes: Vec<u8>) {
    Cpz3Index::write(&mut File::create(path).unwrap(), &[(name.to_vec(), bytes)]).unwrap();
}
fn entry(root: &Path, name: &str) -> Vec<u8> {
    let mut f = File::open(root.join("data/pack").join(name)).unwrap();
    ArchiveIndex::read(&mut f)
        .unwrap()
        .read_entry_verified(&mut f, 0)
        .unwrap()
}
fn png(path: &Path, img: &RgbaImage) {
    let mut writer = png::Encoder::new(File::create(path).unwrap(), img.width(), img.height());
    writer.set_color(png::ColorType::Rgba);
    writer.set_depth(png::BitDepth::Eight);
    writer
        .write_header()
        .unwrap()
        .write_image_data(img.pixels())
        .unwrap();
}

#[test]
fn script_container_edits_work_with_any_extension_on_a_cold_build() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("game");
    let dir = source.join("data/pack");
    fs::create_dir_all(&dir).unwrap();
    let start = script(&[0x2000], b"").encode().unwrap();
    fs::write(dir.join("start.ps2"), &start).unwrap();
    pack(&dir.join("script.cpz"), b"main.ps2", start);
    let project = tmp.path().join("project");
    build::export(&source, &project, Some("asukimi")).unwrap();
    let manifest = project.join("game.tsv");
    let rows = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        rows.replace("main.ps2\tps2a\t\t-", "main.ps2\tps2a\t\tedit.bin"),
    )
    .unwrap();
    let edit = script(&[0x2000], b"edited\0").encode().unwrap();
    fs::write(project.join("edit.bin"), &edit).unwrap();
    let out = tmp.path().join("out");
    build::assemble(&source, &project, &out, None).unwrap();
    assert_eq!(entry(&out, "script.cpz"), edit);
}

#[test]
fn unchanged_overlay_inherits_and_redrawn_overlay_rekeys_when_base_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("source");
    let packdir = root.join("data/pack");
    fs::create_dir_all(&packdir).unwrap();
    let mut words = Vec::new();
    let mut text = vec![0];
    for (slot, name) in [b"data\\pack\\\\a.cpz".as_slice(), b"data\\pack\\b.cpz"]
        .iter()
        .enumerate()
    {
        push(&mut words, true, text.len() as u32);
        push(&mut words, false, slot as u32);
        words.push(0x2011);
        text.extend_from_slice(name);
        text.push(0);
    }
    words.push(0x2000);
    let start = script(&words, &text).encode().unwrap();
    fs::write(packdir.join("start.ps2"), &start).unwrap();
    pack(&packdir.join("script.cpz"), b"main.ps2", start);
    let base = RgbaImage::new(16, 8, [1, 2, 3, 255].repeat(128)).unwrap();
    let edited = RgbaImage::new(16, 8, [5, 6, 7, 255].repeat(128)).unwrap();
    let mut p = base.pixels().to_vec();
    p[0] = 42;
    let overlay = RgbaImage::new(16, 8, p).unwrap();
    pack(
        &packdir.join("a.cpz"),
        b"base.pb3",
        pb3::encode_image(&base, ImageType::Interleaved, Compression::Lzss).unwrap(),
    );
    let delta = pb3::encode_image(
        &overlay,
        ImageType::Overlay {
            kind: 8,
            stem: b"base",
            base: &base,
        },
        Compression::Lzss,
    )
    .unwrap();
    pack(&packdir.join("b.cpz"), b"layer.pb3", delta.clone());
    let project = tmp.path().join("project");
    build::export(&root, &project, Some("asukimi")).unwrap();
    let t = project.join("game.tsv");
    let initial = fs::read_to_string(&t).unwrap();
    let redraw = initial.replace(
        "layer.pb3\tpb3/8/32\tbase.pb3\t-",
        "layer.pb3\tpb3/8/32\tbase.pb3\tlayer.png",
    );
    let extracted = project.join("layer.png");
    fs::write(
        &t,
        initial.replace("pb3/8/32\tbase.pb3", "pb3/8/32\tlayer.pb3"),
    )
    .unwrap();
    assert!(
        build::extract(&root, &project, 1, &extracted)
            .unwrap_err()
            .to_string()
            .contains("source codec or reference")
    );
    fs::write(&t, initial.replace("main.ps2\tps2a", "main.ps2\traw")).unwrap();
    assert!(
        build::assemble(&root, &project, &tmp.path().join("bad-codec"), None)
            .unwrap_err()
            .to_string()
            .contains("source codec or reference")
    );
    fs::write(&t, &redraw).unwrap();
    fs::write(
        packdir.join("start.ps2"),
        script(&[0x2000], b"").encode().unwrap(),
    )
    .unwrap();
    assert!(
        build::extract(&root, &project, 1, &extracted)
            .unwrap_err()
            .to_string()
            .contains("startup script differs")
    );
    fs::write(
        packdir.join("start.ps2"),
        script(&words, &text).encode().unwrap(),
    )
    .unwrap();
    build::extract(&root, &project, 1, &extracted).unwrap();
    assert_eq!(
        image::read_image(BufReader::new(
            File::open(project.join("layer.png")).unwrap()
        ))
        .unwrap(),
        overlay
    );
    build::assemble(&root, &project, &tmp.path().join("first"), None).unwrap();
    png(&project.join("base.png"), &edited);
    let with_base =
        |s: &str| s.replace("base.pb3\tpb3/7/32\t\t-", "base.pb3\tpb3/7/32\t\tbase.png");
    fs::write(&t, with_base(&redraw)).unwrap();
    let warm = tmp.path().join("warm");
    build::assemble(&root, &project, &warm, None).unwrap();
    let raw = entry(&warm, "b.cpz");
    assert_eq!(
        pb3::decode_with_base(&raw, edited.clone()).unwrap(),
        overlay
    );
    fs::rename(project.join("cache"), project.join("old-cache")).unwrap();
    let cold = tmp.path().join("cold");
    build::assemble(&root, &project, &cold, None).unwrap();
    for name in ["a.cpz", "b.cpz", "start.ps2", "script.cpz"] {
        assert_eq!(
            fs::read(cold.join("data/pack").join(name)).unwrap(),
            fs::read(warm.join("data/pack").join(name)).unwrap(),
            "{name}"
        );
    }
    fs::write(&t, with_base(&initial)).unwrap();
    let inherited = tmp.path().join("inherit");
    build::assemble(&root, &project, &inherited, None).unwrap();
    assert_eq!(entry(&inherited, "b.cpz"), delta);
    let combined = pb3::decode_with_base(&delta, edited).unwrap();
    assert_eq!(&combined.pixels()[..4], &[42, 2, 3, 255]);
    assert_eq!(&combined.pixels()[32..36], &[5, 6, 7, 255]);
    png(
        &project.join("base.png"),
        &RgbaImage::new(1, 1, vec![0; 4]).unwrap(),
    );
    let invalid = tmp.path().join("invalid");
    assert!(build::assemble(&root, &project, &invalid, None).is_err());
    assert!(!invalid.exists());
}

#[test]
fn cvns_exports_registered_resources_and_only_the_system_save_template() {
    use cmvs_core::format::cpz::ArchiveVersion;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("source");
    for dir in ["data/pack", "data/Save", "Extras/images"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let mut text = Vec::new();
    let mut words = Vec::new();
    for (command, path) in [
        (0x2026, "DATA/save/"),
        (0x2021, "extras\\images\\"),
        (0x2024, "extras\\voice.cpz"),
    ] {
        push(&mut words, true, text.len() as u32);
        push(&mut words, false, 0);
        words.push(command);
        text.extend_from_slice(path.as_bytes());
        text.push(0);
    }
    words.push(0x2000);
    let start = script_as(Container::Pss0, &words, &text).encode().unwrap();
    fs::write(root.join("data/pack/start.pss"), &start).unwrap();
    for (path, name, content) in [
        (
            "data/pack/script.cpz",
            b"main.pss".as_slice(),
            start.as_slice(),
        ),
        ("Extras/voice.cpz", b"voice.bin", b"voice"),
    ] {
        ArchiveVersion::Cpz2
            .write_flat(&mut File::create(root.join(path)).unwrap(), &[name], |_| {
                Ok((content.len() as u32, content))
            })
            .unwrap();
    }
    for path in [
        "data/Save/SAVE000.DAT",
        "data/Save/save001.dat",
        "data/Save/backup",
        "Extras/images/scene.bin",
    ] {
        fs::write(root.join(path), path).unwrap();
    }
    let project = tmp.path().join("project");
    let published = tmp.path().join("published");
    build::export(&root, &project, Some("cvns-20090203")).unwrap();
    build::assemble(&root, &project, &published, None).unwrap();
    assert_eq!(
        fs::read_dir(published.join("data/Save")).unwrap().count(),
        1
    );
    for path in [
        "data/Save/SAVE000.DAT",
        "Extras/images/scene.bin",
        "Extras/voice.cpz",
    ] {
        assert_eq!(
            fs::read(published.join(path)).unwrap(),
            fs::read(root.join(path)).unwrap()
        );
    }
    let manifest = project.join("game.tsv");
    let mut old = fs::read_to_string(&manifest).unwrap();
    let id = old.lines().filter(|row| row.starts_with("asset\t")).count();
    let slot = "data/Save/save001.dat";
    let digest =
        cmvs_core::format::digest::to_hex(&cmvs_core::format::digest::sha256(slot.as_bytes()));
    old.push_str(&format!(
        "file\t{slot}\tloose\t{digest}\nasset\t{id}\t-\t\tsave001.dat\traw\t\t-\n"
    ));
    fs::write(manifest, old).unwrap();
    assert!(
        build::assemble(&root, &project, &tmp.path().join("old"), None)
            .unwrap_err()
            .to_string()
            .contains("gameplay save directory")
    );
}

#[test]
fn first_script_lookup_uses_the_taken_branch_and_current_per_archive_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("data/pack");
    fs::create_dir_all(&dir).unwrap();
    let keys = [11, 22, 33, 44];
    let mut words = Vec::new();
    push(&mut words, false, 13);
    words.push(0x201f);
    let register = |words: &mut Vec<u16>, v: [u32; 4], path, slot| {
        for k in v.into_iter().rev() {
            push(words, false, k);
        }
        push(words, true, path);
        push(words, false, slot);
        words.push(0x2268);
    };
    let mut text = b"\0data\\pack\\script.cpz\0".to_vec();
    let other_path = text.len() as u32;
    // Exercise a case-only directory collision on filesystems that support it.
    let other_dir = if tmp.path().join("data/Pack").exists() {
        "other"
    } else {
        "data/Pack"
    };
    text.extend(format!("{other_dir}/script.cpz\0\0").bytes());
    let main_name = text.len() as u32;
    text.extend(b"main.ps3\0");
    register(&mut words, [0; 4], 1, 0);
    let pc = words.len();
    words.extend([0x400, 0, 0]);
    register(&mut words, [5; 4], 1, 0);
    words[pc + 1] = (words.len() * 2) as u16;
    register(&mut words, keys, 1, 0);
    register(&mut words, [7; 4], other_path, 1);
    push(&mut words, true, main_name);
    words.push(0x2080);
    let startup = script(&words, &text);
    fs::write(dir.join("start.ps3"), startup.encode().unwrap()).unwrap();
    let main = script(&[0x2000], b"").encode().unwrap();
    let path = dir.join("script.cpz");
    let other = tmp.path().join(other_dir);
    fs::create_dir_all(&other).unwrap();
    for (archive, parameters) in [(&path, keys), (&other.join("script.cpz"), [7; 4])] {
        DirectoryFormat::Cpz7 {
            parameters: parameters.into(),
            encrypted: true,
        }
        .write(
            &mut File::create(archive).unwrap(),
            &[DirectoryNames {
                name: vec![],
                files: vec![b"main.ps3".to_vec()],
            }],
            |_| Ok((main.len() as u32, main.as_slice())),
        )
        .unwrap();
    }
    build::export(tmp.path(), &tmp.path().join("export"), Some("realive")).unwrap();
    let game = Registry::builtin()
        .identify_game(tmp.path(), Some("realive"))
        .unwrap();
    let prefix = Startup::execute(&startup, game.selection.profile).unwrap();
    let mut file = File::open(&path).unwrap();
    assert_eq!(
        prefix
            .open_relative(
                &mut file,
                &tmp.path().canonicalize().unwrap(),
                Path::new("data/pack/script.cpz"),
            )
            .unwrap()
            .read_entry_verified(&mut file, 0)
            .unwrap(),
        main
    );
    assert!(
        prefix
            .open(&mut file, b"script.cpz")
            .unwrap_err()
            .to_string()
            .contains("conflicting")
    );
    let err = prefix
        .open(&mut file, b"other.cpz")
        .unwrap_err()
        .to_string();
    assert!(err.contains("PC") && err.contains("parameters"));
    let mut run = Runtime::new(&startup, tmp.path(), &game, 0).unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..100 {
        run.tick(&mut backend).unwrap();
        if run.machine().current_name() == b"main.ps3" {
            break;
        }
    }
    assert_eq!(run.machine().current_name(), b"main.ps3");
}

#[test]
fn pb3_extraction_follows_one_chain_and_assembly_bounds_every_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("source");
    let dir = root.join("data/pack");
    fs::create_dir_all(&dir).unwrap();
    let mut words = Vec::new();
    push(&mut words, true, 1);
    push(&mut words, false, 0);
    words.extend([0x2011, 0x2000]);
    let start = script(&words, b"\0data\\pack\\\0").encode().unwrap();
    fs::write(dir.join("start.ps2"), &start).unwrap();
    pack(&dir.join("script.cpz"), b"main.ps2", start);
    let pixel = RgbaImage::new(1, 1, vec![1, 2, 3, 255]).unwrap();
    let mut base =
        pb3::encode_image(&pixel, ImageType::Interleaved, Compression::Literals).unwrap();
    let layer = |stem: &[u8]| {
        pb3::encode_image(
            &pixel,
            ImageType::Overlay {
                kind: 8,
                stem,
                base: &pixel,
            },
            Compression::Literals,
        )
        .unwrap()
    };
    fs::write(dir.join("base.pb3"), &base).unwrap();
    fs::write(dir.join("layer.pb3"), layer(b"late")).unwrap();
    let project = tmp.path().join("project");
    build::export(&root, &project, Some("asukimi")).unwrap();
    let png = tmp.path().join("base.png");
    build::extract(&root, &project, 0, &png).unwrap();
    assert_eq!(
        image::read_image(BufReader::new(File::open(png).unwrap())).unwrap(),
        pixel
    );

    // The encoder writes zero header keys. Set each plane to exactly 64 MiB
    // without allocating pixels: the chain must fail before the decoder runs.
    let mut overlay = layer(b"base");
    for raw in [&mut base, &mut overlay] {
        raw[30..34].copy_from_slice(&[0, 16, 0, 16]);
    }
    fs::write(dir.join("base.pb3"), base).unwrap();
    fs::write(dir.join("layer.pb3"), overlay).unwrap();
    let project = tmp.path().join("large");
    build::export(&root, &project, Some("asukimi")).unwrap();
    let output = tmp.path().join("output");
    for error in [
        build::extract(&root, &project, 1, &output).unwrap_err(),
        build::assemble(&root, &project, &output, None).unwrap_err(),
    ] {
        assert!(error.to_string().contains("byte budget"), "{error}");
    }
    assert!(!output.exists());
}
