# 参考与许可

本页集中登记公共来源、固定提交与依赖。原参考文档中的逐阶段来源、二进制地址和本作对照说明完整保留在[《アマツツミ》来源记录](games/amatsutsumi/references.md)，不作为其他游戏的语义证据。

## 参考项目

| 来源 | 固定提交 / 许可 | 用途与复用边界 |
|---|---|---|
| [xmoezzz/CMVS-Engine](https://github.com/xmoezzz/CMVS-Engine) | `a6bc9501d82264e228943f894af1b969ab98bee5`；MIT | PS2A、LZSS、CPZ6、PB3/JBP 格式研究；许可副本见 [THIRD_PARTY_NOTICES.txt](../THIRD_PARTY_NOTICES.txt)。Rust 有界实现结合本作二进制与独立对照，不引入 C++ 运行核心 |
| [ZQF-ReVN/RxCMVS](https://github.com/ZQF-ReVN/RxCMVS) | `584f9f58e994f3648f94d0e58ca35da1a916e968`；当时检出的根目录未发现 LICENSE | 仅作符号、格式和命令研究资料，未整合其源码；其他游戏符号与旧指令注释不能直接套用 |
| [cokkeijigen/cmvs-android-amatsutsumi](https://github.com/cokkeijigen/cmvs-android-amatsutsumi) | 尚未固定提交或完成许可审查 | 已有记录将其定位为 GBK/字体 Hook 线索，未纳入源码依赖；复用前需重新核对 |

以上为既有研究记录，不表示本次重新核验了上游状态。新增来源须记录 URL、提交、文件、用途、许可与验证方法；公开可读不等于可以再分发源码。

## 后端依赖

版本集中在 [Cargo.toml](../Cargo.toml)，校验值与传递依赖固定在 [Cargo.lock](../Cargo.lock)；完整通知见 [THIRD_PARTY_NOTICES.txt](../THIRD_PARTY_NOTICES.txt)。

| 依赖 | 固定版本 | 许可 | 用途 |
|---|---|---|---|
| winit | 0.30.13 | Apache-2.0 | 窗口、事件、图标与光标 |
| softbuffer | 0.4.8 | MIT OR Apache-2.0 | 软件帧缓冲 |
| ico | 0.5.0 | MIT | ICO 解码 |
| encoding_rs | 0.8.42 | (Apache-2.0 OR MIT) AND BSD-3-Clause | 显式文本编码转换 |
| ab_glyph | 0.2.32 | Apache-2.0 | OpenType 字形光栅化 |
| rodio | 0.21.1 | MIT OR Apache-2.0 | 原生混音、播放与源位置 |
| lewton | 0.10.2 | MIT OR Apache-2.0 | Vorbis 解码 |
| cpal（传递依赖） | 0.16.0 | Apache-2.0 | 原生音频设备 |

这些依赖留在后端，默认核心无第三方依赖。未复制库示例源码，使用公开 API 实现适配；本机 Python/NumPy、pefile、capstone 等仅作为研究对照工具，不构成运行核心依赖。

## 规范与本作证据

`0x22c9` 的能力回退另参考 Microsoft [D3DCAPS9](https://learn.microsoft.com/en-us/windows/win32/api/d3d9caps/ns-d3d9caps-d3dcaps9) 与 [D3DPTFILTERCAPS](https://learn.microsoft.com/en-us/windows/win32/direct3d9/d3dptfiltercaps) 文档（2026-10-31 读取），仅用于解释二进制复制的能力结构及放大过滤标志。无新增源码复用或第三方依赖。

`0x2322` 纹理操作常量及公式另参考 Microsoft 的 [D3DTEXTUREOP](https://learn.microsoft.com/en-us/windows/win32/direct3d9/d3dtextureop) 和 [D3DTA](https://learn.microsoft.com/en-us/windows/win32/direct3d9/d3dta) 文档（2026-10-31 读取）。仅用于解释本作二进制中已确认的纹理级状态参数，未复制外部实现或新增依赖；具体地址与边界见本作 VM 语义。

图标/光标格式、音频 API、字体与 Rust 标准库的既有参考链接，以及各阶段未复制外部源码的说明，见[完整来源记录](games/amatsutsumi/references.md)。指令地址绑定[本作二进制 SHA-256](games/amatsutsumi/research.md#二进制身份)，新增游戏另建档案并保留自己的证据，流程见[研究约定](research.md)。
