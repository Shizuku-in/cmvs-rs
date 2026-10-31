# M1：受限契约提取、证据绑定与离线报告

> 2026-10-31 完成。对应[规划](command-contracts.md) M1，输入为 [M0 模型](command-contracts-m0.md)。只产出离线报告；运行闸门、档案状态和 Rust 分派均未改变，没有新增 `ContractEquivalent`。

## 1. 交付

| 文件 | 作用 |
|---|---|
| `tools/command_contract.py` | IR 校验、规范化、三值比较（M1a），新增契约信封与证据校验 `compare_envelopes` |
| `tools/contract_x64.py` | 仅标准库的 x64 整数子集解码器；不支持的编码直接报错 |
| `tools/contract_extract.py` | 从 v3 导出对 case 块做符号执行，生成 `cmvs-command-contract-v1` 信封，可选读取原 exe 绑定字节，并输出双边报告 |
| `tools/test_contract_extract.py`、`tools/test_command_contract.py` | 合成机器码与 IR 测试，含 M0 证据向量 |
| `docs/games/chronoclock/contract-*.json` | 首个构建对的上下文、试点字段映射与（空）摘要；见[试点报告](../games/chronoclock/contract-m1.md) |

旧的“常量返回片段”提取器已由完整提取器取代，不再输出 `cmvs-return-fragment-v1`。

## 2. 提取模型

**入口与 ABI。** 每条命令从 switch case 地址开始，入口状态来自分派器：`rbx` 为 VM 对象，`[rsp+0x20]` 是保存的 `rbx`，`[rsp+0x28]` 是分派器返回地址，分派器的 32 字节 home space 位于其上。`rax/rcx/rdx/r8–r11` 为不可用的分派器临时值，其他非易失寄存器为入口标识。命令只能在 `rsp` 回到 `+0x28`、返回地址未被改写、全部非易失寄存器复原时，以 `ret` 退出；否则判为 `cfg-gap`。直接 `call` 按真实压栈内联，`jmp` 到函数按尾调用处理。

**参数栈。** 只识别 `movsxd SP, [VM+sp_offset]` 与 `[VM+base_offset]` 相加后的寻址：`[base+SP+k]` 成为 `stack_offset=k` 的 `read_parameter`。偏移由上下文文件按二进制登记。重叠宽度读取、SP 之上的读取以及写参数栈或 SP 都会阻断。

**返回码。** 退出码必须是常量。按 `SystemResult` 规则，`0x4000` 位前进 PC 并回收低字节；提取器在叶尾生成 `release_stack`。让出（`0x8000`）、重复计数（`0x2000`）、不前进或高位非零均为 `unsupported-effect`。分派器的调用者不在提取范围内，返回码规则作为两边共用的信任边界登记在上下文证据中。

**内存与字段。** 地址只能由已知根加常量偏移组成：本地帧、参数栈或逻辑对象。对象访问必须命中字段映射 `(二进制, 基对象, 偏移, 宽度)`；带 `points_to` 的指针字段读取后得到另一个逻辑对象的根，从而形成从 VM 出发的对象来源链。读写分别生成有序 `read_state`/`write_state`，读改写指令先读后写。全局/RIP 相对地址、动态或缩放索引、未映射偏移和指针写入均阻断（`unknown-memory`、`unproven-mapping`、`unsupported-effect`）。

**表达式与路径。** 支持 `mov/movzx/movsx/movsxd/lea`、`add/sub/and/or/xor/cmp/test`、`inc/dec/neg/not`、常量移位、`push/pop`、`setcc/cmovcc`、条件与无条件局部跳转。32 位写入零扩展，8/16 位部分写入只在原值已知为零时合并，否则后续更宽的读取阻断。常量条件静态选路；其余条件按标志来源（减法、按位与、结果）转成 `eq/ult/slt`，不支持的条件码阻断。`jcc`、`setcc`、`cmovcc` 都在决策树上分叉，前置读取进入 `branch.events`。

**预算与回边。** 默认每侧4096条已执行指令、64个叶路径、直接调用深度8。同一路径重回同一位置判为 `loop`，调用链中重入判为 `recursion`。只报告第一个阻断点（含函数 RVA 与指令序号），不会跳过阻断继续拼接。

**helper 摘要。** 摘要按 `(二进制, 函数 RVA)` 匹配 `call` 目标，必须与导出函数节点的哈希及依赖哈希一致，否则作为证据错误拒绝。M1 只接受单路径摘要，参数是寄存器中的位向量，事件仅限状态读写；摘要不能通过尾跳转使用。首个构建对尚未登记任何摘要。

## 3. 证据绑定

信封 `identity` 按 M0 记录配置、二进制哈希、架构、image base、opcode 与 case RVA；`evidence` 记录导出哈希、提取器版本（四个工具源码的联合哈希）、上下文/映射/摘要文件哈希，并在 M0 字段之外新增 `binary_checked`、`mappings_used`、`summaries_used`。

`compare_envelopes` 拒绝缺字段、非 x64、同一二进制、opcode 不一致及两边工具/规则版本不一致；未经原 exe 绑定的信封最多得到 `unknown`（`binary-not-checked`）。

提供 `--base-exe/--derived-exe` 时，工具独立核对 exe SHA-256、PE32+ image base、唯一的分派器序言与 switch 形态、该 opcode 的表项，以及每条已执行指令的原始字节。导出会把分支位移清零，因此分支指令改为用真实编码计算目标，并核对其与导出引用一致。指令有其他引用（数据、导入等）时提取已先阻断，不需要绑定这些字段。

M0 的三个证据向量：缺映射哈希和跨架构在比较器测试中拒绝；摘要函数体哈希不符在提取器测试中拒绝。

## 4. 使用

```powershell
python -I tools/contract_extract.py BASE.json DERIVED.json `
  --context docs/games/chronoclock/contract-context.json `
  --mappings docs/games/chronoclock/contract-mappings.json `
  --summaries docs/games/chronoclock/contract-summaries.json `
  --base-exe BASE.exe --derived-exe DERIVED.exe [0x203f ...] > local/report.json

python -I tools/test_contract_extract.py
python -I tools/test_command_contract.py
python -I tools/test_profile_compare.py
```

不给 opcode 时报告目标构建的全部 handler。报告退出码0；规则文件或输入格式错误分别为4和3。逐行状态为 `equivalent`、`different`、`unknown` 或 `invalid-evidence`（如 exe 字节不符）。真实导出、exe 和报告留在 `local/` 或仓库外。

## 5. 检查

- 提取器测试10项：常量返回、已映射偏移移动判等价、未映射判未知、参数次序/回收字节/字段替换三类机器码反例判差异、错误位条件分支等价及分支反转判差异、尾调用与内联 helper、六种阻断、摘要绑定与失效、输入不变，以及合成 PE 上的指令/分支目标绑定和哈希不符拒绝。
- 比较器测试14项：M0 的18个语义向量、2个信封证据向量、绑定规则及原有 IR 用例；无跳过项。
- 指纹比较回归25项。
- 解码器在两份真实导出的约21.6万条可解码指令上与 capstone 的助记符和操作数一致（立即数按位宽比较）；capstone 只用于这次仓库外核对，不是工具依赖。

## 6. 限制与进入 M2 的条件

- 一处阻断即放弃整条命令；报告只给出第一个原因，不是完整未知清单。
- 字段映射与 `points_to` 依赖人工证据；`points_to` 不建模空指针路径，限制写在每条映射的 `review` 中。
- 不支持全局区域、动态索引、导入/库调用、间接调用、循环、浮点、乘除和位移变量，这些正是首个构建对的主要阻断（见试点报告）。
- 等价仅表示在登记的上下文、映射与工具版本下两边契约规范形式相同；不代表 Rust 实现正确，也不改变运行放行。

M2 按报告中的阻断分布推进：逻辑全局区域与只读数据、动态索引表、常用 helper 的多路径摘要和导入调用模型。
