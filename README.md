# CMVS 多平台兼容实现：第一阶段

目前提供的是可运行的脚本解码研究工具，**尚不能运行游戏**，也没有生成 iOS/Android 应用。

## 已实现

- 不依赖 Windows API 的 PS2A 头部解析、字节解码与 LZSS 解压。
- 标签、代码、未知数据和文本分段；文本编码可指定为 cp932 或 gbk。
- 长度与边界检查；字符串引用模式的候选扫描（不是完整反汇编）。
- 当前 Windows x64 版本的 IDA 地址和行为记录，见 `docs/research.md`。

需要 Python 3.10+，无第三方依赖。Windows 已实际运行验证；其他系统未测试。

```powershell
python tools/ps3_inspect.py "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --output ../../work/start-inspection.json --payload ../../work/start-payload.bin
python -m unittest discover -s tests -v
```

`--payload` 保存的是不含原始头部的解压数据，不能直接放回游戏。输入文件保持只读。
如分析汉化脚本，可使用 `--encoding gbk`，但这不代表已经确认当前游戏是汉化版。

## 后续实现顺序

1. 以当前游戏二进制核对表达式求值、参数栈和系统命令；建立带未知指令即停止的启动脚本执行器。
2. 实现 CPZ6 目录读取和文件提取，验证 intproc.ps3、intcode.ps3、main.ps3 等实际内容。
3. 建立平台无关的 VM 和资源核心（计划 C++），先在桌面实现文字、背景、立绘、语音与选择分支。
4. 加入图形、音频、字体、输入、存档的后端，再接 Windows/macOS/Linux/Android/iOS。
5. 使用同一段剧情对照原程序验证；iOS 编译、签名和设备测试仍需要对应环境。

采用哪种图形/窗口库尚未定案，优先保证 VM 和资源逻辑不绑定平台。
现在的 Python 工具用于验证格式，不是未来手机端的运行核心。

## 参考与许可

- xmoezzz/CMVS-Engine：MIT，提交 `a6bc9501d82264e228943f894af1b969ab98bee5`；解码算法参考，许可附在 `THIRD_PARTY_NOTICES.txt`。
- ZQF-ReVN/RxCMVS：提交 `584f9f58e994f3648f94d0e58ca35da1a916e968`；作为格式/命令研究资料，未将其源码整合入工程。
- 当前游戏的 IDA 静态分析用于核对实际行为；参考仓库中其他游戏的地址不能直接套用。

工程不包含游戏素材、解包后的剧本或可执行文件。运行检查工具时自行指定本地数据路径。
