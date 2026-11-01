# 系统命令分派简明指南

runner 与 trace 共用 [dispatch](../../crates/cmvs-core/src/dispatch.rs)。已知 491 条命令均登记在 `ROOT`；实际执行仍受档案闸门和宿主能力限制，不代表全部游戏语义已验证。

## 调用路径

`CommandGate::resolve` 检查证据与运行策略，选择语义；`dispatch::run` 查找 ROOT 或指定研究覆盖，再调用处理器。选中的研究覆盖不能退回 ROOT。宿主只执行语义，不判断游戏版本。

`Hosts` 显式提供所需能力；缺能力返回 `Unavailable`，不能当作成功完成。runner 负责运行时编排，trace 按工具支持范围提供能力。

后续游戏语义推进也遵循此边界：复用共享分派，差异放独立覆盖，宿主不增加版本判断。不继续扩展临时的档案ID/opcode分支；需要新的差异选择时先落实正式变体声明与证据绑定。研究放行不会自动升级证据状态。

现有研究覆盖由档案行的 `research=<实现>:<架构>:<exe SHA-256> evidence=<docs相对路径>` 声明，加载时校验身份及处理器存在。它仍要求Unreviewed研究放行，不等于正式Variant默认放行；比较与契约登记工具不能静默将其改回根语义。

## 新增或修改命令

1. 在 ROOT 登记 opcode、能力和处理器；语义差异放入显式覆盖表。
2. 用 `Call::argument` 读取命名参数，忽略的参数用 `ignore_argument` 标记，保留原验证与提交顺序。
3. 用 `finish` 完成并核对回收量；等待用 `wait`，保留 pending 供后续重试。不要另加 `try_*` 试探链。
4. 更新归属与行为测试，覆盖缺能力、失败重试、参数回收及结果状态。

跨 VM 与多个宿主的操作使用 `RuntimeCall`：ROOT 返回请求时尚未完成指令；runner 先 `validate`，执行后以 `complete` 确认一次。RESET、成功加载槽等替换 VM 的路径由替换逻辑接管，不再确认旧指令。

## 验证

在仓库根目录运行；核心集成测试位于根包，仅测 core 不足以覆盖分派行为。

```sh
cargo fmt --all -- --check
cargo test --offline
cargo test --offline -p cmvs-core
cargo clippy --offline --all-targets -- -D warnings
cargo clippy --offline -p cmvs-core --all-targets -- -D warnings
```

涉及真实执行路径时补充对应游戏回归。记录层归位、表索引/根地址元数据及覆盖矩阵仍待架构验收；设计与迁移记录见[分派层设计](../design/dispatch.md)。
