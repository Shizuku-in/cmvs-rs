# 《クロノクロック》汉化补丁转换

支持 [葵海计划 1.0 / 1.05 补丁](https://vndb.org/r114391)。依赖、输出目录要求和编码规则见[工具总览](../README.md)。每版约需 4.4 GiB。

## 转换与启动

从仓库根目录执行：

```bash
python3 tools/fan_patch/chronoclock/convert_chronoclock_patch.py \
  "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>" --encoding utf-8 --patch-version 1.05

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20150405 --encoding utf-8 --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

示例输出 UTF-8。如需 GBK，将转换和启动命令中的 `--encoding utf-8` 同时改为 `--encoding gbk`。

## 转换内容

按文本偏移、原文及脚本上下文匹配译文，重定位字符串引用并转换 BMP 图片。每版处理 86 个脚本来源（包括包内和包外各自的 `start.ps3`）、52,362 条译文和 92 张图片。复制原版 `save/system.dat` 模板并保留汉化声明。

`--patch-version` 可选 `1.0`（默认）或 `1.05`。两版使用同一日版资源和译文，仅 `dialog_chip.pb3` 不同；1.05 保留扩展至 1280 像素宽的图集。

## 验证

两版的既有 GBK 运行记录均覆盖标题、中文退出对话框、10 个不同消息等待，以及首次剧情存读档后继续播放。图片完整 RGBA 已与各版 BMP 核对。完整路线、原生音频和原版存档互操作尚未验证。新增 UTF-8 输出的验证范围见[工具总览](../README.md#验证范围)。

```bash
python3 -m unittest discover -s tools/fan_patch/chronoclock -p 'test_*.py'
cargo test --offline --locked --manifest-path tools/fan_patch/chronoclock/Cargo.toml \
  --target-dir target/fan-patch-chronoclock
```
