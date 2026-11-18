# 命令契约比较

契约工具比较两侧命令在登记模型下的参数、栈回收、返回、路径条件和有序副作用。结果为 `equivalent`、`different` 或 `unknown`；证据无效单独报错。相同的未知项不能判等价，字段偏移只能通过有身份绑定的映射归一化。

## 提取与比较

[`contract_extract.py`](../../tools/contract_extract.py) 使用受限 x64 解码器和符号执行；IR 校验与规范化位于 [`command_contract.py`](../../tools/command_contract.py)。未支持指令、未证明映射、循环、递归和预算耗尽均保留未知，不跳过阻断。字段映射、上下文和 helper 摘要是受信任输入，须有独立证据。

入口 ABI 来自已核对的 x64 分派器：`rbx` 为 VM，`[rsp+0x20]` 保存旧 `rbx`，`[rsp+0x28]` 为返回地址。退出必须恢复栈与非易失寄存器；原 exe 绑定检查分派器形态、case、指令字节和分支目标。

```sh
python3 -I tools/contract_extract.py BASE.json DERIVED.json \
  --context docs/games/chronoclock/contract-context.json \
  --mappings docs/games/chronoclock/contract-mappings.json \
  --summaries docs/games/chronoclock/contract-summaries.json \
  --base-exe BASE.exe --derived-exe DERIVED.exe > local/report.json
```

省略 opcode 时报告全部目标 handler。真实 exe、导出和运行报告留在仓库外或 `local/`；试点证据见[构建对审计](../games/chronoclock/contract-m2.md#global-pilot)。

## 登记与失效

[`contract_register.py`](../../tools/contract_register.py) 接收上述导出、规则和 exe，并要求 `--base-profile`、`--derived-profile`、`--report` 及显式 opcode。工具重新提取并核对报告，输出证书、档案行和迁移 TSV 候选，不直接修改 Registry，也不覆盖已有继承或人工复核行。完整参数见 `--help`。

档案的 `contract=<SHA-256>` 引用 `profiles/contracts.txt` 中规范化 `proof v1` 行的哈希。证明绑定两侧身份、架构、case、opcode、导出、契约及工具/规则依赖；v1 要求 x64、完整覆盖、原二进制绑定、相同契约哈希，并直接对本家族根档案的 `Base` 命令比较。

`Registry::parse_with_contracts` 要求明确受信任的证书目录和活动规则；普通 `parse` 不接受契约引用。内置 Registry 只使用编译时登记数据。规则哈希按实际工具及上下文、映射、摘要文件重算，变更后旧证书失效，必须重新提取和复核。

发现反证时追加 `revoked <证书ID>`，同时将引用行改回 `unreviewed`；遗漏档案修改会使 Registry 拒绝加载。发布撤销需要重建程序。哈希绑定不代替模型正确性或人工复核，契约一致也不代表 Rust 实现正确或游戏已验证。

运行策略统一见[引擎档案](engine-profiles.md)，无需在契约文档重复维护。

## 检查

```sh
python3 -I tools/test_command_contract.py
python3 -I tools/test_contract_extract.py
python3 -I tools/test_contract_register.py
python3 -I tools/test_profile_compare.py
```

变更证明或 Registry 时还需[核心检查](../architecture.md#检查)。保留行为差异、未知依赖、错误身份、规则过期和撤销的拒绝用例。
