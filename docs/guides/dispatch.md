# 系统命令分派简明指南

Runtime 与 trace 共用 [dispatch](../../crates/cmvs-core/src/dispatch.rs)。命令登记在 `ROOT` 与变体表；实际执行仍受档案闸门和宿主能力限制，不代表全部游戏语义已验证。

## 代码组织

`call`、`hosts`、`registry` 和 `handlers` 为私有实现，对外路径仍为 `dispatch::*`。

| 位置（相对 `cmvs-core/src`） | 职责 |
|---|---|
| `dispatch.rs` | 公开类型与导出、能力检查后的统一分发 |
| `dispatch/hosts.rs` | 服务集合、能力定义与可用性检查 |
| `dispatch/call.rs` | 参数记账、指令完成、延迟运行时请求 |
| `dispatch/registry.rs` | 命令索引、能力分类查询、变体选择 |
| `dispatch/registry/{root,cvns,variants}.rs` | 基准、CVNS 专有与变体登记表，只关联命令、能力和处理函数 |
| `dispatch/handlers/` | 按文本、输入、场景、媒体、存储、平台和基础服务组织的处理器 |
| `dispatch/ownership.rs` | runner/trace 的只读归属清单 |

## 调用路径

`CommandGate::resolve` 检查证据与运行策略，选择语义；`dispatch::run` 查找 ROOT 或指定 variant，再调用处理器。选中的 variant 不能退回 ROOT。宿主只执行语义，不判断游戏版本。

`Hosts` 显式提供所需能力；缺能力返回 `Unavailable`，不能当作成功完成。Runtime 负责运行时编排，trace 按工具支持范围提供能力。

后续游戏语义推进也遵循此边界：复用共享分派，差异放独立覆盖，宿主不增加版本判断。不继续扩展临时的档案ID/opcode分支；需要新的差异选择时先落实正式变体声明与证据绑定。研究放行不会自动升级证据状态。

构建专属处理器由档案行的 `variant=<实现>:<架构>:<exe SHA-256> evidence=<docs相对路径>` 声明，加载时校验身份及处理器存在。`variant` 选择实现，命令状态记录证据等级；它不是新的命令状态，也不代表与基准等价。当前绑定仍保留 `unreviewed`，受 `--block-unreviewed` 阻断；比较与契约登记工具不能静默将其改回根语义。绑定仅对声明它的 profile 生效，其他 profile 不自动继承。

## 新增或修改命令

1. 在 `handlers/` 对应领域实现处理器；在 `registry/root.rs`（或 CVNS 专有 `registry/cvns.rs`）登记 opcode、能力和处理器，语义差异登记在 `registry/variants.rs`。
2. 用 `Call::argument` 读取命名参数，忽略的参数用 `ignore_argument` 标记，保留原验证与提交顺序。
3. 用 `finish` 完成并核对回收量；等待用 `wait`，保留 pending 供后续重试。不要另加 `try_*` 试探链。
4. 更新归属与行为测试，覆盖缺能力、失败重试、参数回收及结果状态。

跨 VM 与多个宿主的操作使用 `RuntimeCall`：ROOT 返回请求时尚未完成指令；Runtime 先 `validate`，执行后以 `complete` 确认一次。RESET、成功加载槽等替换 VM 的路径由替换逻辑接管，不再确认旧指令。

## 验证

核心分派的单元与集成测试都在 `cmvs-core`。从仓库根目录运行：

```sh
cargo fmt --all --check
cargo test --offline -p cmvs-core
cargo clippy --offline -p cmvs-core --all-targets -- -D warnings
```

工作区检查见[开发检查](../architecture.md#检查)。涉及真实执行路径时补充对应游戏回归；档案格式见[引擎档案](../design/engine-profiles.md#复核与变体)。
