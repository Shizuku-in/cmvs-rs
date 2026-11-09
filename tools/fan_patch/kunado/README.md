# 《クナド国記》 汉化补丁转换

支持《クナド国記》 [Dir-A 移植官中 1.2 补丁 (20230421)](https://vndb.org/r106394)，输入为一个 ZIP。通用约定见[上级说明](../README.md)，每套输出约需 3.8 GiB。

## 转换与启动

```sh
python3 tools/fan_patch/kunado/convert_kunado_patch.py \
  /path/to/KUNADO /path/to/patch.zip local/kunado/chs-game-utf8

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  local/kunado/chs-game-utf8 --profile cmvs-20211006 --encoding utf-8 \
  --audio native --font /path/to/cjk-font.ttc \
  --user-data-dir local/kunado/chs-user-utf8
```

默认 UTF-8，也接受 `--encoding utf8`。GBK 转换使用 `--encoding gbk`，另选输出目录，启动时同样指定 `--encoding gbk`；其中 `♪`、`・` 分别替换为 `☆`、`·`。

## 转换内容

- 转换 57 个补丁脚本，分别解码保留的日文前缀和追加的 GBK 译文，重定位字符串引用；保留补丁代码、标签表及 `sn1260.ps3` 删除重复「ため」的修正。`intcode.ps3` 保留日文。
- 补丁启动脚本依赖当前原版没有的 `update.cpz`，因此保留原版包内、包外启动流程并替换中文标题。
- 转换 48 张以 `.pb3` 命名的 PNG 为 PB3B 无损类型 1，保留原图尺寸及位深；重建 `script.cpz`、`bg.cpz`、`chip.cpz` 为原版 CPZ7，保留目录、密钥和加密方式。其他包直接复制。

不安装汉化 exe/DLL 或剧情存档；`ed_bg.pb3` 不存在于原版封包，且全部补丁脚本均未引用，故不安装。`conversion.json` 记录编码、输入及输出校验值和转换报告。

## 验证

合成测试覆盖补丁成员边界、资源筛选、混合编码、日文修正及字符串引用重定位。实际补丁已完成 UTF-8/GBK 转换；60 份脚本来源完成转换，59 个脚本的静态结构检查通过，48 张图片逐像素回读一致，修改的 CPZ7 成员逐项回读通过。两种编码的桌面检查均已确认中文标题菜单及原生音频初始化。完整路线及原版存档互操作未验证。

```sh
python3 -m unittest discover -s tools/fan_patch/kunado -p 'test_*.py'
cargo test --offline --locked --manifest-path tools/fan_patch/kunado/Cargo.toml \
  --target-dir target/fan-patch-kunado
```
