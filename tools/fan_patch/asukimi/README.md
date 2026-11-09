# 《明日の君と逢うために》汉化补丁转换

支持 [CK-GAL V1.0 补丁](https://vndb.org/r20550)。依赖、输出目录要求和编码规则见[工具总览](../README.md)。

## 转换与启动

从仓库根目录执行：

```bash
python3 tools/fan_patch/asukimi/convert_asukimi_patch.py \
  "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>" --encoding utf-8

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20071117 --encoding utf-8 --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

示例输出 UTF-8（默认，也接受 `utf8`）。如需 GBK，将转换和启动命令中的 `--encoding utf-8` 同时改为 `--encoding gbk`。UTF-8 转换需要 Rust/Cargo 和已缓存依赖；GBK 转换仅需 Python 标准库。

## 转换内容

按所选编码写入补丁文本，修正 `start.ps2` 长度字段，重建脚本包校验值，将 28 张 BMP 转为 PB3B。共处理 126 个包内脚本及启动脚本。不安装补丁配置或 `save125.dat`；仅沿用原版 `save/system.dat` 模板，补丁说明保存为 `patch-readme.txt`。

UTF-8 模式以原版指令布局校验补丁，仅修改字符串引用，支持引用字符串内部字符及终止符。补丁的字节模式替换误覆盖了两处代码：`snas09.ps2` 的 `code+0x3a6c1` 和 `snma10.ps2` 的 `code+0x20e51`。转换器核对原版和补丁的精确字节后修复，并在 `conversion.json` 中记录；遇到其他代码变化则停止。原版 `snsy13.ps2` 的空整数表达式按原字节保留，不改变其运行时语义。GBK 模式不做这些代码修复。

## 验证

UTF-8 已完成全部 126 个包内脚本及启动脚本的实际转换和格式检查；逐条比对确认文本内容与 GBK 源补丁一致。桌面版以 `--encoding utf-8` 启动并显示中文标题菜单。字符串内部引用、非法字符、精确修复和空表达式均有自动测试。

已有 GBK 运行记录覆盖中文标题、8 个不同消息等待，以及首次剧情存读档后继续播放。补丁说明中的朝日线 7 月 2 日崩溃尚未验证是否解决。完整路线、原生音频和原版存档互操作尚未验证。

```bash
python3 -m unittest discover -s tools/fan_patch/asukimi -p 'test_*.py'
cargo test --offline --locked --manifest-path tools/fan_patch/asukimi/Cargo.toml \
  --target-dir target/fan-patch-asukimi
```
