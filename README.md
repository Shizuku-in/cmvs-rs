# cmvs-rs

以 **Rust** 研究并重实现 CMVS，目标是让 CMVS 获得全平台支持。从《アマツツミ》开始，逐步扩展到其他 CMVS 版本与作品。

Windows、macOS、Linux、Android、iOS 是主要平台目标；当前 Windows 研究工具与原生桌面预览经过实测。

## Rust 核心与命令行

已实现 `cmvs_rs` 库和 `cmvs-inspect` 命令行：PS2A 解析、字节解码、LZSS 解压、标签及分段读取。
默认核心无第三方依赖，项目代码禁止 unsafe；可选 `desktop` 功能使用窗口与图像依赖。
核心文本保留原始字节，CLI 以十六进制显示，避免误判 Shift-JIS/GBK 编码。

在项目目录中，使用支持 Rust 2024 edition 的 stable 工具链：

```powershell
cargo run --offline -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3"
cargo run --offline -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --payload ../../work/start-rust-new.bin
cargo run --offline --bin cmvs-disasm -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3"
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --steps 10000
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap --save-dir "E:/Download/AMATSUTSUMI/save"
cargo run --offline --bin cmvs-cpz -- "E:/Download/AMATSUTSUMI/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --list "E:/Download/AMATSUTSUMI/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --script "E:/Download/AMATSUTSUMI/data/pack/script.cpz" main.ps3
cargo run --offline --bin cmvs-save -- "E:/Download/AMATSUTSUMI/save/save000.dat"
cargo run --offline --bin cmvs-save -- --records "E:/Download/AMATSUTSUMI/save/save000.dat"
cargo run --offline --bin cmvs-system-save -- "E:/Download/AMATSUTSUMI/save/system.dat"
cargo run --offline --bin cmvs-cursor -- "E:/Download/AMATSUTSUMI/data/icon/01.cur"
cargo run --offline --bin cmvs-pb3 -- "../../work/logo.pb3"
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap --save-dir "E:/Download/AMATSUTSUMI/save" --system-save "E:/Download/AMATSUTSUMI/save/system.dat" --script-archive "E:/Download/AMATSUTSUMI/data/pack/script.cpz"
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap --save-dir "E:/Download/AMATSUTSUMI/save" --system-save "E:/Download/AMATSUTSUMI/save/system.dat" --script-archive "E:/Download/AMATSUTSUMI/data/pack/script.cpz" --record-platform "E:/Download/AMATSUTSUMI"
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap --save-dir "E:/Download/AMATSUTSUMI/save" --system-save "E:/Download/AMATSUTSUMI/save/system.dat" --script-archive "E:/Download/AMATSUTSUMI/data/pack/script.cpz" --record-platform "E:/Download/AMATSUTSUMI" --continue-without-snapshot
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/script.cpz" --script main.ps3
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --entry 0x1214 --script-archive "E:/Download/AMATSUTSUMI/data/pack/script.cpz"
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --check
```

`--payload` 仅允许创建新文件，输出不包含原始头部。默认 159 项测试、启用 desktop 后 164 项测试和严格 Clippy 检查通过。
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
新增 `save_menu`：通过显式 `--save-dir` 只读扫描 `save000.dat` 等剧情存档头，
生成槽状态、最新槽和菜单启用状态；不载入剧情、不写文件、不创建原生菜单。
本机真实目录仅有 `system.dat`，扫描得到 10 个空槽；正常启动可处理 0x22b3/0x22b4，
在 PC=0x0276 停于下一条未实现命令 0x2098。
随后用户生成的 3 份真实 CSV2 存档均通过 Rust 头部识别，最新槽正确选择为 2；完整恢复尚未实现。
现已支持 0x2098 的光标名登记与独立的整数系统结果寄存器；不加载光标图像。
`--bootstrap --save-dir <目录>` 从正常入口继续执行，共 287 次 step 后在 PC=0x1136 停于
读取系统存档的 0x2128；保留未知格式断点，没有跳过 `system.dat`。
新增 `save` 模块及 `cmvs-save`：CSV2 容器边界检查、第一段原版累加校验和、字节逆变换和三段 LZSS 解压。
三份真实存档的全部 9 个解码段与 Python 逐字节一致，原始存档未修改。
`cmvs-save --section <0|1|2> <存档> <新输出文件>` 可导出指定段，拒绝覆盖现有文件。
当前仅支持 CSV2 的已核对写入模式；格式和限制见 [save-format](docs/save-format.md)，尚不恢复 VM 或引擎状态。
新增 CSS1 `system_save` 与检查工具 `cmvs-system-save`，解码真实 `system.dat` 得到的 44864 字节与 Python 完全一致。
`--system-save <文件>` 显式处理 0x2128，只恢复经核对的持久变量区和两组文件名列表；
默认不从 `--save-dir` 推断系统存档，读取或校验失败保留 VM 状态，原存档始终只读。
启用完整示例中的选项后，正常入口执行 292 次 step，在 PC=0x1174 停于窗口标题命令 0x2160。
此时已知整数 4097、位变量 262144；窗口与画面后端尚未实现。格式见 [system-save-format](docs/system-save-format.md)。
新增 `platform` 宿主接口和显式 `--record-platform <游戏根目录>`：记录标题/图标/光标请求，
应用已核对的 0x22b0 缩略图尺寸配置（0x2350 现由 `gesture` 模块处理），不创建原生窗口、不采集或伪造输入。
**记录模式**从正常入口执行 479 次 step，完成两个公共脚本初始化和 13 个回调登记，
准备 main.ps3 后停在引擎快照断点。它验证了启动状态流，不能当作原生平台效果或完整启动成功。
新增 `cursor`、`cmvs-cursor`，读取本作单图像 8 位 CUR，保留 AND/XOR 像素与热点；
四份真实光标的 4096 字节平面结果均与 Python 逐字节一致。`--planes <新文件>` 可导出原始平面，格式不是 RGBA。
平台接口、错误策略和当前支持范围见 [platform-host](docs/platform-host.md)。
新增可选原生桌面预览，运行命令：

```powershell
cargo run --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis
```

原生窗口已实测显示日文标题、提交 32×32 图标，从正常入口执行 479 步到主快照断点并保持响应。
棋盘格是后端测试画面；尚无 CMVS 场景绘制、剧情文字、音频或输入指令绑定。
暂停后 F1 恢复默认光标、F2～F5 预览登记光标，Esc/关闭按钮退出；预览快捷键不修改 VM 状态。
用户已确认四个光标和 Esc 正常，事件日志记录了真实键盘、鼠标和焦点事件。
标题必须显式选择 `--encoding shift-jis|gbk|utf-8`；`--exit-after <秒>` 可用于有界运行检查。
桌面后端支持范围和验证方式见 [desktop](docs/desktop.md)，其他平台尚未构建验证。
新增 PB3B type 1（24/32 位）有界解码及显式素材预览，四份真实素材 RGBA 均与 Python 逐字节一致。
下面直接只读加载资源包中的 logo，无需把素材放入项目：

```powershell
cargo run --offline --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --image-archive "E:/Download/AMATSUTSUMI/data/pack/chip.cpz" --preview-image logo.pb3
```

用户已确认 logo 比例正常、无花屏并可关闭。图像按比例居中、以透明度合成到预览背景上；
这是显式素材预览，未执行 VM 绘图命令，主快照断点默认保留。背景样本使用的 type 3/JBP 暂不支持。
`cmvs-pb3 <文件> --rgba <新输出>` 可导出顶向下 RGBA，拒绝覆盖；详情见 [pb3-format](docs/pb3-format.md)。
`cmvs-cpz` 默认读取 64 字节头部；`--list` 解密索引，`--script` 读取并解析 PS2A，
`--extract` 将指定 ASCII 文件名的内容写入用户指定的新文件。支持当前 CPZ6 加密模式 1，尚未验证校验和。
本作全部 13 个包的索引已解析，`script.cpz` 列出 87 个文件。
主脚本结构解码为 26257 条指令，可直接加载进 VM，执行 8 条指令后停在 0x2089。
这次独立主脚本实验没有继承完整启动状态，也不代表完整游戏已经运行。
新增 `script_control`：公共脚本槽、执行初始化后跨脚本返回、显式回调登记/调用，以及主脚本核心切换。
`--script-archive` 显式提供公共脚本来源，使用可指定的虚拟 tick（默认 0），不自动运行回调调度器。
上面的 `--entry 0x1214` 是尾段隔离实验：执行 172 次 step，依次初始化 intproc/intcode，登记 13 个回调，
准备 main.ps3 后停在尚未实现的引擎存档快照步骤。它没有运行此前启动配置，不可宣称完整启动成功。
参数字节栈与返回标志见 `vm` 模块。
详细证据见 [VM 语义](docs/vm-semantics.md)。CLI 的线性解码不跟随分支，尚不能运行游戏。

新增 `snapshot`：核对 0x2080/0x22b2 调用的引擎快照 0x140069270。快照缓冲只供存档写入使用，
对 VM 唯一可见的实时影响是位变量 0 最终被清零。`cmvs-save --records` 遍历 CSV2 第 0 段记录帧，
三份真实存档各 45 条记录，与独立 Python 遍历器逐条一致。上面带 `--continue-without-snapshot` 的命令
只应用这一项影响、不构建存档镜像，执行 479 步后进入 main.ps3。此时存档镜像为“不可用”，存档功能不能使用。
新增 `scene`：0x22cb 设置清屏色（不透明 0xRRGGBB）、0x2250 设置深度清除标志，只保存状态、不绘制。
main.ps3 将清屏色设为 0，再经已支持的 0x22b4 调用标签 186。新增 0x0201 字符串表达式（累加器模型）和帧参数读取：
该函数把 `- メイン -` 写入字符串变量 4 和 14，再以 0x2162 设为存档标题、以 0x2161 设为场景标题，
窗口标题变为「アマツツミ　- メイン -」。随后返回 main.ps3，执行指针手势配置 0x2351–0x2364（新增 `gesture`，只保存配置、不识别输入），
再经一段整数赋值和新增的浮点表达式、帧局部变量循环，共执行 805 步后在 PC=0x3962e 停于待处理的 0x22cd。
`cmvs-desktop` 支持同名选项，Windows 实测停点相同；窗口仍显示诊断棋盘格。
表达式求值改用撤销日志实现事务后，到当时停点（504 步）的 trace 在 debug 构建下由 3682 ms 降至约 300 ms，输出逐行不变。
格式与证据见 [snapshot-format](docs/snapshot-format.md)。

目前提供研究工具和原生桌面预览，**尚不能运行游戏剧情**，也没有生成 iOS/Android 应用。

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
