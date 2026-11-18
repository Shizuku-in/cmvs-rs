//! Zero-dependency microbenchmark runner for CMVS format, archive, video, rendering,
//! and VM execution hot paths.
use cmvs_core::{
    format::{
        cpz::{ArchiveDirectory, ArchiveFile, ArchiveIndex, Cpz5Index, Cpz5Mode},
        lzss,
        pb3::{self, EncodeDepth},
        ps3::Script,
    },
    host::input::ActionInput,
    profile::Semantics,
    render::{
        camera::CameraHost,
        message::MessageHost,
        raster::RgbaImage,
        scene::{DrawState, ObjectImage, SceneHost, SceneObject},
        scene_render::prepare_scene,
        viewport::ScreenSize,
    },
    vm::{
        dispatch::{self, Hosts},
        machine::{Machine, Step},
    },
};
use cmvs_font::font::FontRenderer;
use std::{
    error::Error,
    hint::black_box,
    io::Cursor,
    process::ExitCode,
    time::{Duration, Instant},
};

const USAGE: &str = "\
Usage: cmvs-bench [--smoke|--quick] [FILTER]

Options:
  FILTER             Run benchmarks whose names contain this text
  --smoke, --quick   Run with fewer iterations for a quick smoke check
  -h, --help         Print this help message";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchMode {
    Standard,
    Smoke,
}

#[derive(Debug)]
struct BenchResult {
    name: &'static str,
    iterations: u64,
    elapsed: Duration,
    units_per_iter: Option<(u64, &'static str)>,
}

impl BenchResult {
    fn ns_per_iter(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1e9 / (self.iterations.max(1) as f64)
    }

    fn throughput_display(&self) -> String {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            return "-".to_owned();
        }
        match self.units_per_iter {
            Some((bytes, "B")) => {
                let mib_s = (bytes as f64 * self.iterations as f64) / (1024.0 * 1024.0) / secs;
                format!("{mib_s:8.1} MiB/s")
            }
            Some((ops, unit)) => {
                let mops_s = (ops as f64 * self.iterations as f64) / 1e6 / secs;
                format!("{mops_s:8.2} M{unit}/s")
            }
            None => "-".to_owned(),
        }
    }
}

fn measure(
    name: &'static str,
    iterations: u64,
    units_per_iter: Option<(u64, &'static str)>,
    mut f: impl FnMut(),
) -> BenchResult {
    // One untimed warmup iteration primes caches and lazy initializers.
    f();
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    BenchResult {
        name,
        iterations,
        elapsed: start.elapsed(),
        units_per_iter,
    }
}

fn synthetic_lzss_input(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let motif = b"CMVS_SCRIPT_DIALOGUE_RUN_0123456789_BACKGROUND_CG_LAYER_";
    let mut state = 0x1234_5678u32;
    while out.len() < len {
        if (state & 3) != 0 {
            let take = motif.len().min(len - out.len());
            out.extend_from_slice(&motif[..take]);
        } else {
            for _ in 0..16.min(len - out.len()) {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                out.push((state >> 24) as u8);
            }
        }
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    }
    out
}

fn synthetic_rgba_sprite(width: u32, height: u32) -> RgbaImage {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 7 + y * 3) & 0xff) as u8;
            let g = (((x * 5) ^ (y * 11)) & 0xff) as u8;
            let b = ((x + y * 13) & 0xff) as u8;
            let a = if (x + y) % 5 == 0 { 192 } else { 255 };
            pixels.extend_from_slice(&[r, g, b, a]);
        }
    }
    RgbaImage::new(width, height, pixels).expect("valid synthetic RGBA sprite")
}

fn build_cpz5_archive(entry_count: usize) -> Vec<u8> {
    let mut files = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        files.push(ArchiveFile {
            name: format!("res_{i:04}_scene.pb3").into_bytes(),
            data: vec![(i & 0xff) as u8; 8],
        });
    }
    let dirs = [ArchiveDirectory {
        name: b"pack".to_vec(),
        files,
    }];
    let mut cursor = Cursor::new(Vec::new());
    Cpz5Index::write(&mut cursor, Cpz5Mode::Mode9, &dirs).expect("valid CPZ5 archive");
    cursor.into_inner()
}

fn minimal_bench_ttf() -> Vec<u8> {
    let mut cmap = Vec::new();
    for v in [0u16, 1, 3, 1] {
        cmap.extend(v.to_be_bytes());
    }
    cmap.extend(12u32.to_be_bytes());
    for v in [
        4u16,
        32,
        0,
        4,
        4,
        1,
        0,
        0x0041,
        0xffff,
        0,
        0x0041,
        0xffff,
        (-64i16) as u16,
        1,
        0,
        0,
    ] {
        cmap.extend(v.to_be_bytes());
    }
    let mut glyf = Vec::new();
    for v in [1i16, 0, 0, 500, 700] {
        glyf.extend(v.to_be_bytes());
    }
    glyf.extend(3u16.to_be_bytes());
    glyf.extend(0u16.to_be_bytes());
    glyf.extend([0x01u8; 4]);
    for v in [0i16, 500, 0, -500, 0, 0, 700, -700] {
        glyf.extend(v.to_be_bytes());
    }
    let mut head = vec![0u8; 54];
    head[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[38..40].copy_from_slice(&1000i16.to_be_bytes());
    head[40..42].copy_from_slice(&1000i16.to_be_bytes());
    head[46..48].copy_from_slice(&8u16.to_be_bytes());
    head[48..50].copy_from_slice(&2i16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[10..12].copy_from_slice(&1000u16.to_be_bytes());
    hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
    let mut hmtx = Vec::new();
    for (adv, lsb) in [(500u16, 0i16), (600u16, 0i16)] {
        hmtx.extend(adv.to_be_bytes());
        hmtx.extend(lsb.to_be_bytes());
    }
    let mut loca = Vec::new();
    for off in [0u16, 0, (glyf.len() / 2) as u16] {
        loca.extend(off.to_be_bytes());
    }
    let mut maxp = vec![0u8; 32];
    maxp[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&2u16.to_be_bytes());
    maxp[6..8].copy_from_slice(&4u16.to_be_bytes());
    maxp[8..10].copy_from_slice(&1u16.to_be_bytes());
    let mut name = Vec::new();
    let family_utf16: Vec<u8> = "CmvsBench"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    for v in [0u16, 1, 18, 3, 1, 0x0409, 1, family_utf16.len() as u16, 0] {
        name.extend(v.to_be_bytes());
    }
    name.extend(family_utf16);

    let tables: [(&[u8; 4], Vec<u8>); 8] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
    ];
    let mut sfnt = Vec::new();
    sfnt.extend(0x0001_0000u32.to_be_bytes());
    sfnt.extend((tables.len() as u16).to_be_bytes());
    sfnt.extend(128u16.to_be_bytes());
    sfnt.extend(3u16.to_be_bytes());
    sfnt.extend(0u16.to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut payload = Vec::new();
    for (tag, data) in &tables {
        sfnt.extend_from_slice(*tag);
        sfnt.extend(0u32.to_be_bytes());
        sfnt.extend((offset as u32).to_be_bytes());
        sfnt.extend((data.len() as u32).to_be_bytes());
        payload.extend_from_slice(data);
        let pad = (4 - data.len() % 4) % 4;
        payload.resize(payload.len() + pad, 0);
        offset += data.len() + pad;
    }
    sfnt.extend(payload);
    sfnt
}

fn ps2_script(code: &[u8], text: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend_from_slice(code);
    payload.extend_from_slice(text);
    let mut raw = b"PS2A".to_vec();
    for word in [
        48u32,
        0,
        0,
        1,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(word.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).expect("valid synthetic PS2A script")
}

fn push_call(code: &mut Vec<u8>, command: u16, words: &[u32]) {
    for &word in words {
        code.extend(0x0440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(word.to_le_bytes());
    }
    code.extend(command.to_le_bytes());
}

fn build_vm_loop_script() -> Script {
    // Loop at pc = 0:
    //   var[8] = var[8] + 3
    //   var[12] = var[8]
    //   jump 0
    let int_lit = |val: i32| -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(0x0200u16.to_le_bytes());
        out.extend(0x0100u16.to_le_bytes());
        out.extend((val as u32).to_le_bytes());
        out.extend(0x020fu16.to_le_bytes());
        out
    };
    let var_ref = |offset: i32| -> Vec<u8> {
        let mut out = 0x0101u16.to_le_bytes().to_vec();
        out.extend(int_lit(offset));
        out
    };

    let mut code = Vec::new();
    // Expression 1: var[8] = var[8] + 3
    code.extend(0x0200u16.to_le_bytes());
    code.extend(var_ref(8));
    code.extend(var_ref(8));
    code.extend(0x0100u16.to_le_bytes());
    code.extend(3u32.to_le_bytes());
    code.extend(0x0163u16.to_le_bytes()); // +
    code.extend(0x0170u16.to_le_bytes()); // =
    code.extend(0x020fu16.to_le_bytes());

    // Expression 2: var[12] = var[8]
    code.extend(0x0200u16.to_le_bytes());
    code.extend(var_ref(12));
    code.extend(var_ref(8));
    code.extend(0x0170u16.to_le_bytes()); // =
    code.extend(0x020fu16.to_le_bytes());

    // Jump back to pc = 0
    code.extend(0x0400u16.to_le_bytes());
    code.extend(0u32.to_le_bytes());

    ps2_script(&code, b"\0")
}

fn run_suite(
    mode: BenchMode,
    filter: Option<&str>,
) -> Result<Vec<BenchResult>, Box<dyn Error + Send + Sync>> {
    let matches = |name: &str| filter.is_none_or(|pat| name.contains(pat));
    let mut results = Vec::new();

    // 1. LZSS encode & decode
    let lzss_len = if mode == BenchMode::Smoke {
        4 * 1024
    } else {
        32 * 1024
    };
    let lzss_plain = synthetic_lzss_input(lzss_len);
    let lzss_compressed = lzss::encode(&lzss_plain);
    assert_eq!(
        lzss::decode(&lzss_compressed, lzss_plain.len())?,
        lzss_plain
    );

    if matches("lzss_encode") {
        let iters = if mode == BenchMode::Smoke { 2 } else { 40 };
        results.push(measure(
            "lzss_encode",
            iters,
            Some((lzss_plain.len() as u64, "B")),
            || {
                black_box(lzss::encode(black_box(&lzss_plain)));
            },
        ));
    }
    if matches("lzss_decode") {
        let iters = if mode == BenchMode::Smoke { 4 } else { 200 };
        results.push(measure(
            "lzss_decode",
            iters,
            Some((lzss_plain.len() as u64, "B")),
            || {
                let decoded =
                    lzss::decode(black_box(&lzss_compressed), black_box(lzss_plain.len()))
                        .expect("valid LZSS stream");
                black_box(decoded);
            },
        ));
    }

    // 2. PB3 encode & decode
    let (pb3_w, pb3_h) = if mode == BenchMode::Smoke {
        (64, 64)
    } else {
        (128, 128)
    };
    let sprite = synthetic_rgba_sprite(pb3_w, pb3_h);
    let pb3_bytes = pb3::encode(&sprite, EncodeDepth::Rgba32)?;
    assert_eq!(pb3::decode(&pb3_bytes)?, sprite);
    let raw_rgba_len = u64::from(pb3_w) * u64::from(pb3_h) * 4;

    if matches("pb3_encode") {
        let iters = if mode == BenchMode::Smoke { 2 } else { 25 };
        results.push(measure(
            "pb3_encode",
            iters,
            Some((raw_rgba_len, "B")),
            || {
                let encoded =
                    pb3::encode(black_box(&sprite), EncodeDepth::Rgba32).expect("PB3 encode");
                black_box(encoded);
            },
        ));
    }
    if matches("pb3_decode") {
        let iters = if mode == BenchMode::Smoke { 4 } else { 80 };
        results.push(measure(
            "pb3_decode",
            iters,
            Some((raw_rgba_len, "B")),
            || {
                let decoded = pb3::decode(black_box(&pb3_bytes)).expect("PB3 decode");
                black_box(decoded);
            },
        ));
    }

    // 3. CPZ O(1) case-folded archive lookup (1,200 entries)
    if matches("cpz_lookup_o1") {
        let cpz_bytes = build_cpz5_archive(1_200);
        let index = ArchiveIndex::read(&mut Cursor::new(&cpz_bytes))?;
        assert_eq!(index.entries().len(), 1_200);
        assert_eq!(index.lookup(b"RES_1199_SCENE.PB3")?, Some(1_199));
        let queries: [&[u8]; 4] = [
            b"RES_0000_SCENE.PB3",
            b"Res_0599_Scene.pb3",
            b"RES_1199_SCENE.PB3",
            b"MISSING_RESOURCE.PB3",
        ];
        let batch = if mode == BenchMode::Smoke { 64 } else { 2_000 };
        let iters = if mode == BenchMode::Smoke { 4 } else { 50 };
        results.push(measure(
            "cpz_lookup_o1",
            iters,
            Some((batch * queries.len() as u64, "ops")),
            || {
                for _ in 0..batch {
                    for query in queries {
                        let hit = index.lookup(black_box(query)).expect("valid lookup query");
                        black_box(hit);
                    }
                }
            },
        ));
    }

    // 4. YUV 4:2:0 -> RGBA conversion (`cmvs-video` SIMD/scalar BT.601 kernel)
    if matches("yuv420_to_rgba") {
        let (w, h) = (640u32, 360u32);
        let plane_w = w as usize;
        let plane_h = h as usize;
        let y_plane: Vec<u8> = (0..plane_w * plane_h)
            .map(|i| ((i.wrapping_mul(37)) & 0xff) as u8)
            .collect();
        let u_plane = vec![96u8; (plane_w / 2) * (plane_h / 2)];
        let v_plane = vec![160u8; (plane_w / 2) * (plane_h / 2)];
        let frame_bytes = u64::from(w) * u64::from(h) * 4;
        let iters = if mode == BenchMode::Smoke { 8 } else { 400 };
        results.push(measure(
            "yuv420_to_rgba",
            iters,
            Some((frame_bytes, "B")),
            || {
                let img = cmvs_video::convert_yuv420_planar(
                    black_box(w),
                    black_box(h),
                    black_box(&y_plane),
                    black_box(&u_plane),
                    black_box(&v_plane),
                )
                .expect("valid YUV420 buffers");
                black_box(img);
            },
        ));
    }

    // 5. Software scene preparation & CPU compositing with idle FontRenderer glyph-scene cache
    if matches("scene_render") {
        let (canvas_w, canvas_h) = (320u32, 180u32);
        let screen = ScreenSize::new([canvas_w, canvas_h]).expect("valid screen size");
        let mut scene = SceneHost::default();
        scene.set_screen_size(screen);
        let cameras = CameraHost::default();

        let mut font = FontRenderer::from_bytes(minimal_bench_ttf(), 0)?;
        let mut messages = MessageHost::default();
        messages.set_glyph_metrics(font.metrics());
        messages.set_record_output(true);

        let [logical_w, logical_h] = messages.screen_size();
        let mut setup_code = Vec::new();
        push_call(
            &mut setup_code,
            0x215d,
            &[logical_h, logical_w, 60, 280, 110, 20, 0, 0],
        );
        push_call(&mut setup_code, 0x214e, &[1, 0]);
        push_call(&mut setup_code, 0x2152, &[0, u32::MAX, 0]);
        let setup_script = ps2_script(&setup_code, b"AAAAAAAA\0");
        let mut setup_vm = Machine::new(&setup_script)?;
        for _ in 0..2 {
            while !matches!(setup_vm.step()?, Step::System { .. }) {}
            let mut hosts = Hosts {
                messages: Some(&mut messages),
                ..Hosts::default()
            };
            dispatch::run(Semantics::Root, &mut setup_vm, &mut hosts)?;
        }
        while !matches!(setup_vm.step()?, Step::System { .. }) {}
        let mut input = ActionInput::default();
        messages.try_record_output(&mut setup_vm, &mut input, Default::default())?;
        messages.advance_frame(1_000);

        let bg_pixels = RgbaImage::new(
            canvas_w,
            canvas_h,
            [0x18u8, 0x24, 0x38, 0xff].repeat((canvas_w * canvas_h) as usize),
        )?;
        let mut bg_draw = DrawState::default();
        bg_draw.set_source_size(canvas_w, canvas_h);
        bg_draw.set_layer(0);
        let bg_obj = SceneObject {
            image: Some(ObjectImage {
                filter: None,
                name: b"bg.pb3".to_vec(),
                pixels: bg_pixels,
                alpha: false,
                surface: false,
            }),
            draw: Some(bg_draw),
            ..SceneObject::default()
        };

        let char_pixels = synthetic_rgba_sprite(96, 128);
        let mut char_draw = DrawState::default();
        char_draw.set_source_size(96, 128);
        char_draw.set_position([112, 24]);
        char_draw.set_blend_mode(1);
        char_draw.set_layer(1);
        let char_obj = SceneObject {
            image: Some(ObjectImage {
                filter: None,
                name: b"ch01.pb3".to_vec(),
                pixels: char_pixels,
                alpha: true,
                surface: false,
            }),
            draw: Some(char_draw),
            ..SceneObject::default()
        };

        let iters = if mode == BenchMode::Smoke { 3 } else { 40 };
        let canvas_bytes = u64::from(canvas_w) * u64::from(canvas_h) * 4;
        results.push(measure(
            "scene_render",
            iters,
            Some((canvas_bytes, "B")),
            || {
                let glyphs = font
                    .prepare_objects(black_box(&messages))
                    .expect("glyph objects");
                let mut additional: Vec<&SceneObject> = Vec::with_capacity(2 + glyphs.len() + 1);
                additional.push(&bg_obj);
                additional.push(&char_obj);
                additional.extend(glyphs.iter());
                additional.extend(messages.visible_window_objects());
                let frame = prepare_scene(&scene, Some(&cameras), &additional, canvas_w, canvas_h)
                    .expect("prepare_scene");
                let out = frame.render_cpu().expect("render_cpu");
                black_box(out);
            },
        ));
    }

    // 6. VM instruction stepping
    if matches("vm_step") {
        let loop_script = build_vm_loop_script();
        let mut vm = Machine::new(&loop_script)?;
        vm.state_mut().set_integer(8, 0).expect("slot 8");
        vm.state_mut().set_integer(12, 0).expect("slot 12");
        let steps_per_iter = if mode == BenchMode::Smoke { 300 } else { 6_000 };
        let iters = if mode == BenchMode::Smoke { 4 } else { 50 };
        results.push(measure(
            "vm_step",
            iters,
            Some((steps_per_iter, "inst")),
            || {
                for _ in 0..steps_per_iter {
                    let step = vm.step().expect("valid VM loop step");
                    black_box(step);
                }
            },
        ));
    }

    Ok(results)
}

enum CliOutcome {
    Help,
    Run {
        mode: BenchMode,
        filter: Option<String>,
    },
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<CliOutcome, String> {
    let mut mode = BenchMode::Standard;
    let mut filter = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(CliOutcome::Help),
            "--smoke" | "--quick" => mode = BenchMode::Smoke,
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option `{flag}` (pass --help for usage)"));
            }
            pat => {
                if filter.is_some() {
                    return Err("at most one benchmark filter pattern may be specified".to_owned());
                }
                filter = Some(pat.to_owned());
            }
        }
    }
    Ok(CliOutcome::Run { mode, filter })
}

fn main() -> ExitCode {
    let parsed = match parse_args(std::env::args().skip(1)) {
        Ok(CliOutcome::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(run) => run,
        Err(msg) => {
            eprintln!("error: {msg}");
            return ExitCode::from(2);
        }
    };
    let CliOutcome::Run { mode, filter } = parsed else {
        unreachable!();
    };

    let suite_start = Instant::now();
    let results = match run_suite(mode, filter.as_deref()) {
        Ok(results) => results,
        Err(err) => {
            eprintln!("benchmark failure: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "{:<18} {:>10} {:>14} {:>16}",
        "benchmark", "iters", "ns/iter", "throughput"
    );
    println!("{}", "-".repeat(62));
    for r in &results {
        println!(
            "{:<18} {:>10} {:>14.1} {:>16}",
            r.name,
            r.iterations,
            r.ns_per_iter(),
            r.throughput_display()
        );
    }
    println!(
        "\ncompleted {} benchmark(s) in {:.3}s ({:?} mode)",
        results.len(),
        suite_start.elapsed().as_secs_f64(),
        mode
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_benchmark_suite_runs_cleanly_and_covers_all_hot_paths() {
        let results = run_suite(BenchMode::Smoke, None).expect("smoke suite must succeed");
        let names: Vec<&str> = results.iter().map(|r| r.name).collect();
        assert_eq!(
            names,
            vec![
                "lzss_encode",
                "lzss_decode",
                "pb3_encode",
                "pb3_decode",
                "cpz_lookup_o1",
                "yuv420_to_rgba",
                "scene_render",
                "vm_step",
            ]
        );
        for res in &results {
            assert!(res.iterations > 0);
            assert!(!res.throughput_display().is_empty());
        }
        assert!(matches!(
            parse_args(["--quick".to_owned()]).unwrap(),
            CliOutcome::Run {
                mode: BenchMode::Smoke,
                filter: None
            }
        ));
        assert!(matches!(
            parse_args(["--help".to_owned()]).unwrap(),
            CliOutcome::Help
        ));
        assert!(parse_args(["--unknown".to_owned()]).is_err());
    }
}
