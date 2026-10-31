# 契约 M1：首个构建对的离线报告

2026-10-31。工具与模型见[契约 M1](../../design/command-contracts-m1.md)。本页只记录离线报告，不修改[游戏档案](README.md)状态或运行放行。

## 输入与身份

| 项 | 基准 | 目标（本作） |
|---|---|---|
| 配置 | `cmvs-20160201` | `cmvs-20150405` |
| exe SHA-256 | `dc74d2001387ae1aefbdc4204216de5ebb9d3896009280720c4e7a91b42d84f5` | `d1c4ade9551b827b8e401fc10f68a73e2525a67a217b63747898f5797894911d` |
| v3 x64 导出 SHA-256 | `7d1f49d573c1ef912cf05e40c12c3a6dd136c6465c596f56784f140f50a7fdae` | `5a3cefbac5ff268e28bcce9cc17baf246051b42c53548a639734d8a876e9742a` |
| 分派器（从 exe 字节核对） | `0x1400823a0` | `0x14007ea60` |

规则文件：[上下文](contract-context.json) `4e1f9290…2b`、[字段映射](contract-mappings.json) `09638ffd…80`、[摘要](contract-summaries.json)（空）`993cd8ee…6d`。提取器版本 `30c98f1df4a747526e1377dcc2ea2e3de65a419ab7cf8fec689486ced94dc7b3`。两侧均提供原 exe，所有已执行指令和分支目标都通过字节绑定。报告保存在仓库外 `local/chronoclock/contract-m1-report.json`（SHA-256 `45102bfb…80`）。

## 结果

目标构建712条 handler：**4条 `equivalent`、708条 `unknown`，0条 `different`、0条证据错误。**

| opcode | 契约 | 当前档案 |
|---|---|---|
| `0x2004`、`0x2361`、`0x23b7` | 共用同一 case：无参数读取，回收4字节，返回 `0x4004` | `inherited` |
| `0x203f` | 读 `vm.main_image_manager`，写 `main_image_manager.next_read_cache_hint = 1`，回收0，返回 `0x4000` | `reviewed` |

`0x203f` 是 M0 指定的试点：基准写 +0x60d8、本作写 +0x58b8，经两条试点映射归一化为同一逻辑字段。提取结果与[人工复核](vm-semantics.md#0x203f下一次图像读取缓存提示)一致：无参数，只设置下一次读取提示。四条结论都没有与已有人工或继承结论冲突。

目标侧首个阻断原因（每条命令一个）：

| 原因 | 数量 | 主要来源 |
|---|---:|---|
| `unproven-mapping` | 395 | VM +0x3990（85）、+0x1230（59）、+0x7b0（55）、+0x2f18 错误位（34）、+0x1238（21）、+0x14380（18） |
| `unknown-memory` | 263 | 缩放的动态索引（对象/窗口表，229）、可写或未解析的全局数据（33）、image base 寻址（1） |
| `unsupported-instruction` | 44 | `movaps` 栈保存（29）、间接 `call`（7）、`imul`（4）、`cdq`（2）、`f3` 前缀（2） |
| `unresolved-call` | 4 | FLIRT 库函数 |
| `unsupported-effect` | 1 | 让出返回 |
| `unsupported-expression` | 1 | 变量位移 |

基准侧原因分布相同。数量只表示第一个阻断点，补上一个映射后命令通常还会遇到下一个阻断，不能把395条视为“只差映射”。

## 试点映射

| 映射 | 两侧位置 | 证据 |
|---|---|---|
| `vm.main-image-manager` | VM +0x1298（两侧相同），指向主图像管理器 | 写入：基准 `0x14001530b`/`0x140016bd3`，本作 `0x14001430b`/`0x140015bd3`；读取：`0x203f` case 与换图协调函数 |
| `image-manager.next-read-hint` | 基准 +0x60d8 / 本作 +0x58b8，32位 | 写1：提示函数及预加载路径；消费：原始读取函数在相同指令序号先读后清零，再决定是否查缓存 |

两条映射的 `review.status` 都是 `pilot`。`points_to` 假定管理器已成功初始化；分配失败时两边都存入失败结果，处理函数也都不检查，这一路径没有建模。

## 未做的事

没有改动档案、Registry 或闸门，未动态运行游戏。报告只覆盖 x64；x86 需要另行登记 ABI。将阻断原因转成 M2 的工作量估计前，还应按命令统计**全部**阻断点，而不只是第一个。
