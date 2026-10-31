# 未发布变更

## 2026-11-01：派生档案契约与默认闸门

新增独立 `ContractEquivalent` 证据来源，CHRONOCLOCK 的 `0x2008`、`0x2009`、`0x2087` 使用经离线提取、原 exe 绑定及人工审计的登记证明。

**行为变更：旧 `Presumed` 不再默认放行。** runner、trace、桌面统一停止这类命令；需要继续此前研究路线时，显式使用 `--allow-presumed`（库调用方用 `Startup::set_allow_presumed`）。它不隐含 `--allow-unreviewed`，后者也不会放行旧推定。不存在和空操作命令仍然停止。

证明过期、缺失或撤销时 Registry 拒绝加载；契约放行后的未实现处理器仍保持待处理状态。新增日志统计分派尝试，包含等待重试，不代表完成或覆盖率。游戏可玩性与已验证范围未升级。

迁移清单、证据格式、失效与撤销方法见 [M3](design/command-contracts-m3.md)，参数见[运行指南](guides/running.md)。
