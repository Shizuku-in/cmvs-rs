# 《アオイトリ》 汉化补丁转换

支持《アオイトリ》 [天之圣杯汉化组 20181111 补丁](https://vndb.org/r57712)，输入为一个 ZIP。通用约定见[上级说明](../README.md)，每套输出约需 4.2 GiB。

## 转换与启动

```sh
python3 tools/fan_patch/aoitori/convert_aoitori_patch.py \
  /path/to/AOITORI /path/to/patch.zip local/aoitori/chs-game-utf8

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  local/aoitori/chs-game-utf8 --profile cmvs-20171114 --encoding utf-8 \
  --audio native --font /path/to/cjk-font.ttc \
  --user-data-dir local/aoitori/chs-user-utf8
```

默认 UTF-8，也接受 `--encoding utf8`。GBK 转换使用 `--encoding gbk`，另选输出目录，启动时同样指定 `--encoding gbk`；其中 `♪`、`・` 分别替换为 `☆`、`·`。

20240709 Steam 补丁的图片尺寸与当前原版不匹配，转换器明确拒绝；尚未验证对应 Steam 原版。

## 转换内容

- 解析 `Longinus.dat` 的文件名哈希索引，转换 84 个 GBK 补丁脚本，恢复补丁借位音符并重定位字符串引用；补全 24 份脚本省略的末尾空字符串。`intcode.ps3` 保留日文。
- 保留原版包内、包外启动流程并替换中文标题；转换 52 张原始 BGRA 图片为 PB3B 无损类型 1，保留原图尺寸和位深。
- 重建 `script.cpz`、`bg.cpz`、`chip.cpz`、`event.cpz` 为原版 CPZ7，保留目录、密钥和加密方式；其他包直接复制。

不安装补丁 exe/DLL、配置或剧情存档。`conversion.json` 记录编码、输入及输出校验值、图片来源和转换报告。

## 验证

合成测试覆盖封包边界、重复哈希、末尾空字符串、文件名哈希、借位音符及 GBK 字符串引用重定位。实际补丁已完成 UTF-8/GBK 转换；87 份脚本来源完成转换，86 个脚本的静态结构检查通过，52 张图片逐像素回读一致，修改的 CPZ7 成员逐项回读通过。两种编码的桌面检查均已确认中文标题菜单及原生音频初始化。完整路线及原版存档互操作未验证。

```sh
python3 -m unittest discover -s tools/fan_patch/aoitori -p 'test_*.py'
cargo test --offline --locked --manifest-path tools/fan_patch/aoitori/Cargo.toml \
  --target-dir target/fan-patch-aoitori
```
