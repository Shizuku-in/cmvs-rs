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
