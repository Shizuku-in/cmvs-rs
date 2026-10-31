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
