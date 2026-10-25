# 研究资料与来源

## xmoezzz/CMVS-Engine

- 地址：https://github.com/xmoezzz/CMVS-Engine
- 固定提交：`a6bc9501d82264e228943f894af1b969ab98bee5`
- MIT；许可副本保留在 `THIRD_PARTY_NOTICES.txt`。
- 当前参考 `Ps2TextDumper/Ps2TextDumper/Ps2TextDumper.cpp` 的字节变换与 LZSS、
  `Ps3TextDumper/Ps3TextDumper/Ps3TextDumper.cpp` 的头部布局。
- Rust 模块基于这些格式资料及当前二进制核对结果实现，增加边界检查和合成测试。
- 当前另参考 `CPZV6Unpacker/CPZV6/CPZV6.cpp` 的 `CPZHEADER` 和头部 XOR 字段，
  实现 CPZ6 元数据读取；没有直接运行该解包器或复制游戏素材。
- CPZ6 索引、数据解码与 PB3 仍待研究；该仓库不是完整运行引擎。

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

## 本作证据

IDA 地址、文件哈希和实际指令差异见 `research.md`。当前样本是 PC 版，未分析 Android 包。
新增参考需记录 URL、提交、文件、用途、许可和验证方式。
其他视觉小说运行器可以提供架构思路，不意味着它们支持 CMVS 指令。
