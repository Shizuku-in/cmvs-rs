use cmvs_core::{
    format::cpz::{ArchiveIndex, Cpz3Index},
    host::image::{self, ImageArchives},
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Cursor,
    path::Path,
    process::{Command, Output},
};

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cmvs-assembler"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn indexed_png_edits_the_requested_entry_and_loads_through_the_player() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut png = png::Encoder::new(File::create(root.join("edit.png")).unwrap(), 2, 1);
    png.set_color(png::ColorType::Indexed);
    png.set_depth(png::BitDepth::One);
    png.set_palette([20, 40, 60, 80, 100, 120].as_slice());
    png.set_trns([0, 180].as_slice());
    png.write_header()
        .unwrap()
        .write_image_data(&[0b0100_0000])
        .unwrap();
    success(cli(
        root,
        &[
            "pb3",
            "encode",
            "edit.png",
            "new.pb3",
            "--profile",
            "asukimi",
        ],
    ));
    let path = root.join("source.cpz");
    let entries = [
        (b"scene.pb3".to_vec(), vec![1]),
        (b"scene.pb3".to_vec(), vec![2]),
        (b"misc".to_vec(), vec![3]),
    ];
    Cpz3Index::write(&mut File::create(&path).unwrap(), &entries).unwrap();
    fs::write(root.join("edit.bin"), b"updated").unwrap();
    success(cli(
        root,
        &[
            "cpz",
            "replace",
            "source.cpz",
            "patched.cpz",
            "2",
            "edit.bin",
            "0",
            "new.pb3",
            "--verify-all",
        ],
    ));
    let mut archives = ImageArchives::new(root).unwrap();
    let paths = BTreeMap::from([((0x2011, 0), b"patched.cpz".to_vec())]);
    let loaded = image::load(&mut archives.with_paths(&paths), b"SCENE.PB3")
        .unwrap()
        .unwrap();
    assert_eq!((loaded.pixels.width(), loaded.pixels.height()), (2, 1));
    assert_eq!(loaded.pixels.pixels(), [20, 40, 60, 0, 80, 100, 120, 180]);
    let mut output = File::open(root.join("patched.cpz")).unwrap();
    let index = ArchiveIndex::read(&mut output).unwrap();
    assert_eq!(index.read_entry_verified(&mut output, 1).unwrap(), [2]);
    assert_eq!(
        index.read_entry_verified(&mut output, 2).unwrap(),
        b"updated"
    );
    assert!(
        !cli(
            root,
            &["pb3", "encode", "edit.png", "bad.pb3", "--bpp", "24"]
        )
        .status
        .success()
    );
    assert!(
        !cli(
            root,
            &["pb3", "encode", "edit.png", "bad.pb3", "--profile", "aruto"]
        )
        .status
        .success()
    );
    assert!(!root.join("bad.pb3").exists());
}

#[test]
fn corrupt_untouched_payload_aborts_publication_and_preserves_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut source = Cursor::new(Vec::new());
    Cpz3Index::write(
        &mut source,
        &[(b"a".to_vec(), vec![1]), (b"b".to_vec(), vec![2])],
    )
    .unwrap();
    let mut bytes = source.into_inner();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(root.join("source.cpz"), &bytes).unwrap();
    fs::write(root.join("input"), b"replacement").unwrap();
    let args = ["cpz", "replace", "source.cpz", "patched.cpz", "0", "input"];
    let result = cli(root, &args);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("checksum"));
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    assert_eq!(fs::read(root.join("source.cpz")).unwrap(), bytes);

    fs::write(root.join("patched.cpz"), b"keep me").unwrap();
    assert!(!cli(root, &args).status.success());
    assert_eq!(fs::read(root.join("patched.cpz")).unwrap(), b"keep me");
}

#[test]
fn bottom_up_bmp_preserves_row_order_and_alpha() {
    let mut bmp = [0u8; 62];
    bmp[..2].copy_from_slice(b"BM");
    for (at, value) in [(2, 62u32), (10, 54), (14, 40), (18, 1), (22, 2)] {
        bmp[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bmp[26..30].copy_from_slice(&[1, 0, 32, 0]);
    bmp[54..].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let image = cmvs_assembler::image::read_image(Cursor::new(bmp)).unwrap();
    assert_eq!(image.pixels(), [7, 6, 5, 8, 3, 2, 1, 4]);
}

#[test]
fn mask_cli_keeps_each_sample_and_rejects_transparency() {
    use cmvs_core::format::msk;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let samples = [0, 1, 127, 128, 254, 255];
    let raw = msk::encode(3, 2, &samples).unwrap();
    fs::write(root.join("mask.msk"), &raw).unwrap();
    success(cli(
        root,
        &[
            "msk",
            "decode",
            "mask.msk",
            "gray.png",
            "--profile",
            "aruto",
        ],
    ));
    let png = png::Decoder::new(Cursor::new(fs::read(root.join("gray.png")).unwrap()))
        .read_info()
        .unwrap();
    assert_eq!(
        png.output_color_type(),
        (png::ColorType::Grayscale, png::BitDepth::Eight)
    );
    success(cli(root, &["msk", "encode", "gray.png", "roundtrip.msk"]));
    assert_eq!(fs::read(root.join("roundtrip.msk")).unwrap(), raw);

    let mut encoder = png::Encoder::new(File::create(root.join("transparent.png")).unwrap(), 3, 2);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_trns([0, 127].as_slice());
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&samples)
        .unwrap();
    assert!(
        !cli(root, &["msk", "encode", "transparent.png", "invalid.msk"])
            .status
            .success()
    );
    assert!(!root.join("invalid.msk").exists());
}

#[test]
fn archive_manifest_rejects_external_inputs_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let outside = temp.path().join("outside.bin");
    fs::write(&outside, b"fixture").unwrap();
    let manifest = root.join("archive.tsv");
    let output = root.join("result.cpz");
    for input in ["../outside.bin", outside.to_str().unwrap()] {
        fs::write(
            &manifest,
            format!("cmvs-cpz-v1\tcpz3\nfile\titem\t{input}\n"),
        )
        .unwrap();
        assert!(cmvs_assembler::archive::pack(&manifest, &output, None, None).is_err());
        assert!(!output.exists());
    }
}

#[test]
fn flat_manifest_keeps_duplicate_raw_names_and_checks_the_target_profile() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("first.bin"), [1, 2, 3]).unwrap();
    fs::write(root.join("second.bin"), [4, 5, 6, 7, 8]).unwrap();
    fs::write(
        root.join("items.tsv"),
        "cmvs-cpz-v1\t-\nfile\t\\x83\\x65.bin\tfirst.bin\nfile\t\\x83\\x65.bin\tsecond.bin\n",
    )
    .unwrap();
    for format in ["cpz1", "cpz2", "cpz3"] {
        let output = format!("new.{format}");
        success(cli(
            root,
            &["cpz", "pack", "items.tsv", &output, "--format", format],
        ));
        let mut reader = File::open(root.join(output)).unwrap();
        let index = ArchiveIndex::read(&mut reader).unwrap();
        assert_eq!(index.entries().len(), 2);
        assert!(
            index
                .entries()
                .iter()
                .all(|entry| entry.name == b"\x83\x65.bin")
        );
        assert_eq!(
            index.read_entry_verified(&mut reader, 0).unwrap(),
            [1, 2, 3]
        );
        assert_eq!(
            index.read_entry_verified(&mut reader, 1).unwrap(),
            [4, 5, 6, 7, 8]
        );
    }
    assert!(
        !cli(
            root,
            &[
                "cpz",
                "pack",
                "items.tsv",
                "wrong.cpz",
                "--format",
                "cpz3",
                "--profile",
                "aruto"
            ]
        )
        .status
        .success()
    );
    fs::write(
        root.join("items.tsv"),
        format!("cmvs-cpz-v1\tcpz3\nfile\t{}\tfirst.bin\n", "x".repeat(64)),
    )
    .unwrap();
    assert!(
        !cli(root, &["cpz", "pack", "items.tsv", "wrong.cpz"])
            .status
            .success()
    );
    assert!(!root.join("wrong.cpz").exists());
}

#[test]
fn pb2_static_formats_preserve_partial_tiles_and_alpha() {
    use cmvs_assembler::image::encode_pb2;
    use cmvs_core::{
        format::{
            pb2::{self, StaticType},
            pb3::EncodeDepth,
        },
        render::raster::RgbaImage,
    };
    let (width, height) = (73, 29);
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend([
                if x < 8 { 17 } else { (x + y) as u8 },
                (x * 5) as u8,
                23,
                255,
            ]);
        }
    }
    for kind in [StaticType::Swizzled, StaticType::Blocked] {
        for depth in [EncodeDepth::Rgb24, EncodeDepth::Rgba32] {
            let mut rgba = pixels.clone();
            if depth == EncodeDepth::Rgba32 {
                rgba[3] = 0;
            }
            let image = RgbaImage::new(width, height, rgba).unwrap();
            let packed = encode_pb2(&image, kind, depth).unwrap();
            let header = pb2::Header::parse(&packed).unwrap();
            assert_eq!((header.kind, header.bpp), (kind as u16, depth.bpp()));
            assert!(packed.len() < pixels.len() / 2);
            assert_eq!(pb2::decode(&packed).unwrap(), image);
            if depth == EncodeDepth::Rgba32 {
                assert!(encode_pb2(&image, kind, EncodeDepth::Rgb24).is_err());
            }
        }
    }
}

#[test]
fn directory_manifest_preserves_empty_groups_duplicate_names_and_keyed_cpz7_tails() {
    use cmvs_core::format::cpz::{Cpz7Parameters, DirectoryFormat};
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let bytes: Vec<_> = (0..65539).map(|i| (i * 31 + 7) as u8).collect();
    fs::write(dir.join("input"), &bytes).unwrap();
    for encrypted in [false, true] {
        let manifest = format!(
            "cmvs-cpz-v1\tcpz7\nparameters\t11111111\t22222222\t33333333\t44444444\nencryption\t{}\ndir\tsame\ndir\tsame\nfile\tname\tinput\nfile\tname\tinput\n",
            u8::from(encrypted)
        );
        fs::write(dir.join("in.tsv"), manifest).unwrap();
        let path = dir.join(format!("out-{encrypted}.cpz"));
        cmvs_assembler::archive::pack(&dir.join("in.tsv"), &path, None, None).unwrap();
        let mut f = File::open(&path).unwrap();
        let format = DirectoryFormat::Cpz7 {
            parameters: Cpz7Parameters::from([0x11111111, 0x22222222, 0x33333333, 0x44444444]),
            encrypted,
        };
        let index = format.read(&mut f).unwrap();
        // Independent whole-entry decoder checks the streaming encoder across the block boundary.
        assert_eq!(index.read_entry_verified(&mut f, 1).unwrap(), bytes);
        let project = dir.join(format!("export-{encrypted}"));
        success(cli(
            dir,
            &[
                "cpz",
                "export",
                path.to_str().unwrap(),
                project.to_str().unwrap(),
                "--keys",
                "0x11111111,0x22222222,0x33333333,0x44444444",
            ],
        ));
        let out = dir.join(format!("again-{encrypted}.cpz"));
        cmvs_assembler::archive::pack(&project.join("archive.tsv"), &out, None, None).unwrap();
        assert_eq!(fs::read(path).unwrap(), fs::read(out).unwrap());
    }
}
