//! Android game host with native lifecycle, touch input and GPU presentation.
#[cfg(target_os = "android")]
mod app;
#[cfg(target_os = "android")]
mod backend;

#[cfg(target_os = "android")]
#[allow(unsafe_code)]
// SAFETY: unique entry point required by android-activity, with its Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("CMVS")
            .with_max_level(log::LevelFilter::Info),
    );
    log::info!("Android game starting");
    let directory = app.internal_data_path();
    if let Err(error) = app::run(app) {
        log::error!("Android game failed: {error}");
        if let Some(directory) = directory {
            let _ = std::fs::write(
                directory.join("launch-error.txt"),
                "游戏加载或运行失败。请确认游戏数据完整、字体有效且游戏版本受支持。",
            );
        }
    }
    log::info!("Android game stopped");
}
