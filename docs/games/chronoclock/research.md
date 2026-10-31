# CHRONOCLOCK 研究记录

身份和建档时的结构检查见[游戏档案](README.md)，逐条语义见[VM 证据](vm-semantics.md)。原游戏只读，真实日志、存档副本和探针仅在忽略的 `local/chronoclock/`，不进入 Git。未修改资源解码算法，未引入外部源码或依赖。

## 2026-10-31：对象可见性与オートデモ

在 IDA 中重新核对输入哈希、`0x2026` 的 switch、处理函数、对象 setter/getter、初始化和绘制队列调用。另用只读 PE/Capstone 核对基准 x64 对应完整函数，接入共用的 `SceneObject::shown`。未进行 P2 全表指纹继承，档案状态及默认闸门保持不变。

### trace

沿用建档命令，加 `--allow-unreviewed`：种子0、帧16ms、预算100万步、读取原目录 CSS1/剧情槽目录、RecordingBackend、`--continue-without-snapshot`。现在完成首条 `cdemo.ps3 / 0x341a2 / 0x2026`，随后在 `intproc.ps3 / 0x4021a / 0x2141` 保持挂起（日志编号43920）。trace 没有接入 MessageHost；`0x2141` 已在完整 runner 实现，这个工具停点不能当作缺失游戏指令。

### 完整 runner

仓库外素材通过 `Startup` 正常启动，自动按数据/二进制识别档案。研究参数：virtual_tick=0、随机种子0、每次 Yield 后 `advance_frame(16)`、记录文字、`ContinueWithoutImage`、显式允许未复核命令、无输入、预算100万次 tick。使用 `local/chronoclock/runner-user/` 独立用户目录，初始 CSS1 来自原 `save/system.dat` 副本；不启用本作未登记的 skip-auto-resume 策略。

沿标题空闲进入 `cdemo.ps3`，经过公共脚本消息窗口处理，执行 `0x2026` 三次：`0x341a2`、`0x342d4`、`0x342a8`。在 steps=102011、`cdemo.ps3 / 0x1ef16 / 0x21fa` 明确报错；六参数见 VM 证据。这里的 steps 属于完整 runner，不与 trace 的编号直接比较（回调与宿主调度不同）。

这是无窗口记录运行；没有原生音轨、字体/画面对照、剧情路线、选择或结局通过的结论。CHRONOCLOCK 的全局初值仍未单独复核，也未用基准初值补零。下一步应研究 `0x21fa` 的 `0x00140041` 模式，再逐步复核所经过命令；默认运行仍停在首个未复核命令。

### 输出身份

| 忽略目录内文件 | SHA-256 |
|---|---|
| `visibility-trace.log` | `3a6007d894100e6a1e5c74297555202857b595dfab4efa065912fe3018a13a23` |
| `runner.log` | `f2f1e33da12366683c4b34d832bc4041281de83c82dc6dd747e8fd61756a8825` |
| `visibility-base-disasm.log` | `3e951e8d5c916fc91667440ccd505168c900c12dde14bf7d3b420276c8019561` |

### 检查

`cargo fmt --check`、`cargo test --offline`、`cargo clippy --offline --all-targets -- -D warnings` 和 `git diff --check` 通过。默认测试合计831项通过、12项忽略；新增三项可见性测试仅用合成脚本和像素。未改后端/feature，未新增原生设备验证。

## 2026-10-31：单周期Y摆动与显示字节计数

基线 `9fb0fd2`，分支 `feat/chronoclock`。IDA仍为已登记的CHRONOCLOCK x64，重新确认输入哈希。核对 `0x21fa` 的键写入和帧更新模式65，并以基准exe独立反汇编确认高16位振幅/单周期360度Y偏移。新增该分支后，完整runner到steps=102479、`cdemo.ps3 / 0x10396 / 0x2119`。

继续核对 `0x2119` 的字符串解析、有效文字对象前置条件、字节/注音/控制码计数及返回；基准构建完整函数对照一致。实现后同一真实命令返回22，runner到steps=102522、`cdemo.ps3 / 0x10afe / 0x2112` 停止。当前停点是布局测量命令，不能按计数命令或现有 `0x211f` 直接放行。

保持前轮的种子0、virtual_tick=0、16ms帧、无输入、记录文字、ContinueWithoutImage和显式允许未复核命令；使用相同来源CSS1副本和独立用户目录。最终日志 `text-count-runner.log` SHA-256 `c33ba4691a40b372d796849c5c234309acd8bfa771348c235bc1839af8c3957d`。仅在出错前取得模式65的1个活动帧（slot20、elapsed16、duration5000、mode0x00140041、基准Y=36），Python独立f32/f64模拟与原始XYZ位一致；未执行整个轨道周期或新增原版画面对照。基准反汇编日志 `motion-base-disasm.log` SHA-256 `46f7d8e39280d47f00580648908983e71f1e34a06917c8d78690b46740b31513`。

资源解码未改，无新依赖或外部源码；档案仍为712条Unreviewed，默认闸门保留。新增测试全部为合成数据。当前工作不包含原生窗口、字体音轨输出、完整剧情或全表复核。

检查：`cargo fmt --check`、`cargo test --offline`（833项通过，12项忽略）、`cargo test --offline -p cmvs-core`（56项通过，1项忽略）、`cargo clippy --offline --all-targets -- -D warnings` 通过。新增核心单测1项、消息集成测试2项，扩展既有轨道命令参数覆盖。

## 2026-10-31：回到P2，完整指纹与默认闸门验收

先将前述语义改动提交为`8eb8708`，本阶段以其为基线，只改工具、档案与闸门。实现IDAPython导出、标准库比较、带强制证据的`Inherited`解析/验证、状态统计及闸门来源计数；具体策略和命令见[设计第13节](../../design/engine-profiles.md#13-p2-实施与使用2026-10-31)。全局/间接依赖、未知解码、取地址/索引数据和递归截断均不自动继承。

IDA 9.4分别导出两个构建的x64/x86，二进制身份来自各自档案，原exe只读。CHRONOCLOCK x64使用当前IDA会话，其他三份使用`local/chronoclock/p2-*.i64`新建分析库，未切换当前会话或改写原游戏目录。工具核对输入文件哈希并拒绝数据库补丁；同一CHRONOCLOCK x64重新导出逐字节一致。原指令图JSON只留local，仓库报告仅含指纹和分类。

| 比较 | 完整匹配 | 未复核 | NoOp | 报告 |
|---|---|---|---|---|
| x64对x64（档案采用） | 387 | 325 | 264 | [x64 TSV](profiles/cmvs-20150405-x64.tsv) |
| x86对x86（独立核对） | 362 | 350 | 264 | [x86 TSV](profiles/cmvs-20150405-x86.tsv) |

报告头保留归一化策略、深度8、两边exe和导出JSON的SHA-256；档案以`inherit x64 base-sha256=`头部行绑定根exe，继承行只记架构和完整指纹，派生exe取本档案同架构`binary`行。x64报告SHA-256 `e867b33857fda781f9f40e56cea2792ee213a79c16531766341cdd5b60905a4e`，x86报告 `fe472ddd98df13a92655e43ad82f5d6e0b4bd70a3b932245d04e246966aa865f`（LF；首次生成时工具在Windows上写出CRLF，对应哈希为`62495b45…`/`7e5c6d0e…`，内容逐行相同，工具已改为强制LF）。x86不同/截断不等于语义不同，不直接合并两份结果。

### 默认运行

采用建档trace参数（种子0、帧16ms、原CSS1只读、RecordingBackend、ContinueWithoutImage、百万步预算），**不加`--allow-unreviewed`**。`start.ps3 / 0xc / 0x201e`因完整匹配通过，在`start.ps3 / 0x1a / 0x201f`报未复核，退出码1，栈顶10与PC保留。统计为Base=0、Inherited=1、Unreviewed=0；统计包含等待重试的分派尝试，不能当作已完成指令数。未复核原因是可写全局依赖，下一步P3应人工复核它，不能直接沿用基准状态。

默认日志`p2-default.log` SHA-256 `13dfd56569b83b66f1d17242a28259aa825caddf37fb163d3c6ecd3104de372b`。先前`0x2112`是研究放行路径的历史停点，不能替代默认兼容状态。

### 基准回归与检查

从Git导出`8eb8708`到忽略目录独立构建，与P2版本同参数运行《アマツツミ》trace：virtual_tick=0、种子0、16ms、百万步、RecordingBackend、ContinueWithoutImage，输入CSS1为`local/amatsutsumi/user-data/system.dat`（SHA-256 `dc20174a630f6525d70d6c4452896c716249ef2c469adf472bf6053ac0e5ef1a`），剧情槽扫描同目录；输入不写回。两份日志完整121639651字节逐字节一致，SHA-256均为`9d8e45d8e8aa5dc14202fb460f953acad04cd5f02d1d93ad83397d17bb48ca21`；两边都在第1000000步预算耗尽，退出码1。日志在`p2-amatsu-before-saved.log`、`p2-amatsu-after-saved.log`。游戏原目录没有CSS1时两版也一致早停未知变量，该早停不计入百万步验收。

检查通过：`cargo fmt --check`、默认离线测试834项（12项忽略）、core离线测试56项（1项忽略）、默认/core严格Clippy、Python标准库合成指纹测试10项（含候选档案头部行）。未新增命令语义、依赖或原生设备验证；`Reviewed`/`Variant`及剩余差异人工复核仍属于P3。

## 2026-10-31：M1，`0x201f` 人工复核

按[分派层设计](../../design/dispatch.md) M1 引入 `Reviewed`。默认停点 `0x201f` 的指纹原因是可写全局（`unresolved-data`、`unresolved-or-mutable-data`）。CHRONOCLOCK x64 在当前 IDA 会话只读查询；基准 x64 用 IDA 9.4 `idat` 对 `local/chronoclock/p2-amatsu-x64.i64` 的临时副本批处理读取，两边输入哈希与档案一致。结论与地址表见 [VM 语义](vm-semantics.md)：处理函数相同；全局只被归档类工厂读取，取值 1、2 在基准中有专用类而本作走默认，取值 10 两边都构造 0x1C8 字节的归档类；Rust 只记录该值不解释，故登记为 `Reviewed`。

默认运行参数同上节。停点前移到 `start.ps3 / 0x50 / 0x2091`，退出码 1，统计 base=0、inherited=3、reviewed=1、unreviewed=0。日志 `local/chronoclock/m1-default.log` SHA-256 `130018b6d499460b16f7bbe16bdc619e69e6d11697d7c0906c35d129f102d5a5`；去掉统计与错误行后，P2 日志是它的逐字节前缀。

《アマツツミ》同上节参数重跑，日志 `local/chronoclock/m1-amatsu.log`（LF）SHA-256 `4803fe2eff8f8e1d48730847e4b94f69f6e6af356b05c2580808a72f48b28d90`，与 P2 的 `p2-amatsu-after-saved.log` 去掉 CR 后的哈希相同。P2 记录的 `9d8e45d8…` 是经 PowerShell 重定向写出的 CRLF 版本，内容一致。

`0x2091` 的工具原因为未建模/可写数据引用，留待下一条复核。

## 2026-10-31：指纹策略 v2（运行库叶子）

按[档案设计第 14 节](../../design/engine-profiles.md#14-指纹策略-v22026-10-31)改进导出与比较工具。先在当前 IDA 会话只读分类 v1 中被数据问题阻断的 469 个函数，再离线模拟各项修复的效果；运行库边界（上限 58 条）加段外引用（上限 78 条）收益最高且无需新的配对推理，故先做这两项。

导出均为 IDA 9.4 `idat` 批处理：基准 x64/x86 读取 `local/chronoclock/p2-amatsu-*.i64` 的临时副本；CHRONOCLOCK x64 用 `-o` 新建分析库 `local/chronoclock/v2-chrono-x64.i64`（不再借用当前会话），x86 读取 `p2-chrono-x86.i64` 的临时副本。游戏目录除当前会话自身的 `cmvs64.exe.id0` 外无文件变化。导出 JSON 的 SHA-256：

| 导出 | SHA-256 |
|---|---|
| 基准 x64 | `cf7a9adce3a229fc61fe52c093a5dc7a2b4bc6376aa532db405d9d1a739a0c48` |
| CHRONOCLOCK x64 | `bab7f8b73ac6f238cb8bc575d1b4ba6b9ee61d2e18a5d434c7f6553d2927ee21` |
| 基准 x86 | `d9dbab950e5a890c049e9a2e73843fedad98740d1a8bf9fb523539a04de53bf0` |
| CHRONOCLOCK x86 | `2a90040813148256aa259534950be37fbd1292a5fdb4168c3e1c87d762bf60a1` |

[比较报告](profiles/)替换为 v2 版本：x64 `161a628bcdb1bfcadfee4483e7a27e8eed7a40033befa1cee33aaeb6fd677933`，x86 `e27c5081ea55fcee04b87a38f46097335347dcd19e15cd81a36471409b8d43bb`。x64 继承 387 → 434，x86 362 → 415，v1 的继承无一丢失。第一次导出时有一对占位名库函数（`sub_…`、`unknown_libname_N`）被当作叶子；它们因名称不同从未匹配，但编号可能偶然相同，故改为按普通函数展开并在比较工具中拒绝，重新导出后继承数不变。

默认 trace 同上节参数，仍停在 `start.ps3 / 0x50 / 0x2091`，除统计行外与 M1 日志逐字节一致（`local/chronoclock/v2-default.log`）。Rust 代码未改，只更新本作档案数据，故未重跑《アマツツミ》百万步回归。检查：`cargo fmt --check`、默认离线测试 836 项（12 项忽略）、严格 Clippy、Python 合成测试 15 项。

## 2026-10-31：复核辅助 `--explain`

见[档案设计第 14 节](../../design/engine-profiles.md#复核辅助---explain)。带诊断字段的 x64 导出为 `local/chronoclock/v2d-*-x64.json`（基准 x64 读 `p2-amatsu-x64.i64` 临时副本，CHRONOCLOCK x64 读 `v2-chrono-x64.i64` 临时副本）；与已提交报告逐行相同。`0x2091` 的解释：处理函数 `0x7f1fa` → `0x6eb60` → `0x82330`（基准 `0x82b61` → `0x72100` → `0x85ce0`），在 `+0x32` 读取可写全局（CHRONOCLOCK `qword_140131ea0`），`+0x15` 的映像基址 `lea` 位移不同。人工复核从这两处开始。

## 2026-10-31：P3，`0x2091` 位变量初始化复核

基线 `763a74e`。沿默认闸门停点人工核对两个 x64 构建的完整处理函数、位写入函数、错误标志指针 setter 和唯一构造调用者；两边 IDA 输入哈希均与档案一致。CHRONOCLOCK 使用当前会话只读查询，基准由 IDA 9.4 批处理读取 `p2-amatsu-x64.i64` 的新副本 `review-2091-base.i64`。原游戏保持只读，分析库和输出仅留在忽略的 `local/chronoclock/`。

两边对合法范围逐位写入的语义相同，差异为数据区和代码地址；可写全局实际指向 VM `+12056` 错误标志，位区另有逐字节同步的冗余校验副本。现有 Rust 根语义保留变量值，不模拟副本失配标志；非法范围继续明确报错。证据与边界见 [VM 语义](vm-semantics.md#位变量范围初始化-0x20912026-10-31-复核)。仅将 `0x2091` 从 `Unreviewed` 改为 `Reviewed`，不改指纹算法、指纹报告或运行时代码。当前统计为 Inherited=434、Reviewed=2、Unreviewed=276、NoOp=264。

### 默认启动验收

使用 debug `cmvs-trace`，建档命令原参数：`--bootstrap`、原游戏 `save/` 和 CSS1 只读、`--script-archive`、`--record-platform`、`--continue-without-snapshot`、帧16ms、种子0、百万步预算；**不加 `--allow-unreviewed`**。第16步的 `start.ps3 / 0x50 / 0x2091` 以 start=0、count=65536、value=0 完成，回收12字节并前进至 `0x52`。第23步在 `start.ps3 / 0x76 / 0x2092` 明确报未复核，PC和12字节参数保持挂起；其参数为 start=0、count=2048、value=0。闸门统计 base=0、inherited=3、reviewed=2、unreviewed=0。

对 CR 规范化并去掉旧日志末尾统计及错误行后，`v2-default.log` 是新日志的逐字节前缀。没有新增原生窗口、字体、音轨或完整剧情验证；研究放行的 `0x2112` 停点仍为历史结果。下一步复核 `0x2092`，不能据位写入相同直接放行整数初始化。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-2091-chrono.json`（反汇编与伪代码） | `b9337987c169147fdc56ef1f8abd3026601968e6b48a0babf7eee28472843372` |
| `review-2091-base.json`（反汇编、伪代码和指针引用） | `fa22a5fe2837226d1f1c093a644f2f22b5d16164f31dea98f566d26e9f86a0da` |
| `review-2091-default.log` | `8179bc6b3882e3a0c38e7b8d394d77125fdedcbe685f2ffb386f2a0efab185be` |

检查通过：`cargo fmt --check`、`cargo test --offline`（836项通过、12项忽略，含复核文档引用及现有合成初始化测试）、`cargo clippy --offline --all-targets -- -D warnings`、`git diff --check`。未修改解码算法、后端或 feature，无新依赖及外部源码；不重复计入《アマツツミ》百万步回归，本轮只改派生档案数据与文档。

## 2026-10-31：指纹策略 v3 与 `Presumed`

见[档案设计第 15 节](../../design/engine-profiles.md#15-指纹策略-v3-与-presumed2026-10-31)。先在当前 IDA 会话只读统计：间接控制流中 switch 跳转 24 处、间接调用 247 处；switch 信息可由 IDA 完整给出。随后逐步核对导出：`lea reg, [映像基址]` 未建模、IDA 未解析的映像相对表访问（如 `0x2091` 的 `movzx ecx, byte [r8+rdx+131EB0h]`）、不返回调用后的 `int3` 填充被当作缺失后继。映像相对位移只在 IDA 已解析或同一基本块内可证明寄存器装有映像基址时置零；该证明在全部 CHRONOCLOCK x64 函数中认定 140 处，抽查均为跳转表与索引表，没有结构体偏移。

导出用 IDA 9.4 `idat` 顺序批处理（并行启动时两次出现数据库输入名读取异常，改为顺序执行后正常），数据库均为 `local/chronoclock/` 中分析库的临时副本：

| 导出 | SHA-256 |
|---|---|
| 基准 x64 | `7d1f49d573c1ef912cf05e40c12c3a6dd136c6465c596f56784f140f50a7fdae` |
| CHRONOCLOCK x64 | `5a3cefbac5ff268e28bcce9cc17baf246051b42c53548a639734d8a876e9742a` |
| 基准 x86 | `88bd9b1d2c0eee29fb339582621cf1edc1e42ddf719b2a015ccddf424bd25f59` |
| CHRONOCLOCK x86 | `cc888234a0efb9c0289c6badb131e9cc56c4ca5f9f6ac22e57fb00a9836b6665` |

[比较报告](profiles/)：x64 `d9a0afbc53cc9688741c99e46505d9a16b18e00ed2595650dc80c29b6ab34e8d`，x86 `1aee44ae1d1b51bfe537d59f419987fec7c8d41ac6135bab0482dbb4de63f8e6`。档案由工具从当前档案生成，保留 `0x201f`、`0x2091` 两条人工复核行。

默认 trace（参数同上）停在第 43 步 `start.ps3 / 0xde / 0x2013`，统计 inherited=4、reviewed=2、presumed=3（`0x2092`–`0x2094`），日志 `local/chronoclock/v3-default.log` SHA-256 `29cc6581ee28eb35c5b1a21c6cce091a5f5cb4b2d654daa5f966b54c3816cc11`。`0x2092` 的解释显示可比较部分全部相同，阻断的只有整数区、其校验副本和错误标志指针三处可写数据，与 `0x2091` 复核所见的结构一致。

`--explain` 逐条查看研究路径上剩余的未复核命令：`0x2010`、`0x2012`–`0x2014`、`0x2019` 的首处差异在归档类工厂 `0x1400273d0`，`0x2011` 在 `0x140034260`，`0x201a` 在 `0x1400362f0`，都是 `0x201f` 复核中记录的取值 1、2 差异；`0x2030`、`0x2141` 为对象字段偏移不同，`0x2080` 有多处常量不同，`0x2128` 的首处差异是一处未证明的映像相对地址。

《アマツツミ》百万步 trace 与 M1 日志逐字节一致。检查：`cargo fmt --check`、默认离线测试、严格 Clippy、Python 合成测试 21 项。

## 2026-10-31：P3，七条归档路径注册命令复核

复核 `0x2010`–`0x2014`、`0x2019`、`0x201a`，全部由 Unreviewed 登记为 Reviewed；档案现为 Inherited=455、Presumed=167、Reviewed=9、Unreviewed=81、NoOp=264。不改运行时、指纹策略、导出或自动比较报告。各命令参数、返回值、管理器及工厂地址见 [VM语义](vm-semantics.md#归档路径注册组2026-10-31-复核)。

CHRONOCLOCK 使用当前 IDA 会话只读查询，输入 SHA-256 与档案一致；基准用 IDA 9.4 批处理读取新副本 `review-archives-base.i64`，输入 SHA-256 亦核对一致。完整处理函数均取槽号和字符串、拼接根目录、传入两个校验开关，返回 `0x4008`。三个工厂的类型1、2分支沿用 `0x201f` 证据，但这不是全部差异：图像工厂的路径数组/指针数组从基准 +4232/+6280 移至本作 +2152/+4200。记录步长2056、槽数10、析构/替换和校验配置操作一致，Rust路径映射不依赖原版内存布局。另两种工厂接受五槽（0..4）；读取器当前只消费音频/影片前四槽的限制保留。

为避免首差异遮住后续依赖，只在两份导出的内存副本中将三对已人工核对的工厂替换成相同叶子，用现有v3 shape诊断：七条命令的工厂外可比较部分全部一致；剩余阻断来自共享字符串解析器可写全局字符串区。随后人工核对该解析器的标签、槽数、步长和越界回退，以及CPZ6构造与vtable +40校验setter。此诊断不是完整指纹继承，不改变生产算法或其他命令状态；其他归档类内部、后续加载及原生输出未验证。

### 默认启动验收

debug `cmvs-trace`，仍用建档参数：`--bootstrap`、`--script-archive`、`--record-platform`、`--continue-without-snapshot`、`--frame-ms 16`、`--rand-seed 0`、`--steps 1000000`，不加 `--allow-unreviewed`。本次将原CSS1复制到新的 `local/chronoclock/review-archives-input/system.dat`，`--save-dir` 与 `--system-save` 均指向该独立目录，原游戏只读。副本运行前后 SHA-256 均为 `39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`；游戏档案记录原目录无剧情槽，因此没有新增剧情槽输入。

原第43步 `start.ps3 / 0xde / 0x2013` 完成后推进到 `0xe0`，共13次归档路径注册全部完成。第295步停在 `start.ps3 / 0x11f8 / 0x2128`，退出码1，PC和8字节参数保持挂起；统计 base=0、inherited=5、reviewed=15、presumed=12、unreviewed=0（含等待重试的分派尝试）。执行过的推定命令为 `0x2015`、`0x2016`、`0x2092`–`0x2094`、`0x2098`、`0x22b3`。去除旧日志末尾统计、推定列表及错误行并规范化换行后，v3默认日志为本次日志的相同前缀。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-archives-base.json`（完整反汇编、伪代码） | `2290027b9872d0ce9fc3648c2f85105f68ac09173cf30acf77cadcdcdbd9cda5` |
| `review-archives-chrono.json`（完整反汇编、伪代码） | `54342643438d6cf050f9987abd90269011e4320dc71ec07a6c2c5cca005a3d0a` |
| `review-archives-compare.log`（工厂边界诊断） | `daf041e00580b03697e767c7058becaf4a7b5d45cd65f2b10d5ddc085b65a344` |
| `review-archives-default.log` | `0b859806f8e42716c9d915ab922f3ddffccc60695ad3f310b26947acb948a67d` |

研究路径还剩四条未复核命令（`0x2128`、`0x2080`、`0x2030`、`0x2141`），下一步按默认停点复核 `0x2128`。没有新增原生窗口、字体、音轨或完整剧情验证，也未改基准档案及运行代码，因此本轮不重复计入《アマツツミ》百万步回归。

检查通过：`cargo fmt --check`、`cargo test --offline`（836项通过、12项忽略，含逐命令复核小节引用检查）、`cargo clippy --offline --all-targets -- -D warnings`、`git diff --check`。新旧默认trace经LF规范化后的5553字节前缀完全一致。本轮不新增外部来源、依赖或解码算法，不需新增真实解码对照；原始研究输出与分析库继续留在忽略目录。

## 2026-10-31：P3，原采样路径剩余四条复核

`0x2128`、`0x2080`、`0x2030`、`0x2141`登记为Reviewed；档案统计为Inherited=455、Presumed=167、Reviewed=13、Unreviewed=77、NoOp=264。各命令的两个x64构建完整哈希、case/函数、参数、返回值、状态、错误路径和支持边界见[VM语义](vm-semantics.md#系统存档读取-0x21282026-10-31-复核)。x86本轮未单独复核，不改独立指纹报告、算法或其状态。

CHRONOCLOCK仍使用当前IDA会话只读查询并重新核对输入哈希；基准从 `p2-amatsu-x64.i64` 新复制为 `review-four-base.i64` 后用IDA 9.4批处理读取，哈希与档案一致。遍历v3依赖树找出全部可见差异，指令不再对齐/调用数不同的分支另按语义配对；未把诊断树当作完整继承证明。

`0x2128`首差异为持久位区及镜像的映像相对地址，已与前轮位区证据核对。`0x2141`差异为文字对象缓冲指针从基准+560移至本作+352，销毁顺序和对象归属不变。`0x2030`深入图像管理器后确认2080字节字段移动、基准新增PB2A分派，以及JBP1跨基本块的映像相对查表地址；查表1024字节逐字节一致，PB3正常分支仍复用已有解码器。没有更改解码算法，PB2A及其他未实现格式继续报错。

`0x2080`主脚本切换和快照实时位0效果一致，但文字记录v2/v3、公共图像8/16槽和效果类型9支持确有不同。本轮不构造伪兼容快照，也不把这些常量抹去；runner新增派生档案快照格式错误屏障，统一阻断其构造/检查和槽恢复。命令复核允许现有切换与无快照策略，不表示已支持本作原生CSV2子记录。新增合成测试验证错误不改变PC、栈和快照图像，且不生成备份槽；根档案存读档由现有合成测试回归。未来派生快照编码/恢复仍需独立证据与实现。

### 默认闸门验收

trace继续使用上一轮全部参数和 `review-archives-input` 独立CSS1副本，不加 `--allow-unreviewed`。越过启动`0x2128`和主脚本切换后，在第43920步 `intproc.ps3 / 0x4021a / 0x2141` 停止；命令已通过闸门，但工具缺少MessageHost。统计inherited=5962、reviewed=24、presumed=2429、unreviewed=0。上一轮默认trace在旧统计行前、LF规范化后的29803字节是新日志的逐字节前缀。

完整runner探针采用 `SnapshotPolicy::ContinueWithoutImage`、记录文字、RecordingBackend、种子0、16ms、无输入、百万tick预算，默认闸门；新建 `review-four-user` 独立用户目录并复制同一CSS1。实际完成 `0x2128` 1次、`0x2080` 4次、`0x2030` 3次、`0x2141` 1次。经main/logo/menu进入cdemo后，steps=82416在 `intproc.ps3 / 0x4027c / 0x215d` 报未复核；前8参数 `[7,10,0,0,1280,54,1280,720]`，继续保持挂起。统计base=0、inherited=15058、reviewed=24、presumed=2429、unreviewed=0，统计仍是分派尝试数。执行过的推定命令完整列表保留在日志。

CSS1运行前后SHA-256均为 `39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`。原游戏未写入，未测试原生窗口/字形/音轨、剧情快照或完整路线。此前“剩余四条”仅描述旧采样路径；新观察到的`0x215d`不能跳过。下一步仍按新默认停点复核，不启用研究放行追历史`0x2112`。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-four-base.json` | `a7ab9681d26c78c1d467f66ec88bd480925edfc1433b848d50a905c3518f968d` |
| `review-four-chrono.json` | `a9c7cd791c613dc457378c6f22afee4ba774efaa0f1a7afb33390ab46d3ae617` |
| `review-four-audit.log` | `646e06e41d2491d309d043c24817354d9c8c5718400b9749ca66bd61878dc6f3` |
| `review-four-image-audit.log` | `427d8d5ed5e8eaa9539d60039b92144db4efddc39885e2f75549d7e53b404e37` |
| `review-four-default.log` | `c12ef9416c0c8781f5f6ca20055cf9151b891f7416f3fd71c7ed4059cbf77a22` |
| `review-four-runner.log` | `5a90f791c4601bb74bf7cabda493396dd7a3a1db2fbfe7466a6f785b6c17dc09` |

检查通过：`cargo fmt --check`、`cargo test --offline`（837项通过、12项忽略）、`cargo clippy --offline --all-targets -- -D warnings`、`git diff --check`。本轮无后端/feature、解码算法、依赖或外部源码变更；没有重新计入根游戏百万步回归，根快照正常行为由现有合成测试覆盖。

## 2026-10-31：P3，按共享差异复核文字/窗口组

基线 `62c9c57`。从新停点 `0x215d` 出发，新增只读诊断 `profile_compare.py --group-differences [命令号…]`，将多个命令的差异函数边界聚合。只沿自身函数体完全对齐的调用边展开，不在不同父体下面按调用次序强行配对；输出包含架构、两边exe哈希和深度，并明确不比较边界后的后代、不改变状态。没有参数时按自动Unreviewed报告选择，因此可能含已经人工Reviewed的命令。工具不引入手工叶子覆盖或字段归一化，指纹策略v3和原比较报告保持不变。

窗口创建处理函数各145条指令，唯一自身差异是文字对象从0x180增至基准0x250字节。人工核对完整构造发现基准多清零208字节扩展块、后续六字渲染参数/工作缓冲/上下文字体设备字段整体移动；共同初始化与后续操作一致。构造的额外memset使调用数不同，按角色重新配对三个子函数继续检查；没有用“首处只差分配大小”代替整个构造复核。与先前析构、快照字段移动形成同一证据组。

共登记九条Reviewed：`0x2140`、`0x215d`、`0x2102`、`0x2118`、`0x214e`、`0x2150`、`0x22e0`、`0x2170`、`0x2176`。后两条复用 `0x2030` 已核对的图像管理器和PB3差异链；其包装函数无新增差异。地址、参数、返回值、支持与错误边界见[共享布局组](vm-semantics.md#窗口与文字共享布局组2026-10-31-复核)。`0x2118`、`0x2176`尚无Rust处理器，Reviewed与已实现分开记录；不为未执行命令补造动态验证。档案现为Inherited=455、Presumed=167、Reviewed=22、Unreviewed=68、NoOp=264。

本作当前IDA输入哈希再次确认，基准从 `p2-amatsu-x64.i64` 新复制为 `review-windows-base.i64` 批处理读取。仅查询反汇编/伪代码，原游戏只读，x86本轮未单独复核。没有新增外部资料、核心语义、解码算法、依赖或原生设备验证；不重复根游戏百万步回归。

### 默认闸门验收与下一组

沿用上一轮runner探针参数：RecordingBackend、记录文字、种子0、16ms、无输入、`ContinueWithoutImage`、百万tick预算，未启用 `--allow-unreviewed`。新建 `review-windows-user` 独立用户目录，复制同一CSS1；运行前后SHA-256均为 `39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`。

先越过旧 `intproc.ps3 / 0x4027c / 0x215d`，初次验证停在 `0x2170`；补齐这条共用图像证据后，最终到steps=93397、`cdemo.ps3 / 0x2105e / 0x203f`。实际执行本轮新增命令 `0x215d` 2次、`0x2150` 1次、`0x2170` 2次，其余六条只做静态复核。闸门统计base=0、inherited=16305、reviewed=32、presumed=2599、unreviewed=0；这是分派尝试数。已执行推定命令列表完整保留在日志，新增的 `0x2178`、`0x214a`、`0x2033` 等不计作人工复核。

新停点 `0x203f` 的共享差异边界为基准 `0x140035a90` / 本作 `0x1400341f0`；下一轮可聚合 `0x2032`、`0x22c4` 排查全部边界，再决定放行。当前实际推进10981步，不表示完整自动演示、呈现、剧情或存档验证通过。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-windows-base.json` | `ebac8f62da872bf822dcb046f3f6f1220479ff019e2993d1c50f169c7f5893c4` |
| `review-windows-chrono.json` | `bae42a3411f7b1cc078ac2a4e3f7a82082bd7a86544ac2daa4a1caf1b11805dc` |
| `review-windows-audit.log` | `21c5f4b2017e115e5a9aa454b519ad9195b5c1a72e389ac724ef7b02eb4508e3` |
| `review-windows-helpers.log` | `e511ba7d441dc628cc2fb719106d232cdbd3e7a69b078981a81fe2f3b96bc7cf` |
| `review-windows-groups.log` | `bb38acb8cdfb0fa20ec4670015ac9aeb757b373fc807ef5c19ca705ab3a2a9d2` |
| `review-windows-runner.log` | `d660951f9d3d1adc64e6304086ed98a602555a92d2aec50c7c19d3ad472e6978` |

检查：Python合成测试25项通过，新增覆盖共享分组、全部对齐兄弟分支、父体失配停止、深度/参数边界、保守数据及输入不变；`cargo fmt --check`、`cargo test --offline`（837项通过、12项忽略）、严格Clippy和`git diff --check`通过。首次Cargo测试遇到既有资源测试按时间戳命名目录的并行撞名，已加进程内原子序号并完整重跑通过；不改变资源读取行为。

## 2026-10-31：P3，图像缓存提示与换图组

从默认停点`0x203f`继续，按共享差异聚合`0x2032`、`0x22c4`，复核三条并登记Reviewed。当前Inherited=455、Presumed=167、Reviewed=25、Unreviewed=65、NoOp=264。运行时、指纹策略、自动比较报告与解码算法不变；没有新增依赖或外部源码。参数、返回标志、成员对照与实现限制见[图像缓存组](vm-semantics.md#图像缓存提示与换图组2026-10-31-复核)。

本作当前IDA会话输入哈希核对一致；基准从`p2-amatsu-x64.i64`复制到新的`review-cache-base.i64`，用IDA 9.4批处理只读查询，输出身份核对一致。提示位/查询状态分别移动2080字节；预加载结果取得函数58条指令、12处成员位移变化；原始读取166条指令、5处字段/路径位移变化。缓存查询/插入调用树可比较部分相同，格式分派后按资源签名配对，复用先前`0x2030`证据，不按失配父函数下的调用序号判等。未知格式与未建模模式继续显式报错。x86未独立复核。

### 默认闸门验收

复用上一轮完整runner探针，RecordingBackend、记录文字、种子0、每次Yield推进16ms、无输入、`ContinueWithoutImage`、百万tick预算，未启用`--allow-unreviewed`。新建`review-cache-user`并复制CSS1，运行前后SHA-256均为`39a93d66c3020f95f6eca644d4e1a79c9318d0f8f53366a9b7a408f5b2fa5385`。原游戏保持只读。

两次`0x203f`在`cdemo.ps3`的PC `0x2105e`、`0x1ddf6`完成，分别记录steps=93397、93406；本轮其余两条未动态执行，`0x2032`仍无处理器。新默认停点steps=102340、`cdemo.ps3 / 0xf090 / 0x22b2`，比旧停点推进8943步。闸门分派尝试统计inherited=17179、reviewed=36、presumed=2600、base/unreviewed=0，含等待重试，不是完成数或覆盖率。已执行推定命令列表保存在日志，不额外计为人工复核。没有验证原生窗口、文字/音轨、剧情快照或完整自动演示。

下一步复核`0x22b2`的实时状态更新与快照调用，仍保留派生构建快照格式屏障；没有以研究放行绕过该停点。初步诊断涉及快照子记录数量及状态数组差异，不能把它视作单一字段移动。

| 忽略目录内证据 | SHA-256 |
|---|---|
| `review-cache-base.json` | `ffda00334aa7cd5af9df162e61f450540030ca08734e7ec4b07b551c718cf130` |
| `review-cache-chrono.json` | `b876bd25d2a5311544eee0950619d2880dbc4182cc2165db36ca9b401f43966a` |
| `review-cache-audit.log` | `12b17e660c5ed5cfc53fa816328570354f699b6d4dc1cdcf276226734142186f` |
| `review-cache-runner.log` | `3a7c93e19e2f0563a8cb5e5f22241f9405f4a4048232df741082ca4b864ecf10` |

检查：`cargo fmt --check`、`cargo test --offline`（837项通过、12项忽略）、`cargo clippy --offline --all-targets -- -D warnings`通过。无后端/feature或解码变更，不重复计入根游戏百万步重放。既有合成测试覆盖缓存与档案文档引用；未增加与数据改动重复的测试。
