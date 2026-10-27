> [!WARNING]
> 该项目处于开发中

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
cargo run --offline --bin cmvs-cmv -- "E:/Download/AMATSUTSUMI/data/video/op.cmv"
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap --save-dir "E:/Download/AMATSUTSUMI/save" --system-save "E:/Download/AMATSUTSUMI/save/system.dat" --script-archive "E:/Download/AMATSUTSUMI/data/pack/script.cpz"
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap --save-dir "E:/Download/AMATSUTSUMI/save" --system-save "E:/Download/AMATSUTSUMI/save/system.dat" --script-archive "E:/Download/AMATSUTSUMI/data/pack/script.cpz" --record-platform "E:/Download/AMATSUTSUMI"
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --bootstrap --save-dir "E:/Download/AMATSUTSUMI/save" --system-save "E:/Download/AMATSUTSUMI/save/system.dat" --script-archive "E:/Download/AMATSUTSUMI/data/pack/script.cpz" --record-platform "E:/Download/AMATSUTSUMI" --continue-without-snapshot
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/script.cpz" --script main.ps3
cargo run --offline --bin cmvs-trace -- "E:/Download/AMATSUTSUMI/data/pack/start.ps3" --entry 0x1214 --script-archive "E:/Download/AMATSUTSUMI/data/pack/script.cpz"
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --check
```

`--payload` 仅允许创建新文件，输出不包含原始头部。当前默认 361 项测试、全 workspace 全功能 378 项测试及严格 Clippy 检查通过。
真实启动脚本的 Rust 解码结果与 Python 参考工具逐字节一致：5141 字节，SHA-256 为
`ad58f1bb6ffe8e9e93bc291e18df39c0807cdd453d3d39b267dcb9ff9aa84474`。

项目已拆为 workspace：`crates/cmvs-core` 为无依赖核心，`crates/cmvs-audio` 为音频后端，`crates/cmvs-desktop` 为桌面后端与运行入口。根 `src/` 保留检查工具和 `cmvs_rs` 兼容重导出，现有命令与 feature 名不变。Rust 集成测试仍在 `tests/*.rs`；Python 工具保留为研究对照。结构、依赖边界与全 workspace 检查命令见 [workspace](docs/workspace.md)。
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
默认停于快照断点；显式继续后现可呈现脚本驱动的静态 logo（见下文）。尚无剧情文字、真实音频或输入指令绑定。
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
这是独立的显式素材预览；脚本驱动的静态场景现已另行实现，主快照断点默认保留。背景样本使用的 type 3/JBP 暂不支持。
`cmvs-pb3 <文件> --rgba <新输出>` 可导出顶向下 RGBA，拒绝覆盖；详情见 [pb3-format](docs/pb3-format.md)。
`cmvs-cpz` 默认读取 64 字节头部；`--list` 解密索引，`--script` 读取并解析 PS2A，
`--extract` 将指定 ASCII 文件名的内容写入用户指定的新文件。支持当前 CPZ6 加密模式 1；`--list` 列出存储的资源校验值，CLI 读取不验证，0x2030 的图像读取会验证。
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
再经一段整数赋值和新增的浮点表达式、帧局部变量循环，以 0x22cd 关闭弹出菜单后经 0x2080 切换到 logo.ps3；
窗口标题变为「アマツツミ　- ブランドロゴ -」。新增 `random`（MSVC CRT rand()，显式种子 `--rand-seed`）处理 0x20d4，
随机选择品牌语音；种子 0 选中 `sys301.ogg`。新增 `audio`：0x20b0 的效果音通道模型与 `AudioBackend`，
默认后端只记录请求、不发声。0x2020/0x2021 建立场景对象槽（后续已增加图像、子对象和绘制状态），
0x20f8（strlen）后，新增 `image`：0x2030 按 0x2011 注册顺序只读查找图像（CPZ6 资源校验和按原版验证），
把 `logo.pb3` 解码为 1280×720 图像挂到对象槽 10。共执行 894 步在 PC=0x32536 停于 0x2040。
这一步最初只完成图像加载；后续静态场景进展见下文。
表达式求值改用撤销日志实现事务后，到当时停点（504 步）的 trace 在 debug 构建下由 3682 ms 降至约 300 ms，输出逐行不变。
格式与证据见 [snapshot-format](docs/snapshot-format.md)。

新增静态场景属性：0x2040/0x2050 绘制状态、0x2022 子对象、0x2038 子图像，及 0x2044–0x2048、0x204b 属性命令。
`scene_render` 支持顶层图像的裁切、原点、位置、层次和普通 alpha 合成；子对象呈现、缩放/旋转和其他混合模式明确报错。
下面从正常入口由脚本加载并显示品牌 logo，无需 `--preview-image`：

```powershell
cargo run --offline --features desktop --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot
```

Windows 实测显示完整 Purple Software logo，截图与 Esc 退出已验证；第 944 次 step 在 logo.ps3 / PC=0x364ea 停于 0x2220。
当前视口固定为本作研究用 1280×720，窗口按比例适配；这只是静态场景，不是完整开场动画。
快照仍不可用，音频仍只记录，原游戏只读。该静态阶段之后已继续实现纯色淡入/淡出（见下文）。

新增 `fade`：0x2220–0x2223 全屏纯色覆盖层及 0x2224 的无输入等待分支，按显式帧增量更新透明度。
桌面上述命令现可完成 logo 的 2000ms 黑色淡入，执行准备声音的启动请求和计时停留，再经 2000ms 黑色淡出、停止效果音通道并销毁全部对象，切换到 menu.ps3，完成宽栈预留并把窗口标题设为「アマツツミ　- タイトルメニュー -」，载入标题图集 title01_chip.pb3、建立按钮组并经 0x2215 显示按钮子对象，记录 BGM 0x20a4 后完成 1500ms 标题淡入，显示完整标题画面并进入按钮空闲循环；无输入约 30 秒后淡出 BGM 与画面、停止 BGM（与原版空闲 30 秒进入 OP 一致），清理影片槽、按钮组与全部对象，重置动作序列与镜头后切换到 op.ps3（标题「- オープニング -」），停于 0x36278 / 0x20c0（播放 op.cmv）；帧率受当前软件绘制耗时影响。
trace 在完整选项后加 `--frame-ms 16` 会在标题空闲循环中耗尽默认 10000 步预算，再加 `--steps 1000000` 可看到无输入时标题与 OP 的循环（第 42990 步 0x20c0 播放 op.cmv，第 141423 步播完，第 143584 步回到 menu.ps3），直至步数耗尽；未提供时冻结时间并在等待处停止，不用指令次数模拟时间。
活动等待的输入跳过、真实音频、其他转场与完整开场仍未实现；0x20b1 已接入通道与后端契约，默认仍只记录、不发声。已新增十槽 VM 计时器 0x200b/0x200c：在 VM 运行入口按显式帧增量累加，普通指令和协作预算切片不增加计时。logo 的 3500ms 查询/让出循环已执行（16ms 下读到 3504ms 后退出）。0x20b3 已核对为同步的立即停止并释放（不受 SE_USE 限制），清空通道名称与启动标志；记录后端仍不发声。0x202f 销毁全部 256 个对象槽、不影响覆盖层。0x0440 的宽预留只写首个字，其余字节记为未知、读取即报错；整数帧数组 0x109 已支持。0x22eb/0x22e5 读写消息跳过（+1520）与自动送页（+1552）标志，构造均为 0。按钮组 0x2210–0x2215 与布局模式 0x2087 已实现，软件呈现器支持继承父图集/位置/不透明度的子对象；按钮输入尚未接入。BGM 0x20a0/0x20a4 已接入通道状态与 `AudioBackend::play_bgm` 契约，默认只记录、不发声。0x2224 可跳过分支已按 EFFECT_USE、跳过标志与动作输入实现，宿主暂以显式“无输入”运行。`cmvs-desktop --save-frame <新文件.png>` 在退出时导出渲染帧，用于核对画面。按钮交互 0x2216/0x2217（悬停、按下、选择、取消、键盘导航）已按动作输入实现，宿主仍为无输入。BGM 淡出 0x20a2 与停止 0x20a1 已接入后端契约。影片槽销毁 0x230a 与动作序列重置 0x2280 已建模；影片槽创建命令尚未实现；全屏 OP 使用下述 Rust JBPD 解码，Ogg 音频后端待接入。画面镜头 0x2058–0x2068/0x209c 已按原始字保存（不影响 2D 渲染）。OP 影片由 0x20c0 播放，CMV6 内为 JBPD 帧与 Ogg 音频。CMV6 容器（`cmv`、`cmvs-cmv`）与计时播放 0x20c0–0x20c2、输入查询 0x21a0–0x21a3、跳过查询 0x22ef 已实现：OP 按 24fps 计时走完 2907 帧后回到标题；桌面现已接入 JBPD 帧解码并显示 OP，trace 保留计时研究模式；影片音轨仍只记录。JBPD 真实样本 11 帧与独立 Python 逐字节一致，三部影片全 6501 帧解码通过。使用 --release 进行影片预览；格式与限制见 [jbpd-format](docs/jbpd-format.md)。后续原生音频与时钟进展见下文。

新增可选原生 Ogg 音频后端：SE 准备/启动/停止、BGM 两段循环与淡变、OP 音轨和基于音频源位置的影片选帧。默认仍为记录模式；真实播放使用：

```powershell
cargo run --offline --release --features desktop,audio-output --bin cmvs-desktop -- "E:/Download/AMATSUTSUMI" --encoding shift-jis --continue-without-snapshot --audio native
```

Windows 175 秒检查完成有音频输出的启动、标题、OP 与返回标题；四份真实音轨的完整 PCM 样本数与 Python/FFmpeg 一致，浮点最大绝对误差低于 6.6e-7。音频源时钟尚未校准设备缓冲延迟；默认核心仍无依赖，原游戏只读。支持范围和证据见 [audio-output](docs/audio-output.md)。现已接入真实键鼠动作输入，见下文。

新增真实动作输入：桌面默认只读加载根目录 `key.cfg`，鼠标可操作标题，Enter 可跳过 OP；`--input record` 保留此前无输入研究模式。开始游戏已推进到 `intproc.ps3` 表达式 PC `0x4201e`，停于内部操作数 `0x0106`（`0x42020`），SP=124。默认 279 项、全 workspace 全功能 293 项测试与严格 Clippy 通过。输入证据、限制与验证见 [input](docs/input.md)。活动游戏中的 Esc 交给脚本，关闭按钮仍可退出。

后续已实现标量 `0x0104/0x0106` 的脚本数据段整数读写、按脚本槽隔离及表达式失败回滚；真实鼠标和键盘重放均推进到 `intproc.ps3` / PC `0x42306` 的未处理系统命令 `0x2141`，SP=128。证据见 [VM 语义](docs/vm-semantics.md)。本轮未新增窗口实测，尚无剧情画面。

新增 `message`：消息窗口销毁 0x2141（8 个窗口槽与 12 个文字对象，空槽无操作，越界/负槽号显式报错）。窗口创建尚未实现，因此槽目前恒为空。真实鼠标与键盘重放越过 0x2141（槽 7），停于 `intproc.ps3` / PC `0x42368` 的未处理系统命令 `0x215d`（以窗口槽 7、文字对象 10 创建消息窗口），SP=156。默认 282 项、全 workspace 全功能 296 项测试与严格 Clippy 通过；未新增窗口实测。

随后实现消息窗口创建 0x215d：保存窗口槽、文字对象号、位置、尺寸与逻辑尺寸，按需登记字体对象；不创建 GDI 字体、字形纹理或绘制。真实重放以窗口槽 7、文字对象 10、1280×54 / 逻辑 1280×720 创建窗口后，停于 `intproc.ps3` / PC `0x42382` 的未处理系统命令 `0x215b`，SP=132；其后是同一窗口的一串属性命令。默认 285 项、全 workspace 全功能 299 项测试与严格 Clippy 通过。

随后实现窗口属性 0x215b、0x2142–0x2149（层次、字号、间距、区域、颜色、每字延迟、逐字出现、装饰样式）与窗口自身场景对象 0x2170/0x2178/0x217f；0x2170 与 0x2030 共用图像读取。真实重放为窗口 7 载入 `iconwindow.pb3` 并完成文字设置，停于 `intproc.ps3` / PC `0x42502` 的未处理系统命令 `0x2180`，SP=132。默认 292 项、全 workspace 全功能 306 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现窗口对象的显示与绘制属性 0x217a、0x2180–0x2185（与 0x2040–0x2047 共用绘制字段），以及 32 条窗口按钮记录 0x2190–0x2192（状态、命中矩形、启用）。真实重放为窗口 7 建立按钮 0–4，停于 `intproc.ps3` 表达式 PC `0x44e7c` 的内部操作数 `0x0110`（`0x44e7e`），SP=172，属于求值器断点。默认 297 项、全 workspace 全功能 311 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现脚本数据数组操作数 `0x0110`：原版求值器把它改写为标量 `0x0106`，偏移为 base + 4 × 索引。真实重放为窗口 7 建立全部 14 个按钮后，停于 `intproc.ps3` / PC `0x43086` 的未处理系统命令 `0x2150`，SP=140。默认 300 项、全 workspace 全功能 314 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现窗口移动 0x2150、文字对象的注音设置 0x2114–0x2117、换行规则 0x211e（可挂行末字符、不可拆开的重复字符、最大溢出数）与窗口对象逻辑尺寸 0x217c。真实重放离开 intproc.ps3，回到 menu.ps3 创建底部 1280×180 的窗口 0（文字对象 7，`message01.pb3`），停于 `menu.ps3` / PC `0x15382` 的未处理系统命令 `0x21cf`，SP=96。默认 305 项、全 workspace 全功能 319 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现窗口对象帧动画 0x21cf–0x21d1 与每帧推进（循环、结束、-2 销毁）、等待光标模式 0x2151 与不透明度 0x2186。真实重放为窗口 0 建立 36 帧与 2 帧两组等待光标动画并开始播放，停于 `menu.ps3` / PC `0x12c8c` 的未处理系统命令 `0x215a`，SP=92。默认 313 项、全 workspace 全功能 327 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现窗口坐标 0x215a、清空文字 0x214a、存档信息 0x22b9 与场景对象查询/移动 0x2027/0x2028。真实重放经 `cmovie.ps3` 进入第一个剧本 `sn1001.ps3`，在公共脚本 `intcode.ps3` / PC `0x1a5a2` 以 0x2030 载入背景 `white.pb3` 时停止：它与 bg.cpz 的抽查背景都是 PB3 type 3，尚未实现。默认 317 项、全 workspace 全功能 331 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现 PB3 type 3（JBP1：共用 JBPD 熵解码、本作标量整数 IDCT、768 字节截断表、16 对齐裁剪与 alpha）。全部资源包的 188 个 type 3 均可解码，13 个真实样本的 RGBA 与仓库外独立 Python 逐字节一致，导出 PNG 目视正常；JBPD 回归哈希不变。真实重放载入 `white.pb3` 后停于 `sn1001.ps3` / PC `0x2de9c` 的未处理系统命令 `0x2033`（对象图像信息查询），SP=40。默认 321 项、全 workspace 全功能 335 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现对象图像信息 0x2033、EFFECT_USE 读写 0x22d1、对象不透明度轨道 0x21ea–0x21ee 与其每帧插值，并接入 256 槽对象的帧更新（含 -2 结束后销毁）。真实重放完成剧本开头的白底淡入等待，停于 `sn1001.ps3` / PC `0xc58c` 的未处理系统命令 `0x20ba`（效果音），SP=40。默认 327 项、全 workspace 全功能 341 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现淡入效果音 0x20ba（原生后端支持效果音淡入）、消息窗口显示/隐藏 0x214e、窗口存在查询 0x214b、临时隐藏 0x214c 与指针命中测试 0x215f。真实重放推进到第 38782 次 tick，停于 `sn1001.ps3` / PC `0xe2ba` 的动作序列查询 `0x2285`，SP=156。默认 331 项、全 workspace 全功能 345 项测试与严格 Clippy 通过；尚无文字绘制。

随后实现动作序列条目数查询 0x2285：返回头部 count，不消费参数、不推进序列。真实鼠标与键盘重放均越过旧断点，鼠标重放第 38793 次 tick 停于 `sn1001.ps3` / PC `0xe39c` 的 `0x22b2`（生成引擎快照），SP=156。默认 332 项、全 workspace 全功能 346 项测试与严格 Clippy 通过；尚无文字绘制，未新增原生窗口实测。

随后实现剧情快照 0x22b2 的显式不建镜像路径：清零位变量 0 和重复计数，记录下一条 PC，存档仍不可用；默认不跳过。鼠标与键盘真实重放停于 `sn1001.ps3 / 0xf062 / 0x20ec`，鼠标第 38854 次 tick、SP=164。默认 334 项、全功能 348 项测试与严格 Clippy 通过，未新增窗口实测或文字绘制。详见 [snapshot-format](docs/snapshot-format.md)。

随后实现消息窗口统一缩放 0x20ec，保存窗口与文字缩放状态，尚不呈现文字。两条真实输入重放停于 `sn1001.ps3 / 0xf082 / 0x2155`（鼠标第 38859 次 tick，SP=164）。默认 336 项、全功能 350 项测试与严格 Clippy 通过；未新增窗口实测。

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

消息窗口命令 0x2155 已实现：把 SP-8 的原始 32 位值写入窗口关联文字对象 +128（glyph_parameter），回收 8 字节参数。真实鼠标与键盘重放均停于 sn1001.ps3 / PC=0x116f8 / 0x214f、SP=228（鼠标第 38922 次 tick，键盘第 38962 次 tick）。当前仍无文字绘制，本轮未新增原生窗口实测；完整快照仍不可用。详见 vm-semantics.md。

消息窗口文字光标定位 0x214f 已实现：SP-8/SP-12 原样设置关联文字对象的 cursor，不改区域或窗口位置。真实鼠标与键盘重放均停于 sn1001.ps3 / PC=0x11738 / 0x211f、SP=224（鼠标第 38928 次 tick，键盘第 38968 次 tick）。尚无文字绘制或本轮原生窗口实测，完整快照仍不可用。详见 vm-semantics.md。

文字排版计数查询 0x211f 已实现：按原始字节、控制串、注音与禁则换行计算结果，恢复文字光标，写系统结果 0/1/2/4。真实鼠标与键盘重放均停于 sn1001.ps3 / PC=0xf2bc / 0x2152、SP=168（鼠标第 40528 次 tick，键盘第 40568 次 tick）。查询不生成字形，尚无文字绘制或本轮原生窗口实测；完整快照仍不可用。详见 vm-semantics.md。

新增显式消息记录模式：Startup::set_record_text(true) / 桌面 --record-text 只处理 0x2152 的无换行、无控制串/注音组分支，记录字形描述与逐字 alpha 计时，不生成字体像素。默认仍停于 sn1001.ps3 / 0xf2bc / 0x2152。显式记录模式下，鼠标/键盘重放均到 sn1001.ps3 / PC=0xf6f2 / 0x2136、SP=196（鼠标第 40579 次 tick、键盘第 40619 次 tick）。尚无剧情文字绘制，本轮无原生窗口实测。详见 vm-semantics.md。

新增可选 OpenType 字体预览后端：桌面 --font <文件> [--font-face <索引>] 显式只读加载字体，以 Rust 光栅器生成首句字形并与消息窗口/场景/淡出共用图层队列。无窗口真实重放已导出含消息窗口与首句文字的 1280×720 帧，仍停于 sn1001.ps3 / 0xf6f2 / 0x2136。另完成 16 秒 Windows 原生窗口启动至标题及帧导出；尚未在原生窗口实测开始游戏后的文字。字体像素与原版 GDI 不逐像素一致，完整剧情、自动换行/注音输出与存档仍未完成。见 font-output.md。

字体预览命令与限制见 [font-output](docs/font-output.md)。

新增平台无关 history 模块处理 0x2136：保存四段原始字符串与六个 u32，最新记录在前，默认 128 项、满容量淘汰最旧项。显式文字记录/字体路径的鼠标和键盘重放均推进到 sn1001.ps3 / PC=0xf6f4 / 0x2281、SP=156（鼠标第 40580 次 tick、键盘第 40620 次 tick）。历史 UI、存档序列化和动作序列条目仍未实现；本轮未新增窗口实测或字形呈现能力。详见 vm-semantics.md。

## 后续实现顺序

1. 核对动作序列条目命令 `0x2281`（sn1001.ps3 / 0xf6f4，处理函数 0x14007f830），继续消息路径；0x2136 历史记录已支持。显式 `--font` 可合成首句文字与消息窗口，默认无字体仍保留 `0x2152` 断点，`--record-text` 仍只记录。
2. 沿该路径补齐换行、注音、其他文字样式、立绘与语音，并核对原生窗口中的剧情显示。
3. 补齐选择分支、必要转场、回调调度与完整存档镜像，并对照原程序验证同一段剧情。
4. 验证 Windows/macOS/Linux 构建和行为，再实现 Android/iOS 后端与真机检查。
5. 扩展引擎版本与作品兼容矩阵。iOS 编译、签名和设备测试仍需对应环境。

平台无关核心优先，窗口、音频与物理输入留在后端；Python 工具仅作研究对照。

## 参考与许可

- xmoezzz/CMVS-Engine：MIT，提交 `a6bc9501d82264e228943f894af1b969ab98bee5`；解码算法参考，许可附在 `THIRD_PARTY_NOTICES.txt`。
- ZQF-ReVN/RxCMVS：提交 `584f9f58e994f3648f94d0e58ca35da1a916e968`；作为格式/命令研究资料，未将其源码整合入工程。
- 当前游戏的 IDA 静态分析用于核对实际行为；参考仓库中其他游戏的地址不能直接套用。

工程不包含游戏素材、解包后的剧本或可执行文件。运行检查工具时自行指定本地数据路径。
