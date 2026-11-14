# 未发布变更

## Rust API 整理

- 运行时入口为 `runtime::Runtime`、`RuntimeEvent`，创建选项集中在 `RuntimeOptions`，保存策略使用 `SaveStorage`。`runner` 路径和 `Startup` / `StartupEvent` 名称仍可用。
- `Script.header` 改为只读的 `header()`；数据段使用 `data()`，保留旧 `unknown()` 别名。需要编辑头部时应重新编码并解码校验，不能直接修改已校验对象的长度。
- `Player::tick` 返回 `TickOutcome`，错误使用 `PlayerError`。Rust 调用方应匹配枚举；C 函数名、API 版本及状态码不变。
- Player 推荐使用 `set_key_state`、`set_pointer_position`、`set_paused` 和 `frame_serial`。运行时的 `advance_playback`、`set_decode_cmv_frames` 明确播放与 CMV 解码的职责，旧方法保留别名。
- 桌面的 `TextEncoding` 直接复用 core 类型。原来的 `.encode()` / `.decode()` / `TextEncoding::parse()` 改用 `cmvs_font::text_codec`（desktop 也重导出该模块）；无需再调用 `.engine_encoding()`。

## 当前命令闸门策略

当前默认放行 `Presumed` 和 `Unreviewed`，分别使用 `--block-presumed`、`--block-unreviewed` 显式阻止；两项互不隐含。`Absent`、`NoOp` 始终停止。默认放行不代表语义已复核、处理器完整或游戏可玩。 CLI 已移除旧 `--allow-presumed` / `--allow-unreviewed`；库调用方改用 `Startup::set_block_presumed` / `Startup::set_block_unreviewed`。

## 2026-11-01：派生档案契约与默认闸门（历史）

新增独立 `ContractEquivalent` 证据来源，CHRONOCLOCK 的 `0x2008`、`0x2009`、`0x2087` 使用经离线提取、原 exe 绑定及人工审计的登记证明。

**当时的行为变更：旧 `Presumed` 不再默认放行。** runner、trace、桌面统一停止这类命令；需要继续此前研究路线时，显式使用 `--allow-presumed`（库调用方用 `Startup::set_allow_presumed`）。它不隐含 `--allow-unreviewed`，后者也不会放行旧推定。不存在和空操作命令仍然停止。

证明过期、缺失或撤销时 Registry 拒绝加载；契约放行后的未实现处理器仍保持待处理状态。新增日志统计分派尝试，包含等待重试，不代表完成或覆盖率。游戏可玩性与已验证范围未升级。

迁移清单、证据格式、失效与撤销方法见 [M3](design/command-contracts-m3.md)，参数见[运行指南](guides/running.md)。
