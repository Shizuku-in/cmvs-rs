# 研究资料与来源

## xmoezzz/CMVS-Engine

- 地址：https://github.com/xmoezzz/CMVS-Engine
- 固定提交：`a6bc9501d82264e228943f894af1b969ab98bee5`
- MIT；许可副本保留在 `THIRD_PARTY_NOTICES.txt`。
- 当前参考 `Ps2TextDumper/Ps2TextDumper/Ps2TextDumper.cpp` 的字节变换与 LZSS、
  `Ps3TextDumper/Ps3TextDumper/Ps3TextDumper.cpp` 的头部布局。
- Rust 模块基于这些格式资料及当前二进制核对结果实现，增加边界检查和合成测试。
- 当前另参考 `CPZV6Unpacker/CPZV6/CPZV6.cpp` 的头部、索引和资源变换，
  以及同目录 `cmvs_md5.cpp` 的自定义初始化值、单块处理和输出顺序，重新实现为有界 Rust 读取器。
- 使用独立 Python 实现进行真实主脚本逐字节对照；合成加密测试向量不含游戏内容。
- 原工具的 Windows 解包入口没有被运行；不将任何游戏素材或真实脚本放入仓库。
- CPZ 校验和、其他加密模式及 PB3 仍待研究；该仓库不是完整运行引擎。

## ZQF-ReVN/RxCMVS

- 地址：https://github.com/ZQF-ReVN/RxCMVS
- 固定提交：`584f9f58e994f3648f94d0e58ca35da1a916e968`
- 当前检出的根目录未发现 LICENSE；公开可读不等于源码可以再分发。
- 当前仅作为研究资料：`etc/note.md`、`etc/CMVS_Symbol.json` 和脚本处理实现。
- 未将其源码整合入工程。符号来自其他游戏，不能直接套用；旧笔记中 0x0400 的 NOP 标注已被本作反证。

## cokkeijigen/cmvs-android-amatsutsumi

- 地址：https://github.com/cokkeijigen/cmvs-android-amatsutsumi
- 此前已核实 README 描述本作 GBK 与字体 Hook，不是完整引擎重实现。
- 尚未固定提交、完成许可审查或纳入源码依赖。
- 后续用于分析 Android 版的线索，代码复用前重新核对。

## Windows 图标/光标格式说明

- Microsoft [The evolution of the ICO file format, part 2](https://devblogs.microsoft.com/oldnewthing/20101019-00/?p=12503)：DIB、双倍高度、调色板及 AND/XOR 平面语义。
- Microsoft [About Cursors](https://learn.microsoft.com/en-us/windows/win32/menurc/about-cursors)：热点、CUR、多图像选择及窗口类光标行为。
- 仅参考格式和行为描述，未复制示例源码。CUR 解码器仅实现本作当前单图像 8 位样本，使用外部 Python 和合成样本验证。

## 可选桌面依赖

- [winit 0.30.13](https://docs.rs/winit/0.30.13/winit/)，Apache-2.0：原生事件循环、窗口、图标与光标接口。
- [softbuffer 0.4.8](https://docs.rs/softbuffer/0.4.8/softbuffer/)，MIT OR Apache-2.0：桌面测试帧缓冲呈现。
- [ico 0.5.0](https://docs.rs/ico/0.5.0/ico/)，MIT：预检尺寸后的 ICO 图像解码。
- [encoding_rs 0.8.42](https://docs.rs/encoding_rs/0.8.42/encoding_rs/)，(Apache-2.0 OR MIT) AND BSD-3-Clause：显式编码的严格标题转换。
- 版本和传递依赖固定在 Cargo.lock；这些依赖只在 `desktop` 功能启用，不引入默认核心依赖。未复制库示例源码，使用公开 API 实现适配。

## 本作证据与验证约定

IDA 地址、文件哈希和实际指令差异见 `research.md`。当前样本是 PC 版，未分析 Android 包。
新增参考需记录 URL、提交、文件、用途、许可和验证方式。
其他视觉小说运行器可以提供架构思路，不意味着它们支持 CMVS 指令。
