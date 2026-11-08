# CK-GAL 汉化离线转换试验

`tools/fan_patch/asukimi/convert_asukimi_patch.py` 将已核对的 Asukimi CK-GAL V1.0 ZIP 转为现有读取器支持的 PS2A、CPZ3、PB3B，不执行或安装汉化 exe，不给运行时添加补丁识别分支。Python 3.10+，仅使用标准库。工具仅接受下列固定补丁及[原版身份](../../../docs/games/asukimi/README.md#文件身份)，其他输入报错。

补丁 ZIP SHA-256：`9aab18642d9177dfb83a2c792dec62a0c71cc3474166ebf9091229ef64a024fc`。本地文件名含 `20070922明日の君と逢うために`、`CK-GAL中文化小组V1020101007CHS.zip`；文件名中的日期不作为实际游戏发行日的证明。

## 使用

本目录集中保存补丁转换工具、说明及 Python 转换器测试。以下命令从仓库根目录执行。

```bash
python3 tools/fan_patch/asukimi/convert_asukimi_patch.py "<原版游戏目录>" "<补丁.zip>" "<新输出目录>"

cargo run --offline --release --features desktop --bin cmvs-desktop -- \
  "<转换输出目录>" --profile cmvs-20071117 --encoding gbk \
  --font /usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc \
  --user-data-dir "<独立用户目录>"
```

字体路径按本机实际文件替换；该示例使用记录音频后端。需要声音时构建 `desktop,audio-output` 并增加 `--audio native`，本次未验证原生音频。

输出必须不存在且位于原版游戏目录之外。工具复制整个 `data/`、原版 exe 和必要配置/图标，仅以原版 `save/system.dat` 作为可选系统档模板，不复制剧情存档。完成后写 `conversion.json`，包含输入身份、输出哈希、图像尺寸/位深/预期 RGBA 哈希、编码和已知限制。失败可能留下未完成目录，需另选新输出目录重试；缺少 `conversion.json` 表示未完成。

转换脚本与归档的哈希已改变，当前原版游戏登记不能自动识别，必须显式指定已有的 `--profile cmvs-20071117`。原版 exe 用于保留引擎身份及已有哈希绑定语义，输出应通过 cmvs-rs 启动；直接运行该 Windows exe 不会获得完整汉化行为。

## 转换规则

- 外部 `start.ps2`：原始长度 956 字节，三处长度字段不自洽。修正 text_size 60 → 382、compressed_size 915 → 908、decoded_size 7130 → 3756。修正后现有 PS2A 严格解码成功，保留压缩流、代码及译文，不填补缺失内容。
- `script.cpz`：包含 126 个脚本。保留原脚本密文和索引结构，重新计算 125 项失效资源校验值及索引密文校验值。转换后标准校验和 PS2A 解码通过。未修改剧情指令，未放宽运行时的校验或长度检查。
- `image/*.bmp`：28 张 32 位 BI_RGB BMP 对应原版 `chip.cpz` 中的同名 PB3。校验原资源尺寸，将像素转成 PB3B type 1，以原资源顺序重建 CPZ3；其余资源保留原密文且验证校验值。
- 两张 `title_bg*.pb3` 原本为 24 位，补丁 BMP 的第四通道全为零，输出继续使用 24 位并补不透明 alpha。其余 26 张原本为 32 位，保留 BMP alpha。将所有 BMP 一律转成 32 位会得到不可见的标题图。
- 不安装汉化 exe、`cmvs.cfg` 或 `save125.dat`。保留补丁说明为 `patch-readme.txt`。文本保持补丁的 GBK 编码，由既有通用 GBK 能力处理。

CPZ3 重建仅写此实验需要的平面 ASCII 资源名，PB3 写入器使用字面流，未实现压缩优化。这不是通用补丁安装器或全格式打包工具。

## 验证（2026-11-09）

真实输入及输出在忽略目录 `local/asukimi/`，不提交游戏素材、译文或可执行文件。

- Python 合成回归 5 项通过，包含已有独立 CPZ3 测试向量、密文反向验证、重打包、失效校验修复、BMP 行方向、24/32 位 alpha 和错误输入不创建输出。
- 对转换结果使用现有 Rust 读取器验证 126 个脚本的 PS2A 解码；Python 严格检查全部脚本密文校验。28 张 PB3 的 Rust 完整 RGBA 输出与从 BMP 计算的预期像素逐字节哈希一致，包含两张 24 位图的 alpha 处理。
- 原生桌面启动并导出 800×600 标题画面，目视确认中文“开始新游戏 / 继续游戏 / 系统设置 / 鉴赏 / 退出游戏”。没有执行原生窗口中的剧情或存档操作。
- 无窗口完整软件绘制从标题点击开始，读取首个剧情快照的存档并继续，验证 8 个不同中文消息等待，包括“想要回去――”。104,598 步、871 帧；保存/加载后捕获快照字节保持一致。
- 默认 Rust 回归 1,082 项通过，17 项忽略。格式和带 desktop 的 all-targets 严格 Clippy 通过；本次未运行全 workspace 全功能、其他平台或完整路线。

运行转换器测试：

```bash
python3 -m unittest discover -s tools/fan_patch/asukimi -p test_convert_asukimi_patch.py
```
