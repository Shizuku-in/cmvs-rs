# 引擎档案

档案对应具体引擎构建，游戏记录绑定启动脚本、脚本包和可执行文件身份。源数据位于 [`crates/cmvs-profile-research/profiles`](../../crates/cmvs-profile-research/profiles/)；核心的 `profile::Registry` 使用生成的静态表识别游戏。

`cmvs-profile-research::ResearchRegistry` 解析源档案并校验证据和契约，`cmvs-profile` 提供 PE 分派分析及静态表生成。生成数据包含游戏与构建身份、命令状态、变体 ID 和 CVNS 映射。

修改源档案后，在仓库根目录重新生成。CI 会校对生成结果。

```sh
cargo run --offline -p cmvs-profile-research --bin cmvs-profile -- generate-playback \
  --output crates/cmvs-core/src/profile/playback/data.rs
```

## 选择与闸门

默认根据数据哈希和可执行文件身份识别游戏。研究时可用 `--profile <档案ID或游戏别名>` 选档案。别名解析到对应的引擎构建，游戏记录和脚本声明的游戏 ID 仍按输入证据核对。

| 命令状态 | 含义与运行处理 |
|---|---|
| `Base` | 根构建语义来源，进入现有处理器 |
| `Inherited` | 同架构完整依赖指纹一致，进入根语义 |
| `Reviewed` | 有人工复核记录，进入登记语义 |
| `ContractEquivalent` | 有有效独立证明，进入根语义，见[契约比较](command-contracts-m1.md) |
| `Presumed` | 不完整依赖下的形状相同；默认放行，`--block-presumed` 阻止 |
| `Unreviewed` | 尚无等价结论；默认放行，`--block-unreviewed` 阻止 |
| `Absent` / `NoOp` | 始终报错 |

证据状态、处理器是否实现、游戏是否验证分别记录。闸门计数包含等待重试和未实现处理器的分派尝试，不是完成数或覆盖率。

## 复核与变体

复核文档路径相对于 `docs/`：

```text
0x201f reviewed 0x7ecd9 0x70d92 review=games/chronoclock/vm-semantics.md
```

文档须有对应命令号标题，记录两侧 exe SHA-256、架构、函数地址、比较范围与结论。修改档案或文档时须核对引用和命令小节。

`variant=<实现ID>:<架构>:<exe SHA-256> evidence=<文档>` 声明独立处理器，由 `ResearchRegistry` 校验绑定。宿主执行静态表选定的变体。处理器约定见[分派指南](../guides/dispatch.md)。

## 指纹工具

`tools/ida_export_commands.py` 从匹配的 IDA 数据库导出命令及依赖，校验二进制身份并拒绝补丁字节。导出和数据库副本留在仓库外或 `local/`。同架构、同策略的两份导出由标准库工具比较：

```sh
python3 -I tools/profile_compare.py local/base.json local/derived.json \
  --report local/comparison.tsv \
  --base-profile crates/cmvs-profile-research/profiles/cmvs-20160201.profile \
  --profile crates/cmvs-profile-research/profiles/cmvs-20150405.profile \
  --output-profile local/derived.profile
```

完整指纹保留指令、参数、控制流、调用与数据依赖。未知数据、间接控制流、递归和截断不能视为完整继承；形状比较只提供 `Presumed`。候选档案经源档案校验和证据复核后登记；人工复核行不能被自动比较覆盖。可加 `--group-differences` 聚合差异函数以安排复核，或列出命令号限定范围。分组沿相同函数体展开，遇到不同函数体即停止。

新构建用 `cmvs-profile identify` / `table --base` 建立初始资料，见 [CLI](../guides/cli.md)。新增游戏遵循[研究约定](../research.md)。

## CVNS

`family cvns` 复用 CMVS 运行时，但命令重新编号，必须逐条复核映射。只有 `reviewed` 行可以携带 `map=`，目标为根档案的 `base` 命令或 `0x2400` 起已登记的 CVNS 专有处理器。

`Machine` 将挂起命令翻译为运行时编号，闸门和诊断仍按原编号核对。未映射命令始终报 `Unmapped`，不受 `--block-unreviewed` 影响。CVNS 不使用跨代指纹继承或推定；当前构建与证据见[みはる档案](../games/miharu/README.md)及[命令映射](../games/miharu/README.md#命令映射)。
