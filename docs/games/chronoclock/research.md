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
