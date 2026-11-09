# 《明日の君と逢うために》汉化补丁转换

支持 [CK-GAL V1.0 补丁](https://vndb.org/r20550)。依赖、输出目录要求和编码规则见[工具总览](../README.md)。

## 转换与启动

从仓库根目录执行：

```bash
python3 tools/fan_patch/asukimi/convert_asukimi_patch.py \
  "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>"

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20071117 --encoding gbk --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

输出仅支持 GBK，转换命令没有 `--encoding` 参数；原因见[编码说明](../README.md#选择编码)。

## 转换内容

保留补丁的 GBK 文本，修正 `start.ps2` 长度字段，重建脚本包校验值，将 28 张 BMP 转为 PB3B。共处理 126 个包内脚本及启动脚本。不安装补丁配置或 `save125.dat`；仅沿用原版 `save/system.dat` 模板，补丁说明保存为 `patch-readme.txt`。

## 验证

已有 GBK 运行记录覆盖中文标题、8 个不同消息等待，以及首次剧情存读档后继续播放。补丁说明中的朝日线 7 月 2 日崩溃尚未修复。完整路线、原生音频和原版存档互操作尚未验证。

```bash
python3 -m unittest discover -s tools/fan_patch/asukimi -p 'test_*.py'
```
