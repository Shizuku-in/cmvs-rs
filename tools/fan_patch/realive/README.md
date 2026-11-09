# 《リアライブ》 汉化补丁转换

支持《リアライブ》 [天之圣杯汉化组补丁](https://vndb.org/r76601)。通用约定见[上级说明](../README.md)，每套输出约需 4.7 GiB。

## 转换与启动

```sh
python3 tools/fan_patch/realive/convert_realive_patch.py \
  /path/to/REALIVE /path/to/patch.zip local/realive/chs-game-utf8

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  local/realive/chs-game-utf8 --profile cmvs-20190305 --encoding utf-8 \
  --audio native --font /path/to/cjk-font.ttc \
  --user-data-dir local/realive/chs-user-utf8
```

默认 UTF-8，也接受 `--encoding utf8`。GBK 转换使用 `--encoding gbk`，另选输出目录，启动时同样指定 `--encoding gbk`。GBK 的字符替换如下，UTF-8 保留原字符：

| 原字符 | 转换后 |
| --- | --- |
| ♪、・、‧ | ☆、·、· |
| ≪、≫、∀、´ | 《、》、▽、′ |
| ｡、ﾉ、ﾞ、ﾟ | 。、ノ、゛、゜ |

## 转换内容

- 转换 81 份脚本来源，包括 71 个补丁剧情脚本及包内、包外的 `start.ps3`；重定位字符串引用，保留补丁代码和标签表，恢复字符表遗漏的半角颜文字。未提供译文的脚本保留日文。
- 转换 188 张 RGB/RGBA PNG 为游戏已有的 PB3B 无损类型 1，保留原图尺寸及 24/32 位深，补全省略的 `.pb3` 扩展名；四组同名图片优先使用显式 `.pb3.mai` 文件。
- 重建 `script.cpz`、`chip.cpz` 为原版 CPZ7，保留目录、密钥和加密方式；其他包直接复制。

不安装补丁 exe/DLL 或存档。`conversion.json` 记录编码、输入及输出校验值、图片来源和转换报告。

## 验证

合成测试覆盖字符分配、混合编码、半角字符、图片名称优先级、RGB/RGBA 像素及 CPZ7 加密封包。实际补丁已完成 UTF-8/GBK 转换；全部脚本文本与源补丁对照通过，188 张图片逐像素回读一致，原版 11 组封包密钥保持不变。桌面检查覆盖中文标题菜单、剧情文字及原生音频初始化，用户实测确认正常；完整路线及原版存档互操作未验证。

```sh
python3 -m unittest discover -s tools/fan_patch/realive -p 'test_*.py'
cargo test --offline --locked --manifest-path tools/fan_patch/realive/Cargo.toml \
  --target-dir target/fan-patch-realive
```
