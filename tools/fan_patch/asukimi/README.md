# 《明日の君と逢うために》 CK-GAL 汉化转换

将《明日の君と逢うために》 [CK-GAL V1.0 补丁](https://vndb.org/r20550)转为标准 PS2A、CPZ3 和 PB3B，供 cmvs-rs 使用。

## 使用

转换器需要 Python 3.10+，仅使用标准库。从仓库根目录执行：

```bash
python3 tools/fan_patch/asukimi/convert_asukimi_patch.py \
  "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>"

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20071117 --encoding gbk --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

输出目录必须不存在且位于原版目录之外。工具复制原版资源、程序和配置，再替换脚本及图片；仅使用原版 `save/system.dat` 作为可选系统档模板，不复制剧情存档。原始输入只读，完成后生成 `conversion.json`。失败留下的目录不能直接使用，重试时另选新目录。

## 转换方式

修正补丁 `start.ps2` 的长度字段，重算 `script.cpz` 的失效校验值，并将 28 张 BMP 转成 PB3B 后重建 `chip.cpz`。图片沿用原资源的 24/32 位透明度语义，保留剧情指令和译文，不修改运行时。

不安装补丁 exe、配置或 `save125.dat`，补丁说明保留为 `patch-readme.txt`。脚本哈希改变后需显式指定上述 profile 和 GBK 编码。

## 验证

已通过 5 项合成测试、126 个包内脚本解码及全部脚本校验值检查；28 张图片的完整 RGBA 与预期一致。真实运行确认中文标题、8 个不同中文消息等待，以及首次剧情存读档后继续播放。

补丁说明中的朝日线 7 月 2 日崩溃未修复；完整路线、原生音频和原版存档互操作尚未验证。

```bash
python3 -m unittest discover -s tools/fan_patch/asukimi -p test_convert_asukimi_patch.py
```
