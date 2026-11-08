# 《クロノクロック》 葵海计划汉化转换

将《クロノクロック》 [葵海计划 1.0 / 1.05 补丁](https://vndb.org/r114391)中的译文和 BMP 转为标准 PS2A、CPZ3 和 PB3B，供 cmvs-rs 使用。

## 使用

需要 Python 3.10+、本项目要求的 Rust/Cargo 和已缓存的依赖。从仓库根目录执行，两版分别输出：

```bash
for version in 1.0 1.05; do
  python3 tools/fan_patch/chronoclock/convert_chronoclock_patch.py \
    "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>-$version" --patch-version "$version"
done

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20150405 --encoding gbk --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

输出目录必须不存在且位于原版目录之外，每版约需 4.4 GiB。输入只读；复制原版资源、程序、配置和 `save/system.dat` 模板，保留汉化声明，不安装补丁 exe/DLL 或剧情存档。完成后生成 `conversion.json`；失败后另选新目录重试。

## 转换与验证

每版处理 86 个脚本来源（含包内、包外各自的 `start.ps3`）、52,362 条译文和 92 张图片。按文本偏移、原文及脚本上下文匹配译文，调整字符串引用；复用已有脚本、封包和图片写入器，不修改运行时。GBK 不支持的 `♪` / `・` / `䇲` 分别替换为 `☆` / `·` / `筴`。

两版使用同一日版资源，译文相同，仅 `dialog_chip.pb3` 不同；1.05 保留扩展至 1280 像素宽的图集。已通过 10 项合成测试、严格 Clippy、全部输出脚本及封包校验；图片完整 RGBA 与各版 BMP 一致。两版均确认标题、中文退出对话框、10 个不同中文消息等待及首次剧情存读档后继续播放。完整路线、原生音频和原版存档互操作尚未验证。

```bash
python3 -m unittest discover -s tools/fan_patch/chronoclock -p test_convert_chronoclock_patch.py
cargo test --offline --locked --manifest-path tools/fan_patch/chronoclock/Cargo.toml \
  --target-dir target/fan-patch-chronoclock
```
