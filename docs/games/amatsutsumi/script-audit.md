# 脚本审计

## 样本与结构

| 输入 | SHA-256 |
|---|---|
| `cmvs64.exe`，x64，image base `0x140000000` | `dc74d2001387ae1aefbdc4204216de5ebb9d3896009280720c4e7a91b42d84f5` |
| `data/pack/script.cpz` | `92b367e8c3d2791f1dd2ca8426581974fbb640d2766f2e82ab12383d2f5359e4` |
| 包外 `data/pack/start.ps3` | `c338313949a803e834acf0d746c93f09c4d41f558af8fb6c13ab1ee55aff2bd0` |

包内87个脚本与包外启动脚本共88个来源全部通过PS2A解码、完整线性指令解码、入口/标签/直接跳转目标边界检查，共2,957,772条顶层指令。87个归档资源的数据校验和通过；CPZ头部/索引摘要未验证。

包内、包外的 `start.ps3` 分开统计：代码长度分别为4752/4726字节、指令数335/330；实际启动使用包外版本。

## 系统命令覆盖

| 分类 | 命令数 |
|---|---:|
| 语料出现的系统命令 | 431 |
| 合成参数下完成或等待 | 210 |
| 宿主认领，因合成状态报错 | 221 |
| 没有处理入口 | 0 |

这些命令全部属于档案 `base`，未落到原版空操作存根或越界。分派探针对每条命令测试全0、全1、首参2三组参数；因对象、窗口、字符串或参数缺失而报错仍计为有入口，panic不作为入口证据。

入口覆盖与完整语义支持分开判断。已实现命令仍可能拒绝未经核对的模式、存储类型、资源格式或快照状态。命令行为见[实现与验证表](semantics-progress.md)及[VM语义](vm-semantics.md)。

## 验证范围

五条结局选择链 `[1]`、`[2,1]`、`[2,2,1]`、`[2,2,2,1]`、END1后的 `[2,2,2,2]` 及OP均在记录音频、文字记录、种子0、16ms帧推进和独立用户目录下完成无窗口重放并返回标题。路线使用 `SnapshotPolicy::Capture`，共捕获120002次快照；捕获不等于逐次写槽和读回。

这组路线基线为 `89e1d50`，覆盖86/88个来源、1,057,150个去重地址（35.74%）、268种系统命令。UI入口单独检查，不并入该路线地址比例。完整逐脚本、路线和输入哈希见[覆盖报告](script-coverage.md)。

设置、游戏内存读档和回看入口检查通过；CG/H回想/音乐/角色鉴赏四页入口及CG列表滚轮、拖动通过无窗口重放与呈现。鉴赏入口的H页切换验证不等同于全部H场景重放，音乐入口不等同于物理音轨或全部曲目验证。各入口及原生实测范围以[游戏档案](README.md)为准。

没有采集条件跳转两侧结果，因此没有分支覆盖率；地址比例不是游戏完成度。未被路线命中的命令不等于未实现，合成或受控原库调用也不等于自然玩法触发。

## 重复运行

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

## 证据与输出

覆盖汇总只接受明确成功结束的路线；预算耗尽、探针失败和截断日志不计入通过。包外启动脚本必须映射到确切来源，包内同名脚本不得合并归因。

游戏文件保持只读。探针、日志、PC清单、存档副本和输出保存在忽略的 `local/`；仓库只保留说明、哈希及必要元数据。
