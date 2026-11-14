# 参考与许可

- [xmoezzz/CMVS-Engine](https://github.com/xmoezzz/CMVS-Engine)，提交 `a6bc9501d82264e228943f894af1b969ab98bee5`，MIT：PS2A/LZSS、CPZ6、PB3/JBP 格式参考；未整合其 C++ 解码器。
- [ZQF-ReVN/RxCMVS](https://github.com/ZQF-ReVN/RxCMVS)，提交 `584f9f58e994f3648f94d0e58ca35da1a916e968`：符号、格式与命令研究；检出时未发现 LICENSE，未整合源码。
- [Xiph Theora I 规范](https://www.theora.org/doc/Theora.pdf)与 MIT 的 `oxideav-theora` 0.0.12：自研解码器的算法参考与固定差分数据来源；后者已移除依赖，许可声明继续保留，见[解码器说明](../crates/cmvs-video/README.md#implementation-and-validation)。
- Microsoft [ICO 格式](https://devblogs.microsoft.com/oldnewthing/20101019-00/?p=12503)与 [Cursor 文档](https://learn.microsoft.com/en-us/windows/win32/menurc/about-cursors)：图标、光标布局和系统行为；未复制示例源码。
- PNG、zlib/deflate（RFC 1950/1951）规范用于诊断图像导出；pefile、Capstone、Python/NumPy 和 FFmpeg 仅用于仓库外分析与独立验证。

依赖版本见各包 `Cargo.toml`，精确版本与校验值见 [Cargo.lock](../Cargo.lock)，第三方许可见 [THIRD_PARTY_NOTICES.txt](../THIRD_PARTY_NOTICES.txt)。依赖通过公开 API 使用，游戏地址与二进制身份归[游戏档案](games/README.md)。新增来源记录固定版本、用途、许可和复用范围，见[研究约定](research.md)。
