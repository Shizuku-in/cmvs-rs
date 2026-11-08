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

CPZ6/7 的文件名/密钥扫描保持 SJIS；字体面名称和宿主 UI 编码也没有在本次改动中调整。

## 后续编码扩展

`TextEncoding` 标为 `#[non_exhaustive]`，调用方需保留未知编码分支。核心的字节扫描不依赖 `encoding_rs`；字体后端集中选择解码器，对尚未支持的编码明确报错。

当前 `is_lead_byte` 专用于单/双字节引擎扫描，不是通用字符长度接口。UTF-8 等变长编码需要新的扫描路径，处理字符边界、字节偏移和控制符，并明确 Unicode 字符到 `u16` 字形码的映射；不能仅增加枚举成员或替换字体解码器。现有 `u16` 字形与存档模型在本次改动中保留。
