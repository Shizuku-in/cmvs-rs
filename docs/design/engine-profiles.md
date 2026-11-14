# 引擎档案

档案对应具体引擎构建，游戏记录绑定启动脚本、脚本包和可执行文件身份。数据位于 [`crates/cmvs-core/profiles`](../../crates/cmvs-core/profiles/)，解析和选择由 `profile::Registry` 完成。

## 选择与闸门

默认根据数据哈希和可执行文件身份识别游戏；未知或冲突身份报错。研究时可显式指定 `--profile`，但不会据此伪造匹配的游戏记录。脚本声明的游戏 ID 须与记录一致。

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

文档须有对应命令号标题，记录两侧 exe SHA-256、架构、函数地址、比较范围与结论。核心解析不读取文档内容；修改档案或文档时须核对引用和命令小节。

`variant=<实现ID>:<架构>:<exe SHA-256> evidence=<文档>` 在状态之外选择独立处理器；Registry 校验身份和实现。选中的变体不能回退根实现，宿主只执行已选语义。处理器约定见[分派指南](../guides/dispatch.md)。

## 指纹工具

`tools/ida_export_commands.py` 从匹配的 IDA 数据库导出命令及依赖，校验二进制身份并拒绝补丁字节。导出和数据库副本留在仓库外或 `local/`。同架构、同策略的两份导出由标准库工具比较：

```sh
python3 -I tools/profile_compare.py local/base.json local/derived.json \
  --report local/comparison.tsv \
  --base-profile crates/cmvs-core/profiles/cmvs-20160201.profile \
  --profile crates/cmvs-core/profiles/cmvs-20150405.profile \
  --output-profile local/derived.profile
```

完整指纹保留指令、参数、控制流、调用与数据依赖。未知数据、间接控制流、递归和截断不能视为完整继承；形状比较只提供 `Presumed`。候选档案经 Registry 校验和证据复核后登记；人工复核行不能被自动比较覆盖。

新构建用 `cmvs-profile identify` / `table --base` 建立初始资料，见 [CLI](../guides/cli.md)。新增游戏遵循[研究约定](../research.md)。

## CVNS

`family cvns` 复用 CMVS 运行时，但命令重新编号，必须逐条复核映射。只有 `reviewed` 行可以携带 `map=`，目标为根档案的 `base` 命令或 `0x2400` 起已登记的 CVNS 专有处理器。

`Machine` 将挂起命令翻译为运行时编号，闸门和诊断仍按原编号核对。未映射命令始终报 `Unmapped`，不受 `--block-unreviewed` 影响。CVNS 不使用跨代指纹继承或推定；当前构建与证据见[みはる档案](../games/miharu/README.md)及[命令映射](../games/miharu/commands.md)。
