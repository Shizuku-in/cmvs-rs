# cmvs-rs

以 **Rust** 研究并重实现 CMVS，目标是让 CMVS 获得全平台支持。从《アマツツミ》开始，逐步扩展到其他 CMVS 版本与作品。

Windows、macOS、Linux、Android、iOS 是主要平台目标；当前仅 Windows 研究工具经过实测。

## Rust 核心与命令行

已实现 `cmvs_rs` 库和 `cmvs-inspect` 命令行：PS2A 解析、字节解码、LZSS 解压、标签及分段读取。
无第三方依赖，禁止 unsafe；文本保留原始字节，CLI 以十六进制显示，避免误判 Shift-JIS/GBK 编码。

在项目目录中，使用支持 Rust 2024 edition 的 stable 工具链：

```powershell
cargo run --offline -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3"
cargo run --offline -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --payload ../../work/start-rust-new.bin
cargo run --offline --bin cmvs-disasm -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3"
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --steps 10000
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap
cargo run --offline --bin cmvs-cpz -- "E:/Download/AMATSUTSUMI/data/pack/script.cpz"
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --check
```

`--payload` 仅允许创建新文件，输出不包含原始头部。53 项 Rust 测试和严格 Clippy 检查通过。
真实启动脚本的 Rust 解码结果与 Python 参考工具逐字节一致：5141 字节，SHA-256 为
`ad58f1bb6ffe8e9e93bc291e18df39c0807cdd453d3d39b267dcb9ff9aa84474`。

源代码在 `src/`，Rust 测试在 `tests/*.rs`；Python 工具保留为研究对照。
架构见 [architecture](docs/architecture.md)，参考来源见 [references](docs/references.md)。
当前仅建立本地 Git 仓库，未配置远端或推送。

新增 `bytecode` 模块和 `cmvs-disasm`：结构化解码嵌套表达式、控制流和系统指令，
并验证标签与直接跳转目标落在指令边界上。本作启动代码完整解析为 330 条顶层指令、30 种系统命令。
`evaluate` 支持整数/位变量、赋值和复合赋值，`machine` 提供部分单步 VM；未实现系统命令停下等待宿主。
`cmvs-trace` 默认以未知变量状态启动，有执行步数上限；`--entry 0x02a2` 可独立研究赋值段，不能当作完整启动。
该真实赋值段已执行 153 条指令后停在 0x2128；严格模式正常入口停在首个未处理系统命令 0x201e。
启用 `--bootstrap` 后，从正常入口处理 25 次已验证的启动系统调用，注册 16 条资源路径，停在 0x22b3。
此模式只应用配置、变量初始化和路径注册，不打开存档、不生成窗口、不忽略未实现命令。
`cmvs-cpz` 每包只读取 64 字节头部，已检查本作全部 13 个 CPZ6 包的布局；尚未解密索引或验证校验和。
参数字节栈与返回标志见 `vm` 模块。
详细证据见 [VM 语义](docs/vm-semantics.md)。CLI 的线性解码不跟随分支，尚不能运行游戏。

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
3. 用 Rust 建立平台无关的 VM 和资源核心，先在桌面实现文字、背景、立绘、语音与选择分支。
4. 加入图形、音频、字体、输入、存档的后端，再接 Windows/macOS/Linux/Android/iOS。
5. 使用同一段剧情对照原程序验证；iOS 编译、签名和设备测试仍需要对应环境。

采用哪种图形/窗口库尚未定案，优先保证 VM 和资源逻辑不绑定平台。
现在的 Python 工具用于验证格式，不是未来手机端的运行核心。

## 参考与许可

- xmoezzz/CMVS-Engine：MIT，提交 `a6bc9501d82264e228943f894af1b969ab98bee5`；解码算法参考，许可附在 `THIRD_PARTY_NOTICES.txt`。
- ZQF-ReVN/RxCMVS：提交 `584f9f58e994f3648f94d0e58ca35da1a916e968`；作为格式/命令研究资料，未将其源码整合入工程。
- 当前游戏的 IDA 静态分析用于核对实际行为；参考仓库中其他游戏的地址不能直接套用。

工程不包含游戏素材、解包后的剧本或可执行文件。运行检查工具时自行指定本地数据路径。
