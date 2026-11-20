use cmvs_assembler::{archive, image, script};
use cmvs_core::{MAX_PAYLOAD, format::cpz::Cpz7Parameters};
use cmvs_tooling::{
    Result,
    cli::{Arguments, wants_help},
    read_bounded, write_new,
};
use std::{
    collections::BTreeMap,
    env,
    fs::File,
    io::{BufReader, Write},
    path::Path,
};

const USAGE: &str = "\
Usage:
  cmvs-assembler build export <game> <new-project> [--profile <id|game>]
  cmvs-assembler build extract <project> <asset-id> <new-output> --source <game>
  cmvs-assembler build assemble <project> <new-game> --source <game> [--profile <id|game>]
  cmvs-assembler script assemble <project> <new-script> [--profile <id|game>] [--compressed|--uncompressed]
  cmvs-assembler script import-text <script> <table.tsv> <new-script> --target-encoding <shift-jis|gbk|utf-8>
  cmvs-assembler msk encode <grayscale.png> <new.msk> [--profile <id|game>]
  cmvs-assembler msk decode <mask.msk> <new.png> [--profile <id|game>]
  cmvs-assembler pb2 decode <static.pb2> <new.png> [--profile <id|game>]
  cmvs-assembler pb2 encode <input.png|input.bmp> <new.pb2> [--type 1|2] [--bpp 24|32] [--profile <id|game>]
  cmvs-assembler cursor decode <input.cur> <new.tsv>
  cmvs-assembler cursor encode <input.tsv> <new.cur>
  cmvs-assembler pb2 export <collection.pb2> <new-directory>
  cmvs-assembler pb2 pack <images.tsv> <new.pb2>
  cmvs-assembler pb3 decode <image.pb3> <new.png> [--base <base.png>]
  cmvs-assembler pb3 encode <input.png|input.bmp> <new.pb3> [--type 1|5|6|7|8] [--bpp 24|32]
                            [--base <base.png> --reference <stem>] [--profile <id|game>]
  cmvs-assembler cpz pack <manifest> <new.cpz> [--format cpz1|cpz2|cpz3|cpz5|cpz6|cpz7] [--profile <id|game>]
  cmvs-assembler cpz export <source.cpz> <new-directory> [--profile <id|game>] [--start-script <path>]
  cmvs-assembler cpz replace <source.cpz> <new.cpz> <index> <file> [<index> <file> ...]
                             [--start-script <path>] [--verify-all] [--profile <id|game>]

Export an editable project with cmvs-disassembler --export.
Create a translation table with cmvs-disassembler --export-text.
PB2A and PB3B encode accept static PNG (up to 8 bits per channel) and 32-bit BI_RGB BMP.
Use --uncompressed for literal PB image streams. PB2 defaults to type 2. To retain a source image format, select its type and bit depth.
The default depth is 32. A 24-bit output requires fully opaque pixels.
CPZ1/2/3/5/6/7 replacements use zero-based entry numbers from cmvs-cpz --list.
CPZ5 detects its packing mode or reads it from --start-script. CPZ7 uses the
adjacent start.ps3, --start-script, or --keys <k0,k1,k2,k3> (decimal or 0x-prefixed).
Outputs must be new paths. Use -- before filenames beginning with a dash.";

fn run() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if wants_help(&args) {
        println!("{USAGE}");
        return Ok(());
    }
    let [resource, action, args @ ..] = args.as_slice() else {
        return Err("expected resource and action".into());
    };
    match (resource.to_str(), action.to_str()) {
        (Some("build"), Some(action @ ("export" | "extract" | "assemble"))) => {
            use cmvs_assembler::build;
            let options: &[&str] = match action {
                "export" => &["--profile"],
                "extract" => &["--source"],
                _ => &["--profile", "--source"],
            };
            let args = Arguments::parse(args, options, &[])?;
            let profile = args.profile()?.map(|p| p.id.as_str());
            if action == "export" {
                let [game, out] = args.paths.as_slice() else {
                    return Err("expected game and new project paths".into());
                };
                return build::export(Path::new(game), Path::new(out), profile);
            }
            let game = Path::new(
                args.option("--source")
                    .ok_or("expected --source <original-game>")?,
            );
            match args.paths.as_slice() {
                [project, out] if action == "assemble" => {
                    build::assemble(game, Path::new(project), Path::new(out), profile)
                }
                [project, id, out] if action == "extract" => build::extract(
                    game,
                    Path::new(project),
                    id.to_str().ok_or("invalid asset id")?.parse()?,
                    Path::new(out),
                ),
                _ => Err("invalid build arguments".into()),
            }
        }
        (Some("cursor"), Some(action @ ("decode" | "encode"))) => {
            let args = Arguments::parse(args, &["--profile"], &[])?;
            args.profile()?;
            let [input, output] = args.paths.as_slice() else {
                return Err("expected input and output cursor paths".into());
            };
            if action == "decode" {
                let raw = read_bounded(Path::new(input), MAX_PAYLOAD)?;
                write_new(Path::new(output), |f| {
                    cmvs_assembler::cursor::export(&raw, f)
                })
            } else {
                let raw = cmvs_assembler::cursor::assemble(BufReader::new(File::open(input)?))?;
                write_new(Path::new(output), |f| Ok(f.write_all(&raw)?))
            }
        }
        (Some("msk"), Some(action @ ("encode" | "decode"))) => {
            let args = Arguments::parse(args, &["--profile"], &[])?;
            if args.profile()?.is_some_and(|p| !p.is_cvns()) {
                return Err("MSK0 requires a CVNS profile".into());
            }
            let [input, output] = args.paths.as_slice() else {
                return Err("expected input and output paths".into());
            };
            if action == "encode" {
                let mask = image::png_to_msk(BufReader::new(File::open(input)?))?;
                write_new(Path::new(output), |file| Ok(file.write_all(&mask)?))
            } else {
                let mask = read_bounded(Path::new(input), MAX_PAYLOAD)?;
                write_new(Path::new(output), |file| image::msk_to_png(&mask, file))
            }
        }
        (Some(resource @ ("pb2" | "pb3")), Some(action)) => {
            cmvs_assembler::pb::run(resource, action, args)
        }
        (Some("cpz"), Some("export")) => {
            let args = Arguments::parse(args, &["--profile", "--start-script", "--keys"], &[])?;
            let [input, output] = args.paths.as_slice() else {
                return Err("expected archive and new project directory".into());
            };
            let mut file = File::open(input)?;
            let index = archive::open(
                &mut file,
                Path::new(input),
                args.option("--start-script").map(Path::new),
                args.profile()?,
                keys(&args)?,
            )?;
            archive::export(&index, &mut file, Path::new(output))
        }
        (Some("cpz"), Some("pack")) => {
            let args = Arguments::parse(args, &["--profile", "--format"], &[])?;
            let [manifest, output] = args.paths.as_slice() else {
                return Err("expected manifest and output archive paths".into());
            };
            let format = args
                .option("--format")
                .map(|v| v.to_str().ok_or("invalid format name"))
                .transpose()?;
            archive::pack(
                Path::new(manifest),
                Path::new(output),
                format,
                args.profile()?,
            )
        }
        (Some("cpz"), Some("replace")) => {
            let args = Arguments::parse(
                args,
                &["--start-script", "--profile", "--keys"],
                &["--verify-all"],
            )?;
            replace(&args)
        }
        (Some("script"), Some("assemble")) => {
            let args = Arguments::parse(args, &["--profile"], &["--compressed", "--uncompressed"])?;
            let [input, output] = args.paths.as_slice() else {
                return Err("expected project and output script paths".into());
            };
            let compressed = match (args.option("--compressed"), args.option("--uncompressed")) {
                (Some(_), Some(_)) => return Err("choose one script compression mode".into()),
                (Some(_), None) => Some(true),
                (None, Some(_)) => Some(false),
                _ => None,
            };
            let (bytes, moved) = script::assemble(
                BufReader::new(File::open(input)?),
                args.profile()?,
                compressed,
            )?;
            write_new(Path::new(output), |file| Ok(file.write_all(&bytes)?))?;
            if moved {
                eprintln!(
                    "Code layout changed. Saved program counters may require a new game or save."
                );
            }
            Ok(())
        }
        (Some("script"), Some("import-text")) => {
            let args = Arguments::parse(args, &["--target-encoding", "--profile"], &[])?;
            let [input, table, output] = args.paths.as_slice() else {
                return Err("expected script, translation table and output paths".into());
            };
            let encoding = args.encoding("--target-encoding")?;
            let original = args.read_script(Path::new(input))?;
            let script =
                script::import_text(&original, BufReader::new(File::open(table)?), encoding)?;
            write_new(Path::new(output), |file| {
                Ok(file.write_all(&script.encode()?)?)
            })
        }
        _ => Err("unknown resource or action".into()),
    }
}

fn keys(args: &Arguments<'_>) -> Result<Option<Cpz7Parameters>> {
    args.option("--keys")
        .map(|v| Ok(v.to_str().ok_or("invalid CPZ7 keys")?.parse()?))
        .transpose()
}
fn replace(args: &Arguments<'_>) -> Result<()> {
    let [input, output, pairs @ ..] = args.paths.as_slice() else {
        return Err("expected source and destination archives".into());
    };
    let (pairs, tail) = pairs.as_chunks::<2>();
    if pairs.is_empty() || !tail.is_empty() {
        return Err("expected pairs of entry number and replacement file".into());
    }
    let mut source = File::open(input)?;
    let index = archive::open(
        &mut source,
        Path::new(input),
        args.option("--start-script").map(Path::new),
        args.profile()?,
        keys(args)?,
    )?;
    let mut replacements = BTreeMap::new();
    for [number, path] in pairs {
        let i: usize = number.to_str().ok_or("expected entry number")?.parse()?;
        if i >= index.entries().len() || replacements.contains_key(&i) {
            return Err(format!("out-of-range or repeated entry number {i}").into());
        }
        replacements.insert(i, Path::new(path));
    }
    archive::replace_files(
        &mut source,
        &index,
        Path::new(output),
        &replacements,
        args.option("--verify-all").is_some(),
    )
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-assembler --help' for usage.");
        std::process::exit(1);
    }
}
