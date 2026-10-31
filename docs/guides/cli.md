# CLI 检查指南

所有命令从项目根目录执行。这里使用 `<游戏目录>` 和 `<新输出文件>` 作为占位符，请替换后运行。工具只支持已核对的格式分支，不能据解析成功推断整款游戏可运行。当前真实样本验证来自[《アマツツミ》](../games/amatsutsumi/README.md)。

## 脚本与执行跟踪

```bash
cargo run --offline -- "<游戏目录>/data/pack/start.ps3"
cargo run --offline -- "<游戏目录>/data/pack/start.ps3" --payload "<新输出文件>"
cargo run --offline --bin cmvs-disasm -- "<游戏目录>/data/pack/start.ps3"
cargo run --offline --bin cmvs-trace -- "<游戏目录>/data/pack/start.ps3" --steps 10000
```

默认 `cmvs-inspect` 检查 PS2A 头部、分段和解码结果，文本保留原始字节并以十六进制显示。`--payload` 输出不含原始头部，拒绝覆盖。线性反汇编不跟随分支；trace 有步数上限，未知状态和未实现命令显式停止。

完整启动研究示例：

```bash
cargo run --offline --bin cmvs-trace -- "<游戏目录>/data/pack/start.ps3" --bootstrap --save-dir "<游戏目录>/save" --system-save "<游戏目录>/save/system.dat" --script-archive "<游戏目录>/data/pack/script.cpz" --record-platform "<游戏目录>" --continue-without-snapshot --frame-ms 16 --steps 1000000
```

trace先识别引擎档案：输入脚本（或`--script`归档）、`--script-archive`及`--record-platform`根目录exe参与哈希比对，以`# engine profile`和`# profile evidence`打印。未登记的游戏或合成脚本需显式`--profile <id>`。命令先经闸门：Base、Inherited、Reviewed与带有效登记证明的ContractEquivalent进入现有分派，Absent/NoOp总是报错。**2026-11-01 M3 变更：旧Presumed不再默认放行**，仅`--allow-presumed`研究放行；Unreviewed仍需独立的`--allow-unreviewed`并记录首次使用。派生档案在退出时打印各来源已放行分派尝试数、契约/研究命令列表及策略（含等待重试和未实现处理器的尝试，不等于完成或覆盖率），闸门错误以非零退出码结束。完整迁移与证明失效规则见[M3](../design/command-contracts-m3.md)。

`--bootstrap` 应用已核对的初始化；`--save-dir` 只读扫描剧情槽，不会自动推断 `--system-save`。记录平台请求不创建窗口，帧时间必须显式提供；省略 `--frame-ms` 时冻结时间并在等待处停止。增加步数不能绕过未知命令、快照或完整恢复屏障。VM 地址及语义见[本作证据](../games/amatsutsumi/vm-semantics.md)。

## 引擎档案

已有IDA导出可用 `python -I tools/profile_compare.py BASE.json DERIVED.json --group-differences` 聚合共享差异函数，或在选项后列出命令号（如 `0x2140 0x215d`）限定范围。输出包含两边exe哈希、架构与深度；仅作人工复核排序，不生成档案或更改状态。

分组只沿整个自身函数体对齐的调用边展开，遇到不同函数体就停止，不断言其后代相同。它仅显示能配对的处理函数；无命令号时依据自动比较的Unreviewed列表，因此可能包含已人工Reviewed的命令。保守数据、间接调用和深度限制仍须结合普通报告及 `--explain` 检查。

```bash
cargo run --offline --release --bin cmvs-profile -- identify "<游戏目录>"
cargo run --offline --release --bin cmvs-profile -- identify "<游戏目录>" --profile cmvs-20160201
cargo run --offline --release --bin cmvs-profile -- table "<游戏目录>" --id <新档案> [--base cmvs-20160201]
cargo run --offline --release --bin cmvs-profile -- show cmvs-20150405
```

`identify` 只读哈希启动脚本、脚本包和 exe，打印封包/脚本签名、每个 exe 的 PE 时间戳、链接器版本、switch 位置与处理函数/空操作数量；exe 已登记时与内嵌表逐项比对，不一致即报错。最后报告档案选择结果；未登记的游戏以非零退出码结束，CVNS/PVNS 特征另行提示为"不支持的家族"。`table` 从游戏根目录的 exe 生成新档案表到标准输出：不带 `--base` 时有处理函数的命令记为 `base`，带 `--base` 时记为 `unreviewed`；生成的表要放进 `crates/cmvs-core/profiles/`、登记游戏记录并重新构建才会生效。`show` 汇总内嵌档案的状态统计和非基准命令列表。设计与普查见[引擎档案层](../design/engine-profiles.md)。

## 归档与资源

```bash
cargo run --offline --bin cmvs-cpz -- "<游戏目录>/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --list "<游戏目录>/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --script "<游戏目录>/data/pack/script.cpz" main.ps3
cargo run --offline --bin cmvs-cursor -- "<游戏目录>/data/icon/01.cur"
cargo run --offline --bin cmvs-pb3 -- "<PB3 文件>" --rgba "<新输出文件>"
cargo run --offline --bin cmvs-cmv -- "<游戏目录>/data/video/op.cmv"
```

`cmvs-cpz` 支持当前 CPZ6 模式 1，默认检查头部，`--list` 解析索引；CLI 读取不验证资源校验值，运行时图像读取会验证。PB3 的导出为顶向下 RGBA；CUR 的可选 `--planes <新文件>` 导出 AND/XOR 平面，不是 RGBA。具体布局和限制见[格式目录](../README.md)。

## 存档

```bash
cargo run --offline --bin cmvs-save -- "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --records "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --vm-state "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --visual-state "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --section 0 "<游戏目录>/save/save000.dat" "<新输出文件>"
cargo run --offline --bin cmvs-system-save -- "<游戏目录>/save/system.dat"
```

CSV2 检查与解压不会恢复 VM，段导出拒绝覆盖；运行时存读档由 `Startup`/桌面在 `--user-data-dir` 下完成，`cmvs-trace` 不构建快照镜像。CSS1 的模板编码、可选输出与独立用户目录策略见[系统存档](../formats/system-save-format.md)，剧情恢复边界见[快照](../formats/snapshot-format.md)。
`--vm-state` 检查保存的脚本名称、PC/栈、局部变量和主脚本内存布局，并逐项列出未安装的对象/宿主记录；活动公共脚本的PC没有提供对应图像时明确标为未验证。这是离线检查模型，工具不改写游戏或存档。
`--visual-state` 检查已支持版本的文字、消息窗口、场景树、图像引用与绘制/帧动画负载，列出顶层资源名和窗口未解释状态字；不读取图像资源、不恢复引擎。已有显示字形、旋转轨道、复合/特殊图像或效果分支显式报错。核心 `VisualSnapshot::preview` 可另行通过调用者的图像源生成独立场景/消息宿主供离线渲染，完整限制见[快照格式](../formats/snapshot-format.md)。

## 通用探针

```bash
cargo run --offline --release -p cmvs-probe -- "<游戏目录>" --out "<新输出目录>" --font "<字体文件>"
cargo run --offline --release -p cmvs-probe -- "<游戏目录>" --out "<新输出目录>" --checks R1,D1 --device keyboard
cargo run --offline --release -p cmvs-probe -- "<游戏目录>" --out "<新输出目录>" --font "<字体文件>" --checks S2,R2,R3 --max-steps 800000000
```

对任意已识别构建运行与游戏无关的检查，写出 `summary.json`（身份、能力、参数和各检查结果及判定等级）与 `events.jsonl`。游戏目录只读，每次运行在输出目录下新建用户目录；输出目录必须不存在。命令闸门照常生效，`--allow-presumed`/`--allow-unreviewed` 仅供研究且记入报告。R2 从标题自动找到剧情入口并按重放方式探索选项树，R3 把执行过的地址写入 `coverage.tsv`；两者耗时较长，只在显式列出时运行。任何检查失败或被闸门阻止时退出码为 1。检查清单与判定规则见[探针设计](../design/probe.md)。

## 外部样本审计

可复用的脚本结构测试适配器接受独立 PS2A 与 CPZ6 输入，默认忽略，需显式设置输入路径。当前完整命令、报告协议及边界见[脚本审计方法](../games/amatsutsumi/script-audit.md#重复运行)。测试本身只保存合成数据，真实输入与元数据报告按游戏分开归档。
