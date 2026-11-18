# CLI 检查指南

从项目根目录执行，用实际路径替换占位符。路径含空格时加引号。导出文件和 `cmvs-probe --out` 目录必须是新路径。

各工具支持 `-h` 和 `--help`。

```bash
cargo run --offline --bin cmvs-cpz -- --help
cargo run --offline --features desktop --bin cmvs-desktop -- --help
cargo run --offline --features audio-output --bin cmvs-audio -- --help
```

`cargo run` 的 `--` 后传工具参数。`cmvs-cli` 是默认成员，直接用 `--bin` 选工具。`-p cmvs-cli` 默认运行 `cmvs-inspect`。 [工具与构建特性](../../crates/cmvs-cli/README.md#binaries)

## 脚本与执行跟踪

```bash
cargo run --offline -p cmvs-cli -- "<游戏目录>/data/pack/start.ps3"
cargo run --offline --bin cmvs-inspect -- "<游戏目录>/data/pack/start.ps3" --payload "<新输出文件>"
cargo run --offline --bin cmvs-disasm -- "<游戏目录>/data/pack/start.ps3"
cargo run --offline --bin cmvs-trace -- "<游戏目录>/data/pack/start.ps3" --steps 10000
```

`cmvs-inspect` 检查 PS2A/PSS0 的头部与分段，以十六进制显示原始文本字节。`--payload` 导出解码负载。反汇编按指令顺序展开，trace 按步数上限执行。

完整启动研究示例：

```bash
cargo run --offline --bin cmvs-trace -- "<游戏目录>/data/pack/start.ps3" --bootstrap --save-dir "<游戏目录>/save" --system-save "<游戏目录>/save/system.dat" --script-archive "<游戏目录>/data/pack/script.cpz" --record-platform "<游戏目录>" --continue-without-snapshot --frame-ms 16 --steps 1000000
```

trace 会报告识别到的档案及证据。未登记的游戏或合成脚本需指定 `--profile <id>`。派生档案会报告分派尝试次数，含等待重试和未实现的处理器。闸门错误以非零状态退出。 [档案与闸门](../design/engine-profiles.md#选择与闸门)

`--bootstrap` 初始化已核对的状态。`--save-dir` 扫描槽头部，系统存档另用 `--system-save` 指定。记录模式下用 `--frame-ms` 推进帧时钟，省略时会停在等待处。

## 引擎档案

已有IDA导出可用 `python -I tools/profile_compare.py BASE.json DERIVED.json --group-differences` 聚合共享差异函数，或在选项后列出命令号（如 `0x2140 0x215d`）限定范围。输出包含两边exe哈希、架构与深度；仅作人工复核排序，不生成档案或更改状态。

分组只沿整个自身函数体对齐的调用边展开，遇到不同函数体就停止，不断言其后代相同。它仅显示能配对的处理函数；无命令号时依据自动比较的Unreviewed列表，因此可能包含已人工Reviewed的命令。保守数据、间接调用和深度限制仍须结合普通报告及 `--explain` 检查。

```bash
cargo run --offline --release --bin cmvs-profile -- identify "<游戏目录>"
cargo run --offline --release --bin cmvs-profile -- identify "<游戏目录>" --profile cmvs-20160201
cargo run --offline --release --bin cmvs-profile -- table "<游戏目录>" --id <新档案> [--base cmvs-20160201]
cargo run --offline --release --bin cmvs-profile -- show cmvs-20150405
```

`identify` 根据启动脚本、脚本包及 exe 识别档案，校验已登记的分派表。`table` 将候选档案输出到标准输出，带 `--base` 时命令标为 `unreviewed`，CVNS 构建须指定 CMVS 基准。`show` 列出内嵌档案的状态与命令映射。 [档案登记与复核](../design/engine-profiles.md)

## 归档与资源

```bash
cargo run --offline --bin cmvs-cpz -- "<游戏目录>/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --list "<游戏目录>/data/pack/script.cpz"
cargo run --offline --bin cmvs-cpz -- --script "<游戏目录>/data/pack/script.cpz" main.ps3
cargo run --offline --bin cmvs-cursor -- "<游戏目录>/data/icon/01.cur"
cargo run --offline --bin cmvs-pb3 -- "<PB3 文件>" --rgba "<新输出文件>"
cargo run --offline --bin cmvs-cmv -- "<游戏目录>/data/video/op.cmv"
cargo run --offline --release --bin cmvs-mgv -- "<游戏目录>/data/video/op.mgv" --check-frames
```

`cmvs-cpz` 支持 CPZ1/2/3、CPZ5 模式 5～9、CPZ6 模式 1 和 CPZ7 资源模式 0/1。`--list` 列目录，`--extract` 提取资源，`--script` 解码脚本。

CPZ5/7 默认从同目录的启动脚本取参数，可用 `--start-script`、CPZ5 的 `--packing-mode` 或 CPZ7 的 `--keys` 指定。缺少设置时，CPZ5 根据索引识别模式，CPZ7 使用零参数。 [CPZ 格式](../formats/cpz-format.md)

PB3 导出顶向下 RGBA。CUR 支持单图 4/8/24/32 位 BI_RGB，带反色像素的光标可用 `--planes` 导出。 [光标格式](../backends/platform-host.md#标题图标与光标)

## 存档

```bash
cargo run --offline --bin cmvs-save -- "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --records "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --vm-state "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --visual-state "<游戏目录>/save/save000.dat"
cargo run --offline --bin cmvs-save -- --section 0 "<游戏目录>/save/save000.dat" "<新输出文件>"
cargo run --offline --bin cmvs-system-save -- "<游戏目录>/save/system.dat"
```

`cmvs-save` 检查 CSV2，`cmvs-system-save` 检查 CSS1。前者的 `--vm-state` 校验 VM 元数据、局部变量和主脚本 PC，活动公共脚本的 PC 标为未验证。`--visual-state` 检查支持的文字、窗口和场景记录，不读取图像资源。桌面从游戏菜单载入剧情档。 [快照格式](../formats/snapshot-format.md) · [运行指南](running.md#剧情存档)

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

单脚本接口位于 `crates/cmvs-core/tests/integration/support/script_validation.rs`：`validate(&Script) -> Result<ScriptStructure, DecodeError>`，返回指令边界、代码/标签数量和系统命令频次。它不依赖路径、归档、游戏名或输出协议，不向运行时核心增加仅供测试的 API。`crates/cmvs-core/tests/integration/script_validation.rs` 用合成数据验证接口，并提供默认忽略的外部样本适配器。

从项目目录显式执行外部样本检查（游戏输入只读，报告仅含元数据）：

```powershell
New-Item -ItemType Directory -Force local | Out-Null
$env:CMVS_SCRIPT_TEST_INPUTS = "<游戏目录>/data/pack/script.cpz;<游戏目录>/data/pack/start.ps3"
$env:CMVS_SCRIPT_TEST_REPORT = "local/script-validation-sites.tsv"
cargo test --offline --release -p cmvs-core --test integration script_validation::external_script_structure -- --ignored --exact --nocapture
Remove-Item Env:CMVS_SCRIPT_TEST_INPUTS, Env:CMVS_SCRIPT_TEST_REPORT
```

`CMVS_SCRIPT_TEST_INPUTS` 支持任意数量的 CPZ6 包与独立 PS2A 文件，路径列表采用宿主平台的 `std::env::split_paths` 规则（Windows 用分号，Unix 用冒号）。`.cpz` 输入校验资源校验和并检查其中 `.ps3` 条目，其他包内资源明确 skip；其他路径作为独立 PS2A 输入。每项失败记录后继续后续检查，最终由 Cargo 测试状态报告失败；零脚本、缺少路径、损坏归档也不能算通过。不设置报告变量时仅做断言，不产生输出文件；设置时用 create_new 拒绝覆盖。报告保留旧 TSV 的 script/system/site/failure/summary 行，便于与已有 PC 元数据比对；重定向 Cargo 标准输出不能代替报告文件。

系统命令盘点在结构检查报告之外运行分派探针并合并（不需要游戏文件，输出拒绝覆盖）。探针开启文字记录，并为 `0x2129` 提供游戏目录外的合成用户目录；否则会将条件禁用的已实现服务误记为无入口。自检不接受panic作为这些入口已认领的证明：

```powershell
$env:CMVS_COMMAND_INVENTORY = "local/inventory/dispatch.tsv"
cargo test --offline --release -p cmvs-core --test integration command_inventory::command_inventory -- --ignored --exact
Remove-Item Env:CMVS_COMMAND_INVENTORY
python -I tools/command_inventory.py local/script-validation-sites.tsv local/inventory/dispatch.tsv crates/cmvs-core/profiles/cmvs-20160201.profile local/inventory/joined.tsv
```


覆盖汇总只接受明确成功结束的路线；预算耗尽、失败和截断不算通过。包内 / 包外同名启动脚本分开归因。路线结果记入对应游戏档案，不提交逐脚本清单或本地日志哈希。
