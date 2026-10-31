# M3：契约证明、运行闸门与旧推定迁移

2026-11-01。建立在 [M2 离线试点](command-contracts-m2.md) 上；当前只登记人工核对过的三个 x64 契约，未扩大游戏验证范围。

## 运行策略变更

默认允许 `Base`、`Inherited`、`Reviewed`、`ContractEquivalent` 进入已有 Rust 分派。旧 `Presumed` 不再默认放行，必须显式选择 `--allow-presumed`；`Unreviewed` 仍须独立的 `--allow-unreviewed`。两项研究策略互不隐含，`Absent`、`NoOp` 继续停止。runner 的 `set_allow_presumed`、trace 和桌面参数都委托同一个 `CommandGate`。

这项默认收紧同时记录在[运行指南](../guides/running.md)和[未发布变更](../release-notes.md)。

CHRONOCLOCK 全部976条档案行的[迁移清单](../games/chronoclock/contract-m3-migration.tsv) SHA-256 为 `f21fe7741e181ec8d6772d252d5f20f6145e6014409ff62c8a96f1efbd22afab`。仅三条旧推定升级：

| 命令 | 新来源 | 证明 ID |
|---|---|---|
| `0x2008` | `contract-equivalent` | `dfb2c06a890f52bf957fac9882e81ad7590f99c62024748bf63190ee0e3dc6f7` |
| `0x2009` | `contract-equivalent` | `16928ecfe979e512ef18dcc27189a26ab86084f52528de1583f925d483e12d57` |
| `0x2087` | `contract-equivalent` | `350d62c4348a8acd12def023e1076441c762b4521ce81d7950d797bf268dc9b6` |

其余455条继承、25条人工复核、164条旧推定、65条未复核和264条空操作保留来源。M2 的29条契约接受中，已有继承/人工复核的26条不改写。离线 `different` 不产生证明，`unknown` 不升级旧证据；目前真实构建报告的差异数为0，差异拒绝由合成反例与 M2 变体审计覆盖。

## 证明和信任边界

`profile/contract.rs` 定义独立的 `ContractProof`、`ContractSide` 和 `ContractRules`，不复用机器指纹或人工复核字段。档案行尾的 `contract=<SHA-256>` 引用内嵌 `profiles/contracts.txt` 中一行规范化 `proof v1 ...` 的哈希（不含末尾换行）。目录只存身份、哈希、依赖 ID 和完整性声明，不存原始指令。

每条证书保存同架构的两侧档案 ID、exe 哈希、image base、case RVA、完整 opcode、导出哈希、规范契约哈希、四项工具/规则依赖、映射/摘要依赖 ID 以及来源报告哈希。v1 只接受 x64、`cmvs-command-contract-v1`、v3 导出、`equivalent`、`coverage=closed`、`binary=checked`，并要求两侧契约哈希相同。

Registry 核对证书引用、字段格式、版本和活动规则；核对两侧二进制、架构、case 和 opcode 与档案一致，且直接对本家族根档案的 `Base` 命令比较。拒绝缺失、未知、撤销、过期或跨架构的证明，拒绝经派生档案传递契约。只有档案文本不能建立信任：`Registry::parse` 遇到契约引用会拒绝；调用方必须用 `parse_with_contracts` 提供明确受信任的目录和独立的活动规则。内置 Registry 只用随程序编译的登记数据，不从游戏目录加载证书。

哈希不是等价性的数学证明。证书目录和活动规则属于已复核的受信任输入；Rust 不重新反编译或执行 IR。登记工具重新提取并核对 exe 和报告，人工复核负责映射、模型与接受项范围。修改受信任目录的人仍能作出错误声明，文件自带的哈希不能消除该信任边界。

构建档案仍共用 x86/x64 的命令来源表，与既有机器证据方式一致；这些新增证书只记录所分析的 x64 构建对，不宣称 x86 契约也已提取或验证。若另一架构出现确定反证，须撤销对应构建证书并复核。

## 失效与撤销

活动规则由 `ContractRules::builtin()` 对实际编译输入重新计算：提取器四个 Python 源文件按 M1 的带名称/零分隔算法联合哈希；上下文、映射、摘要按完整文件字节哈希。`include_bytes!` 使编译器追踪这些依赖。任一依赖改变后，旧证书与活动规则不匹配，Registry 拒绝；必须重新提取、审计并登记新证书，不能只把旧证书的哈希改成新值。

发现反证时在目录追加 `revoked <证书ID>`，同时把引用该证书的档案行改回 `unreviewed` 并记录原因；保留撤销行供审计。若遗漏档案修改，Registry 会拒绝加载。删除证书同样使引用失效。已经构建的程序使用当次编译的规则与撤销数据，源文件编辑不会修改正在运行的二进制；发布撤销必须重建程序。

已撤销的证书可保留原行用于审计，其旧规则不会再授权命令，也不会阻碍退役规则；未撤销证书仍必须匹配活动规则。

当前目录 SHA-256 为 `633c30055ecb0cd5de886880952925320f5d6540e4f28db96a7653c05919669e`，来源是 [M2c 报告及审计](../games/chronoclock/contract-m2.md#global-pilot)；报告哈希仍为 `28ca8843e71903187d947211d8766cfed71002d62e8973e6213bf5cb85d88cce`。

## 生成与登记

新增 `tools/contract_register.py`，显式指定试点 opcode、两侧原 exe/导出/档案、三类规则文件和完整离线报告。工具重新提取所选命令并执行原 exe 绑定，要求结果逐字段重现报告的接受项；报告必须覆盖所有目标 handler，不含证据错误。拒绝覆盖已有继承或人工复核行。输出 JSON 内含目录、档案及完整迁移 TSV 候选，不直接写 Registry。

```powershell
python -I tools/contract_register.py BASE.json DERIVED.json `
  --context docs/games/chronoclock/contract-context.json `
  --mappings docs/games/chronoclock/contract-mappings.json `
  --summaries docs/games/chronoclock/contract-summaries.json `
  --base-profile crates/cmvs-core/profiles/cmvs-20160201.profile `
  --derived-profile crates/cmvs-core/profiles/cmvs-20150405.profile `
  --base-exe BASE.exe --derived-exe DERIVED.exe --report REPORT.json `
  0x2008 0x2009 0x2087 > local/candidate.json
```

将复核后的候选证书与对应档案行一起登记。旧 `profile_compare.py` 不认识契约来源，会拒绝覆盖它；它不负责刷新或降级契约证据。新登记不要求将真实输入、原始反汇编或大报告加入 Git。

## 统计与检查

`GateCounts.contract_equivalent` 记录已放行分派尝试，`contracted()` 记录唯一命令索引。等待重试、缺少处理器的尝试同样计数；不是完成数或覆盖率。`diagnostic_lines()` 输出各来源计数、契约和研究命令列表、两项研究策略。runner 暴露此 API；trace 在所有返回路径、桌面在待处理停点及事件循环退出后打印派生档案诊断。

合成检查覆盖证明往返、独立证书/旧指纹/复核字段、错误身份/架构/opcode/case、未知版本、过期四类规则、不完整/未绑定/不同结果、撤销、重复依赖、缺证据及重用证书。候选工具的两项测试涵盖原 PE 重新提取、输入不变、确定性输出，以及篡改/过期/缺行报告、错误 exe 和覆盖人工复核的拒绝。

CLI 合成脚本依次执行三条契约命令，再触发旧推定与未复核命令，验证两个研究参数独立。runner 合成证书刻意指向没有 Rust 处理器的命令，确认保持 PC、参数、待处理命令并按尝试计数；另测旧推定拒绝后可显式研究重试。

真实样本回归（原文件只读、无窗口）：

- 基准同参数启动 trace 在改动前后13,934字节逐字节相同，SHA-256 `d2185c24c7a30a64d2102d3dfa5e3c09c9e4fa0719546e1decd3f84340979bf2`。本次参数未提供存档目录，停于 `start.ps3 / 0x22b3`，只证明该启动子路径不变。
- CHRONOCLOCK 完整 runner 的新独立用户目录、种子0、16ms、记录文字、无输入、继续但不构建快照：严格策略在步骤24、`start.ps3 / 0x76 / 0x2092` 停为旧推定，未修改参数；仅增加 `--allow-presumed` 后重现既有步骤102340、`cdemo.ps3 / 0xf090 / 0x22b2` 的未复核停点。后者计数继承17179、人工复核36、旧推定2600、契约0、未复核0。

这条真实路径没有执行新增三条契约命令；三条的运行分派由合成集成测试覆盖，二进制契约正确性由 M2 审计支撑。没有新增原生桌面、画面、音轨、存读档或整部路线验证。

检查：默认842项测试、全 workspace 全功能926项测试通过（分别另有12/22项外部样本或硬件检查默认忽略）；四组 Python 工具测试共58项通过。格式、默认与全 workspace 全功能的严格 Clippy 通过。未修改 VM 命令实现或资源解码算法，没有新增依赖。M3 的运行接入与迁移退出条件已满足，后续实际路线验证属于 M4。
