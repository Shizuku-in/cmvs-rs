# 《未来ノスタルジア》 CK-GAL 汉化转换

将《未来ノスタルジア》 [CK-GAL 补丁](https://vndb.org/r24913)中的 `mirai.bin` 转为标准 PS2A 脚本和 CPZ3 脚本包，供 cmvs-rs 使用。

## 使用

需要 Python 3.10+、本项目要求的 Rust/Cargo 和已缓存的依赖。从仓库根目录执行：

```bash
python3 tools/fan_patch/miranosu/convert_miranosu_patch.py \
  "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>"

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20110525 --encoding gbk --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

输出目录必须不存在且位于原版目录之外，约需 3.2 GiB。工具复制原版资源、程序和配置，再替换脚本；不安装补丁 exe/DLL 或补丁存档。原始输入只读，完成后生成 `conversion.json`。失败留下的目录不能直接使用，重试时另选新目录。

## 转换方式

Python 解压补丁数据，再调用 `rewrite_scripts.rs` 复用 `cmvs-core` 的 CPZ5、PS2A 和字节码解码器。Rust 程序负责替换文本、调整字符串引用并重建脚本包，是当前转换流程的一部分。它保持指令长度、跳转和数据段不变，不修改运行时。

共处理 115 个脚本、95,112 条补丁字符串记录；记录中也包含保留的日文和 ASCII。脚本哈希改变后需显式指定上述 profile 和 GBK 编码。

## 验证

已通过 7 项合成测试、严格 Clippy、全部输出脚本和归档校验。真实运行确认标题显示、10 个不同中文消息等待，以及首次剧情存读档后继续播放。完整路线、原生音频和原版存档互操作尚未验证。

```bash
python3 -m unittest discover -s tools/fan_patch/miranosu -p test_convert_miranosu_patch.py
cargo test --offline --locked --manifest-path tools/fan_patch/miranosu/Cargo.toml \
  --target-dir target/fan-patch-miranosu
```
