# M0：契约模型与首版实现边界

> 2026-10-31 完成设计基线。对应[规划](command-contracts.md) M0；本文是 M1 的实现输入，不是已实现工具的说明。未增加契约放行状态，未改变现有档案和闸门。

## 1. 交付与范围

M0 固定以下内容：契约 IR v1 的字段和语义、完整性判定、字段映射与 helper 摘要证据格式、合成验收向量、真实差异分类及 M1 支持清单。

- [合成验收向量](command-contracts-v1-cases.json)：一个完整的合成契约及独立变更用例，供 M1 比较器验收。它们是预期结果，不是已经执行通过的契约测试。
- [首个构建对的分类快照](../games/chronoclock/contract-m0.md)：从已有报告和语义证据整理，未重新分析原始二进制。

首版只比较同架构 x64、同一基准、有限无环路径和整数/位操作。目标是先正确处理简单状态设置与布局移动；不承诺首版自动接受复杂文字构造或图像加载链。

## 2. 契约 IR v1

顶层 `schema = cmvs-command-contract-v1`，仅接受已知字段；必填字段缺失、未知操作或类型不符都拒绝输入，不补默认空值。合成向量的 `contract` 仅包含语义体；真实输入必须另外携带下述 `identity` 与 `evidence`，合成数据不得登记到 Registry。

| 字段 | 类型与含义 |
|---|---|
| `identity` | `profile_id`、`binary_sha256`、`arch`、`image_base`、`opcode`、`case_rva`；地址使用十六进制字符串，哈希为64位小写十六进制 |
| `evidence` | `export_sha256`、`extractor_revision`、`rules_sha256`、`mapping_sha256`、`summaries_sha256`；各证据节点含所属二进制哈希及 RVA |
| `contract.precondition` | 布尔表达式；M1 必须为 `true`，不允许自动缩小输入域来隐藏错误路径 |
| `contract.parameters` | 参数声明列表：稳定 `id`、相对命令入口 SP 的 `stack_offset`、`width`（位）、`interpretation`（`bits`/`signed`/`unsigned`）；读取事件单独记录 |
| `contract.tree` | `branch` 或 `leaf` 的有限决策树；`branch` 含 `condition`、`then`、`else`，两个后继不可缺失 |
| `leaf.events` | 有序事件列表，每个事件有局部唯一 `id`；参数读取、状态读写、栈回收均在其中 |
| `leaf.exit` | `kind` 和 `code`；M1 支持 `return`，其他退出种类预留但返回 `unknown` |
| `contract.coverage` | `cfg_closed`、`effects_closed`、`calls_closed`、`memory_closed`、`domain_closed` 五个布尔值，以及 `issues` 列表；不得由调用者任意宣称完整，提取器须提供对应审计结果 |

参数声明不表示所有路径都读取参数；读取多少字节与回收多少字节必须独立表示。`stack_offset=-4` 表示入口 SP 前4字节，不是机器函数调用栈。只有已知 ABI/VM 规则能把原始内存访问映射到这里。

### 表达式

表达式采用 JSON 数组前缀形式：

```text
["bool", true]
["bv", 32, "0000400c"]
["arg", "flag"]
["ref", "read_flags"]
["ne", expression, expression]
["and", expression, expression]
```

位向量宽度限定为 8/16/32/64，常量为定宽小写十六进制；布尔与位向量不可混用。M1 支持 `eq/ne`、位向量 `and/or/xor/not`、模 2^width 的 `add/sub`、明确位宽的 `zext/sext/trunc` 和 `ult/slt`。扩展/截断形式为 `[op, target_width, expression]`。其他一元/二元操作分别为 `[op, operand]`、`[op, left, right]`，操作数类型必须一致。

`arg` 只能在对应 `read_parameter` 之后使用；`ref` 引用同路径此前 `read_state` 事件的值。分支条件只能引用进入该分支前已取得的值；M1 分支所需的前置读取以 `branch.events` 表示，其执行先于 `condition`。M1 不折叠不同符号比较，不把未知初值置零，不支持浮点与隐式转换。

### 有序事件

| `op` | 必填字段 | 语义 |
|---|---|---|
| `read_parameter` | `id`, `parameter` | 在此时读取声明的参数，后续 `arg` 可引用 |
| `read_state` | `id`, `target`, `width` | 读取逻辑状态，结果由 `ref` 引用 |
| `write_state` | `id`, `target`, `width`, `value` | 写逻辑状态，值类型必须匹配；其他位置保持不变 |
| `release_stack` | `id`, `bytes` | 此时回收参数栈字节，非负整数；不能由返回码或参数数量猜出 |

`target` 是 `{region, object, field}`，region 限定 `vm`、`script`、`object`、`save`、`config`；`object` 使用稳定逻辑对象名或已证明的索引表达式，`field` 是逻辑字段标识。磁盘字段需另绑定格式与版本；仅名称相同不能自动映射。M1 只支持固定目标的 `vm/object`；其他区域是明确待支持项。

后端调用、异步请求、资源 I/O、动态索引及保存/恢复等必须进入后续事件模型，M1 遇到这些效果返回 `unknown`，不能忽略。没有写事件仅在效果闭包已成立时才表示保持原值。

## 3. 比较与完整性规则

先验证输入与证据，再检查完整性，最后比较语义。身份/版本不合法返回工具错误 `invalid-evidence`（不参与三值比较）；合法但不完整返回 `unknown`。发现已证实的差异可以同时报告，但不完整数据不能产生 `equivalent`。

完整性必须同时满足：CFG 所有可达退出有建模；所有读写可定位；全部调用已展开或有合法摘要；内存访问无未解决别名和逃逸；所有输入分支（包括错误分支）有覆盖；五个闭包标志全真且 `issues` 为空。越界、空指针或原版未定义行为若不能完整建模，M1 不接受整条命令，不能因 Rust 会报错而删除这些路径。

固定阻断码：`unsupported-instruction`、`cfg-gap`、`unresolved-call`、`unknown-memory`、`unproven-mapping`、`pointer-escape`、`unsupported-effect`、`undefined-behavior`、`loop`、`recursion`、`budget-exceeded`、`unsupported-expression`、`unmatched-paths`。每项附二进制身份、RVA 和说明；预算或同样的阻断码相同不构成证明。

M1 规范化只做局部临时值重命名、常量定宽、已批准字段映射及同类型交换律操作数排序；保留事件顺序、路径条件和返回码。事件按树的先序（branch.events、then、else）编号为 e0、e1…，同时改写对应 ref；参数按声明顺序编号为 a0、a1…，同时改写 arg 和 read_parameter 引用。JSON 对象键排序，数组顺序保留；UTF-8、无 BOM、`ensure_ascii=True`、紧凑分隔符，禁止浮点/NaN。语义哈希只覆盖规范化的 `contract`；证据整体另算哈希。不同二进制地址不能参与语义哈希，但必须参与证据绑定。

规范形式相同且完整才为 `equivalent`。规范形式不同不能直接称为行为不同：对齐路径且有具体可满足条件/输入能证明某个效果或返回值不同，才为 `different`，报告见证；否则为 `unknown`。M1 先支持无条件路径和相同条件树的常量反例，不需要 SMT。不同的前置条件或无法匹配的分支树保持未知。

## 4. 字段映射记录 v1

独立 JSON 对象包含 `schema=cmvs-contract-mappings-v1`、`revision`、`entries`。每条 entry 必须有：

| 字段 | 要求 |
|---|---|
| `id` | 稳定规则名，不使用自动生成函数名 |
| `logical_target` | region、object、field 及位宽/类型 |
| `sides` | 基准和目标的 binary_sha256、arch、对象来源、offset、width |
| `evidence` | 初始化/写入者及读取者、查询或持久化消费者的身份与 RVA，附游戏语义文档小节 |
| `scope` | 适用对象类型、构建、调用约束；未经证明不能扩展到其他对象 |
| `review` | 人工复核来源、日期、规则版本和限制 |

所有映射必须核对对象来源，不能只按 offset 替换。若欲忽略新增内部字段，还须单独记录它不会影响外部效果的消费者闭包证据；“本命令没读到”不足以证明。磁盘布局、容量、格式版本或脚本索引不按对象偏移规则归一化。

M0 不预先颁发任何生产可用映射。现有人工文档作为待提炼证据，M1 实际抽取前核对其适用范围，未满足闭包时仍未知。

## 5. helper 摘要记录 v1

独立 JSON 对象包含 `schema=cmvs-contract-summaries-v1`、`revision`、`entries`。entry 必填 `id`、`binary_sha256`、`arch`、`function_rva`、`body_sha256`、`abi`、`arguments`、`precondition`、`tree`、`coverage`、`dependencies`、`evidence`、`review`。

`arguments` 映射 ABI 输入至逻辑参数，包含指针指向的区域与别名约束；`tree` 使用相同事件/表达式语义，并记录返回值、内存读写、失败与不返回路径。摘要依赖须完整、无环；改变函数体、规则或任一依赖会使摘要失效。M1 仅接受可内联为其支持事件的摘要。

不继承旧指纹策略中 FLIRT 库叶子的不透明行为假设；名称、同名导入或相同占位摘要都不能单独通过契约比较。分配器、文件读取及字体/音频接口暂不提供“一切无副作用”的摘要。

## 6. M1 支持清单与优先级

| 形态 | M1 决定 | 后续条件 |
|---|---|---|
| 固定参数读取、独立回收、常量返回 | 支持 | 含回收多于读取的合成例 |
| 固定 VM/对象字段整数写入与掩码修改 | 支持 | 双边字段映射和对象来源有效 |
| 无环条件、早退及错误位 | 支持有限决策树 | 所有分支闭合，拒绝路径预算截断 |
| 短小直接 helper | 展开或内联受限摘要 | 必须解释完整效果 |
| 不同寄存器/已证明的字段偏移 | 支持归一化 | 不屏蔽其他常量 |
| 可写全局、脚本动态索引 | 暂不支持 | M2 引入逻辑区域、索引与别名分析 |
| 构造/分配、格式分派和资源加载 | 暂不支持 | M2 拆分共享摘要，包含失败路径 |
| 间接调用、循环、递归 | 暂不支持 | 有证明的目标集或循环摘要；不沿用 Presumed |
| 等待恢复、异步效果、存档与浮点 | 暂不支持 | 建立专门状态/时序/格式模型 |

建议初始预算：单命令双边各4096条指令、128个基本块、64个叶路径、直接调用深度8；任一超过即 `budget-exceeded`。这些是工具资源限制，不能推断为游戏限制；实际值随报告记录，修改预算须可重现。

## 7. M0 验收结论

模型、完整性规则、证据格式、合成向量和已有差异分类已具备，M0 完成。M1 尚未开始实现；没有提取准确率、契约通过数量或游戏可玩性提升结果。

M1 开始时先实现格式校验、合成向量驱动及三值报告，再接入反汇编提取。任何真实命令只有有限范围人工语义证据时，不把那份证据直接标为自动完整契约。

本轮已检查21个向量的JSON解析、唯一ID、预期标签和变更路径，以及文档本地链接的目标文件存在性，`git diff --check`通过。这是交付物完整性检查，不是提取器或比较器测试。规划提交前的 Rust fmt、离线测试、严格 Clippy 均通过；其后仅增加本节文档与合成数据，未修改 Rust 或工具实现。
