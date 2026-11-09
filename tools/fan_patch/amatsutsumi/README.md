# 《アマツツミ》 汉化补丁转换

支持《アマツツミ》 [绿茶汉化组 2022 年补丁及包内 20221105 脚本更新](https://vndb.org/r97972)，输入为一个 ZIP。通用约定见[上级说明](../README.md)。

## 转换与启动

```sh
python3 tools/fan_patch/amatsutsumi/convert_amatsutsumi_patch.py \
  /path/to/AMATSUTSUMI /path/to/patch.zip local/amatsutsumi/chs-game-utf8

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  local/amatsutsumi/chs-game-utf8 --profile cmvs-20160201 --encoding utf-8 \
  --audio native --font /path/to/cjk-font.ttc \
  --user-data-dir local/amatsutsumi/chs-user-utf8
```

默认 UTF-8，也接受 `--encoding utf8`。GBK 转换使用 `--encoding gbk`，另选输出目录，启动时同样指定 `--encoding gbk`；其中 `♪`、`・` 分别替换为 `☆`、`·`。

原补丁是两个 RAR 分卷时，从第一卷解包全部内容，再将解包目录内的 `[绿茶汉化组]アマツツミ` 文件夹压成一个 ZIP。不要直接拼接两个 RAR 的字节。ZIP 的目录结构须保持原样；转换器校验所用成员的 SHA-256，允许重新压缩。

## 转换内容

- 合并 `Scripts.pk` 与 `Scriptspk.UPD`，重定位两种编码的字符串引用；保留未翻译的日文，恢复补丁自定义音符。
- 转换 114 张 PNG 为 PB3，重建 `chip.cpz`、`event.cpz`；复制六段默认字幕视频。
- 补丁启动脚本依赖当前原版没有的 `update.cpz`，因此保留原版启动流程并替换中文标题；其他脚本使用经校验的补丁代码和标签表。

不安装汉化 exe/DLL、内嵌字体、选装图片或其他画质视频。`conversion.json` 记录输入、编码、资源校验值与转换报告。

## 验证

合成测试覆盖 PKP 边界检查、脚本封装、混合编码、引用重定位及 RGB/RGBA 像素。实际补丁已完成 UTF-8/GBK 转换；全部脚本跨编码文本对照通过，114 张图片逐像素回读一致，六段视频的首、中、末帧可解码。UTF-8 已确认中文标题菜单，两套输出均已初始化原生音频。完整路线和原版存档互操作未验证。
