# 引擎层文本编码

`cmvs_core::TextEncoding` 提供 `ShiftJis`（默认）和 `Gbk`，用于表示以相应编码编译或修补过的 CMVS 引擎。

创建 VM 时使用 `Machine::new_with_encoding(&script, TextEncoding::Gbk)`；从指定入口运行可用 `Machine::at_with_encoding`。完整 runner 使用 `Startup::new_with_encoding(&script, root, &game, tick, TextEncoding::Gbk)`。现有 `new` / `at` 调用继续使用 Shift-JIS。

消息输出、测量、ruby、辅助文本、字符串比较/截取和文本对话框过滤统一调用 `TextEncoding::is_lead_byte`：SJIS 为 `81–9F | E0–FC`，GBK 为 `>= 80`。扫描器沿用引擎的字节配对行为，不校验尾字节。

字体后端也需传入相同编码：

```rust,ignore
let font = FontRenderer::read(path, face)?.with_text_encoding(TextEncoding::Gbk);
runner.set_glyph_metrics(font.metrics());
```

`FontRenderer::metrics()` 保留创建时的编码，因此先选择编码，再取得 metrics。切换 renderer 编码会清空字形缓存。字形仍以原有 `u16` 字节码记录，字体后端按选择的编码严格解码；快照格式不变。编码由创建 VM 的调用方提供，不写入存档。

`with_text_encoding` 同时设置字体库的面名称编码：游戏字体字段（LOGFONT 面名称）按所选编码解码后再按家族名查找，例如 GBK 游戏中的 `黑体`；导入字体时，家族名能以该编码放进字段才写入真实名称，否则使用 ASCII 句柄。切换编码会清空已解析的面。引擎内置默认面 `ＭＳ ゴシック` 是固定的 SJIS 字节常量，已知 GBK 补丁 exe 也未改动它，因此在任何编码下都按字节识别，名称固定为 `ＭＳ ゴシック`。

CPZ6/7 的文件名/密钥扫描保持 SJIS，宿主 UI 编码没有调整。

## 固定的 SJIS 码值

以下比较使用原版 exe 中的立即数，GBK 模式下保持原码值，不换成中文标点：

- 行尾不换行例外 `0x8141 、`、`0x8142 。`、`0x8176 」`、`0x8178 』`
- 浊点 `0x814a ゛` 的左移与步进
- proportional ruby 的居中点 `0x8145 ・`
- 窗口标题分隔符 `81 40`（全角空格）

CK-GAL 的 GBK 补丁 exe 同样保留了这些立即数，因此 GBK 下这些码值对应的是其他字符，中文标点没有不换行例外。这是与已知 GBK 修补版引擎一致的行为；如需为中文标点增加例外，属于原版之外的增强，应单独决定。脚本通过 0x211e 设置的禁则表不受影响，由脚本按自身编码提供。

## 桌面播放器测试

```bash
cargo run --release --features desktop --bin cmvs-desktop -- \
  "<GBK 游戏目录>" --encoding gbk --font "<包含中文字形的字体.ttf>" \
  --user-data-dir "<独立测试存档目录>"
```

桌面 `--encoding` 同时配置宿主文本、VM 和字体 metrics/渲染。`--encoding auto` 识别为 GBK 时走相同路径。`--record-text` 也使用选定的引擎编码，与 `--font` 互斥。UTF-8 可用于宿主编解码和自动识别，但引擎扫描尚未支持；播放器对显式或识别出的 UTF-8 明确报错。

## 后续编码扩展

`TextEncoding` 标为 `#[non_exhaustive]`，调用方需保留未知编码分支。核心的字节扫描不依赖 `encoding_rs`；字体后端集中选择解码器，对尚未支持的编码明确报错。

当前 `is_lead_byte` 专用于单/双字节引擎扫描，不是通用字符长度接口。UTF-8 等变长编码需要新的扫描路径，处理字符边界、字节偏移和控制符，并明确 Unicode 字符到 `u16` 字形码的映射；不能仅增加枚举成员或替换字体解码器。现有 `u16` 字形与存档模型在本次改动中保留。
