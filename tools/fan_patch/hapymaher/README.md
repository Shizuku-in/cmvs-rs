# 《ハピメア》 里想奈骑士团汉化转换

将《ハピメア》 [里想奈骑士团测试版补丁](https://vndb.org/r56370)转为标准 PS2A、CPZ3 和 PB3B，供 cmvs-rs 使用。

## 使用

需要 Python 3.10+、本项目要求的 Rust/Cargo 和已缓存的依赖。从仓库根目录执行：

```bash
python3 tools/fan_patch/hapymaher/convert_hapymaher_patch.py \
  "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>"

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20130131 --encoding gbk --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

输出目录必须不存在且位于原版目录之外，约需 3.8 GiB。工具复制原版资源、程序和配置，仅沿用原版 `save/system.dat` 模板，不安装补丁 exe/DLL、配置或存档。输入只读，完成后生成 `conversion.json`；失败后另选新目录重试。

## 转换与验证

从补丁 `word.mai` 和 DLL 数据表还原自定义字码，转为 GBK 并调整字符串引用；复用未来乡愁转换器的脚本重写及封包代码，不修改运行时。共输出 159 个脚本（151 个使用补丁）、111 张图片，包含新增的 `message_log1.pb3`。保留补丁脚本行为，`♪` / `・` 因 GBK 限制分别替换为 `☆` / `·`，其余未译日文保留。

已通过 9 项合成测试、严格 Clippy、全部输出脚本和封包校验；111 张图片的完整 RGBA 与补丁 PNG 一致。真实运行确认中文标题、10 个不同中文消息等待及首次剧情存读档后继续播放。补丁本身为测试版；完整路线、原生音频和原版存档互操作尚未验证。

```bash
python3 -m unittest discover -s tools/fan_patch/hapymaher -p test_convert_hapymaher_patch.py
cargo test --offline --locked --manifest-path tools/fan_patch/hapymaher/Cargo.toml \
  --target-dir target/fan-patch-hapymaher
```
