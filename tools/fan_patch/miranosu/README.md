# 《未来ノスタルジア》汉化补丁转换

支持 [CK-GAL 补丁](https://vndb.org/r24913)。依赖、输出目录要求和编码规则见[工具总览](../README.md)。约需 3.2 GiB。

## 转换与启动

从仓库根目录执行：

```bash
python3 tools/fan_patch/miranosu/convert_miranosu_patch.py \
  "<原版游戏目录>" "<汉化补丁.zip>" "<新输出目录>" --encoding utf-8

cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20110525 --encoding utf-8 --audio native \
  --font "<中文字体文件>" --user-data-dir "<独立用户目录>"
```

示例输出 UTF-8。如需 GBK，将转换和启动命令中的 `--encoding utf-8` 同时改为 `--encoding gbk`。

## 转换内容

从补丁 `mirai.bin` 提取译文，替换脚本文本、重定位字符串引用并按原 CPZ5 模式 8 写回脚本包，保留目录与密钥。共处理 115 个脚本、95,112 条字符串记录，其中也包含保留的日文和 ASCII。指令长度、跳转和数据段保持不变。

## 验证

已有 GBK 运行记录覆盖标题、10 个不同消息等待，以及首次剧情存读档后继续播放。完整路线、原生音频和原版存档互操作尚未验证。新增 UTF-8 输出的验证范围见[工具总览](../README.md#验证范围)。

```bash
python3 -m unittest discover -s tools/fan_patch/miranosu -p 'test_*.py'
cargo test --offline --locked --manifest-path tools/fan_patch/miranosu/Cargo.toml \
  --target-dir target/fan-patch-miranosu
```
