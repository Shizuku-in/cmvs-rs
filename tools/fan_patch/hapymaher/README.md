# 《ハピメア》汉化补丁转换

支持 [里想奈骑士团 2017-07-11 测试版补丁](https://vndb.org/r56370)。依赖、输出目录要求和编码规则见[工具总览](../README.md)。约需 3.8 GiB。

## 转换与启动

从仓库根目录执行：

```bash
python3 tools/fan_patch/hapymaher/convert_hapymaher_patch.py \
  "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>" --encoding utf-8

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20130131 --encoding utf-8 --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

示例输出 UTF-8。如需 GBK，将转换和启动命令中的 `--encoding utf-8` 同时改为 `--encoding gbk`。

## 转换内容

从补丁 `word.mai` 和 DLL 数据表还原自定义字码，按所选编码写入脚本并重定位字符串引用。共输出 159 个脚本（151 个使用补丁）和 111 张图片，包含新增的 `message_log1.pb3`；保留未译日文。封包保持原 CPZ5 模式 9；`chip.cpz` 因新增回看图按原目录重建，其余修改按原包替换写回。仅沿用原版 `save/system.dat` 模板。

## 验证

已有 GBK 运行记录覆盖中文标题、10 个不同消息等待，以及首次剧情存读档后继续播放。图片完整 RGBA 已与补丁 PNG 核对。补丁本身为测试版。完整路线、原生音频和原版存档互操作尚未验证。新增 UTF-8 输出的验证范围见[工具总览](../README.md#验证范围)。

```bash
python3 -m unittest discover -s tools/fan_patch/hapymaher -p 'test_*.py'
cargo test --offline --locked --manifest-path tools/fan_patch/hapymaher/Cargo.toml \
  --target-dir target/fan-patch-hapymaher
```
