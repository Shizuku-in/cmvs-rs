# CLI 检查指南

所有命令从项目根目录执行。这里使用 `<游戏目录>` 和 `<新输出文件>` 作为占位符，请替换后运行。工具只支持已核对的格式分支，不能据解析成功推断整款游戏可运行。当前真实样本验证来自[《アマツツミ》](../games/amatsutsumi/README.md)。

每个工具均支持 `-h` / `--help`，帮助输出到标准输出并以0退出；参数错误输出到标准错误并以非零状态退出，同时提示帮助入口。路径含空格时使用引号。导出文件必须不存在，`cmvs-probe --out` 也必须指定新目录；桌面端独立用户目录中的存档和配置则会正常更新。

```bash
cargo run --offline --bin cmvs-cpz -- --help
cargo run --offline --features desktop --bin cmvs-desktop -- --help
cargo run --offline --features audio-output --bin cmvs-audio -- --help
```

`cargo run` 的 `--` 用于分隔Cargo选项与工具参数。`cmvs-desktop`、`cmvs-mgv` 需要 `desktop` feature，`cmvs-audio` 需要 `audio-output`；`cmvs-probe` 使用 `-p cmvs-probe` 运行。

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

trace先识别引擎档案：输入脚本（或`--script`归档）、`--script-archive`及`--record-platform`根目录exe参与哈希比对，以`# engine profile`和`# profile evidence`打印。未登记的游戏或合成脚本需显式`--profile <id>`。命令先经闸门：Base、Inherited、Reviewed与带有效登记证明的ContractEquivalent进入现有分派，Absent/NoOp总是报错。Presumed、Unreviewed默认放行，分别用`--block-presumed`、`--block-unreviewed`显式阻止；未复核命令记录首次使用。派生档案在退出时打印各来源已放行分派尝试数、契约/研究命令列表及策略（含等待重试和未实现处理器的尝试，不等于完成或覆盖率），闸门错误以非零退出码结束。证明登记与失效规则见[契约比较](../design/command-contracts-m1.md)。

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

`identify` 只读哈希启动脚本、脚本包和 exe，打印封包/脚本签名、每个 exe 的 PE 时间戳、链接器版本、switch 位置与处理函数/空操作数量；exe 已登记时与内嵌表逐项比对，不一致即报错。最后报告档案选择结果；未登记的游戏以非零退出码结束，PVNS 特征另行提示为"不支持的家族"。CVNS 的 `cvns.exe` 与 `start.pss` 按 CVNS 家族识别。`table` 从游戏根目录的 exe 生成新档案表到标准输出：不带 `--base` 时有处理函数的命令记为 `base`，带 `--base` 时记为 `unreviewed`；目录里只有 `cvns.exe` 时生成 CVNS 档案，必须用 `--base` 指定 CMVS 根档案；生成的表要放进 `crates/cmvs-core/profiles/`、登记游戏记录并重新构建才会生效。`show` 汇总内嵌档案的状态统计和非基准命令列表，CVNS 档案另外列出命令映射。设计与普查见[引擎档案层](../design/engine-profiles.md)。

## 归档与资源

```bash
cargo run --offline --bin cmvs-cpz -- "<游戏目录>/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --list "<游戏目录>/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --script "<游戏目录>/data/pack/script.cpz" main.ps3
cargo run --offline --bin cmvs-cursor -- "<游戏目录>/data/icon/01.cur"
cargo run --offline --bin cmvs-pb3 -- "<PB3 文件>" --rgba "<新输出文件>"
cargo run --offline --bin cmvs-cmv -- "<游戏目录>/data/video/op.cmv"
cargo run --offline --release --features desktop --bin cmvs-mgv -- "<游戏目录>/data/video/op.mgv" --check-frames
```

`cmvs-cpz` 支持 CPZ3、CPZ5（打包模式 5～9）、CPZ6 模式 1 和 CPZ7（资源模式 0/1）。默认检查头部，`--list` 列目录，`--extract` 提取资源，`--script` 解码 PS2A 容器；CPZ3 不需要启动参数，列目录和提取前始终验证索引密文校验值。

读取 CPZ5/7 时自动查找封包同目录的 `start.ps3`（CPZ5 也查找 `start.ps2`），可用 `--start-script <路径>` 覆盖；CPZ7 另支持 `--keys <k0,k1,k2,k3>`。CPZ5 可用 `--packing-mode <5..9>` 指定模式，无启动设置时按索引结构自动识别；CPZ7 未找到设置时使用零参数。CPZ5/7 始终验证头部及索引，CLI 提取不验证资源校验值。PB3 的导出为顶向下 RGBA；CUR 支持单图 8/24/32 位 BI_RGB，`--planes <新文件>` 导出 AND/RGB 平面（不含 alpha），`--rgba <新文件>` 导出 straight RGBA，遇到旧式反色像素显式报错。具体布局和限制见[格式目录](../README.md)。

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
cargo run --offline --release -p cmvs-probe -- --help
cargo run --offline --release -p cmvs-probe -- "<游戏目录>" --out "<新输出目录>" --font "<字体文件>"
cargo run --offline --release -p cmvs-probe -- "<游戏目录>" --out "<新输出目录>" --font "<字体文件>" --checks all --max-steps 800000000
cargo run --offline --release -p cmvs-probe -- "<游戏目录>" --out "<新输出目录>" --font "<字体文件>" --checks R2,R3 --fork save --max-steps 800000000
cargo run --offline --release -p cmvs-probe -- "<游戏目录>" --out "<新输出目录>" --font "<字体文件>" --manifest "<清单文件>" --checks P1,R2
python -I tools/probe_matrix.py "<输出目录1>" "<输出目录2>" --markdown
```

对任意已识别构建运行与游戏无关的检查，写出 `summary.json`（身份、能力、参数、清单使用情况，以及各检查的结果和判定等级）与 `events.jsonl`。`--help` 逐行列出全部检查 ID 及其含义，并标出默认运行的那几项。默认只运行静态检查（S1–S5）和启动检查（R1、D1），几秒即可完成；其余检查要玩剧情或整条路线，只在 `--checks` 中列出时运行，`--checks all` 运行全部。

- 游戏目录只读。每次运行都在输出目录下新建用户目录，输出目录本身必须不存在。
- 命令闸门照常生效；`--block-presumed`/`--block-unreviewed` 分别阻止旧推定和未复核命令，默认均放行；策略记入报告。
- `--device keyboard` 使用 `key.cfg` 中的按键，按钮通过导航键沿按钮链接选中。
- `--fork save` 让路线分支从选项处写下的检查点读档开始，而不是每次从头重放；构建缺少快照布局能力时自动退回重放。
- `--without-snapshots` 显式选择无快照研究：保留快照命令的实时状态影响，但不生成可保存图像；V1–V4标为跳过，不能与 `--fork save` 合用。适用于已知部分快照布局、但剧情中的活动效果仍无法保存的构建，不代表存读档通过。
- `--manifest` 读取可选的每游戏纠正清单，格式见[清单格式](../../crates/cmvs-probe/README.md#manifest)。
- `tools/probe_matrix.py` 把多个输出目录汇总为"游戏 × 检查"矩阵，只读。

任何检查失败或被闸门阻止时退出码为 1。检查清单与判定规则见[探针说明](../../crates/cmvs-probe/README.md)。

## 外部样本审计

游戏输入只读，测试只保留合成数据；真实输出在仓库外。结构通过、命令入口覆盖、路线地址覆盖和完整语义支持分别报告。

单脚本接口位于 `tests/support/script_validation.rs`：`validate(&Script) -> Result<ScriptStructure, DecodeError>`，返回指令边界、代码/标签数量和系统命令频次。它不依赖路径、归档、游戏名或输出协议，不向运行时核心增加仅供测试的 API。`tests/script_validation.rs` 用合成数据验证接口，并提供默认忽略的外部样本适配器。

从项目目录显式执行外部样本检查（游戏输入只读，报告仅含元数据）：

```powershell
New-Item -ItemType Directory -Force local | Out-Null
$env:CMVS_SCRIPT_TEST_INPUTS = "<游戏目录>/data/pack/script.cpz;<游戏目录>/data/pack/start.ps3"
$env:CMVS_SCRIPT_TEST_REPORT = "local/script-validation-sites.tsv"
cargo test --offline --release --test script_validation external_script_structure -- --ignored --exact --nocapture
Remove-Item Env:CMVS_SCRIPT_TEST_INPUTS, Env:CMVS_SCRIPT_TEST_REPORT
```

`CMVS_SCRIPT_TEST_INPUTS` 支持任意数量的 CPZ6 包与独立 PS2A 文件，路径列表采用宿主平台的 `std::env::split_paths` 规则（Windows 用分号，Unix 用冒号）。`.cpz` 输入校验资源校验和并检查其中 `.ps3` 条目，其他包内资源明确 skip；其他路径作为独立 PS2A 输入。每项失败记录后继续后续检查，最终由 Cargo 测试状态报告失败；零脚本、缺少路径、损坏归档也不能算通过。不设置报告变量时仅做断言，不产生输出文件；设置时用 create_new 拒绝覆盖。报告保留旧 TSV 的 script/system/site/failure/summary 行，便于与已有 PC 元数据比对；重定向 Cargo 标准输出不能代替报告文件。

系统命令盘点在结构检查报告之外运行分派探针并合并（不需要游戏文件，输出拒绝覆盖）。探针开启文字记录，并为 `0x2129` 提供游戏目录外的合成用户目录；否则会将条件禁用的已实现服务误记为无入口。自检不接受panic作为这些入口已认领的证明：

```powershell
$env:CMVS_COMMAND_INVENTORY = "local/inventory/dispatch.tsv"
cargo test --offline --release --test command_inventory command_inventory -- --ignored --exact
Remove-Item Env:CMVS_COMMAND_INVENTORY
python -I tools/command_inventory.py local/script-validation-sites.tsv local/inventory/dispatch.tsv crates/cmvs-core/profiles/cmvs-20160201.profile local/inventory/joined.tsv
```


覆盖汇总只接受明确成功结束的路线；预算耗尽、失败和截断不算通过。包内 / 包外同名启动脚本分开归因。路线结果记入对应游戏档案，不提交逐脚本清单或本地日志哈希。
